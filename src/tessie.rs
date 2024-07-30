use std::sync::{atomic::AtomicBool, Arc};

use rocket::{futures::lock::Mutex, tokio};

const TESSIE_API_TOKEN: &str = "1234";


#[derive(Clone, Debug)]
struct ChargingAmpsSet {
    reported_to_us: f64,
    set_by_us: f64,
}

impl ChargingAmpsSet {
    fn new(reported_to_us: f64, set_by_us: f64) -> Self {
        Self {reported_to_us, set_by_us }
    }
}


#[derive(Debug)]
#[allow(dead_code)]
struct TessieApiHandlerState {
    current_saved_location_name: String,
    expected_location_name: String,

    cached_car_state: Option<GetVehicleStateResponse>,
    
    // We get pairs of (Actual Amps Measured, Amps Set in the Car) to avoid
    // jerking the car's charging rate through the API, but ensuring that
    // meaningful deviations are taken into account.
    last_charging_amps_set: Vec<ChargingAmpsSet>,
    last_api_call: std::time::Instant,
    next_check_car_in_location_after: Option<std::time::Instant>,
    next_check_car_still_plugged_after: Option<std::time::Instant>,
}

pub(crate) struct TessieApiHandler {
    car_vin: Arc<Mutex<String>>,
    car_in_expected_location: AtomicBool,
    car_plugged_in_location: AtomicBool,
    state: Arc<Mutex<TessieApiHandlerState>>,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
pub struct GetVehicleStateResponse {
    id: i64,
    vin: String,
    id_s: String,
    color: String,
    state: String,
    tokens: Vec<String>,
    user_id: i64,
    in_service: bool,
    vehicle_id: i64,
    access_type: String,
    api_version: String,
    drive_state: DriveState,
    charge_state: ChargeState,
    display_name: String,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct DriveState {
    power: f64,
    speed: String,
    heading: f64,
    latitude: f64,
    longitude: f64,
    gps_as_of: i64,
    timestamp: i64,
    native_type: String,
    shift_state: String,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
struct ChargeState {
    timestamp: i64,
    charge_amps: f64,
    charge_rate: f64,
    battery_level: f64,
    battery_range: f64,
    charger_power: f64,
    trip_charging: bool,
    charger_phases: String,
    charging_state: String,
    charger_voltage: f64,
    charge_limit_soc: f64,
}

#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
pub struct GetLocationResponse {
    latitude: f64,
    longitude: f64,
    address: String,
    saved_location: String,
}

impl TessieApiHandler {
    pub fn new() -> Self {
        Self {
            car_vin: Arc::new(Mutex::new("".to_string())),
            car_in_expected_location: AtomicBool::new(false),
            car_plugged_in_location: AtomicBool::new(false),
            state: Arc::new(Mutex::new(TessieApiHandlerState {
                cached_car_state: None,
                last_charging_amps_set: Vec::new(),
                current_saved_location_name: "".to_string(),
                expected_location_name: "".to_string(),
                last_api_call: std::time::Instant::now(),
                next_check_car_in_location_after: None,
                next_check_car_still_plugged_after: None,
            })),
        }
    }

    pub async fn ensure_fresh(&self) {
        let mut state = self.state.lock().await;
        if state.cached_car_state.is_none() || state.last_api_call.elapsed().as_secs() > 3600 {
            state.cached_car_state = Some(self.get_car_info().await);
        } else if let Some(cached_car_state) = &state.cached_car_state {
            
            if state.last_api_call.elapsed().as_secs() > 300 {
                state.cached_car_state = Some(self.get_car_info().await);
            }

            if state
                .next_check_car_in_location_after
                .map_or(false, |t| t.elapsed().as_secs() > 0)
            {
                self.check_car_in_location().await;
            }

            if state
                .next_check_car_still_plugged_after
                .map_or(false, |t| t.elapsed().as_secs() > 0)
            {
                self.check_car_still_plugged().await;
            }
        }
    }

    pub async fn inform_amps_at_location(&self, amps: f64) {
        self.ensure_fresh().await;

        if self
            .car_in_expected_location
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.set_unjerked_charging_amps(amps).await;
        }
    }

    pub async fn check_car_in_location(&self) {
        let mut state = self.state.lock().await;
        let car_vin = self.car_vin.lock().await;
        let car_vin = car_vin.clone();
        let response = self
            ._request_api_get(&format!("{}/location", car_vin))
            .await;

        // Decode response using GetLocationResponse serde
        let response: GetLocationResponse =
            serde_json::from_str(&response).expect("GetLocationResponse");
        let result = response.saved_location == state.expected_location_name;

        self.car_in_expected_location
            .store(result, std::sync::atomic::Ordering::Relaxed);

        let wait_duration = if result {
            std::time::Duration::from_secs(120)
        } else {
            std::time::Duration::from_secs(600)
        };

        state.next_check_car_in_location_after = Some(std::time::Instant::now() + wait_duration);
    }

    pub async fn check_car_still_plugged(&self) {
        let mut state = self.state.lock().await;
        let car_vin = self.car_vin.lock().await;
        let car_vin = car_vin.clone();
        let response = self
            ._request_api_get(&format!("{}/state?use_cache=true", car_vin))
            .await;

        // Decode response using GetVehicleStateResponse serde
        let response: GetVehicleStateResponse =
            serde_json::from_str(&response).expect("GetVehicleStateResponse");
        let result = response.charge_state.charging_state == "Charging";

        self.car_plugged_in_location
            .store(result, std::sync::atomic::Ordering::Relaxed);

        let wait_duration = if result {
            state.next_check_car_in_location_after = None; // Stop checking if the car is in the location: we already know it's charging there
            std::time::Duration::from_secs(60)
        } else {
            state.next_check_car_in_location_after =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(120));
            std::time::Duration::from_secs(600)
        };

        state.next_check_car_still_plugged_after = Some(std::time::Instant::now() + wait_duration);
    }

