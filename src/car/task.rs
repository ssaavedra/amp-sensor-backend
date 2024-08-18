use core::panic;
use std::{cmp::max, sync::Arc};

use rocket::tokio::sync::Mutex;
use serde::{Deserialize, Serialize};

use super::tessie_api::{ChargingState, TessieAPIHandler, TessieCarState};

#[derive(Debug)]
pub(super) struct TessieHandlerParams {
    pub vin: String,
    pub token: String,
    pub charger_location: LatLon,
    pub max_amps: f64,
    pub max_amps_car: usize,
}

impl TessieHandlerParams {
    pub fn from_figment(figment: &rocket::figment::Figment) -> Self {
        let vin = figment
            .extract_inner("car_vin")
            .unwrap_or_else(|_| panic!("Missing VIN"));
        let token = figment
            .extract_inner("tessie_token")
            .unwrap_or_else(|_| panic!("Missing token"));
        let charger_location_str: String = figment
            .extract_inner("charger_location")
            .unwrap_or_else(|_| panic!("Missing charger location"));
        let charger_location = LatLon::try_from(charger_location_str)
            .unwrap_or_else(|_| panic!("Invalid charger location"));
        let max_amps = figment
            .extract_inner("max_amps")
            .unwrap_or_else(|_| panic!("Missing max amps"));
        let max_amps_car = figment
            .extract_inner("max_amps_car")
            .unwrap_or_else(|_| panic!("Missing max amps car"));
        Self {
            vin,
            token,
            charger_location,
            max_amps,
            max_amps_car,
        }
    }
}


#[derive(Debug, Clone)]
struct TessieCarStateWrapper {
    state: TessieCarState,
    last_update: i64,
    last_amps_requested: usize,
    last_amps_requested_time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

// Allow String -> LatLon using "lat,lon" format
impl TryFrom<String> for LatLon {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let parts: Vec<&str> = value.split(',').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid format"));
        }
        let lat = parts[0].parse::<f64>()?;
        let lon = parts[1].parse::<f64>()?;
        Ok(Self { lat, lon })
    }
}

#[derive(Debug, Clone)]
pub struct HomeState {
    /// Average amps drawn by the home (including the car) over the last 30 seconds
    pub avg_amps: f64,

    /// Maximum amps drawn by the home (including the car) over the last 30 seconds
    pub max_amps: f64,

    /// Amps drawn by the car over the last 30 seconds
    pub car_amps: f64,

    /// Timestamp of the measurement
    pub timestamp: i64,
}

pub struct HomeStateWrapper {
    state: Vec<HomeState>,
}

pub struct TessieCarHandler {
    // API to control the car
    api: TessieAPIHandler,
    charger_location: LatLon,
    max_amps: f64,
    max_amps_car: usize,
    last_state: Arc<Mutex<Option<TessieCarStateWrapper>>>,
    home_state: Arc<Mutex<HomeStateWrapper>>,
}

// Impl LatLon distance calculation using Haversine formula
impl LatLon {
    // Returns the distance in kilometers between two LatLon points in Earth
    pub fn distance(&self, other: &LatLon) -> f64 {
        const EARTH_RADIUS: f64 = 6371.0;
        let d_lat = (other.lat - self.lat).to_radians();
        let d_lon = (other.lon - self.lon).to_radians();
        let lat1 = self.lat.to_radians();
        let lat2 = other.lat.to_radians();

        let a = (d_lat / 2.0).sin() * (d_lat / 2.0).sin()
            + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin() * (d_lon / 2.0).sin();

        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
        // 6371 is the Earth radius in kilometers
        EARTH_RADIUS * c
    }
}

impl TessieCarHandler {
    pub fn from_figment(figment: &rocket::figment::Figment) -> Self {
        let params = TessieHandlerParams::from_figment(figment);
        Self::new(
            params.vin,
            params.token,
            params.charger_location,
            params.max_amps,
            params.max_amps_car,
        )
    }

    pub fn new(
        vin: String,
        token: String,
        charger_location: LatLon,
        max_amps: f64,
        max_amps_car: usize,
    ) -> Self {
        let api = TessieAPIHandler::new(vin, token);
        Self {
            api,
            charger_location,
            max_amps,
            max_amps_car,
            last_state: Arc::new(Mutex::new(None)),
            home_state: Arc::new(Mutex::new(HomeStateWrapper {
                state: Vec::new(),
            })),
        }
    }

    async fn force_update_state_cache(&self) -> anyhow::Result<TessieCarState> {
        let (mut last_amps_requested, mut last_amps_requested_time) = self
            .last_state
            .lock()
            .await
            .as_ref()
            .map(|x| (x.last_amps_requested, x.last_amps_requested_time))
            .unwrap_or((0, 0));
        let state = self.api.get_state().await?;
        log::info!("Tessie: Updated state cache {:?}", state);
        let mut guard = self.last_state.lock().await;

        // Check if somebody outside of this function has requested a different charge
        if last_amps_requested != state.charge_state.charge_current_request {
            last_amps_requested = state.charge_state.charge_current_request;
            last_amps_requested_time = chrono::Utc::now().timestamp() - 30; // Allow immediate update if required
            log::info!("Tessie: External Amps change: last requested {}A", last_amps_requested);
        }

        guard.replace(TessieCarStateWrapper {
            state: state.clone(),
            last_update: chrono::Utc::now().timestamp(),
            last_amps_requested,
            last_amps_requested_time,
        });
        Ok(state)
    }

