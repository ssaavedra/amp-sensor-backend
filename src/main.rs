//! This application is a simple energy logger that logs energy consumption data
//! to a SQLite database.
//!
//! The application has a few routes:
//! - POST /log/:token/ to insert data into the database
//! - GET /log/:token/html to get the data in HTML format
//! - GET /log/:token/json to get the data in JSON format
//!
//! There is no built-in token administration or rotation yet. You have to
//! manually add tokens to the database using the SQLite CLI or a SQLite
//! database management tool like DB Browser for SQLite.
//!
//! We recommend using a tool such as Python's secrets module to generate
//! cryptographically secure tokens.
//!
//! ```python
//! import secrets
//! token = secrets.token_urlsafe(32)
//! print(token)
//! ```
//!
//! The application uses the rocket-governor crate to rate limit the POST
//! requests to 4 requests per second per IP address, to prevent abuse.
//!
//! The application also uses the rocket-db-pools crate to manage the SQLite
//! database connection pool.
//!
//! There are a few custom fairings in the application:
//! - The [AliveCheckFairing](alive_check::AliveCheckFairing) checks if the
//!   sensor is alive by checking if there has been any input in the last 60
//!   seconds. If there hasn't been any input, it sends a message via webhook.
//! - The [EVChargeFairing](car::fairing::EVChargeFairing) automatically
//!   requests an EV to charge according to a maximum charge budget, dynamically
//!   adjusted depending on the total energy consumption of the house. It
//!   requires an [car::EVChargeHandler] as a type parameter, and the current
//!   implementation uses [car::tessie]
//! - New fairings like the EVChargeFairing could be implmented in the future to
//!   add add other IoT devices or additional functionality.
//!
use form::ParseableDateTime;
use governor::Quota;
use print_table::{get_avg_max_rows_for_token, get_paginated_rows_for_token};
use rocket::http::ContentType;
use rocket::serde::{json::Json, Deserialize};
use rocket::{catchers, fairing, get, launch, post, routes};
use rocket_db_pools::{sqlx, Connection, Database};
use rocket_governor::{rocket_governor_catcher, RocketGovernable, RocketGovernor};
use token::{Token, ValidDbToken};

mod alive_check;
mod car;
mod cli;
pub mod form;
mod print_table;
mod token;

/// The energy log database pool
#[derive(Database)]
#[database("sqlite_logs")]
struct Logs(sqlx::SqlitePool);

/// Rate limit guard implementation, allowing 4 requests per second per IP
/// address, bursting up to 15 requests.
pub struct RateLimitGuard;

impl<'r> RocketGovernable<'r> for RateLimitGuard {
    fn quota(_method: rocket_governor::Method, _route_name: &str) -> governor::Quota {
        Quota::per_second(Self::nonzero(4u32)).allow_burst(Self::nonzero(15u32))
    }
}

/// Expected JSON body for the POST /log/:token/ route
#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct LogData {
    amps: f64,
    volts: Option<f64>,
    watts: f64,
}

/// User-Agent header
#[derive(Debug)]
struct UserAgent<'a>(&'a str);

/// Client IP address
#[derive(Debug)]
struct ClientIP(String);

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for UserAgent<'r> {
    type Error = ();

    async fn from_request(
        request: &'r rocket::Request<'_>,
    ) -> rocket::request::Outcome<Self, Self::Error> {
        let agent = request.headers().get_one("User-Agent").unwrap_or("Unknown");
        log::info!("User-Agent: {}", agent);
        rocket::request::Outcome::Success(UserAgent(agent))
    }
}

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for ClientIP {
    type Error = ();

    async fn from_request(
        request: &'r rocket::Request<'_>,
    ) -> rocket::request::Outcome<Self, Self::Error> {
        let ip = request
            .client_ip()
            .map(|ip| ip.to_string())
            .unwrap_or("Unknown".to_string());
        rocket::request::Outcome::Success(ClientIP(ip))
    }
}

/************************* ROUTES *************************/

/// Route POST /log/:token/ will INSERT value into the database (if token is valid and rate limit is not exceeded)
#[post("/log/<_>", data = "<log>", rank = 2)]
async fn post_token(
    token: &ValidDbToken,
    log: Json<LogData>,
    ip: ClientIP,
    ua: UserAgent<'_>,
    mut db: Connection<Logs>,
    _ratelimit: RocketGovernor<'_, RateLimitGuard>,
) -> String {
    let volts = log.volts.unwrap_or(220.0f64);
    let _rows = sqlx::query!(
        "INSERT INTO energy_log (token, amps, volts, watts, user_agent, client_ip) VALUES (?, ?, ?, ?, ?, ?)",
        token,
        log.amps,
        volts,
        log.watts,
        ua.0,
        ip.0
    )
    .execute(&mut **db)
    .await
    .unwrap()
    .rows_affected();

    log::info!("Inserted row from IP {:?} and UA {:?}", ip, ua);

    format!("OK")
}