    pub async fn set_car_vin(&self, vin: &str) {
        let mut car_vin = self.car_vin.lock().await;
        *car_vin = vin.to_string();
    }

    pub async fn set_unjerked_charging_amps(&self, amps: f64) {
        // Ensures that the car's charging rate is not jerked through the API
        // That is, the behavior when being called like this should result in the following output:
        // 5.01 -> 5.0
        // 5.09 -> 5.0
        // 5.11 -> 5.0
        // 5.19 -> 5.0
        // 6.21 -> 5.0
        // 6.29 -> 5.0
        // 6.31 -> 6.0
        // 6.39 -> 6.0
        // 6.41 -> 6.0
        // 5.4 -> 6.0
        // 5.3 -> 5.0
        // 5.2 -> 5.0
        // 15.0 -> 6.0
        // 15.0 -> 6.0
        // 15.0 -> 6.0
        // 15.0 -> 15.0
        // 15.0 -> 15.0
        // Meaning: we round to the lowest near integer, but consider a window size of the last 10 elements
        // to avoid jerking the car's charging rate.
        let mut state = self.state.lock().await;
        let car_vin = self.car_vin.lock().await;
        let car_vin = car_vin.clone();

        let car_state = self.get_car_info().await;
        let current_charge_amps = car_state.charge_state.charge_amps;

        let mut set_amps = amps;
        if state.last_charging_amps_set.len() >= 10 {
            state.last_charging_amps_set.remove(0);
        }

        // Calculate the average of the last 10 elements
        let mut sum = 0.0;
        for charging_amps_set in &state.last_charging_amps_set {
            sum += charging_amps_set.set_by_us;
        }
        let average = sum / state.last_charging_amps_set.len() as f64;

        // If the current value is within 0.1 of the average, we set the average, rounding to the lowest integer
        if (average - set_amps).abs() < 0.5 {
            set_amps = average.floor();
        }

        state.last_charging_amps_set.push(ChargingAmpsSet::new(current_charge_amps, set_amps));

        self.set_charging_amps(set_amps).await;
    }

    pub async fn set_charging_amps(&self, amps: f64) {
        let car_vin = self.car_vin.lock().await;
        let car_vin = car_vin.clone();
        self._request_api_post(
            &format!("{}/command/set_charging_amps?amps={}", car_vin, amps),
            "",
        )
        .await;
    }

    pub async fn get_car_info(&self) -> GetVehicleStateResponse {
        let car_vin = self.car_vin.lock().await;
        let car_vin = car_vin.clone();
        let response = self
            ._request_api_get(&format!("{}/state?use_cache=true", car_vin))
            .await;

        // Decode response using GetVehicleStateResponse serde
        let response: GetVehicleStateResponse =
            serde_json::from_str(&response).expect("GetVehicleStateResponse");

        response
    }

    async fn _request_api_get(&self, request: &str) -> String {
        self.state.lock().await.last_api_call = std::time::Instant::now();
        let client = reqwest::Client::new();
        let response = client
            .get(format!("https://api.tessie.com/{}", request))
            .header("Authorization", format!("Bearer {}", TESSIE_API_TOKEN))
            .send()
            .await;

        response.expect("Resposne").text().await.expect("Text")
    }

    async fn _request_api_post<T: Into<reqwest::Body>>(&self, request: &str, body: T) -> String {
        self.state.lock().await.last_api_call = std::time::Instant::now();
        let client = reqwest::Client::new();
        let response = client
            .post(format!("https://api.tessie.com/{}", request))
            .header("Authorization", format!("Bearer {}", TESSIE_API_TOKEN))
            .body(body)
            .send()
            .await;

        response.expect("Resposne").text().await.expect("Text")
    }
}


// Implement "Tessie fairing" to be able to use it in Rocket
// This will start a thread that will check the car's state every 5 minutes
// on_ignite, and then every 5 minutes, it will check if the car is in the expected location
// When the rocket is shut down, the thread will be stopped.

pub struct TessieRocketFairing {
    // Use a tokio task
    task: tokio::task::JoinHandle<()>,
}

#[rocket::async_trait]
impl rocket::fairing::Fairing for TessieRocketFairing {
    fn info(&self) -> rocket::fairing::Info {
        rocket::fairing::Info {
            name: "Tessie Rocket Fairing",
            kind: rocket::fairing::Kind::Ignite | rocket::fairing::Kind::Shutdown,
        }
    }

    async fn on_ignite(&self, rocket: rocket::Rocket<rocket::Build>) -> rocket::fairing::Result {
        let tessie = TessieApiHandler::new();
        let task = TessieRocketFairing {
            task: tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                tessie.ensure_fresh().await;
            }
        })};

        let rocket = rocket.manage(task);

        Ok(rocket)
    }

    async fn on_shutdown(&self, rocket: &rocket::Rocket<rocket::Orbit>) { 
        let task = &rocket.state::<TessieRocketFairing>().expect("TessieRocketFairing").task;
        task.abort();
    }
}
