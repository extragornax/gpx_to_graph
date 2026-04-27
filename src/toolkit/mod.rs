use axum::{Router, routing::get, response::Html};

const INDEX_HTML: &str = include_str!("../../static/toolkit/index.html");
const APP_CSS: &str = include_str!("../../static/toolkit/app.css");

async fn index() -> Html<String> {
    Html(INDEX_HTML.replace("<!-- CSS_PLACEHOLDER -->", &format!("<style>{}</style>", APP_CSS)))
}

pub fn router() -> Router {
    Router::new().route("/", get(index))
}
