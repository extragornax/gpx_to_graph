pub mod db;
pub mod routes;
pub mod session;

use std::sync::Arc;
use axum::Router;
use tower_http::services::ServeDir;

pub struct AppState {
    pub db: db::Db,
    pub channels: session::Channels,
}

pub type SharedState = Arc<AppState>;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .nest("/api", routes::api_router())
        .fallback_service(ServeDir::new("static/trace"))
        .with_state(state)
}
