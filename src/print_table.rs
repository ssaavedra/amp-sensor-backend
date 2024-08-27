//! A simple module to help print the energy log table in HTML and JSON format.
//!
//! This module contains the [RowInfo] struct that represents a row in the energy
//! log table. It also contains the [get_paginated_rows_for_token] function that
//! retrieves the rows from the database for a given token and page.
//!
//! The rows are returned as a vector of [RowInfo] structs, and a boolean that
//! indicates if there are more rows to be fetched.

use chrono::{DateTime, NaiveDateTime};
use rocket_db_pools::Connection;
use serde::Serialize;

use crate::{
    form::HtmlInputParseableDateTime,
    token::{DbToken, Token, ValidViewToken},
};

pub struct Pagination {
    pub page: Option<i32>,
    pub count: Option<i32>,
    pub start: HtmlInputParseableDateTime,
    pub end: HtmlInputParseableDateTime,
    pub tz: chrono_tz::Tz,
    pub interval: Option<i32>,
}

pub struct PaginationResult {
    pub page: i32,
    pub count: i32,
    pub start: DateTime<chrono::Utc>,
    pub end: DateTime<chrono::Utc>,
    pub interval: i32,
    pub offset: i32,
}

impl Pagination {
    pub fn result(&self) -> PaginationResult {
        let page = self.page.unwrap_or(1);
        let default_count = {
            if self.start.is_some() && self.end.is_some() {
                10000000
            } else {
                10
            }
        };
        let count = self.count.unwrap_or(default_count);
        let start = self
            .start
            .with_tz(self.tz, true)
            .with_default(chrono::Utc::now() - chrono::Duration::days(1))
            .utc();
        let end = self
            .end
            .with_tz(self.tz, false)
            .with_default(chrono::Utc::now())
            .utc();
        let interval = self.interval.unwrap_or(300);
        let offset = (page - 1) * count;

        PaginationResult {
            page,
            count,
            start,
            end,
            interval,
            offset,
        }
    }
}

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
        datetime: &chrono::NaiveDateTime,
        tz: &chrono_tz::Tz,
        ua: &str,
        amps: f64,
        volts: f64,
        watts: f64,
    ) -> Self {
        Self {
            location: location.to_string(),
            token,
            datetime: datetime.and_utc().with_timezone(tz).to_string(),
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
    token: &ValidViewToken,
    pagination: &PaginationResult,
    tz: &chrono_tz::Tz,
) -> (Vec<RowInfo>, bool) {
    let mut rows = Vec::new();
    let PaginationResult {
        page: _,
        interval: _,
        count,
        start,
        end,
        offset,
    } = pagination;
    let count = *count;
    let offset = *offset;
    let db_count = count + 1;
    let start = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end = end.format("%Y-%m-%d %H:%M:%S").to_string();

    let db_rows = sqlx::query!(
        "SELECT amps, volts, watts, energy_log.created_at as created_at, user_agent, client_ip, energy_log.token as token, u.location as location 
        FROM energy_log
        INNER JOIN tokens t
        ON t.token = energy_log.token
        INNER JOIN users u
        ON u.id = t.user_id
        INNER JOIN view_tokens vt
        ON vt.user_id = u.id
        WHERE vt.token = ?
        AND energy_log.created_at BETWEEN ? AND ?
        ORDER BY created_at DESC
        LIMIT ?
        OFFSET ?",
        token,
        start,
        end,
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
            &row.created_at,
            tz,
            ua,
            row.amps,
            row.volts,
            row.watts,
        ));
    }
    let has_next = db_rows.len() > count as usize;

    (rows, has_next)
}

