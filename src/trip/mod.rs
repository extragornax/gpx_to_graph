pub mod auth;
pub mod db;
pub mod routes;

use std::sync::Arc;
use axum::Router;

pub struct AppState {
    pub db: db::Db,
}

pub type SharedState = Arc<AppState>;

pub fn router(state: SharedState) -> Router {
    routes::router().with_state(state)
}
