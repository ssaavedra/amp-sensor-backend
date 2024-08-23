
/// A struct to hold the returned rows from the DB
/// 
/// With the implementations of `AddAssign`, `Sum` and `Div<f64>`, we can easily
/// sum up the rows and divide them by a number.
/// 
/// This allows us to calculate an average of amps, volts and watts while
/// respecting the other fields' contents.
#[derive(Default, Debug)]
pub(super) struct DbRow {
    pub token: String,
    pub amps: f64,
    pub volts: f64,
    pub watts: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub user_agent: String,
    pub client_ip: String,
}

impl DbRow {
    pub fn new(
        token: String,
        amps: f64,
        volts: f64,
        watts: f64,
        created_at: chrono::NaiveDateTime,
        user_agent: &Option<String>,
        client_ip: &Option<String>,
    ) -> Self {
        Self {
            token,
            amps,
            volts,
            watts,
            created_at: created_at.and_utc(),
            user_agent: user_agent.clone().unwrap_or_default(),
            client_ip: client_ip.clone().unwrap_or_default(),
        }
    }
}


impl std::ops::AddAssign for DbRow {
    /// Add the amps, volts and watts of another `DbRow` to this one, keeping the
    /// other fields as they are.
    fn add_assign(&mut self, other: Self) {
        self.amps += other.amps;
        self.volts += other.volts;
        self.watts += other.watts;
    }
}

impl core::iter::Sum for DbRow {
    /// Sum up the amps, volts and watts of all `DbRow`s in the iterator, keeping
    /// the other fields as they are.
    /// 
    /// If the iterator is empty, a default `DbRow` is returned, which will not
    /// have a token, user_agent or client_ip.
    fn sum<I>(mut iter: I) -> Self
    where
        I: Iterator<Item = Self>,
    {
        let zero = iter.next().unwrap_or_default();
        iter.fold(zero, |mut acc, x| {
            acc += x;
            acc
        })
    }
}

impl std::ops::Div<f64> for DbRow {
    type Output = Self;

    /// Divide the amps, volts and watts of this `DbRow` by a number, keeping the
    /// other fields as they are.
    fn div(self, rhs: f64) -> Self {
        DbRow {
            token: self.token,
            amps: self.amps / rhs,
            volts: self.volts / rhs,
            watts: self.watts / rhs,
            created_at: self.created_at,
            user_agent: self.user_agent,
            client_ip: self.client_ip,
        }
    }
}