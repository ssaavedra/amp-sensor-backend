use serde::{Deserialize, Serialize};

use super::task::LatLon;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ChargingState {
    Complete,
    Charging,
    Disconnected,
    Pending,
    Starting,
    Stopped,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ChargePortLatch {
    Engaged,
    Disengaged,

    #[serde(untagged)]
    Unknown(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TessieChargeState {
    pub charge_amps: f64,
    pub charge_current_request: usize,
    pub charge_enable_request: bool,
    pub charge_energy_added: f64,
    pub charge_limit_soc: usize,
    pub charge_limit_soc_max: usize,
    pub charge_limit_soc_min: usize,
    pub charge_limit_soc_std: usize,
    pub charge_miles_added_ideal: f64,
    pub charge_miles_added_rated: f64,
    pub charge_port_cold_weather_mode: bool,
    pub charge_port_door_open: bool,
    pub charge_port_latch: ChargePortLatch,
    pub charge_rate: f64,
    pub charger_actual_current: f64,
    pub charger_phases: Option<usize>,
    pub charger_pilot_current: f64,
    pub charger_power: f64,
    pub charger_voltage: f64,
    pub charging_state: ChargingState,
    pub conn_charge_cable: String,
    pub fast_charger_brand: String,
    pub fast_charger_present: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TessieDriveState {
    pub gps_as_of: i64,
    pub latitude: f64,
    pub longitude: f64,
    pub heading: usize,
    pub speed: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum TessieCarWakeState {
    Asleep,
    WaitingForSleep,
    Online,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TessieCarState {
    access_type: String,
    api_version: usize,
    state: TessieCarWakeState,
    vehicle_name: Option<String>,
    display_name: Option<String>,
    pub drive_state: TessieDriveState,
    pub charge_state: TessieChargeState,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetChargingAmpsResult {
    pub result: bool,

    // Field woke is only present in the response if the car was woken up
    #[serde(default)]
    pub woke: bool,
}

pub struct TessieAPIHandler {
    vin: String,
    token: String,
}


fn fix_optional_body(
    request: reqwest::RequestBuilder,
    method: reqwest::Method,
    body: Option<String>,
) -> reqwest::RequestBuilder {
    match method {
        reqwest::Method::GET => request,
        reqwest::Method::POST => match body {
            Some(body) => {
                let body = reqwest::Body::from(body);
                let len = body.as_bytes().map(|b| b.len()).unwrap_or(0);
                request
                    .header(reqwest::header::CONTENT_LENGTH, len.to_string())
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body)
            }
            None => request
                .header(reqwest::header::CONTENT_LENGTH, "0")
                .header(reqwest::header::CONTENT_TYPE, "application/json"),
        },
        _ => request,
    }
}

impl TessieAPIHandler {
    pub fn new(vin: String, token: String) -> Self {
        Self { vin, token }
    }

    async fn request(
        &self,
        endpoint: &str,
        method: reqwest::Method,
        body: Option<String>,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let client = reqwest::Client::new();
        let url = format!("https://api.tessie.com/{}/{}", self.vin, endpoint);
        let request = fix_optional_body(
            client
                .request(method.clone(), &url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", self.token),
                )
                .header(reqwest::header::ACCEPT, "application/json"),
            method,
            body,
        )
        .build()?;
        client.execute(request).await
    }

    pub async fn get_state(&self) -> anyhow::Result<TessieCarState> {
        let response = self.request("state", reqwest::Method::GET, None).await?;
        let content = response.text().await?;
        serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))
    }

    pub async fn set_charging_amps(&self, amps: usize) -> anyhow::Result<SetChargingAmpsResult> {
        let endpoint = format!(
            "command/set_charging_amps?wait_for_completion=true&amps={}",
            amps
        );
        log::info!("Tessie: Sending request to endpoint: {}", endpoint);
        let response = self.request(&endpoint, reqwest::Method::POST, None).await?;
        let bytes = response.error_for_status()?.text().await;
        log::info!("Tessie: Received response: {}", bytes.as_ref().unwrap());
        serde_json::from_str(&bytes.unwrap())
            .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))
    }
}

impl From<TessieDriveState> for LatLon {
    fn from(state: TessieDriveState) -> Self {
        Self {
            lat: state.latitude,
            lon: state.longitude,
        }
    }
}

// Implement TessieCarState -> LatLon conversion
impl From<TessieCarState> for LatLon {
    fn from(state: TessieCarState) -> Self {
        LatLon::from(state.drive_state)
    }
}