/// Route GET /log/:token/html will return the data in HTML format
#[get("/log/<_>/html?<page>&<count>", rank = 1)]
async fn list_table_html(
    page: Option<i32>,
    count: Option<i32>,
    token: &ValidDbToken,
    mut db: Connection<Logs>,
    _ratelimit: RocketGovernor<'_, RateLimitGuard>,
) -> (ContentType, String) {
    let page = page.unwrap_or(0);
    let count = count.unwrap_or(10);

    let (rows, has_next) = get_paginated_rows_for_token(&mut db, &token, page, count).await;

    let mut result = String::new();
    result.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\"/><title>Consumption info</title></head><body><table>");
    result.push_str(
        "<tr><th>Location (token id/ua)</th><th>Date</th><th>Amps</th><th>Volts</th><th>Watts</th></tr>\n",
    );
    for row in rows {
        result.push_str(&row.to_html());
    }
    result.push_str("\n</table>\n");
    if has_next {
        result.push_str(&format!(
            "<a href=\"/log/{}/html?page={}&count={}\">Next</a>",
            token.full_token(),
            page + 1,
            count
        ));
    }
    result.push_str("</body></html>\n");

    (ContentType::HTML, result)
}

/// Route GET /log/:token/json will return the data in JSON format
#[get("/log/<_>/json?<page>&<count>", rank = 1)]
async fn list_table_json(
    page: Option<i32>,
    count: Option<i32>,
    token: &ValidDbToken,
    mut db: Connection<Logs>,
    _ratelimit: RocketGovernor<'_, RateLimitGuard>,
) -> rocket::response::content::RawJson<String> {
    let page = page.unwrap_or(0);
    let count = count.unwrap_or(10);

    let (rows, has_next) = get_paginated_rows_for_token(&mut db, &token, page, count).await;

    let next_url = if has_next {
        format!(
            "/log/{}/json?page={}&count={}",
            token.full_token(),
            page + 1,
            count
        )
    } else {
        "".to_string()
    };

    let result = serde_json::json!({
        "rows": rows,
        "next": next_url
    });

    rocket::response::content::RawJson(serde_json::to_string_pretty(&result).unwrap())
}


/// Route GET /log/:token/html will return the data in HTML format
#[get("/log/<_>/svg?<start>&<end>&<interval>", rank = 1)]
async fn list_table_svg(
    start: Option<ParseableDateTime>,
    end: Option<ParseableDateTime>,
    interval: Option<i32>,
    token: &ValidDbToken,
    mut db: Connection<Logs>,
    _ratelimit: RocketGovernor<'_, RateLimitGuard>,
) -> (ContentType, String) {
    let start = start
        .unwrap_or(ParseableDateTime(
            chrono::Utc::now() - chrono::Duration::days(1),
        ));
    let end = end.unwrap_or(ParseableDateTime(chrono::Utc::now()));
    let interval = interval.unwrap_or(300);

    let (avg, max) = get_avg_max_rows_for_token(&mut db, &token, &start, &end, interval).await;

    (ContentType::SVG, print_table::to_svg_plot(avg, max))
}

/// Route GET / will return a simple PONG message. By default we don't advertise
/// the functionality of the application to the world.
#[get("/")]
async fn index(_ratelimit: RocketGovernor<'_, RateLimitGuard>) -> String {
    log::info!("Got to index!");
    "PONG".to_string()
}

/// Main function to launch the Rocket application
///
/// This runs the migrations (which are embedded into the binary), attaches the
/// [AliveCheckFairing](alive_check::AliveCheckFairing), and the
/// [car::fairing::EVChargeFairing] (with the [tessie
/// implementation](car::tessie)); and mounts the routes and catchers.
#[launch]
async fn rocket() -> _ {
    // Check if we are being called with the `consolidate_logs` argument, in which case we run the consolidation function
    // instead of starting the Rocket server
    if std::env::args().nth(1).is_some() {
        crate::cli::consolidate_logs::consolidate_logs_cli().await;
        std::process::exit(0);
    }

    rocket::build()
        .attach(Logs::init())
        .attach(fairing::AdHoc::on_ignite(
            "Run DB migrations",
            |rocket| async {
                let db = Logs::fetch(&rocket).expect("DB connection");
                sqlx::migrate!("./migrations").run(&**db).await.unwrap();
                rocket
            },
        ))
        .attach(alive_check::AliveCheckFairing::new())
        .attach(car::fairing::EVChargeFairing::<car::tessie::Handler>::new())
        .mount(
            "/",
            routes![
                index,
                list_table_html,
                list_table_json,
                list_table_svg,
                post_token
            ],
        )
        .register("/", catchers![rocket_governor_catcher])
}
