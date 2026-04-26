pub mod auth;
pub mod climb;
pub mod db;
pub mod routes;
pub mod strava;

use std::sync::Arc;
use axum::Router;

pub struct AppState {
    pub db: db::Db,
    pub strava: Option<strava::StravaConfig>,
}

pub type SharedState = Arc<AppState>;

pub fn router(state: SharedState) -> Router {
    routes::router().with_state(state)
}
