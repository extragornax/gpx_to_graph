use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use serde::Deserialize;

use crate::auth::{self, strava, AuthState, CurrentUser};
use super::climb;
use super::SharedState;

const INDEX_HTML: &str = include_str!("../../static/col/index.html");

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", get(page_index))
        .route("/api/share-id", get(get_share_link).post(regenerate_share_link))
        // Climbs (protected)
        .route("/api/upload/gpx", post(upload_gpx))
        .route("/api/climbs", get(list_climbs))
        .route("/api/climbs/{id}", get(get_climb))
        .route("/api/climbs/{id}/name", put(rename_climb))
        .route("/api/stats", get(get_stats))
        .route("/api/reset", post(reset_data))
        // Strava sync (protected, col-specific)
        .route("/api/strava/sync", post(strava_sync))
        // Strava webhooks (public)
        .route("/webhook/strava", get(strava_webhook_verify))
        .route("/webhook/strava", post(strava_webhook_event))
        // Public profile
        .route("/p/{share_id}", get(public_profile))
        .route("/api/public/{share_id}/climbs", get(public_climbs))
        .route("/api/public/{share_id}/stats", get(public_stats))
}

async fn page_index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

// ── Share link (col-specific) ──

async fn get_share_link(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let share_id = state.db.ensure_profile(user.id).map_err(err500)?;
    Ok(Json(serde_json::json!({ "share_id": share_id })))
}

async fn regenerate_share_link(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    use rand::Rng;
    let bytes: [u8; 8] = rand::rng().random();
    let new_id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    state.db.regenerate_share_id(user.id, &new_id).map_err(err500)?;
    Ok(Json(serde_json::json!({ "share_id": new_id })))
}

// ── Climbs ──

async fn upload_gpx(
    State(state): State<SharedState>,
    user: CurrentUser,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut total_climbs = 0usize;

    while let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
        let file_name = field.file_name().map(|s| s.to_string());
        let data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

        let gpx_profile = climb::profile_from_gpx(&data)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("GPX parse error: {e}")))?;

        let date = gpx_profile.date.as_deref().unwrap_or("unknown");
        let detected = climb::detect_climbs(&gpx_profile.points, 50.0);

        for c in &detected {
            let existing = state.db.find_nearby_climb(user.id, c.lat, c.lon, 0.5).map_err(err500)?;

            let climb_id = match existing {
                Some(id) => id,
                None => state.db.insert_climb(
                    user.id, c.lat, c.lon, c.start_ele, c.end_ele, c.gain,
                    c.end_km - c.start_km, c.gradient, date,
                ).map_err(err500)?,
            };

            state.db.add_attempt(climb_id, date, file_name.as_deref(), None).map_err(err500)?;
            total_climbs += 1;
        }
    }

    Ok(Json(serde_json::json!({ "climbs_processed": total_climbs })))
}