    pub async fn invalidate_state_cache(&self) {
        if let Some(state) = self.last_state.lock().await.as_mut() {
            state.last_update = 0;
        }
        
    }

    pub async fn get_state(&self) -> anyhow::Result<TessieCarState> {
        // Check if the state is already cached
        // if so, return the cached state unless force=true or the state is older than 30 secs
        if let Some(state) = self.last_state.lock().await.as_ref() {
            if state.last_update > (chrono::Utc::now().timestamp() - 30) {
                return Ok(state.state.clone())
            }
        }
        // Fetch the state from the car API
        self.force_update_state_cache().await
    }

    pub async fn get_car_distance_to_charger(&self) -> anyhow::Result<f64> {
        let state = self.get_state().await?;
        let car_location = LatLon::from(state.drive_state);
        Ok(car_location.distance(&self.charger_location))
    }
    pub async fn is_car_nearby(&self) -> anyhow::Result<bool> {
        let distance = self.get_car_distance_to_charger().await?;
        Ok(distance < 0.1)
    }

    pub async fn get_charging_status(&self) -> ChargingState {
        let charging_state = self.get_state()
            .await
            .map(|state| state.charge_state.charging_state)
            .unwrap_or(ChargingState::Starting);

        if charging_state == ChargingState::Starting || charging_state == ChargingState::Pending {
            // If in the middle of a transition, don't wait for the cache to update
            self.invalidate_state_cache().await;
        };
        charging_state
    }

    pub async fn get_amps(&self) -> f64 {
        self.get_state()
            .await
            .map(|s| s.charge_state.charger_actual_current)
            .unwrap_or(0.0)
    }

    pub async fn set_amps(&self, amps: usize) -> anyhow::Result<()> {
        let result = self.api.set_charging_amps(amps).await?;
        log::info!("Set amps to {} result: {:?}", amps, result);
        Ok(())
    }

    pub async fn set_current_home_consumption(&self, avg_amps: f64, max_amps: f64) -> Result<(), reqwest::Error> {
        let mut guard = self.home_state.lock().await;
        let car_amps = self.get_amps().await;
        guard.state.push(HomeState {
            car_amps,
            avg_amps,
            max_amps,
            timestamp: chrono::Utc::now().timestamp(),
        });
        // Ensure we only keep 10 entries
        while guard.state.len() > 10 {
            guard.state.remove(0);
        }
        Ok(())
    }

    pub async fn throttled_calculate_amps(&self) -> anyhow::Result<()> {
        // Only change amps if they are *less* or at least 30 seconds have passed since the last change
        let (last_amps_requested, last_amps_requested_time) = self
            .last_state
            .lock()
            .await
            .as_ref()
            .map(|x| (x.last_amps_requested, x.last_amps_requested_time))
            .unwrap_or((0, 0));

        // Calculate the average amps over the last 30 seconds
        let now = chrono::Utc::now().timestamp();

        let home_amps_without_car = {
            let guard = self.home_state.lock().await;
            let state = guard.state.last().unwrap();
            log::info!("Home states: {:?}", guard.state);
            log::info!("Home amps without car: {} (avg home={}, car={})", state.avg_amps - state.car_amps, state.avg_amps, state.car_amps);

            if state.avg_amps - state.car_amps < 0.0 {
                0.0
            } else {
                state.avg_amps - state.car_amps
            }
        };


        let amps_to_request = max(
            0,
            ((self.max_amps - home_amps_without_car) * 0.95
        ) as usize,
        );

        // If amps to request are equal to the last requested amps, do nothing
        if amps_to_request == last_amps_requested {
            log::info!("Skipping request car charge to {}A, equal to last request {} seconds ago.", amps_to_request, now - last_amps_requested_time);
            return Ok(());
        }

        // If we are diminishing the amps, do this immediately
        // Otherwise, ask the API only every 30 seconds at most
        if amps_to_request < last_amps_requested
            || last_amps_requested_time < now - 30
        {
            let mut guard = self.last_state.lock().await;
            guard.as_mut().map(|x| {
                x.last_amps_requested = amps_to_request;
                x.last_amps_requested_time = now;
            });
            log::info!("Requesting car charge to {}A", amps_to_request);
            self.set_amps(amps_to_request).await?;
        } else {
            log::info!("Skipping request car charge to {}A. We requested {}A {} seconds ago.", amps_to_request, last_amps_requested, now - last_amps_requested_time);
        }

        Ok(())
    }
}
