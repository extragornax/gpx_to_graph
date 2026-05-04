use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use axum::response::Html;
use serde::Deserialize;
use std::io::Cursor;

use crate::auth::CurrentUser;
use super::SharedState;

const INDEX_HTML: &str = include_str!("../../static/trip/index.html");
const BASE_CSS: &str = include_str!("../../static/toolkit/app.css");
const TRIP_CSS: &str = include_str!("../../static/trip/app.css");

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", get(page_index))
        .route("/api/trips", get(list_trips).post(create_trip))
        .route("/api/trips/{id}", get(get_trip).delete(delete_trip))
        .route("/api/trips/{id}/name", put(rename_trip))
        .route("/api/trips/{id}/days", put(update_days))
        .route("/api/trips/{id}/day/{day}/gpx", get(download_day_gpx))
}

async fn page_index() -> Html<String> {
    Html(INDEX_HTML.replace(
        "<!-- CSS_PLACEHOLDER -->",
        &format!("<style>{BASE_CSS}\n{TRIP_CSS}</style>"),
    ))
}

// ── Trips ──

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct TrackPoint {
    lat: f64,
    lon: f64,
    ele: f64,
    km: f64,
}

async fn create_trip(
    State(state): State<SharedState>,
    user: CurrentUser,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
        .ok_or((StatusCode::BAD_REQUEST, "No file uploaded".into()))?;

    let file_name = field
        .file_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "trip.gpx".into());
    let data = field
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let gpx_text = String::from_utf8_lossy(&data).to_string();

    let (raw_points, _) = crate::parse_gpx(Cursor::new(&data))
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("GPX parse error: {e}")))?;

    let points = build_track_points(&raw_points);
    if points.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No track points found".into()));
    }

    let name = file_name.trim_end_matches(".gpx").to_string();
    let points_json =
        serde_json::to_string(&points).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let boundaries: Vec<usize> = Vec::new();
    let boundaries_json = serde_json::to_string(&boundaries).unwrap();

    let trip_id = state
        .db
        .create_trip(user.id, &name, &gpx_text, &points_json, &boundaries_json)
        .map_err(err500)?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": trip_id, "name": name })),
    ))
}

async fn list_trips(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Result<Json<Vec<super::db::TripSummary>>, (StatusCode, String)> {
    state.db.list_trips(user.id).map(Json).map_err(err500)
}

async fn get_trip(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<super::db::TripDetail>, (StatusCode, String)> {
    state
        .db
        .get_trip(user.id, id)
        .map_err(err500)?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "Trip not found".into()))
}

#[derive(Deserialize)]
struct RenameBody {
    name: String,
}

async fn rename_trip(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Json(body): Json<RenameBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let ok = state
        .db
        .update_trip_name(user.id, id, &body.name)
        .map_err(err500)?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "Not found".into()))
    }
}

#[derive(Deserialize)]
struct UpdateDaysBody {
    boundaries: Vec<usize>,
}

async fn update_days(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Json(body): Json<UpdateDaysBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let json = serde_json::to_string(&body.boundaries).map_err(err500)?;
    let ok = state
        .db
        .update_boundaries(user.id, id, &json)
        .map_err(err500)?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "Not found".into()))
    }
}

async fn delete_trip(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    let ok = state.db.delete_trip(user.id, id).map_err(err500)?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "Not found".into()))
    }
}

async fn download_day_gpx(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path((id, day)): Path<(i64, usize)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (points_json, boundaries_json, name) = state
        .db
        .get_trip_for_gpx(user.id, id)
        .map_err(err500)?
        .ok_or((StatusCode::NOT_FOUND, "Trip not found".into()))?;

    let points: Vec<TrackPoint> =
        serde_json::from_str(&points_json).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let boundaries: Vec<usize> =
        serde_json::from_str(&boundaries_json).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let num_days = boundaries.len() + 1;
    if day == 0 || day > num_days {
        return Err((StatusCode::BAD_REQUEST, "Invalid day number".into()));
    }

    let start = if day == 1 { 0 } else { boundaries[day - 2] };
    let end = if day <= boundaries.len() {
        boundaries[day - 1]
    } else {
        points.len()
    };

    let slice = &points[start..end.min(points.len())];
    let gpx_xml = generate_gpx(slice, &format!("{name} - Day {day}"));

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/gpx+xml".parse().unwrap());
    headers.insert(
        "content-disposition",
        format!("attachment; filename=\"{name}_day{day}.gpx\"")
            .parse()
            .unwrap(),
    );

    Ok((headers, gpx_xml))
}

// ── Helpers ──

fn build_track_points(raw: &[crate::RawPoint]) -> Vec<TrackPoint> {
    let mut points = Vec::new();
    let mut cum_km = 0.0;

    for (i, pt) in raw.iter().enumerate() {
        if i > 0 {
            cum_km += haversine_km(raw[i - 1].lat, raw[i - 1].lon, pt.lat, pt.lon);
        }
        points.push(TrackPoint {
            lat: pt.lat,
            lon: pt.lon,
            ele: pt.ele.unwrap_or(0.0),
            km: cum_km,
        });
    }
    points
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    r * 2.0 * a.sqrt().asin()
}

fn generate_gpx(points: &[TrackPoint], name: &str) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <gpx version=\"1.1\" creator=\"GPX Tools\">\n  <trk>\n    <name>",
    );
    xml.push_str(&html_escape(name));
    xml.push_str("</name>\n    <trkseg>\n");
    for pt in points {
        xml.push_str(&format!(
            "      <trkpt lat=\"{:.7}\" lon=\"{:.7}\"><ele>{:.1}</ele></trkpt>\n",
            pt.lat, pt.lon, pt.ele
        ));
    }
    xml.push_str("    </trkseg>\n  </trk>\n</gpx>\n");
    xml
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn err500(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
