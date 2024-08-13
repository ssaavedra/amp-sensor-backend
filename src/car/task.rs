use core::panic;
use std::{cmp::max, sync::Arc};

use rocket::tokio::sync::Mutex;
use serde::{Deserialize, Serialize};

use super::tessie_api::{ChargingState, TessieAPIHandler, TessieCarState};

#[derive(Debug)]
pub(super) struct MainTaskParams {
    pub vin: String,
    pub token: String,
    pub charger_location: LatLon,
    pub max_amps: f64,
    pub max_amps_car: usize,
}

impl MainTaskParams {
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

// Controls two different tasks:
// 1. Check if the car is nearby. This task will return when it needs to be rescheduled depending on the distance.
// 2. Check if the car is charging. This task will be scheduled every minute while the car is nearby.
// 3. If the car is charging, check the amps drawn by the home from the database and update the car API accordingly to not exceed the amp limit.
pub(super) async fn main_task(params: MainTaskParams) {
    log::info!("Tessie: Starting main task");
    // Initialize the car handler
    let handler = TessieCarHandler::new(
        params.vin.clone(),
        params.token.clone(),
        params.charger_location.clone(),
        params.max_amps,
        params.max_amps_car,
    );

    loop {
        loop {
            // Check if the car is nearby
            // If the car is nearby, break the loop
            // If the car is not nearby, sleep for 60 seconds
            if handler.is_car_nearby().await.unwrap_or(false) {
                break;
            } else {
                // Check distance
                let distance_km = handler.get_car_distance_to_charger().await.expect("Distance to charger");
                let max_speed = 150.0; // something above the plausible mean speed in km/h
                let time_seconds = (distance_km / max_speed * 3600.0) as i64;
                let last_update = handler
                    .last_state
                    .lock()
                    .await
                    .as_ref()
                    .map(|x| x.last_update)
                    .unwrap_or(0);
                let min_time_to_sleep = max(0, last_update + 30 - chrono::Utc::now().timestamp());
                let time_to_sleep = max(time_seconds, min_time_to_sleep);
                log::info!(
                    "Car is {} km away. Sleeping for {} seconds",
                    distance_km,
                    time_to_sleep
                );
                rocket::tokio::time::sleep(std::time::Duration::from_secs(time_to_sleep as u64))
                    .await;
            }
        }
        log::info!("Tessie: Car is nearby");

        // The car is nearby
        // Check if the car is charging
        // If the car is charging, check the amps drawn by the home from the database
        // If the amps are higher than the limit, reduce the amps requested by the car
        // If the amps are lower than the limit, increase the amps requested by the car
        // Sleep for 60 seconds
        loop {
            if handler.get_charging_status().await {
                let car_amps = handler.get_amps().await as usize;

                // Get sliding average of the measurements of the last 15 seconds
                let house_amps = 10.0; // TODO: Fetch house amps from the database
                if car_amps > handler.max_amps_car {
                    handler.set_amps(handler.max_amps_car).await.unwrap();
                } else if house_amps < handler.max_amps {
                    let house_amps_without_car = house_amps - (car_amps as f64);
                    let amps_to_request = max(
                        0,
                        ((handler.max_amps - house_amps_without_car) * 0.9) as usize,
                    );
                    handler.set_amps(amps_to_request).await.unwrap();
                }
            } else {
                log::info!("Tessie: Car is not charging");
            }
            rocket::tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    }
}

#[derive(Debug, Clone)]
struct TessieCarStateWrapper {
    state: TessieCarState,
    last_update: i64,
    last_amps_requested: f64,
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

struct TessieCarHandler {
    // API to control the car
    api: TessieAPIHandler,
    charger_location: LatLon,
    max_amps: f64,
    max_amps_car: usize,
    last_state: Arc<Mutex<Option<TessieCarStateWrapper>>>,
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
    fn new(
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
        }
    }

    async fn force_update_state_cache(&self) -> anyhow::Result<TessieCarState> {
        let (last_amps_requested, last_amps_requested_time) = self
            .last_state
            .lock()
            .await
            .as_ref()
            .map(|x| (x.last_amps_requested, x.last_amps_requested_time))
            .unwrap_or((0.0, 0));
        let state = self.api.get_state().await?;
        log::info!("Tessie: Updated state cache {:?}", state);
        let mut guard = self.last_state.lock().await;
        guard.replace(TessieCarStateWrapper {
            state: state.clone(),
            last_update: chrono::Utc::now().timestamp(),
            last_amps_requested,
            last_amps_requested_time,
        });
        Ok(state)
    }

    async fn get_state(&self) -> anyhow::Result<TessieCarState> {
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

    async fn get_car_distance_to_charger(&self) -> anyhow::Result<f64> {
        let state = self.get_state().await?;
        let car_location = LatLon::from(state.drive_state);
        Ok(car_location.distance(&self.charger_location))
    }
    async fn is_car_nearby(&self) -> anyhow::Result<bool> {
        let distance = self.get_car_distance_to_charger().await?;
        Ok(distance < 0.1)
    }

    async fn get_charging_status(&self) -> bool {
        self.get_state()
            .await
            .map(|state| state.charge_state.charging_state == ChargingState::Charging)
            .unwrap_or(false)
    }

    async fn get_amps(&self) -> f64 {
        self.get_state()
            .await
            .map(|s| s.charge_state.charger_actual_current)
            .unwrap_or(0.0)
    }

    async fn set_amps(&self, amps: usize) -> Result<(), reqwest::Error> {
        self.api.set_charging_amps(amps).await?;
        Ok(())
    }
}
