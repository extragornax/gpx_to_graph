use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::Html,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::nearby::{NearbyPoi, rank};
use crate::ravito::overpass::OverpassCache;

#[derive(Clone)]
pub struct AppState {
    pub cache: Arc<OverpassCache>,
}

pub async fn index() -> Html<String> {
    Html(INDEX_HTML.replace(
        "<!-- CSS_PLACEHOLDER -->",
        &format!("<style>{}</style>", APP_CSS),
    ))
}

#[derive(Deserialize)]
pub struct NearbyReq {
    pub lat: f64,
    pub lon: f64,
    /// How far around the rider to look, in metres. Default 2 km.
    #[serde(default)]
    pub radius_m: Option<f64>,
    /// How many of the nearest matches to return. Default 10.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Comma-separated kind names; absent means every kind.
    #[serde(default)]
    pub kinds: Option<String>,
    /// Hide the POIs whose opening hours say they're shut right now.
    #[serde(default)]
    pub open_now: Option<bool>,
}

#[derive(Serialize)]
pub struct NearbyResp {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
    pub limit: usize,
    pub now_unix: i64,
    pub pois: Vec<NearbyPoi>,
}

pub async fn nearby(
    State(state): State<AppState>,
    Query(req): Query<NearbyReq>,
) -> Result<Json<NearbyResp>, (StatusCode, String)> {
    if !(-90.0..=90.0).contains(&req.lat) || !(-180.0..=180.0).contains(&req.lon) {
        return Err((StatusCode::BAD_REQUEST, "lat/lon out of range".into()));
    }
    let radius_m = req.radius_m.unwrap_or(2_000.0).clamp(200.0, 5_000.0);
    let limit = req.limit.unwrap_or(10).clamp(1, 50);
    let open_now = req.open_now.unwrap_or(false);
    let kinds: Option<std::collections::HashSet<String>> = req.kinds.as_ref().map(|s| {
        s.split(',')
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty())
            .collect()
    });

    let pois = state
        .cache
        .pois_near_point(req.lat, req.lon, radius_m)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("overpass: {e}")))?;

    let now = Utc::now();
    let out = rank(
        pois,
        req.lat,
        req.lon,
        radius_m,
        kinds.as_ref(),
        open_now,
        limit,
        &now,
    );

    Ok(Json(NearbyResp {
        lat: req.lat,
        lon: req.lon,
        radius_m,
        limit,
        now_unix: now.timestamp(),
        pois: out,
    }))
}

const INDEX_HTML: &str = include_str!("../../static/autour/index.html");
const APP_CSS: &str = include_str!("../../static/autour/app.css");
