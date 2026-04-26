use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use super::super::error::AppError;
use super::super::services::filter::{ActivityFilter, filter_activities};
use super::super::services::performance as svc;
use super::super::state::SharedState;

#[derive(Deserialize)]
pub struct TrendParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub metric: Option<String>,
    pub window: Option<i64>,
}

pub async fn trends(
    State(state): State<SharedState>,
    Query(params): Query<TrendParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &params.filter);
    let metric = params.metric.as_deref().unwrap_or("speed");
    let window = params.window.unwrap_or(30);
    let points = svc::compute_trends(&filtered, metric, window);
    Ok(Json(serde_json::json!({
        "metric": metric,
        "window_days": window,
        "data_points": points,
    })))
}

pub async fn personal_bests(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<svc::PersonalBests>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_personal_bests(&filtered)))
}

#[derive(Deserialize)]
pub struct FitnessParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub ctl_days: Option<f64>,
    pub atl_days: Option<f64>,
}

pub async fn fitness_curve(
    State(state): State<SharedState>,
    Query(params): Query<FitnessParams>,
) -> Result<Json<Vec<svc::FitnessPoint>>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &params.filter);
    let ctl = params.ctl_days.unwrap_or(42.0);
    let atl = params.atl_days.unwrap_or(7.0);
    Ok(Json(svc::compute_fitness_curve(&filtered, ctl, atl)))
}

pub async fn power_curve(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<Vec<svc::PowerMonth>>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_power_curve(&filtered)))
}

#[derive(Deserialize)]
pub struct HrZoneParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub max_hr: Option<f64>,
}

pub async fn hr_zones(
    State(state): State<SharedState>,
    Query(params): Query<HrZoneParams>,
) -> Result<Json<svc::HrZones>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &params.filter);
    Ok(Json(svc::compute_hr_zones(&filtered, params.max_hr)))
}
