//! A simple module to help print the energy log table in HTML and JSON format.
//!
//! This module contains the [RowInfo] struct that represents a row in the energy
//! log table. It also contains the [get_paginated_rows_for_token] function that
//! retrieves the rows from the database for a given token and page.
//!
//! The rows are returned as a vector of [RowInfo] structs, and a boolean that
//! indicates if there are more rows to be fetched.

use chrono::NaiveDateTime;
use rocket_db_pools::Connection;
use serde::Serialize;

use crate::token::{DbToken, Token, ValidDbToken};

pub struct RowInfo {
    location: String,
    token: DbToken,
    datetime: String,
    ua: String,
    amps: f64,
    volts: f64,
    watts: f64,
}

impl Serialize for RowInfo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_json().serialize(serializer)
    }
}

impl RowInfo {
    fn new(
        location: &str,
        token: DbToken,
        datetime: &str,
        ua: &str,
        amps: f64,
        volts: f64,
        watts: f64,
    ) -> Self {
        Self {
            location: location.to_string(),
            token,
            datetime: datetime.to_string(),
            ua: ua.to_string(),
            amps,
            volts,
            watts,
        }
    }

    /// Returns the row as an HTML table row
    pub fn to_html(&self) -> String {
        format!(
            "<tr><td>{} ({}/{})</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            self.location,
            self.token.simplified(),
            self.ua,
            self.datetime,
            self.amps,
            self.volts,
            self.watts
        )
    }

    /// Returns the row as a JSON object
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "location": self.location,
            "token": self.token.full_token(),
            "datetime": self.datetime,
            "amps": self.amps,
            "volts": self.volts,
            "watts": self.watts
        })
    }
}

/// Returns the rows from the database for a given token and page as tuple with
/// a vector of [RowInfo] structs and a boolean that indicates if there are more
/// rows to be fetched.
pub async fn get_paginated_rows_for_token(
    db: &mut Connection<crate::Logs>,
    token: &ValidDbToken,
    page: i32,
    count: i32,
) -> (Vec<RowInfo>, bool) {
    let mut rows = Vec::new();
    let offset = page * count;
    let db_count = count + 1; // Fetch one more row to check if there are still more rows

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
        token,
        db_count,
        offset
    )
    .fetch_all(&mut ***db)
    .await
    .unwrap();

    // Return only the rows that the user requested
    let db_rows_split = if db_rows.len() > count as usize {
        &db_rows[..count as usize]
    } else {
        &db_rows
    };

    for row in db_rows_split {
        let ua = row
            .user_agent
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("Unknown");
        rows.push(RowInfo::new(
            &row.location,
            DbToken(row.token.to_string()),
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

pub fn to_svg_plot(rows: Vec<RowInfo>) -> String {
    use poloto::build;

    let first_timestamp = NaiveDateTime::parse_from_str(
        &rows.first().unwrap().datetime, "%Y-%m-%d %H:%M:%S",
    ).expect("DateTime format failed").and_utc().timestamp();

    let amps: Vec<(i128, i128)> = rows.iter().map(|r| (NaiveDateTime::parse_from_str(
        &r.datetime, "%Y-%m-%d %H:%M:%S",
    ).unwrap().and_utc().timestamp() as i128, (r.amps * 1000.0) as i128)).collect::<Vec<_>>();
    let iter = amps.iter();

    let p = poloto::plots!(
        poloto::build::plot("amps").line(build::cloned(iter)),
    );

    let hr = {
            // Calculate so that we don't overflow the labels
            30 * 60 * (amps.len() as f64 / 2000.0).ceil() as i128
    };

    let xticks =
    poloto::ticks::TickDistribution::new(std::iter::successors(Some(0), |w| Some(w + hr)))
        .with_tick_fmt(|&v| {
            format!("{}", chrono::DateTime::<chrono::Utc>::from_timestamp(v as i64, 0).unwrap().format("%H:%M"))
        });

    let data = poloto::frame_build().data(p).map_xticks(|_| xticks);

    println!("First and last timetamps are: {:?} {:?}", chrono::DateTime::<chrono::Utc>::from_timestamp(first_timestamp, 0).unwrap().format("%H:%M"), chrono::DateTime::<chrono::Utc>::from_timestamp(amps.last().unwrap().0 as i64, 0).unwrap().format("%H:%M"));

    data.build_and_label(("Amps over time", "Time", "Amps")).append_to(poloto::header().light_theme()).render_string().expect("Failed to render SVG")
}