/// Returns the rows from the database for a given token and page as tuple with
/// a vector of [RowInfo] structs between the given timestamps. It returns two
/// vectors: one with the averages and one with the maximums given the window
/// interval passed as a parameter.
pub async fn get_avg_max_rows_for_token<Tz: chrono::TimeZone>(
    db: &mut Connection<crate::Logs>,
    token: &ValidViewToken,
    start: &DateTime<Tz>,
    end: &DateTime<Tz>,
    interval: i32,
) -> (Vec<RowInfo>, Vec<RowInfo>) {
    let mut rows = Vec::new();
    let mut max_rows = Vec::new();
    let start = start.naive_utc();
    let end = end.naive_utc();

    let db_rows = sqlx::query!(
        "SELECT AVG(amps) as amps, MAX(amps) as max_amps, AVG(volts) as volts, AVG(watts) as watts, MAX(watts) as max_watts, energy_log.created_at as created_at, user_agent, client_ip, energy_log.token as token, u.location as location 
        FROM energy_log
        INNER JOIN tokens t
        ON t.token = energy_log.token
        INNER JOIN users u
        ON u.id = t.user_id
        INNER JOIN view_tokens vt
        ON vt.user_id = u.id
        WHERE vt.token = ? AND energy_log.created_at BETWEEN ? AND ?
        GROUP BY strftime('%s', energy_log.created_at) / ?
        ORDER BY created_at DESC",
        token,
        start,
        end,
        interval
    )
    .fetch_all(&mut ***db)
    .await
    .unwrap();

    for row in db_rows {
        let ua = row
            .user_agent
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("Unknown");
        match (row.location.clone(), row.token.clone(), row.created_at) {
            (Some(location), Some(token), Some(created_at)) => {
                rows.push(RowInfo::new(
                    &location,
                    DbToken(token.to_string()),
                    &created_at,
                    &chrono_tz::UTC,
                    ua,
                    row.amps,
                    row.volts,
                    row.watts,
                ));
                max_rows.push(RowInfo::new(
                    &location,
                    DbToken(token.to_string()),
                    &created_at,
                    &chrono_tz::UTC,
                    ua,
                    row.max_amps,
                    row.volts,
                    row.max_watts,
                ));
            }
            (_, _, _) => {
                log::warn!("Location is None for row {:?}", row);
            }
        }
    }

    (rows, max_rows)
}

fn datetime_to_timestamp(datetime: &str) -> f64 {
    NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d %H:%M:%S %Z")
        .expect("DateTime format failed")
        .and_utc()
        .timestamp() as f64
}

/// Create an error type for to_svg_plot when there are no rows to plot
#[derive(Debug)]
pub struct NoRowsError;

impl std::fmt::Display for NoRowsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No rows to plot")
    }
}

impl std::error::Error for NoRowsError {}

pub fn to_svg_plot<TZ: chrono::TimeZone>(
    avg_rows: Vec<RowInfo>,
    max_rows: Vec<RowInfo>,
    tz: &TZ,
) -> anyhow::Result<String>
where
    <TZ as chrono::TimeZone>::Offset: std::fmt::Display,
{
    use poloto::build;

    if avg_rows.len() < 1 {
        return Err(NoRowsError.into());
    }

    let first_timestamp = datetime_to_timestamp(&avg_rows.first().unwrap().datetime);

    let amps: Vec<(f64, f64)> = avg_rows
        .iter()
        .map(|r| (datetime_to_timestamp(&r.datetime), r.amps))
        .collect::<Vec<_>>();
    let iter = amps.iter();

    let p = poloto::plots!(
        poloto::build::plot("max amps").line(build::cloned(
            max_rows
                .iter()
                .map(|r| (datetime_to_timestamp(&r.datetime), r.amps))
        )),
        poloto::build::plot("avg amps").line(build::cloned(iter))
    );

    // Configure ticks so that we don't overflow the labels (i.e., at most 10 labels in total)
    // Calculate last - first and divide by 10 to get the tick interval
    let tick_interval = (amps.last().unwrap().0 - first_timestamp) / 10.0;
    let tick = tick_interval.abs().ceil();

    // Round to the nearest 30 minutes
    let tick = f64::max(3.0, (tick / 1800.0).ceil() * 1800.0);

    let xticks =
        poloto::ticks::TickDistribution::new(std::iter::successors(Some(0.0), |w| Some(w + tick)))
            .with_tick_fmt(|&v| {
                format!(
                    "{}",
                    chrono::DateTime::<chrono::Utc>::from_timestamp(v as i64, 0)
                        .unwrap()
                        .with_timezone(tz)
                        .format("D%d %H:%M")
                )
            });

    let data = poloto::frame()
        .with_viewbox([1400.0, 500.0])
        .build()
        .data(p)
        .map_xticks(|_| xticks);

    data.build_and_label(("Amps over time", "Time", "Amps"))
        .append_to(
            poloto::header()
                .with_dim([1400.0, 500.0])
                .with_viewbox([1400.0, 500.0])
                .light_theme(),
        )
        .render_string()
        .map_err(anyhow::Error::new)
}
