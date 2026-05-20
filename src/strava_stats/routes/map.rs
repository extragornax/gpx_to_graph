use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::{AuthState, CurrentUser, ensure_fresh_token};

#[derive(Serialize)]
pub struct StravaMapData {
    pub activities: Vec<StravaActivity>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StravaActivity {
    pub id: i64,
    pub name: String,
    pub start_date_local: String,
    #[serde(rename = "type")]
    pub activity_type: String,
    pub total_elevation_gain: Option<f64>,
    pub map_url: String,
    pub has_stream: bool,
}

#[derive(Serialize)]
pub struct StravaMapPoint {
    pub distance_km: f64,
    pub elevation: f64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize)]
pub struct StravaMapResponse {
    pub points: Vec<StravaMapPoint>,
    pub activity_name: String,
}

pub async fn strava_map_access(
    Extension(auth): Extension<AuthState>,
    user: CurrentUser,
) -> Result<Json<StravaMapData>, StatusCode> {
    // Get user's Strava tokens from database
    let tokens = auth
        .db
        .get_strava_tokens(user.id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Get fresh token if needed
    let access_token = ensure_fresh_token(&auth, user.id, &tokens)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Fetch activities from Strava
    let mut all_activities = Vec::new();
    let mut page = 1u32;

    loop {
        let activities = crate::auth::strava::fetch_activities(&access_token, page)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        if activities.is_empty() {
            break;
        }

        for activity in activities {
            let map_url = format!("https://www.strava.com/activities/{}", activity.id);
            let has_stream = activity.total_elevation_gain.is_some();

            all_activities.push(StravaActivity {
                id: activity.id,
                name: activity.name,
                start_date_local: activity.start_date_local,
                activity_type: activity.activity_type,
                total_elevation_gain: activity.total_elevation_gain,
                map_url,
                has_stream,
            });
        }

        page += 1;

        // Safety break to prevent infinite loops
        if page > 10 {
            break;
        }
    }

    Ok(Json(StravaMapData {
        activities: all_activities,
    }))
}

pub async fn strava_map_points(
    Extension(auth): Extension<AuthState>,
    user: CurrentUser,
    Path(activity_id): Path<i64>,
) -> Result<Json<StravaMapResponse>, StatusCode> {
    // Get user's Strava tokens from database
    let tokens = auth
        .db
        .get_strava_tokens(user.id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Get fresh token if needed
    let access_token = ensure_fresh_token(&auth, user.id, &tokens)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Fetch activity details
    let activity = crate::auth::strava::fetch_activity(&access_token, activity_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Fetch stream data for elevation and coordinates
    let points_opt = crate::auth::strava::fetch_streams(&access_token, activity_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let map_points = match points_opt {
        Some(points) => points
            .into_iter()
            .map(|point| StravaMapPoint {
                distance_km: point.distance_km,
                elevation: point.elevation,
                lat: point.lat,
                lon: point.lon,
            })
            .collect(),
        None => vec![], // Return empty array if no stream data available
    };

    Ok(Json(StravaMapResponse {
        points: map_points,
        activity_name: activity.name,
    }))
}
