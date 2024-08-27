//! Custom form fields for Rocket

use std::str::FromStr;

use chrono::NaiveDateTime;
use chrono::TimeZone;

/// Custom form field to parse a datetime string in the format of %Y-%m-%dT%H:%M
/// for [datetime-local inputs](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/input/datetime-local)
/// 
/// This allows some ergonomic features like setting the timezone and default value
/// directly from the form field as a monad
pub enum HtmlInputParseableDateTime {
    Naive(Option<chrono::NaiveDateTime>),
    WithTz(Option<chrono::DateTime<chrono_tz::Tz>>),
}

impl HtmlInputParseableDateTime {
    /// Check if the datetime is set
    pub fn is_some(&self) -> bool {
        match self {
            HtmlInputParseableDateTime::Naive(Some(_)) => true,
            HtmlInputParseableDateTime::WithTz(Some(_)) => true,
            _ => false,
        }
    }

    /// Check if the datetime is not set
    pub fn is_none(&self) -> bool {
        !self.is_some()
    }

    /// Set the local timezone for the datetime
    pub fn with_tz(&self, tz: chrono_tz::Tz, earliest: bool) -> Self {
        let unwrap_date = move |dt: chrono::LocalResult<chrono::DateTime<chrono_tz::Tz>>| {
            if earliest {
                dt.earliest()
            } else {
                dt.latest()
            }
        };

        match self {
            HtmlInputParseableDateTime::Naive(Some(dt)) => {
                HtmlInputParseableDateTime::WithTz(unwrap_date(tz.from_local_datetime(dt) ))
            }
            HtmlInputParseableDateTime::Naive(None) => HtmlInputParseableDateTime::WithTz(None),
            HtmlInputParseableDateTime::WithTz(dt) => HtmlInputParseableDateTime::WithTz(dt.as_ref().map(|dt| tz.from_utc_datetime(&dt.naive_utc()))),
        }
    }

    /// Set the default value for the datetime if the input is empty
    /// 
    /// This value uses chrono::Utc as the timezone instead of chrono_tz::Tz to
    /// make it more ergonomic to use with chrono's built-in functions in order
    /// to make timedeltas from "now" from the user perspective
    pub fn with_default(self, default: chrono::DateTime<chrono::Utc>) -> Self {
        match self {
            HtmlInputParseableDateTime::Naive(None) => HtmlInputParseableDateTime::WithTz(Some(default.with_timezone(&chrono_tz::UTC))),
            HtmlInputParseableDateTime::WithTz(None) => HtmlInputParseableDateTime::WithTz(Some(default.with_timezone(&chrono_tz::UTC))),
            _ => self,
        }
    }

    /// Get the datetime in UTC timezone
    /// 
    /// Returns the value as a chrono::Utc instead of chrono_tz::Tz to make it
    /// more ergonomic to use with chrono's built-in functions
    pub fn utc(&self) -> chrono::DateTime<chrono::Utc> {
        match self {
            HtmlInputParseableDateTime::Naive(Some(dt)) => chrono::Utc.from_utc_datetime(dt),
            HtmlInputParseableDateTime::WithTz(Some(dt)) => dt.with_timezone(&chrono::Utc),
            HtmlInputParseableDateTime::Naive(None) => chrono::Utc.from_utc_datetime(&chrono::Utc::now().naive_utc()),
            HtmlInputParseableDateTime::WithTz(None) => chrono::Utc.from_utc_datetime(&chrono::Utc::now().naive_utc()),
        }
    }

    /// Get the datetime in the local timezone
    pub fn local(&self) -> chrono::DateTime<chrono_tz::Tz> {
        match self {
            HtmlInputParseableDateTime::WithTz(Some(dt)) => *dt,
            _ => self.utc().with_timezone(&chrono_tz::UTC),
        }
    }

    pub fn to_datetime_local(&self) -> String {
        match self {
            HtmlInputParseableDateTime::Naive(Some(dt)) => dt.format("%Y-%m-%dT%H:%M").to_string(),
            HtmlInputParseableDateTime::WithTz(Some(dt)) => dt.format("%Y-%m-%dT%H:%M").to_string(),
            _ => "".to_string(),
        }
    }
}


impl<'r> rocket::form::FromFormField<'r> for HtmlInputParseableDateTime
{
    fn from_value(field: rocket::form::ValueField<'r>) -> rocket::form::Result<'r, Self> {
        let value = field.value.to_string();
        let datetime = NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M")
            .map(Some)
            .map(HtmlInputParseableDateTime::Naive);

        datetime.map_err(|_| {
            let mut errors = rocket::form::Errors::new();
            errors.push(rocket::form::Error::validation(format!(
                "Invalid datetime: {}",
                value
            )));
            errors
        })
    }
    fn default() -> Option<Self> {
        Some(HtmlInputParseableDateTime::Naive(None))
    }
}


#[derive(Default)]
pub struct Tz(pub chrono_tz::Tz);

impl<'r> rocket::form::FromFormField<'r> for Tz {
    fn from_value(field: rocket::form::ValueField<'r>) -> rocket::form::Result<'r, Self> {
        let value = field.value.to_string();
        let tz = match value.as_str() {
            // Try to parse as "Europe/Paris, America/New_York, etc"
            s if s.contains('/') => chrono_tz::Tz::from_str(s)
                .ok()
                .unwrap_or(chrono_tz::Tz::UTC),
            _ => chrono_tz::Tz::UTC,
        };

        Ok(Tz(tz))
    }

    fn default() -> Option<Self> {
        Some(Tz(chrono_tz::Tz::UTC))
    }
}

impl core::ops::Deref for Tz {
    type Target = chrono_tz::Tz;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
