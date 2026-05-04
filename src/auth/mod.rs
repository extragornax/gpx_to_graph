pub mod db;
pub mod strava;
mod handlers;

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::Router;

pub struct AuthService {
    pub db: db::Db,
    pub strava_config: Option<strava::StravaConfig>,
}

pub type AuthState = Arc<AuthService>;

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

        let user_id = auth.db.get_session(token)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let username = auth.db.get_username(user_id)
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
        // Strava OAuth
        .route("/strava", get(handlers::strava_redirect))
        .route("/strava/callback", get(handlers::strava_callback))
        .route("/api/strava/status", get(handlers::strava_status))
        .route("/api/strava", delete(handlers::strava_disconnect))
}

pub fn build_state(db_path: &str, strava_config: Option<strava::StravaConfig>) -> AuthState {
    let db = db::Db::open(db_path).expect("failed to open auth db");
    db.migrate().expect("failed to migrate auth db");
    Arc::new(AuthService { db, strava_config })
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

pub async fn ensure_fresh_token(
    auth: &AuthState,
    user_id: i64,
    tokens: &db::StravaTokens,
) -> anyhow::Result<String> {
    let now = chrono::Utc::now().timestamp();
    if now < tokens.expires_at - 60 {
        return Ok(tokens.access_token.clone());
    }
    let config = auth.strava_config.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Strava not configured"))?;
    let refreshed = strava::refresh_token(config, &tokens.refresh_token).await?;
    auth.db.save_strava_tokens(
        user_id, &refreshed.access_token, &refreshed.refresh_token,
        refreshed.expires_at, tokens.athlete_id, tokens.athlete_name.as_deref(),
    )?;
    Ok(refreshed.access_token)
}
