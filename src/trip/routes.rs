use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use axum::response::Html;
use serde::Deserialize;
use std::io::Cursor;

use super::auth::{self, CurrentUser};
use super::SharedState;

const INDEX_HTML: &str = include_str!("../../static/trip/index.html");
const BASE_CSS: &str = include_str!("../../static/toolkit/app.css");
const TRIP_CSS: &str = include_str!("../../static/trip/app.css");

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", get(page_index))
        .route("/api/challenge", get(challenge))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/me", get(me))
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

// ── Auth ──

async fn challenge() -> Json<crate::pow::Challenge> {
    Json(crate::pow::generate())
}

#[derive(Deserialize)]
struct AuthBody {
    username: String,
    password: String,
    pow: crate::pow::PowSolution,
}

async fn register(
    State(state): State<SharedState>,
    Json(body): Json<AuthBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !crate::pow::verify(&body.pow) {
        return Err((StatusCode::BAD_REQUEST, "Invalid challenge".into()));
    }
    if body.username.len() < 2 || body.password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Username min 2 chars, password min 6 chars".into(),
        ));
    }
    if state
        .db
        .get_user_by_username(&body.username)
        .map_err(err500)?
        .is_some()
    {
        return Err((StatusCode::CONFLICT, "Username taken".into()));
    }

    let hash = auth::hash_password(&body.password).map_err(err500)?;
    let user_id = state
        .db
        .create_user(&body.username, &hash)
        .map_err(err500)?;
    let token = auth::generate_session_token();
    state.db.create_session(&token, user_id).map_err(err500)?;

    Ok((
        StatusCode::CREATED,
        session_headers(&token),
        Json(serde_json::json!({ "username": body.username })),
    ))
}

async fn login(
    State(state): State<SharedState>,
    Json(body): Json<AuthBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if !crate::pow::verify(&body.pow) {
        return Err((StatusCode::BAD_REQUEST, "Invalid challenge".into()));
    }
    let (user_id, hash) = state
        .db
        .get_user_by_username(&body.username)
        .map_err(err500)?
        .ok_or((StatusCode::UNAUTHORIZED, "Invalid credentials".into()))?;

    if !auth::verify_password(&body.password, &hash).map_err(err500)? {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".into()));
    }

    let token = auth::generate_session_token();
    state.db.create_session(&token, user_id).map_err(err500)?;

    Ok((
        session_headers(&token),
        Json(serde_json::json!({ "username": body.username })),
    ))
}

async fn logout(
    State(state): State<SharedState>,
    _user: CurrentUser,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Some(token) = extract_session_cookie(&headers) {
        state.db.delete_session(token).map_err(err500)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn me(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let username = state
        .db
        .get_user_by_id(user.0)
        .map_err(err500)?
        .ok_or((StatusCode::UNAUTHORIZED, "User not found".into()))?;
    Ok(Json(serde_json::json!({ "username": username })))
}

fn session_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "set-cookie",
        format!("trip_session={token}; Path=/trip; HttpOnly; SameSite=Lax; Max-Age=2592000")
            .parse()
            .unwrap(),
    );
    headers
}

fn extract_session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|s| s.trim().strip_prefix("trip_session="))
        .next()
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
        .create_trip(user.0, &name, &gpx_text, &points_json, &boundaries_json)
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
    state.db.list_trips(user.0).map(Json).map_err(err500)
}

async fn get_trip(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<super::db::TripDetail>, (StatusCode, String)> {
    state
        .db
        .get_trip(user.0, id)
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
        .update_trip_name(user.0, id, &body.name)
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
        .update_boundaries(user.0, id, &json)
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
    let ok = state.db.delete_trip(user.0, id).map_err(err500)?;
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
        .get_trip_for_gpx(user.0, id)
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
