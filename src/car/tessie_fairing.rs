use std::sync::Arc;

use rocket::tokio::sync::Mutex;




// TessieFairing schedules a task that will check every minute if the car is nearby
// our amp meter, and if it's charging.
pub struct TessieFairing {
    handler: Arc<Mutex<Option<super::task::TessieCarHandler>>>
}

impl TessieFairing {
    pub fn new() -> Self {
        Self {
            handler: Arc::new(Mutex::new(None))
        }
    }

    async fn check_on_response<'r>(&self, req: &rocket::Request<'r>) -> anyhow::Result<()> {
        let _guard = self.handler.lock().await;
        let handler = _guard.as_ref().unwrap();
        // 1. Check that the car is nearby
        // 2. Check if the car is charging
        // 3. If the car is charging, check the amps drawn by the home from the database over the last 30 seconds and update the car API accordingly to not exceed the amp limit.

        // Check if the car is nearby
        if handler.is_car_nearby().await? {
            log::info!("Car is nearby: TRUE");
            // Check if the car is charging
            let charge_state = handler.get_charging_status().await;
            log::info!("Car charging state is: {:?}", charge_state);
            if charge_state == super::tessie_api::ChargingState::Charging {
                let (avg_amps, max_amps) = self.get_avg_amps_at_location(req).await?;
                handler.set_current_home_consumption(avg_amps, max_amps).await?;
                log::info!("Retrieved current home consumption as: {} amps (max={})", avg_amps, max_amps);
                handler.throttled_calculate_amps().await?;
            }
        } else {
            log::info!("Car is nearby: FALSE");
        }
        
        Ok(())
    }

    async fn get_avg_amps_at_location<'r>(&self, req: &rocket::Request<'r>) -> anyhow::Result<(f64, f64)> {
        let db = req.guard::<&crate::Logs>().await.unwrap();
        let token = req.guard::<&crate::ValidDbToken>().await.unwrap();
        
        log::info!("Checking average amps drawn at location for token: {}", token.0);
        let result = sqlx::query!("SELECT AVG(amps) as avg_amps, MAX(amps) as max_amps FROM energy_log WHERE token = ? AND created_at > datetime('now', '-30 seconds')", token.0)
            .fetch_one(&**db)
            .await?;
        let avg_amps: f64 = result.avg_amps.unwrap_or(0.0);
        let max_amps: f64 = result.max_amps.unwrap_or(0.0);
        log::info!("Retrieved average amps: {} and max amps: {}", avg_amps, max_amps);

        Ok((avg_amps, max_amps))
    }
}

#[rocket::async_trait]
impl rocket::fairing::Fairing for TessieFairing {
    fn info(&self) -> rocket::fairing::Info {
        rocket::fairing::Info {
            name: "Car - Tessie",
            kind: rocket::fairing::Kind::Response | rocket::fairing::Kind::Ignite,
        }
    }

    async fn on_ignite(&self, rocket: rocket::Rocket<rocket::Build>) -> rocket::fairing::Result<rocket::Rocket<rocket::Build>> {
        let handler = super::task::TessieCarHandler::from_figment(rocket.figment());
        let mut guard = self.handler.lock().await;
        *guard = Some(handler);

        Ok(rocket)
    }

    async fn on_response<'r>(&self, req: &'r rocket::Request<'_> , res: &mut rocket::Response<'r>) -> () {
        // Is this a request to log info?
        let route_name = req.route().map(|route| route.name.as_deref()).flatten().unwrap_or("");
        if route_name == "post_token" {
            let _ = self.check_on_response(req).await;
        }
    }

}