//! Task to handle the car API and the home consumption
//!
//! This module contains the main implementation for handling EV charging
//! according to a budget based on the home consumption and the maximum
//! available power according to a figment configuration.
//!
//! Although we only have one implementation for the [EVChargeHandler] at this
//! moment (the [super::tessie::Handler]), we could implement other EV APIs in
//! the future.
//!
//! The Tessie handler works for Tesla EVs enrolled in the Tessie API, but a
//! future implementation could be done to support other EV platforms or IoT
//! devices, by creating a trait over the EV API and implementing it for each
//! platform.
//!
//! If you want to implement an additional platform, head over to the
//! [EVChargeHandler] trait documentation to get started.

use std::{
    cmp::{max, min},
    sync::Arc,
};

use rocket::{figment::Figment, tokio::sync::Mutex};

use crate::car::EVChargeInternalState;

use super::{EVChargeHandler, LatLon};

/// A simple struct to store the car state and the last update time
///
/// This struct is used to store the car state and the last update time to avoid
/// querying the car API too often.
///
/// The last_amps_requested and last_amps_requested_time are used to store the
/// last requested amps to the car and the time of the request. This is used to
/// avoid requesting the car to change the amps too often.
///
/// If the last_amp_requested is different from the current charge state
/// according to the car API, we will adjust the last_amps_requested to the
/// value retrieved from the car API, and the time to the current time minus 30
/// seconds to allow an immediate update.
#[derive(Debug, Clone)]
struct CarStateWrapper<ActualState> {
    state: ActualState,
    last_update: i64,
    last_amps_requested: usize,
    last_amps_requested_time: i64,
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

/// A store for the home state
///
/// This is used to calculate the power budget for the car to charge.
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

/// A simple cache to store the last home states to log them.
pub struct HomeStateWrapper {
    state: Vec<HomeState>,
}

/// The shared configuration for the car handler independent of the API
/// implementation
struct CarHandlerConfig {
    charger_location: LatLon,
    max_amps: f64,
    max_amps_car: usize,
}

/// The main struct to handle information about the car.
///
/// Separating this from the fairing allows configuring the handler from the
/// figment configuration in the Rocket.
///
/// Separating this from the API handler allows us to have a common
/// implementation for caching the state, calculating the power budget for the
/// car to charge, and any other future functionality that we may want to
/// implement that can be independent of the actual API implementation for each
/// EV platform.
pub struct CarHandler<H: EVChargeHandler> {
    inner: H,
    config: CarHandlerConfig,
    last_state: Arc<Mutex<Option<CarStateWrapper<H::InternalState>>>>,
    home_state: Arc<Mutex<HomeStateWrapper>>,
}

impl<H: EVChargeHandler> From<&Figment> for CarHandler<H> {
    fn from(figment: &Figment) -> Self {
        let params: H::ConfigParams = figment.into();
        let api = H::new(params);
        let config = {
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
            CarHandlerConfig {
                charger_location,
                max_amps,
                max_amps_car,
            }
        };

        Self {
            inner: api,
            config,
            last_state: Arc::new(Mutex::new(None)),
            home_state: Arc::new(Mutex::new(HomeStateWrapper { state: Vec::new() })),
        }
    }
}

impl<H: EVChargeHandler> CarHandler<H> {
    /// Retrieves the state from the car API, and updates the cache
    ///
    /// This function is used to force an update of the state cache from the car
    /// API. It will update the cache and return the new state. This should only
    /// be used when we cannot rely on the cache (e.g., when we have just
    /// updated the charge amps), or when we expect the cache to update sooner
    /// than usual (e.g., when the charging state is starting
    /// [EVChargeInternalState::is_charge_starting]).
    ///
    /// This function will also update the last_amps_requested and
    /// last_amps_requested_time if the last requested amps are different from
    /// the current charge state according to the car API.
    async fn force_update_state_cache(&self) -> anyhow::Result<H::InternalState> {
        let (mut last_amps_requested, mut last_amps_requested_time) = self
            .last_state
            .lock()
            .await
            .as_ref()
            .map(|x| (x.last_amps_requested, x.last_amps_requested_time))
            .unwrap_or((0, 0));
        let state = self.inner.get_state().await?;
        log::info!("EV: Updated state cache {:?}", state);
        let mut guard = self.last_state.lock().await;

        // Check if somebody outside of this function has requested a different charge
        let last_requested_amps_according_to_api = state.get_last_requested_amps();
        if last_amps_requested != last_requested_amps_according_to_api {
            last_amps_requested = last_requested_amps_according_to_api;
            last_amps_requested_time = chrono::Utc::now().timestamp() - 30; // Allow immediate update if required
            log::info!(
                "EV: External Amps change: last requested {}A",
                last_amps_requested
            );
        }

        guard.replace(CarStateWrapper {
            state: state.clone(),
            last_update: chrono::Utc::now().timestamp(),
            last_amps_requested,
            last_amps_requested_time,
        });
        Ok(state)
    }

