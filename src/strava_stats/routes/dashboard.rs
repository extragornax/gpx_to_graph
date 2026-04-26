use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use super::super::error::AppError;
use super::super::services::dashboard as svc;
use super::super::services::filter::{ActivityFilter, filter_activities};
use super::super::state::SharedState;

pub async fn summary(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<svc::Summary>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_summary(&filtered)))
}

pub async fn yearly(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<Vec<svc::YearStats>>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_yearly(&filtered)))
}

#[derive(Deserialize)]
pub struct MonthlyParams {
    #[serde(flatten)]
    pub filter: ActivityFilter,
    pub year: Option<i32>,
}

pub async fn monthly(
    State(state): State<SharedState>,
    Query(params): Query<MonthlyParams>,
) -> Result<Json<Vec<svc::MonthStats>>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &params.filter);
    Ok(Json(svc::compute_monthly(&filtered, params.year)))
}

pub async fn streaks(
    State(state): State<SharedState>,
    Query(filter): Query<ActivityFilter>,
) -> Result<Json<svc::Streaks>, AppError> {
    let data = state.read().await;
    if data.activities.is_empty() {
        return Err(AppError::NoData);
    }
    let filtered = filter_activities(&data.activities, &filter);
    Ok(Json(svc::compute_streaks(&filtered)))
}
