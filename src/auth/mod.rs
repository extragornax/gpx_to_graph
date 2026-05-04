pub mod db;
mod handlers;

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;

pub type AuthState = Arc<db::Db>;

pub struct CurrentUser {
    pub id: i64,
    pub username: String,
}

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth: &AuthState = parts.extensions.get()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        let cookie = parts.headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let token = cookie.split(';')
            .filter_map(|s| s.trim().strip_prefix("session="))
            .next()
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let user_id = auth.get_session(token)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let username = auth.get_username(user_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::UNAUTHORIZED)?;

        Ok(CurrentUser { id: user_id, username })
    }
}

pub struct OptionalUser(pub Option<CurrentUser>);

impl<S: Send + Sync> FromRequestParts<S> for OptionalUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match CurrentUser::from_request_parts(parts, state).await {
            Ok(u) => Ok(OptionalUser(Some(u))),
            Err(_) => Ok(OptionalUser(None)),
        }
    }
}

pub fn router() -> Router {
    Router::new()
        .route("/", get(handlers::login_page))
        .route("/api/challenge", get(handlers::challenge))
        .route("/api/register", post(handlers::register))
        .route("/api/login", post(handlers::login))
        .route("/api/logout", post(handlers::logout))
        .route("/api/me", get(handlers::me))
}

pub fn build_state(db_path: &str) -> AuthState {
    let db = db::Db::open(db_path).expect("failed to open auth db");
    db.migrate().expect("failed to migrate auth db");
    Arc::new(db)
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    Ok(bcrypt::hash(password, bcrypt::DEFAULT_COST)?)
}

pub fn verify_password(password: &str, hash: &str) -> anyhow::Result<bool> {
    Ok(bcrypt::verify(password, hash)?)
}

pub fn generate_session_token() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