    /// Forces the next [CarHandler::get_state] call to retrieve the state from
    /// the car API, without performing the call immediately.
    pub async fn invalidate_state_cache(&self) {
        if let Some(state) = self.last_state.lock().await.as_mut() {
            state.last_update = 0;
        }
    }

    /// Wrapper to get the state from the car API, using the cache if possible
    pub async fn get_state(&self) -> anyhow::Result<H::InternalState> {
        // Check if the state is already cached
        // if so, return the cached state unless force=true or the state is older than 30 secs
        if let Some(state) = self.last_state.lock().await.as_ref() {
            if state.last_update > (chrono::Utc::now().timestamp() - 30) {
                return Ok(state.state.clone());
            }
        }
        // Fetch the state from the car API
        self.force_update_state_cache().await
    }

    /// Get the distance from the car to the charger, as configured from the
    /// figment.
    ///
    /// Uses the [LatLon::distance] method to calculate the distance in
    /// kilometers between the car and the charger.
    pub async fn get_car_distance_to_charger(&self) -> anyhow::Result<f64> {
        let state = self.get_state().await?;
        Ok(state.get_car_distance_to_point_km(&self.config.charger_location))
    }

    /// Uses [CarHandler::get_car_distance_to_charger] to check if the car
    /// is nearby, returning true if the distance is less than 0.1km.
    pub async fn is_car_nearby(&self) -> anyhow::Result<bool> {
        let distance = self.get_car_distance_to_charger().await?;
        Ok(distance < 0.1)
    }

    pub async fn is_car_charging(&self) -> anyhow::Result<bool> {
        let state = self.get_state().await?;

        if state.is_charge_starting() {
            // Invalidate the cache for the next call
            self.invalidate_state_cache().await;
        }
        Ok(state.is_charging())
    }

    /// Get the current amps drawn by the car
    pub async fn get_amps(&self) -> f64 {
        self.get_state()
            .await
            .map(|s| s.get_current_charge())
            .unwrap_or(0.0)
    }

    /// Set the charging amps to the car
    pub async fn set_amps(&self, amps: usize) -> anyhow::Result<()> {
        self.inner.request_charge_amps(amps).await
    }

    /// Set the current home consumption to the cache
    ///
    /// This function is used to be able to calculate the power budget remaining
    /// for the car to charge. It will store the current home consumption in the
    /// cache, and keep the last 10 entries.
    pub async fn set_current_home_consumption(
        &self,
        avg_amps: f64,
        max_amps: f64,
    ) -> Result<(), reqwest::Error> {
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

    /// Calculate the amps to request to the car API, and request the change if
    /// necessary
    ///
    /// This function will calculate the average amps drawn by the home over the
    /// last 30 seconds, and request the car to charge accordingly. It will
    /// request the car to charge to the maximum of the configured max_amps_car
    /// and the remaining budget after the home consumption.
    ///
    /// The function will only request the car to change the amps if the last
    /// request was higher (because this means we are immediately over-budget),
    /// or at least 30 seconds have passed since the last request.
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
            log::info!(
                "Home amps without car: {} (avg home={}, car={})",
                state.avg_amps - state.car_amps,
                state.avg_amps,
                state.car_amps
            );

            if state.avg_amps - state.car_amps < 0.0 {
                0.0
            } else {
                state.avg_amps - state.car_amps
            }
        };

        let amps_to_request = min(
            self.config.max_amps_car,
            max(
                0,
                ((self.config.max_amps - home_amps_without_car) * 0.95) as usize,
            ),
        );

        // If amps to request are equal to the last requested amps, do nothing
        if amps_to_request == last_amps_requested {
            log::info!(
                "Skipping request car charge to {}A, equal to last request {} seconds ago.",
                amps_to_request,
                now - last_amps_requested_time
            );
            return Ok(());
        }

        // If we are diminishing the amps, do this immediately
        // Otherwise, ask the API only every 30 seconds at most
        if amps_to_request < last_amps_requested || last_amps_requested_time < now - 30 {
            let mut guard = self.last_state.lock().await;
            guard.as_mut().map(|x| {
                x.last_amps_requested = amps_to_request;
                x.last_amps_requested_time = now;
            });
            log::info!("Requesting car charge to {}A", amps_to_request);
            self.set_amps(amps_to_request).await?;
        } else {
            log::info!(
                "Skipping request car charge to {}A. We requested {}A {} seconds ago.",
                amps_to_request,
                last_amps_requested,
                now - last_amps_requested_time
            );
        }

        Ok(())
    }
}
