//! Fairing to ensure an EV does not exceed a power budget.
//! 
//! This is used to implement a poor man's smart-grid EVSE, where the platform
//! will check the average amps drawn by the home from the database over a
//! period and ensure the car does not exceed the power budget.
//! 
//! If you are thinking on doing something like this at your own place, this
//! solution can be at least 10x cheaper than running cables to the breaker box
//! and installing an EVSE with grid sensing, and provides a good enough
//! solution. It also allows operation with any "dumb" IEC 61851-implementing
//! EVSE, without requiring you to implement of the more expensive ISO 15118-2
//! protocol in the EVSE communication with the car.

use std::sync::Arc;

use rocket::tokio::sync::Mutex;

use super::EVChargeHandler;

/// This fairing checks if the car is nearby and if it's charging.
///
/// Originally it was implemented as a task that would run every 30 seconds, but
/// it was changed to run on every response to the Rocket app. This is because
/// it actually makes sense to react to changes when we know of them happening.
///
/// This also makes the implementation much simpler, as we don't need to worry
/// about having a different DB pool.
///
/// Since requests can come in parallel, by using a Mutex we can ensure that
/// only one request at a time will check the car status, and we can discard the
/// other.
pub struct EVChargeFairing<H: EVChargeHandler> {
    handler: Arc<Mutex<Option<super::task::CarHandler<H>>>>,
}

impl<'a, H: EVChargeHandler> EVChargeFairing<H>
where
    H::ConfigParams: From<&'a rocket::figment::Figment>,
{
    pub fn new() -> Self {
        Self {
            handler: Arc::new(Mutex::new(None)),
        }
    }

    /// This function checks if the car is nearby and if it's charging.
    ///
    /// If it is, it will check the average amps drawn by the home from the
    /// database over the last 30 seconds and update the car API accordingly to
    /// not exceed the amp limit.
    async fn check_on_response<'r>(&self, req: &rocket::Request<'r>) -> anyhow::Result<()> {
        let _guard = match self.handler.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                log::info!("Car handler is currently locked, skipping check on this response.");
                return Ok(());
            } // Ignore if the lock is currently being held elsewhere
        };
        let handler = _guard.as_ref().unwrap();
        // 1. Check that the car is nearby
        // 2. Check if the car is charging
        // 3. If the car is charging, check the amps drawn by the home from the database over the last 30 seconds and update the car API accordingly to not exceed the amp limit.

        // Check if the car is nearby
        if handler.is_car_nearby().await? {
            log::info!("Car is nearby: TRUE");
            // Check if the car is charging
            let car_is_charging = handler.is_car_charging().await?;
            log::info!("Is car charging? {:?}", car_is_charging);
            if car_is_charging {
                let (avg_amps, max_amps) = self.get_avg_amps_at_location(req).await?;
                handler
                    .set_current_home_consumption(avg_amps, max_amps)
                    .await?;
                log::info!(
                    "Retrieved current home consumption as: {} amps (max={})",
                    avg_amps,
                    max_amps
                );
                handler.throttled_calculate_amps().await?;
            }
        } else {
            log::info!("Car is nearby: FALSE");
        }

        Ok(())
    }

    /// This function retrieves the average amps drawn at the location from the
    /// database over the last 30 seconds.
    ///
    /// It returns a tuple with the average amps and the max amps drawn.
    async fn get_avg_amps_at_location<'r>(
        &self,
        req: &rocket::Request<'r>,
    ) -> anyhow::Result<(f64, f64)> {
        let db = req.guard::<&crate::Logs>().await.unwrap();
        let token = req.guard::<&crate::ValidDbToken>().await.unwrap();

        log::info!(
            "Checking average amps drawn at location for token: {}",
            token
        );
        let result = sqlx::query!("SELECT AVG(amps) as avg_amps, MAX(amps) as max_amps FROM energy_log WHERE token = ? AND created_at > datetime('now', '-30 seconds')", token)
            .fetch_one(&**db)
            .await?;
        let avg_amps: f64 = result.avg_amps.unwrap_or(0.0);
        let max_amps: f64 = result.max_amps.unwrap_or(0.0);
        log::info!(
            "Retrieved average amps: {} and max amps: {}",
            avg_amps,
            max_amps
        );

        Ok((avg_amps, max_amps))
    }
}

#[rocket::async_trait]
impl<'a, H: EVChargeHandler> rocket::fairing::Fairing for EVChargeFairing<H>
where
    H: Send + Sync + 'static,
    H::ConfigParams: Send + Sync + 'static,
    H::InternalState: Send + Sync + 'static,
    H::ConfigParams: From<&'a rocket::figment::Figment>,
{
    fn info(&self) -> rocket::fairing::Info {
        let type_name = H::get_name();
        let name = Box::new(format!("EV Charge Fairing ({})", &type_name)).leak();
        rocket::fairing::Info {
            name: name,
            kind: rocket::fairing::Kind::Response | rocket::fairing::Kind::Ignite,
        }
    }

    /// We initialize the [super::task::CarHandler] and store it in the fairing when the
    /// Rocket app is ignited.
    async fn on_ignite(
        &self,
        rocket: rocket::Rocket<rocket::Build>,
    ) -> rocket::fairing::Result<rocket::Rocket<rocket::Build>> {
        let handler = super::task::CarHandler::from(rocket.figment());
        let mut guard = self.handler.lock().await;
        *guard = Some(handler);

        Ok(rocket)
    }

    async fn on_response<'r>(
        &self,
        req: &'r rocket::Request<'_>,
        _res: &mut rocket::Response<'r>,
    ) -> () {
        // Is this a request to log info?
        let route_name = req
            .route()
            .map(|route| route.name.as_deref())
            .flatten()
            .unwrap_or("");
        if route_name == "post_token" {
            match self.check_on_response(req).await {
                Ok(_) => log::info!("Car check succeeded."),
                Err(e) => log::error!("Car check failure: {}", e),
            }
        }
    }
}
