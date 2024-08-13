use rocket::{
    fairing::{Fairing, Info, Kind},
    figment::providers::Serialized,
    tokio::sync::Mutex,
};
use rocket_db_pools::Database;
use rocket_db_pools::Pool;
use std::sync::Arc;

pub struct AliveCheckFairing {
    // Add a field to store the task handle
    task: Arc<Mutex<Option<rocket::tokio::task::JoinHandle<()>>>>,
    webhook_url: String,
}

impl AliveCheckFairing {
    pub fn new(webhook_url: &str) -> Self {
        Self {
            task: Arc::new(Mutex::new(None)),
            webhook_url: webhook_url.to_string(),
        }
    }
}

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
            kind: Kind::Liftoff,
        }
    }

    async fn on_liftoff(&self, rocket: &rocket::Rocket<rocket::Orbit>) -> () {
        let db_conn = get_database::<crate::Logs>(rocket).await;
        let webhook_url = self.webhook_url.clone();
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
}
