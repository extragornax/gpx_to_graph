mod handlers;
pub mod nearby;

use axum::{Router, routing::get};
use handlers::AppState;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::ravito::overpass::OverpassCache;

pub fn router(cache: Arc<OverpassCache>) -> Router {
    let state = AppState { cache };
    Router::new()
        .route("/", get(handlers::index))
        .route("/api/nearby", get(handlers::nearby))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
