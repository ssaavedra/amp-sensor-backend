//! Tessie implementation of the [EVChargeHandler] trait.
//! 
//! This module contains an implementation using [Tessie][tessie-web]. It uses the
//! [TessieAPIHandler] to interact with the Tessie API.
//! 
//! This is also a sample module to show how to implement the `EVChargeHandler` trait
//! for a specific EV platform. You can use this as a reference to implement your own
//! handling of another EV platform.
//! 
//! 
//! The benefit of using the Tessie API is that we abstract the complexity of
//! refreshing the Tesla OAuth Tokens, and the awake/asleep state of the EV
//! itself. Tessie will cache the car state and we will avoid waking up the car
//! unless it's already plugged-in, charging (and thus, awake).
//! 
//! This is also a more friendly interaction to end users since Tessie will
//! provide any subscriber with an API token, while the process to set up a
//! Tesla integration requires the user to register for a Tesla API token and
//! manual review from Tesla.
//! 
//! For example, the [TessieDriveState](api::TessieDriveState) struct is
//! only minimally implemented, and the
//! [TessieAPIHandler] struct only implements the
//! [get_state](api::TessieAPIHandler::get_state) and the
//! [set_charging_amps](api::TessieAPIHandler::set_charging_amps)
//! methods, as they are the only ones needed for the current use case.
//! 
//! [tessie-web]: https://developer.tessie.com/docs/about/
use std::sync::Arc;

use api::{ChargingState, TessieAPIHandler, TessieCarState};
use rocket::tokio::sync::Mutex;

use super::{EVChargeHandler, EVChargeInternalState};

pub mod api;


/// The handler for the Tessie API.
pub struct Handler {
    api: TessieAPIHandler,
    state: Arc<Mutex<Option<TessieCarState>>>,
}

impl EVChargeHandler for Handler {
    /// In this implementation, the `ConfigParams` can be the same as
    /// `TessieAPIHandler`, as the only initialization needed for the API
    /// Handler are basically the configuration params (the VIN and the API
    /// key).
    /// 
    /// This could be different for other implementations.
    type ConfigParams = TessieAPIHandler; // Note for other implementations: this could be separate from the Handler::api field
    
    /// The internal state of the API and EV platform.
    /// 
    /// We use a subset of what the Tessie API provides, as we only need parts
    /// of the charging state and the drive state.
    /// 
    /// We provide more fields than we actually need, to allow for future
    /// expansion, and to be able to show more of the state in log files for
    /// human consumption.
    type InternalState = TessieCarState;

    fn get_name() -> &'static str {
        "Tessie"
    }

    fn new(config: Self::ConfigParams) -> Self {
        Self {
            api: config,
            state: Arc::new(Mutex::new(None)),
        }
    }

    async fn get_state(&self) -> anyhow::Result<Self::InternalState> {
        let new_state = self.api.get_state().await?;
        let mut state = self.state.lock().await;
        state.replace(new_state.clone());

        Ok(new_state)
    }
    async fn request_charge_amps(&self, amps: usize) -> anyhow::Result<()> {
        let result = self.api.set_charging_amps(amps).await;
        log::info!("Setting charging amps to {}A: {:?}", amps, result);
        Ok(())
    }
}

impl EVChargeInternalState for TessieCarState {

    fn get_car_distance_to_point_km(&self, point: &super::LatLon) -> f64 {
        let car_position = {
                let api::TessieDriveState {
                    longitude,
                    latitude,
                    ..
                } = self.drive_state;
                super::LatLon {
                    lat: latitude,
                    lon: longitude,
                }
        };

        car_position.distance(&point)
    }

    #[inline(always)]
    fn is_charging(&self) -> bool {
        let charging_state = self.charge_state.charging_state;
                charging_state == ChargingState::Charging
                    || charging_state == ChargingState::Starting
                    || charging_state == ChargingState::Pending
    }

    #[inline(always)]
    fn is_charge_starting(&self) -> bool {
        self.charge_state.charging_state == ChargingState::Starting || self.charge_state.charging_state == ChargingState::Pending
    }

    #[inline(always)]
    fn get_current_charge(&self) -> f64 {
        self.charge_state.charge_amps
    }

    #[inline(always)]
    fn get_last_requested_amps(&self) -> usize {
        self.charge_state.charge_current_request
    }

}
