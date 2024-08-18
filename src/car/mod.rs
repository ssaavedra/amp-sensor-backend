//! This module contains the EV-energy related modules.
//! 
//! Currently only an implementation for Tesla EVs relying on the 3rd party
//! Tessie API is available. The fairing implementation to interact with the
//! Rocket app is in the [TessieFairing](tessie_fairing::TessieFairing) fairing.
//! 
//! The API interaction is implemented in the [tessie_api] module,
//! see more about the API documentation at
//! [Tessie](https://developer.tessie.com/docs/about/). Only a tiny subset of
//! the API is implemented in this module.
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
//! For example, the [TessieDriveState](tessie_api::TessieDriveState) struct is
//! only minimally implemented, and the
//! [TessieAPIHandler](tessie_api::TessieAPIHandler) struct only implements the
//! [get_state](tessie_api::TessieAPIHandler::get_state) and the
//! [set_charging_amps](tessie_api::TessieAPIHandler::set_charging_amps)
//! methods, as they are the only ones needed for the current use case.

pub mod tessie_fairing;
mod tessie_api;
mod task;