async fn list_climbs(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Result<Json<Vec<super::db::ClimbRecord>>, (StatusCode, String)> {
    state.db.get_climbs(user.id).map(Json).map_err(err500)
}

async fn get_climb(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let climb = state.db.get_climb(user.id, id).map_err(err500)?
        .ok_or((StatusCode::NOT_FOUND, "Climb not found".into()))?;
    let attempts = state.db.get_attempts(id).map_err(err500)?;
    Ok(Json(serde_json::json!({ "climb": climb, "attempts": attempts })))
}

#[derive(Deserialize)]
struct RenameBody {
    name: String,
}

async fn rename_climb(
    State(state): State<SharedState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Json(body): Json<RenameBody>,
) -> Result<StatusCode, (StatusCode, String)> {
    let updated = state.db.rename_climb(user.id, id, &body.name).map_err(err500)?;
    if updated { Ok(StatusCode::NO_CONTENT) } else { Err((StatusCode::NOT_FOUND, "Not found".into())) }
}

async fn get_stats(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Result<Json<super::db::Stats>, (StatusCode, String)> {
    state.db.get_stats(user.id).map(Json).map_err(err500)
}

async fn reset_data(
    State(state): State<SharedState>,
    user: CurrentUser,
) -> Result<StatusCode, (StatusCode, String)> {
    state.db.clear_user_data(user.id).map_err(err500)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Strava sync (background) ──

async fn strava_sync(
    State(state): State<SharedState>,
    Extension(auth): Extension<AuthState>,
    user: CurrentUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let token = get_valid_token(&auth, user.id).await?;

    tokio::spawn({
        let state = state.clone();
        let auth = auth.clone();
        let user_id = user.id;
        async move {
            if let Err(e) = run_sync(&state, &auth, user_id, &token).await {
                tracing::error!(user_id, "background sync failed: {e}");
            }
        }
    });

    Ok(Json(serde_json::json!({ "status": "sync_started" })))
}

async fn run_sync(state: &SharedState, auth: &AuthState, user_id: i64, token: &str) -> anyhow::Result<()> {
    let mut page = 1u32;
    loop {
        let activities = strava::fetch_activities(token, page).await?;
        if activities.is_empty() {
            break;
        }

        for activity in &activities {
            if state.db.is_activity_synced(user_id, activity.id)? {
                continue;
            }

            if !matches!(activity.activity_type.as_str(), "Ride" | "VirtualRide" | "GravelRide" | "EBikeRide" | "Run" | "TrailRun" | "Hike" | "Walk") {
                state.db.mark_activity_synced(user_id, activity.id)?;
                continue;
            }

            process_activity(state, auth, user_id, activity.id, &activity.name, &activity.start_date_local).await?;
            state.db.mark_activity_synced(user_id, activity.id)?;
        }

        if activities.len() < 200 {
            break;
        }
        page += 1;
    }
    tracing::info!(user_id, "sync complete");
    Ok(())
}

async fn process_activity(state: &SharedState, auth: &AuthState, user_id: i64, activity_id: i64, name: &str, start_date: &str) -> anyhow::Result<()> {
    let tokens = auth.db.get_strava_tokens(user_id)?
        .ok_or_else(|| anyhow::anyhow!("No Strava tokens for user {user_id}"))?;

    let access_token = auth::ensure_fresh_token(auth, user_id, &tokens).await?;

    let streams = strava::fetch_streams(&access_token, activity_id).await?;
    let Some(points) = streams else { return Ok(()); };

    let profile: Vec<climb::ProfilePoint> = points.iter()
        .map(|p| (p.distance_km, p.elevation, p.lat, p.lon))
        .collect();

    let date = &start_date[..10.min(start_date.len())];
    let detected = climb::detect_climbs(&profile, 50.0);

    for c in &detected {
        let existing = state.db.find_nearby_climb(user_id, c.lat, c.lon, 0.5)?;
        let climb_id = match existing {
            Some(id) => id,
            None => state.db.insert_climb(
                user_id, c.lat, c.lon, c.start_ele, c.end_ele, c.gain,
                c.end_km - c.start_km, c.gradient, date,
            )?,
        };
        state.db.add_attempt(climb_id, date, Some(name), None)?;
    }
    Ok(())
}

async fn get_valid_token(auth: &AuthState, user_id: i64) -> Result<String, (StatusCode, String)> {
    let tokens = auth.db.get_strava_tokens(user_id).map_err(err500)?
        .ok_or((StatusCode::UNAUTHORIZED, "Strava not connected".into()))?;
    auth::ensure_fresh_token(auth, user_id, &tokens)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Token refresh failed: {e}")))
}

// ── Strava webhooks ──

#[derive(Deserialize)]
struct WebhookVerifyParams {
    #[serde(rename = "hub.mode")]
    mode: String,
    #[serde(rename = "hub.challenge")]
    challenge: String,
    #[serde(rename = "hub.verify_token")]
    verify_token: String,
}

async fn strava_webhook_verify(
    Extension(auth): Extension<AuthState>,
    Query(params): Query<WebhookVerifyParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let config = auth.strava_config.as_ref().ok_or(StatusCode::NOT_FOUND)?;

    if params.mode != "subscribe" || params.verify_token != config.webhook_verify_token {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(serde_json::json!({ "hub.challenge": params.challenge })))
}

#[derive(Deserialize)]
struct WebhookEvent {
    object_type: String,
    object_id: i64,
    aspect_type: String,
    owner_id: i64,
}

async fn strava_webhook_event(
    State(state): State<SharedState>,
    Extension(auth): Extension<AuthState>,
    Json(event): Json<WebhookEvent>,
) -> StatusCode {
    if event.object_type != "activity" || event.aspect_type != "create" {
        return StatusCode::OK;
    }

    tokio::spawn(async move {
        if let Err(e) = handle_webhook_activity(&state, &auth, event.owner_id, event.object_id).await {
            tracing::error!(athlete_id = event.owner_id, activity_id = event.object_id, "webhook processing failed: {e}");
        }
    });

    StatusCode::OK
}

async fn handle_webhook_activity(state: &SharedState, auth: &AuthState, athlete_id: i64, activity_id: i64) -> anyhow::Result<()> {
    let (user_id, tokens) = auth.db.get_strava_tokens_by_athlete(athlete_id)?
        .ok_or_else(|| anyhow::anyhow!("No user linked to athlete {athlete_id}"))?;

    if state.db.is_activity_synced(user_id, activity_id)? {
        return Ok(());
    }

    let access_token = auth::ensure_fresh_token(auth, user_id, &tokens).await?;

    let activity = strava::fetch_activity(&access_token, activity_id).await?;
    let Some(activity) = activity else { return Ok(()); };

    if !matches!(activity.activity_type.as_str(), "Ride" | "VirtualRide" | "GravelRide" | "EBikeRide" | "Run" | "TrailRun" | "Hike" | "Walk") {
        state.db.mark_activity_synced(user_id, activity_id)?;
        return Ok(());
    }

    process_activity(state, auth, user_id, activity_id, &activity.name, &activity.start_date_local).await?;
    state.db.mark_activity_synced(user_id, activity_id)?;
    tracing::info!(user_id, activity_id, "webhook: processed activity");
    Ok(())
}

// ── Public profile ──

async fn public_profile() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn public_climbs(
    State(state): State<SharedState>,
    Path(share_id): Path<String>,
) -> Result<Json<Vec<super::db::ClimbRecord>>, (StatusCode, String)> {
    let user_id = state.db.get_user_by_share_id(&share_id).map_err(err500)?
        .ok_or((StatusCode::NOT_FOUND, "Not found".into()))?;
    state.db.get_climbs(user_id).map(Json).map_err(err500)
}

async fn public_stats(
    State(state): State<SharedState>,
    Path(share_id): Path<String>,
) -> Result<Json<super::db::Stats>, (StatusCode, String)> {
    let user_id = state.db.get_user_by_share_id(&share_id).map_err(err500)?
        .ok_or((StatusCode::NOT_FOUND, "Not found".into()))?;
    state.db.get_stats(user_id).map(Json).map_err(err500)
}

fn err500(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
