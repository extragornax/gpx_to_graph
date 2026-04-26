use axum::Json;
use axum::extract::{Multipart, State};
use serde::Serialize;
use std::collections::HashMap;

use super::super::error::AppError;
use super::super::services::csv_parser;
use super::super::state::SharedState;

#[derive(Serialize)]
pub struct UploadResponse {
    pub activities_loaded: usize,
    pub date_range: DateRange,
    pub activity_types: HashMap<String, usize>,
}

#[derive(Serialize)]
pub struct DateRange {
    pub from: String,
    pub to: String,
}

pub async fn upload_csv(
    State(state): State<SharedState>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, AppError> {
    let mut csv_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("file") {
            csv_data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?
                    .to_vec(),
            );
            break;
        }
    }

    let data = csv_data.ok_or_else(|| AppError::BadRequest("No 'file' field in upload".into()))?;
    let activities = csv_parser::parse_activities(&data)?;

    let mut type_counts: HashMap<String, usize> = HashMap::new();
    for a in &activities {
        *type_counts.entry(a.activity_type.to_string()).or_insert(0) += 1;
    }

    let date_range = DateRange {
        from: activities
            .last()
            .map(|a| a.date.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
        to: activities
            .first()
            .map(|a| a.date.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
    };

    let count = activities.len();
    state.write().await.activities = activities;

    Ok(Json(UploadResponse {
        activities_loaded: count,
        date_range,
        activity_types: type_counts,
    }))
}
