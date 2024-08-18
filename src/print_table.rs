use rocket_db_pools::Connection;

use crate::token::{simplify_token, ValidDbToken};



pub struct RowInfo {
    location: String,
    token: String,
    datetime: String,
    ua: String,
    amps: f64,
    volts: f64,
    watts: f64,
}

impl RowInfo {
    fn new(
        location: &str,
        token: &str,
        datetime: &str,
        ua: &str,
        amps: f64,
        volts: f64,
        watts: f64,
    ) -> Self {
        Self {
            location: location.to_string(),
            token: token.to_string(),
            datetime: datetime.to_string(),
            ua: ua.to_string(),
            amps,
            volts,
            watts,
        }
    }

    pub fn to_html(&self) -> String {
        format!(
            "<tr><td>{} ({}/{})</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            self.location,
            simplify_token(&self.token),
            self.ua,
            self.datetime,
            self.amps,
            self.volts,
            self.watts
        )
    }

    pub fn to_json(&self) -> String {
        format!("{{\"location\": \"{}\", \"token\": \"{}\", \"datetime\": \"{}\", \"amps\": {}, \"volts\": {}, \"watts\": {}}}", self.location, self.token, self.datetime, self.amps, self.volts, self.watts)
    }
}

pub async fn get_paginated_rows_for_token(
    db: &mut Connection<crate::Logs>,
    token: &ValidDbToken,
    page: i32,
    count: i32,
) -> (Vec<RowInfo>, bool) {
    let mut rows = Vec::new();
    let offset = page * count;
    let db_count = count + 1;

    let db_rows = sqlx::query!(
        "SELECT amps, volts, watts, created_at, user_agent, client_ip, energy_log.token as token, u.location as location 
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
        let ua = row.user_agent.as_ref().map(|s| s.as_str()).unwrap_or("Unknown");
        rows.push(RowInfo::new(
            &row.location,
            &row.token,
            &row.created_at.to_string(),
            ua,
            row.amps,
            row.volts,
            row.watts,
        ));
    }
    let has_next = db_rows.len() > count as usize;

    (rows, has_next)
}
