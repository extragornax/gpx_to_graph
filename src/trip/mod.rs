pub mod db;
pub mod routes;

use axum::{Router, extract::DefaultBodyLimit};
use std::sync::Arc;

pub struct AppState {
    pub db: db::Db,
}

pub type SharedState = Arc<AppState>;

pub fn router(state: SharedState) -> Router {
    routes::router()
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .with_state(state)
}
