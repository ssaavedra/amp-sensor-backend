use governor::Quota;
use rocket::serde::{json::Json, Deserialize};
use rocket::{catchers, fairing, get, launch, post, routes};
use rocket_db_pools::{sqlx, Connection, Database};
use rocket_governor::{rocket_governor_catcher, RocketGovernable, RocketGovernor};

#[derive(Database)]
#[database("sqlite_logs")]
struct Logs(sqlx::SqlitePool);

pub struct RateLimitGuard;

impl<'r> RocketGovernable<'r> for RateLimitGuard {
    fn quota(_method: rocket_governor::Method, _route_name: &str) -> governor::Quota {
        Quota::per_second(Self::nonzero(2u32))
    }
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct LogData {
    amps: f64,
    volts: f64,
    watts: f64,
}

struct ValidDbToken(String);

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for ValidDbToken {
    type Error = ();

    async fn from_request(
        request: &'r rocket::Request<'_>,
    ) -> rocket::request::Outcome<Self, Self::Error> {
        let mut db = request.guard::<Connection<Logs>>().await.unwrap();

        let token = request.routed_segment(1).map(|s| s.to_string());

        log::info!("Got token: {:?}", token);

        match token {
            Some(token) => {
                // Now validate against the db!
                let rows = sqlx::query!(
                    "SELECT COUNT(*) as count FROM tokens WHERE token = ?",
                    token
                );
                let count = rows.fetch_one(&mut **db).await.unwrap().count;
                log::info!("Token count in DB: {}", count);
                if count == 0 {
                    return rocket::request::Outcome::Error((rocket::http::Status::NotFound, ()));
                }
                rocket::request::Outcome::Success(ValidDbToken(token))
            }
            _ => {
                log::info!("No token found");
                rocket::request::Outcome::Forward(rocket::http::Status::NotFound)
            }
        }
    }
}

struct RowInfo {
    location: String,
    token: String,
    datetime: String,
    amps: f64,
    volts: f64,
    watts: f64,
}

impl RowInfo {
    fn new(
        location: &str,
        token: &str,
        datetime: &str,
        amps: f64,
        volts: f64,
        watts: f64,
    ) -> Self {
        Self {
            location: location.to_string(),
            token: token.to_string(),
            datetime: datetime.to_string(),
            amps,
            volts,
            watts,
        }
    }

    fn to_html(&self) -> String {
        format!(
            "<tr><td>{} ({})</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            self.location,
            simplify_token(&self.token),
            self.datetime,
            self.amps,
            self.volts,
            self.watts
        )
    }

    fn to_json(&self) -> String {
        format!("{{\"location\": \"{}\", \"token\": \"{}\", \"datetime\": \"{}\", \"amps\": {}, \"volts\": {}, \"watts\": {}}}", self.location, self.token, self.datetime, self.amps, self.volts, self.watts)
    }
}

async fn get_paginated_rows_for_token(
    db: &mut Connection<Logs>,
    token: &ValidDbToken,
    page: i32,
    count: i32,
) -> (Vec<RowInfo>, bool) {
    let mut rows = Vec::new();
    let offset = page * count;
    let db_count = count + 1;

    let db_rows = sqlx::query!(
        "SELECT amps, volts, watts, created_at, energy_log.token as token, u.location as location 
        FROM energy_log
        INNER JOIN tokens t
        ON t.token = energy_log.token
        INNER JOIN users u
        ON u.id = t.user_id
        WHERE energy_log.token = ?
        ORDER BY created_at DESC
        LIMIT ?
        OFFSET ?",
        token.0,
        db_count,
        offset
    )
    .fetch_all(&mut ***db)
    .await
    .unwrap();

    let db_rows_split = if db_rows.len() > count as usize {
        &db_rows[..count as usize]
    } else {
        &db_rows
    };

    for row in db_rows_split {
        rows.push(RowInfo::new(
            &row.location,
            &row.token,
            &row.created_at.to_string(),
            row.amps.parse::<f64>().unwrap_or(0f64),
            row.volts.parse::<f64>().unwrap_or(0f64),
            row.watts.parse::<f64>().unwrap_or(0f64),
        ));
    }
    let has_next = db_rows.len() > count as usize;

    (rows, has_next)
}

fn simplify_token(token: &str) -> String {
    let mut result = String::new();
    result.push_str(&token[..4]);
    result.push_str("...");
    result.push_str(&token[token.len() - 4..]);
    result
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
    mut db: Connection<Logs>,
    log: Json<LogData>,
    token: ValidDbToken,
    ip: ClientIP,
    ua: UserAgent<'_>,
    _ratelimit: RocketGovernor<'_, RateLimitGuard>,
) -> String {
    let _rows = sqlx::query!(
        "INSERT INTO energy_log (token, amps, volts, watts, user_agent, client_ip) VALUES (?, ?, ?, ?, ?, ?)",
        token.0,
        log.amps,
        log.volts,
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
    mut db: Connection<Logs>,
    page: Option<i32>,
    count: Option<i32>,
    token: ValidDbToken,
    _ratelimit: RocketGovernor<'_, RateLimitGuard>,
) -> String {
    let page = page.unwrap_or(0);
    let count = count.unwrap_or(10);

    let (rows, has_next) = get_paginated_rows_for_token(&mut db, &token, page, count).await;

    let mut result = String::new();
    result.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\"/><title>Consumption info</title></head><body><table>");
    result.push_str(
        "<tr><th>Location (token id)</th><th>Amps</th><th>Volts</th><th>Watts</th></tr>\n",
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

    result
}

#[get("/log/<_>/json?<page>&<count>", rank = 1)]
async fn list_table_json(
    mut db: Connection<Logs>,
    page: Option<i32>,
    count: Option<i32>,
    token: ValidDbToken,
    _ratelimit: RocketGovernor<'_, RateLimitGuard>,
) -> String {
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

    result
}

#[get("/")]
async fn index(_ratelimit: RocketGovernor<'_, RateLimitGuard>) -> String {
    log::info!("Got to index!");
    "PONG".to_string()
}

#[launch]
async fn rocket() -> _ {
    rocket::build()
        .attach(Logs::init())
        .attach(fairing::AdHoc::on_ignite("Setup DB", |rocket| async {
            let db = Logs::fetch(&rocket).expect("DB connection");
            sqlx::migrate!("./migrations").run(&**db).await.unwrap();
            rocket
        }))
        .mount(
            "/",
            routes![index, list_table_html, list_table_json, post_token],
        )
        .register("/", catchers![rocket_governor_catcher])
}
