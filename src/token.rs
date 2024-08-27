use rocket_db_pools::Connection;
use sqlx::{Encode, Type};

pub trait Token {
    fn full_token<'a>(&'a self) -> &'a str;
    fn simplified(&self) -> String {
        simplify_token_string(self.full_token())
    }
}

/// This struct is used to store a token. This token is not validated in any
/// way. If you need a valid token, use [`ValidDbToken`] in a rocket guard
/// instead.
pub struct DbToken(pub String);

impl Token for DbToken {
    fn full_token<'a>(&'a self) -> &'a str {
        &self.0
    }
}

/// This struct is used to store the token that is passed in the URL.
///
/// The second argument is a private unit struct, which is used to statically
/// ensure that the token can only be created by its `FromRequest`
/// implementation.
pub struct ValidDbToken(pub DbToken, ());

impl Token for ValidDbToken {
    fn full_token<'a>(&'a self) -> &'a str {
        self.0.full_token()
    }
}

impl<DB: sqlx::Database> Type<DB> for DbToken
where
    std::string::String: Type<DB>,
{
    fn type_info() -> <DB as sqlx::Database>::TypeInfo {
        <String as Type<DB>>::type_info()
    }
    fn compatible(ty: &<DB as sqlx::Database>::TypeInfo) -> bool {
        <String as Type<DB>>::compatible(ty)
    }
}

impl<DB: sqlx::Database> Type<DB> for ValidDbToken
where
    std::string::String: Type<DB>,
{
    fn type_info() -> <DB as sqlx::Database>::TypeInfo {
        <String as Type<DB>>::type_info()
    }
    fn compatible(ty: &<DB as sqlx::Database>::TypeInfo) -> bool {
        <String as Type<DB>>::compatible(ty)
    }
}

impl<'a, DB: sqlx::Database> Encode<'a, DB> for DbToken
where
    std::string::String: Encode<'a, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as sqlx::database::HasArguments<'a>>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
        self.0.encode_by_ref(buf)
    }
}

impl<'a, DB: sqlx::Database> Encode<'a, DB> for ValidDbToken
where
    std::string::String: Encode<'a, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as sqlx::database::HasArguments<'a>>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
        self.0.encode_by_ref(buf)
    }
}

impl std::fmt::Display for DbToken {
    /// User-facing display of the token, showing only the first and last 4
    /// characters.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}>", self.simplified())
    }
}

impl std::fmt::Display for ValidDbToken {
    /// User-facing display of the token, showing only the first and last 4
    /// characters.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// This struct is used to store a view-token passed in the URL.
///
/// The second argument is a private unit struct, which is used to statically
/// ensure that the token can only be created by its `FromRequest`
/// implementation.
pub struct ValidViewToken(pub DbToken, ());

impl Token for ValidViewToken {
    fn full_token<'a>(&'a self) -> &'a str {
        self.0.full_token()
    }
}

impl<DB: sqlx::Database> Type<DB> for ValidViewToken
where
    std::string::String: Type<DB>,
{
    fn type_info() -> <DB as sqlx::Database>::TypeInfo {
        <String as Type<DB>>::type_info()
    }
    fn compatible(ty: &<DB as sqlx::Database>::TypeInfo) -> bool {
        <String as Type<DB>>::compatible(ty)
    }
}

impl<'a, DB: sqlx::Database> Encode<'a, DB> for ValidViewToken
where
    std::string::String: Encode<'a, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as sqlx::database::HasArguments<'a>>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
        self.0.encode_by_ref(buf)
    }
}

impl std::fmt::Display for ValidViewToken {
    /// User-facing display of the token, showing only the first and last 4
    /// characters.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{}>", self.0.simplified())
    }
}

/// This function returns a cleaned up version of the token, showing only the
/// first and last 4 characters.
pub fn simplify_token_string(token: &str) -> String {
    let mut result = String::new();
    result.push_str(&token[..4]);
    result.push_str("...");
    result.push_str(&token[token.len() - 4..]);
    result
}

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for &'r ValidDbToken {
    type Error = ();

    async fn from_request(
        request: &'r rocket::Request<'_>,
    ) -> rocket::request::Outcome<Self, Self::Error> {
        let result = request
            .local_cache_async(async {
                let mut db = request
                    .guard::<Connection<crate::Logs>>()
                    .await
                    .expect("Failed to get db connection");
                let token = request.routed_segment(1).map(|s| s.to_string());
                match token {
                    Some(token) => {
                        let rows = sqlx::query!(
                            "SELECT COUNT(*) as count FROM tokens WHERE token = ?",
                            token
                        );
                        let count = rows.fetch_one(&mut **db).await.unwrap().count;
                        log::info!("Token count in DB: {}", count);
                        if count == 0 {
                            return None;
                        }
                        Some(ValidDbToken(DbToken(token), ()))
                    }
                    _ => {
                        log::info!("No token found");
                        None
                    }
                }
            })
            .await;

        match result {
            Some(token) => rocket::request::Outcome::Success(token),
            None => rocket::request::Outcome::Forward(rocket::http::Status::NotFound),
        }
    }
}


#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for &'r ValidViewToken {
    type Error = ();

    async fn from_request(
        request: &'r rocket::Request<'_>,
    ) -> rocket::request::Outcome<Self, Self::Error> {
        let result = request
            .local_cache_async(async {
                let mut db = request
                    .guard::<Connection<crate::Logs>>()
                    .await
                    .expect("Failed to get db connection");
                let token = request.routed_segment(1).map(|s| s.to_string());
                match token {
                    Some(token) => {
                        let rows = sqlx::query!(
                            "SELECT COUNT(*) as count FROM view_tokens WHERE token = ? AND (view_token_valid_until is null OR view_token_valid_until > datetime(\"NOW\"))",
                            token
                        );
                        let count = rows.fetch_one(&mut **db).await.unwrap().count;
                        log::info!("Token count in DB: {}", count);
                        if count == 0 {
                            return None;
                        }
                        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
                        // Update last accessed time
                        sqlx::query!(
                            "UPDATE view_tokens SET last_accessed_at = ? WHERE token = ?",
                            now,
                            token
                        ).execute(&mut **db).await.unwrap();
                        Some(ValidViewToken(DbToken(token), ()))
                    }
                    _ => {
                        log::info!("No token found");
                        None
                    }
                }
            })
            .await;

        match result {
            Some(token) => rocket::request::Outcome::Success(token),
            None => rocket::request::Outcome::Forward(rocket::http::Status::NotFound),
        }
    }
}
