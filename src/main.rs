use governor::Quota;
use print_table::get_paginated_rows_for_token;
use rocket::http::ContentType;
use rocket::serde::{json::Json, Deserialize};
use rocket::{catchers, fairing, get, launch, post, routes};
use rocket_db_pools::{sqlx, Connection, Database};
use rocket_governor::{rocket_governor_catcher, RocketGovernable, RocketGovernor};
use token::ValidDbToken;

mod alive_check;
mod car;
mod print_table;
mod token;



#[derive(Database)]
#[database("sqlite_logs")]
struct Logs(sqlx::SqlitePool);

pub struct RateLimitGuard;

impl<'r> RocketGovernable<'r> for RateLimitGuard {
    fn quota(_method: rocket_governor::Method, _route_name: &str) -> governor::Quota {
        Quota::per_second(Self::nonzero(4u32)).allow_burst(Self::nonzero(15u32))
    }
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct LogData {
    amps: f64,
    volts: Option<f64>,
    watts: f64,
}


#[derive(Debug)]
struct UserAgent<'a>(&'a str);

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

// Route with POST /log/:token/ will INSERT into the database
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
        token.0,
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
            token.0,
            page + 1,
            count
        ));
    }
    result.push_str("</body></html>\n");

    (ContentType::HTML, result)
}

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
        format!("/log/{}/json?page={}&count={}", token.0, page + 1, count)
    } else {
        "".to_string()
    };

    let mut result = String::new();
    result.push_str("{\n\"rows\": [\n");
    for (i, row) in rows.iter().enumerate() {
        result.push_str(&row.to_json());
        if i < rows.len() - 1 {
            result.push_str(",\n");
        }
    }
    result.push_str(&format!("\n],\n\"next\": \"{}\"}}\n", next_url));

    rocket::response::content::RawJson(result)
}

#[get("/")]
async fn index(__ratelimit: RocketGovernor<'_, RateLimitGuard>) -> String {
    log::info!("Got to index!");
    "PONG".to_string()
}

#[launch]
async fn rocket() -> _ {
    rocket::build()
        .attach(Logs::init())
        .attach(fairing::AdHoc::on_ignite("Run DB migrations", |rocket| async {
            let db = Logs::fetch(&rocket).expect("DB connection");
            sqlx::migrate!("./migrations").run(&**db).await.unwrap();
            rocket
        }))
        .attach(alive_check::AliveCheckFairing::new(""))
        // .attach(car::tessie_fairing::TessieFairing::new())
        .mount(
            "/",
            routes![index, list_table_html, list_table_json, post_token],
        )
        .register("/", catchers![rocket_governor_catcher])
}
