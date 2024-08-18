//! A simple alive check fairing.
//! 
//! This module contains the [AliveCheckFairing] fairing, that checks if any
//! sensor has logged data in the last 60 seconds. If there hasn't been any
//! input, it sends a message via webhook. The webhook URL is read from the
//! figment configuration (Rocket.toml).
//! 
//! This is useful to get notified in case of a network or DNS routing issue.

use rocket::{
    fairing::{Fairing, Info, Kind},
    figment::providers::Serialized,
    tokio::sync::Mutex,
};
use rocket_db_pools::Database;
use rocket_db_pools::Pool;
use std::sync::Arc;

/// This fairing checks if the sensor is alive by checking if there has been any input in the last 60 seconds.
/// If there hasn't been any input, it sends a message via webhook.
/// 
/// The webhook URL is read from the figment configuration (Rocket.toml).
pub struct AliveCheckFairing {
    /// This stores the task that is spawned to check if the sensor is alive
    task: Arc<Mutex<Option<rocket::tokio::task::JoinHandle<()>>>>,
}

impl AliveCheckFairing {
    pub fn new() -> Self {
        Self {
            task: Arc::new(Mutex::new(None)),
        }
    }
}

/// This function initializes a second database connection pool to the Logs
/// database for the AliveCheckFairing. This is necessary because the fairing
/// runs on a separate task and it's not easy to share the database connection
/// pool with the orbiting rocket.
async fn get_database<D: Database>(rocket: &rocket::Rocket<rocket::Orbit>) -> D {
    let workers: usize = rocket
        .figment()
        .extract_inner(rocket::Config::WORKERS)
        .unwrap_or_else(|_| rocket::Config::default().workers);

    let figment = rocket
        .figment()
        .focus(&format!("databases.{}", D::NAME))
        .join(Serialized::default("max_connections", workers * 4))
        .join(Serialized::default("connect_timeout", 5));

    match <D::Pool>::init(&figment).await {
        Ok(pool) => D::from(pool),
        Err(e) => {
            panic!("failed to initialize database: {}", e);
        }
    }
}

#[rocket::async_trait]
impl Fairing for AliveCheckFairing {
    fn info(&self) -> Info {
        Info {
            name: "Sensor Alive Check",
            kind: Kind::Liftoff | Kind::Shutdown,
        }
    }

    async fn on_liftoff(&self, rocket: &rocket::Rocket<rocket::Orbit>) -> () {
        let db_conn = get_database::<crate::Logs>(rocket).await;
        let webhook_url: String = rocket.figment().extract_inner("webhook_url").unwrap_or_default();
        let task = rocket::tokio::task::spawn(async move {
            loop {
                rocket::tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                log::info!("Checking if the sensor is alive");

                // Check using sqlx if there has been any input in the last 60 seconds
                // If there hasn't been any input, send a message via webhook
                let rows = sqlx::query!(
                    "SELECT COUNT(*) as count FROM energy_log WHERE created_at > datetime('now', '-60 seconds')"
                );
                let count = rows.fetch_one(&*db_conn).await.unwrap().count;
                log::info!("Rows in the last 60 seconds: {}", count);

                if count == 0 {
                    log::warn!("No rows in the last 60 seconds!");
                    if !webhook_url.is_empty() {
                        let client = reqwest::Client::new();
                        let res = client.post(&webhook_url).send().await;
                        match res {
                            Ok(res) => {
                                log::info!("Webhook response: {:?}", res);
                            }
                            Err(e) => {
                                log::error!("Failed to send webhook: {:?}", e);
                            }
                        }
                    }
                }
            }
        });
        let old = self.task.lock().await.replace(task);

        old.map(|f| f.abort());
    }

    /// When the rocket is shutting down, we need to abort the task that checks
    /// if the sensor is alive, in order to clean up.
    async fn on_shutdown(&self, _: &rocket::Rocket<rocket::Orbit>) -> () {
        if let Some(task) = self.task.lock().await.take() {
            task.abort();
        }
    }
}
