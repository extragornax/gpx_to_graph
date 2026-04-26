use super::models::Activity;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AppState {
    pub activities: Vec<Activity>,
}

pub type SharedState = Arc<RwLock<AppState>>;
