use rocket_db_pools::Connection;


// The second argument is a private unit struct, which is used to ensure that
// the token can only be created by the `FromRequest` implementation.
pub struct ValidDbToken(pub String, ());

enum RequestTokenDbResult {
    Ok(ValidDbToken),
    NotFound,
}

pub fn simplify_token(token: &str) -> String {
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
        let result = request.local_cache_async(async {
            let mut db = request.guard::<Connection<crate::Logs>>().await.expect("Failed to get db connection");
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
                        return RequestTokenDbResult::NotFound;
                    }
                    RequestTokenDbResult::Ok(ValidDbToken(token, ()))
                }
                _ => {
                    log::info!("No token found");
                    RequestTokenDbResult::NotFound
                }
            }
        }).await;

        match result {
            RequestTokenDbResult::Ok(token) => rocket::request::Outcome::Success(token),
            RequestTokenDbResult::NotFound => rocket::request::Outcome::Forward(rocket::http::Status::NotFound),
        }
    }
}