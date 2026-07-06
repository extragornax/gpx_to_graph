pub mod gpx_parse;
mod handlers;
pub mod weather;
pub mod wind;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use handlers::AppState;
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use weather::WeatherCache;

pub fn router(cache: Arc<WeatherCache>, shares: Arc<crate::share::ShareStore>) -> Router {
    let state = AppState { cache };
    Router::new()
        .route("/", get(handlers::index))
        .route("/api/analyze", post(handlers::analyze))
        .with_state(state)
        .merge(crate::share::routes("meteo", shares))
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
}
