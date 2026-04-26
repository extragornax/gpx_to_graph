use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use super::super::error::AppError;
use super::super::services::filter::{ActivityFilter, filter_activities};
use super::super::services::weather as svc;
use super::super::state::SharedState;

#[derive(Deserialize)]
pub struct CorrelationParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub weather_metric: Option<String>,
    pub performance_metric: Option<String>,
}

pub async fn correlation(
    State(state): State<SharedState>,
    Query(params): Query<CorrelationParams>,
) -> Result<Json<svc::Correlation>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &params.filter);
    let wm = params.weather_metric.as_deref().unwrap_or("temperature");
    let pm = params.performance_metric.as_deref().unwrap_or("speed");
    Ok(Json(svc::compute_correlation(&filtered, wm, pm)))
}

pub async fn summary(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<svc::WeatherSummary>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_summary(&filtered)))
}

pub async fn wind_rose(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<Vec<svc::WindSector>>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_wind_rose(&filtered)))
}
