//! This module contains the EV-energy related modules.
//! 
//! This module has been designed to be extensible to support multiple EV
//! platforms, and to be able to interact with them in a similar way.
//! 
//! Currently only an implementation for Tesla EVs relying on the 3rd party
//! Tessie API is available. It is available in the [tessie] sub-module.
//! 
//! If you want to implement your own EV charge handler, you should implement
//! the [EVChargeHandler] and [EVChargeInternalState] traits in this module. You
//! can look at the [tessie] source code for an example implementation.

use serde::{Deserialize, Serialize};

pub mod fairing;
pub mod tessie;
pub mod task;

/// The internal state of the EV charge handler.
/// 
/// Implementing this trait for your own EV charge handler will allow the
/// [EVChargeFairing](fairing::EVChargeFairing) to interact with it.
pub trait EVChargeInternalState: std::fmt::Debug + Clone {
    /// Returns true if the car is currently charging
    fn is_charging(&self) -> bool;

    /// Returns true if the car is charging or about to start charging.
    ///
    /// This is used to check if we need to quickly check again the state, for
    /// example when the amp reading is not yet useful because you can reliably
    /// know it is still ramping up.
    fn is_charge_starting(&self) -> bool;

    /// Returns the current amps being drawn by the car
    fn get_current_charge(&self) -> f64;

    /// Returns the max amps that we requested the charge to use
    fn get_last_requested_amps(&self) -> usize;

    /// Returns the distance in kilometers between the car and a point
    fn get_car_distance_to_point_km(&self, point: &LatLon) -> f64;
}

pub trait EVChargeHandler {
    type ConfigParams: for<'a> From<&'a rocket::figment::Figment>;
    type InternalState: EVChargeInternalState;

    /// Get the name of the EV charge handler
    /// 
    /// This is just a fancy name to return the name of the handler. It is used
    /// when displaying the full fairing name.
    /// 
    /// If you don't override this method, the default implementation will return
    /// the full type name of the handler.
    fn get_name() -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Create a new instance of the EV charge handler
    /// 
    /// This method should initialize the handler with the given configuration
    /// parameters.
    /// 
    /// The configuration parameters should be extractable from the Rocket.toml
    /// file, so the implementation for the [EVChargeHandler::ConfigParams] must
    /// implement the `From<&'a rocket::figment::Figment>` trait.
    fn new(config: Self::ConfigParams) -> Self;

    /// Get the current state of the EV
    /// 
    /// This method should return the current state of the EV as reported by the
    /// API.
    /// Keep in mind that you have to ask the API enough information to be able
    /// to implement the [EVChargeInternalState] trait.
    /// 
    /// We will do our best to cache the state for you, and to avoid calling this
    /// method too often, but if you need a specific rate limit, you should
    /// implement it in your own handler; or make a PR :-)
    fn get_state(&self) -> impl std::future::Future<Output = anyhow::Result<Self::InternalState>> + std::marker::Send;

    /// Request the car to charge with a specific amount of amps
    fn request_charge_amps(&self, amps: usize) -> impl std::future::Future<Output = anyhow::Result<()>> + std::marker::Send;
}


/// A simple struct to store latitude and longitude
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}

impl LatLon {
    /// Returns the distance in kilometers between two LatLon points in Earth
    /// 
    /// This method uses the Haversine formula to calculate the distance between
    /// two points on Earth.
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
