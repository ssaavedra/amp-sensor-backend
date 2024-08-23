//! A simple module to help print the energy log table in HTML and JSON format.
//! 
//! This module contains the [RowInfo] struct that represents a row in the energy
//! log table. It also contains the [get_paginated_rows_for_token] function that
//! retrieves the rows from the database for a given token and page.
//! 
//! The rows are returned as a vector of [RowInfo] structs, and a boolean that
//! indicates if there are more rows to be fetched.

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
        let ua = row.user_agent.as_ref().map(|s| s.as_str()).unwrap_or("Unknown");
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
