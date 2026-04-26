mod upload;
mod activities;
mod dashboard;
mod performance;
mod training;
mod weather;

use super::state::SharedState;
use axum::Router;
use axum::routing::{get, post};

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/upload", post(upload::upload_csv))
        .route("/activities", get(activities::list))
        .route("/activities/{id}", get(activities::get_by_id))
        .route("/dashboard/summary", get(dashboard::summary))
        .route("/dashboard/yearly", get(dashboard::yearly))
        .route("/dashboard/monthly", get(dashboard::monthly))
        .route("/dashboard/streaks", get(dashboard::streaks))
        .route("/performance/trends", get(performance::trends))
        .route("/performance/personal_bests", get(performance::personal_bests))
        .route("/performance/fitness_curve", get(performance::fitness_curve))
        .route("/performance/power_curve", get(performance::power_curve))
        .route("/performance/hr_zones", get(performance::hr_zones))
        .route("/weather/correlation", get(weather::correlation))
        .route("/weather/summary", get(weather::summary))
        .route("/weather/wind_rose", get(weather::wind_rose))
        .route("/training/weekly", get(training::weekly))
        .route("/training/fitness_fatigue", get(training::fitness_fatigue))
        .route("/training/volume", get(training::volume))
}
