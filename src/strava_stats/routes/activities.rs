use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use super::super::error::AppError;
use super::super::services::filter::{ActivityFilter, filter_activities};
use super::super::state::SharedState;

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub page: Option<usize>,
    pub per_page: Option<usize>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
}

#[derive(Serialize)]
pub struct ActivityListItem {
    pub id: u64,
    pub date: String,
    pub name: String,
    pub activity_type: String,
    pub distance_km: f64,
    pub moving_time_seconds: f64,
    pub elevation_gain_m: Option<f64>,
    pub average_speed_kmh: f64,
    pub average_heart_rate: Option<f64>,
    pub average_watts: Option<f64>,
    pub gear_name: Option<String>,
    pub calories: Option<f64>,
}

#[derive(Serialize)]
pub struct PaginatedResponse {
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
    pub activities: Vec<ActivityListItem>,
}

pub async fn list(
    State(state): State<SharedState>,
    Query(params): Query<ListParams>,
) -> Result<Json<PaginatedResponse>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }

    let mut filtered = filter_activities(&data.activities, &params.filter);

    let sort_by = params.sort_by.as_deref().unwrap_or("date");
    let desc = params.sort_order.as_deref().unwrap_or("desc") == "desc";

    filtered.sort_by(|a, b| {
        let cmp = match sort_by {
            "distance" => a.distance_meters.partial_cmp(&b.distance_meters),
            "elevation_gain" => a.elevation_gain.partial_cmp(&b.elevation_gain),
            "moving_time" => a.moving_time.partial_cmp(&b.moving_time),
            _ => Some(a.date.cmp(&b.date)),
        };
        let cmp = cmp.unwrap_or(std::cmp::Ordering::Equal);
        if desc { cmp.reverse() } else { cmp }
    });

    let total = filtered.len();
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(50).min(500);
    let start = (page - 1) * per_page;

    let items: Vec<ActivityListItem> = filtered
        .iter()
        .skip(start)
        .take(per_page)
        .map(|a| ActivityListItem {
            id: a.id,
            date: a.date.format("%Y-%m-%dT%H:%M:%S").to_string(),
            name: a.name.clone(),
            activity_type: a.activity_type.to_string(),
            distance_km: a.distance_meters / 1000.0,
            moving_time_seconds: a.moving_time,
            elevation_gain_m: a.elevation_gain,
            average_speed_kmh: a.average_speed * 3.6,
            average_heart_rate: a.average_heart_rate,
            average_watts: a.average_watts,
            gear_name: a.gear_name.clone(),
            calories: a.calories,
        })
        .collect();

    Ok(Json(PaginatedResponse {
        total,
        page,
        per_page,
        activities: items,
    }))
}

pub async fn get_by_id(
    State(state): State<SharedState>,
    Path(id): Path<u64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }

    let activity = data
        .activities
        .iter()
        .find(|a| a.id == id)
        .ok_or(AppError::NotFound(id))?;

    Ok(Json(serde_json::to_value(activity).unwrap()))
}
