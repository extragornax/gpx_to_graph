pub mod db;
pub mod engine;
mod handlers;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{Router, extract::DefaultBodyLimit, routing::{get, post}};
use tower_http::trace::TraceLayer;

pub use handlers::{AppState, SharedState};

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(handlers::index))
        .route("/daily", get(handlers::daily_page))
        .route("/api/generate", post(handlers::generate_handler))
        .route("/api/avoid/upload", post(handlers::avoid_upload))
        .route("/api/daily", get(handlers::daily_handler))
        .route("/api/geocode", get(handlers::geocode_handler))
        .with_state(state)
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
}

pub fn build_state(db_conn: rusqlite::Connection, brouter_url: String) -> SharedState {
    let needs_rate_limit = brouter_url.contains("brouter.de");
    Arc::new(AppState {
        db: Mutex::new(db_conn),
        http_client: reqwest::Client::new(),
        brouter_url,
        needs_rate_limit,
        geocode_cache: Mutex::new(HashMap::new()),
    })
}
