pub mod db;
pub mod routes;

use std::sync::Arc;
use axum::{Router, extract::DefaultBodyLimit};

pub struct AppState {
    pub db: db::Db,
}

pub type SharedState = Arc<AppState>;

pub fn router(state: SharedState) -> Router {
    routes::router()
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .with_state(state)
}
