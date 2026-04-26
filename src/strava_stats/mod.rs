pub mod error;
pub mod models;
pub mod routes;
pub mod services;
pub mod state;

use axum::response::Html;
use axum::Router;
use axum::routing::get;
use state::{AppState, SharedState};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

const INDEX_HTML: &str = include_str!("../../static/strava_stats/index.html");

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

pub fn router() -> Router {
    let state: SharedState = Arc::new(RwLock::new(AppState {
        activities: vec![],
    }));

    Router::new()
        .route("/", get(index))
        .nest("/api", routes::router())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
