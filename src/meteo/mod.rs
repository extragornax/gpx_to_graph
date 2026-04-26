pub mod gpx_parse;
mod handlers;
pub mod weather;
pub mod wind;

use std::sync::Arc;
use axum::{Router, extract::DefaultBodyLimit, routing::{get, post}};
use tower_http::trace::TraceLayer;
use handlers::AppState;
use weather::WeatherCache;

pub fn router(cache: Arc<WeatherCache>) -> Router {
    let state = AppState { cache };
    Router::new()
        .route("/", get(handlers::index))
        .route("/static/app.css", get(handlers::app_css))
        .route("/api/analyze", post(handlers::analyze))
        .with_state(state)
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
}
