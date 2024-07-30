use rocket_db_pools::Connection;


pub struct ValidDbToken(pub String);

pub fn simplify_token(token: &str) -> String {
    let mut result = String::new();
    result.push_str(&token[..4]);
    result.push_str("...");
    result.push_str(&token[token.len() - 4..]);
    result
}

#[rocket::async_trait]
impl<'r> rocket::request::FromRequest<'r> for ValidDbToken {
    type Error = ();

    async fn from_request(
        request: &'r rocket::Request<'_>,
    ) -> rocket::request::Outcome<Self, Self::Error> {
        let mut db = request.guard::<Connection<crate::Logs>>().await.unwrap();

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