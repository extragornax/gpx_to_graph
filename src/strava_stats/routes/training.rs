use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use super::super::error::AppError;
use super::super::services::filter::{ActivityFilter, filter_activities};
use super::super::services::training as svc;
use super::super::state::SharedState;

pub async fn weekly(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<Vec<svc::WeekStats>>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_weekly(&filtered)))
}

#[derive(Deserialize)]
pub struct FitnessParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub ctl_days: Option<f64>,
    pub atl_days: Option<f64>,
}

pub async fn fitness_fatigue(
    State(state): State<SharedState>,
    Query(params): Query<FitnessParams>,
) -> Result<Json<svc::FitnessFatigue>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &params.filter);
    let ctl = params.ctl_days.unwrap_or(42.0);
    let atl = params.atl_days.unwrap_or(7.0);
    Ok(Json(svc::compute_fitness_fatigue(&filtered, ctl, atl)))
}

#[derive(Deserialize)]
pub struct VolumeParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub period: Option<String>,
}

pub async fn volume(
    State(state): State<SharedState>,
    Query(params): Query<VolumeParams>,
) -> Result<Json<Vec<svc::VolumePeriod>>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &params.filter);
    let period = params.period.as_deref().unwrap_or("weekly");
    Ok(Json(svc::compute_volume(&filtered, period)))
}
