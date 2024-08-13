use std::sync::Arc;

use rocket::tokio::sync::Mutex;



// TessieFairing schedules a task that will check every minute if the car is nearby
// our amp meter, and if it's charging.
pub struct TessieFairing {
    task: Arc<Mutex<Option<rocket::tokio::task::JoinHandle<()>>>>,
}

impl TessieFairing {
    pub fn new() -> Self {
        Self {
            task: Arc::new(Mutex::new(None)),
        }
    }
}

#[rocket::async_trait]
impl rocket::fairing::Fairing for TessieFairing {
    fn info(&self) -> rocket::fairing::Info {
        rocket::fairing::Info {
            name: "Car - Tessie",
            kind: rocket::fairing::Kind::Ignite | rocket::fairing::Kind::Shutdown,
        }
    }

    async fn on_ignite(&self, rocket: rocket::Rocket<rocket::Build>) -> rocket::fairing::Result<rocket::Rocket<rocket::Build>> {
        let params = super::task::MainTaskParams::from_figment(rocket.figment());
        log::info!("Tessie: Starting main task from params: {:?}", params);
        let task = rocket::tokio::task::spawn(super::task::main_task(params));

        {
            let mut guard = self.task.lock().await;
            guard.replace(task);
        }

        Ok(rocket)
    }
}