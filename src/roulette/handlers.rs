use std::collections::HashMap;
use std::io::BufReader;
use std::sync::{Arc, Mutex};

use axum::{
    Json,
    extract::{Multipart, Query, State},
    http::StatusCode,
    response::Html,
};
use serde_json::{json, Value};

use super::db;
use super::engine::{self, GenerateRequest};

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub http_client: reqwest::Client,
    pub brouter_url: String,
    pub needs_rate_limit: bool,
    pub geocode_cache: Mutex<HashMap<String, (f64, f64)>>,
}

const INDEX_HTML: &str = include_str!("../../static/roulette/index.html");
const APP_CSS: &str = include_str!("../../static/roulette/app.css");

pub async fn index() -> Html<String> {
    Html(INDEX_HTML.replace("<!-- CSS_PLACEHOLDER -->", &format!("<style>{}</style>", APP_CSS)))
}

pub async fn generate_handler(
    State(state): State<SharedState>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let avoid_points = if let Some(ref sid) = req.avoid_session {
        let conn = state.db.lock().unwrap();
        db::get_avoid_points(&conn, sid).unwrap_or_default()
    } else {
        vec![]
    };

    let result = engine::generate_route(
        &state.http_client,
        &state.brouter_url,
        state.needs_rate_limit,
        &req,
        &avoid_points,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "gpx": result.gpx,
        "stats": result.stats,
        "waypoints": result.waypoints,
        "warnings": result.warnings,
    })))
}

pub async fn avoid_upload(
    State(state): State<SharedState>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut all_points: Vec<(f64, f64)> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let data = field
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let gpx = gpx::read(BufReader::new(data.as_ref()))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("GPX invalide: {e}")))?;

        for track in &gpx.tracks {
            for seg in &track.segments {
                for pt in &seg.points {
                    all_points.push((pt.point().y(), pt.point().x()));
                }
            }
        }
    }

    if all_points.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Aucun point dans le(s) GPX".into()));
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    {
        let conn = state.db.lock().unwrap();
        db::store_avoid_session(&conn, &session_id, &all_points)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(json!({ "session_id": session_id })))
}

pub async fn daily_handler(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let city = params
        .get("city")
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "paris".into());

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    {
        let conn = state.db.lock().unwrap();
        if let Ok(Some(row)) = db::get_daily(&conn, &city, &today) {
            return Ok(Json(json!({
                "city": city,
                "date": today,
                "gpx": row.gpx,
                "distance_km": row.distance_km,
                "dplus_m": row.dplus_m,
                "waypoints": serde_json::from_str::<Value>(&row.waypoints).unwrap_or(Value::Null),
            })));
        }
    }

    let (_, lat, lon) = engine::DAILY_CITIES
        .iter()
        .find(|(name, _, _)| *name == city)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Ville inconnue: {city}")))?;

    let seed = engine::daily_seed(&today);
    let city_idx = engine::DAILY_CITIES
        .iter()
        .position(|(n, _, _)| *n == city)
        .unwrap_or(0);
    let _direction = engine::daily_direction(seed, city_idx);

    let req = GenerateRequest {
        start: [*lat, *lon],
        distance_km: 80.0,
        dplus_max: None,
        profile: Some("trekking".into()),
        is_loop: Some(true),
        waypoints: None,
        avoid_session: None,
    };

    let result = engine::generate_route(
        &state.http_client,
        &state.brouter_url,
        state.needs_rate_limit,
        &req,
        &[],
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    {
        let conn = state.db.lock().unwrap();
        let wp_json = serde_json::to_string(&result.waypoints).unwrap_or_default();
        let _ = db::insert_daily(
            &conn,
            &city,
            &today,
            &result.gpx,
            result.stats.distance_km,
            result.stats.dplus_m,
            &wp_json,
        );
    }

    Ok(Json(json!({
        "city": city,
        "date": today,
        "gpx": result.gpx,
        "distance_km": result.stats.distance_km,
        "dplus_m": result.stats.dplus_m,
        "waypoints": result.waypoints,
    })))
}

pub async fn daily_page() -> Html<String> {
    Html(DAILY_HTML.replace("<!-- CSS_PLACEHOLDER -->", &format!("<style>{}</style>", APP_CSS)))
}

const DAILY_HTML: &str = include_str!("../../static/roulette/daily.html");

pub async fn geocode_handler(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let query = params
        .get("q")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Paramètre q manquant".into()))?;

    {
        let cache = state.geocode_cache.lock().unwrap();
        if let Some(&(lat, lon)) = cache.get(query.as_str()) {
            return Ok(Json(json!({ "lat": lat, "lon": lon })));
        }
    }

    let url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=json&limit=1",
        urlencoding(query)
    );

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let resp = state
        .http_client
        .get(&url)
        .header("User-Agent", "roulette-velo/1.0 (extragornax.fr)")
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let results: Vec<Value> = resp
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let first = results
        .first()
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Aucun résultat".into()))?;

    let lat: f64 = first["lat"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "Réponse Nominatim invalide".into()))?;
    let lon: f64 = first["lon"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "Réponse Nominatim invalide".into()))?;

    {
        let mut cache = state.geocode_cache.lock().unwrap();
        cache.insert(query.clone(), (lat, lon));
    }

    Ok(Json(json!({ "lat": lat, "lon": lon, "display": first["display_name"] })))
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                String::from(b as char)
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}
