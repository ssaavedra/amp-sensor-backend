//! Custom form fields for Rocket

use std::str::FromStr;

use chrono::NaiveDateTime;
use chrono::TimeZone;

/// Custom form field to parse a datetime string
/// in the format of %Y-%m-%dT%H:%M:%S
pub struct ParseableDateTime(pub chrono::DateTime<chrono::Utc>);

impl<'r> rocket::form::FromFormField<'r> for ParseableDateTime {
    fn from_value(field: rocket::form::ValueField<'r>) -> rocket::form::Result<'r, Self> {
        let value = field.value.to_string();
        let datetime = NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S")
            .map(|dt| dt.and_utc())
            .map(ParseableDateTime);

        datetime.map_err(|_| {
            let mut errors = rocket::form::Errors::new();
            errors.push(rocket::form::Error::validation(format!(
                "Invalid datetime: {}",
                value
            )));
            errors
        })
    }
}

impl core::ops::Deref for ParseableDateTime {
    type Target = chrono::DateTime<chrono::Utc>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Default)]
pub struct Tz(pub chrono_tz::Tz);

impl<'r> rocket::form::FromFormField<'r> for Tz {
    fn from_value(field: rocket::form::ValueField<'r>) -> rocket::form::Result<'r, Self> {
        let value = field.value.to_string();
        let tz = match value.as_str() {
            // Try to parse as "Europe/Paris, America/New_York, etc"
            s if s.contains('/') => {
                chrono_tz::Tz::from_str(s).ok().unwrap_or(chrono_tz::Tz::UTC)
            },
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