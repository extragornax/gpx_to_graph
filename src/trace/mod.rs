pub mod db;
pub mod routes;
pub mod session;

use std::sync::Arc;
use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;

pub struct AppState {
    pub db: db::Db,
    pub channels: session::Channels,
}

pub type SharedState = Arc<AppState>;

const INDEX_HTML: &str = include_str!("../../static/trace/index.html");
const TRACKER_HTML: &str = include_str!("../../static/trace/tracker.html");
const VIEW_HTML: &str = include_str!("../../static/trace/view.html");
const MANIFEST_JSON: &str = include_str!("../../static/trace/manifest.json");
const SW_JS: &str = include_str!("../../static/trace/sw.js");
const ICON_192: &[u8] = include_bytes!("../../static/trace/icon-192.png");
const ICON_512: &[u8] = include_bytes!("../../static/trace/icon-512.png");

async fn index() -> impl IntoResponse { axum::response::Html(INDEX_HTML) }
async fn tracker() -> impl IntoResponse { axum::response::Html(TRACKER_HTML) }
async fn view() -> impl IntoResponse { axum::response::Html(VIEW_HTML) }
async fn manifest() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/manifest+json")], MANIFEST_JSON)
}
async fn sw() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], SW_JS)
}
async fn icon_192() -> impl IntoResponse {
    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], ICON_192)
}
async fn icon_512() -> impl IntoResponse {
    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], ICON_512)
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/tracker.html", get(tracker))
        .route("/view.html", get(view))
        .route("/manifest.json", get(manifest))
        .route("/sw.js", get(sw))
        .route("/icon-192.png", get(icon_192))
        .route("/icon-512.png", get(icon_512))
        .nest("/api", routes::api_router())
        .with_state(state)
}
