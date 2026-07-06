//! Shareable GPX uploads: store an uploaded GPX (plus the form settings used
//! to analyze it) under a short random id, so the user gets a link they can
//! bookmark or share. Links expire after one month.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use rand::{Rng, distr::Alphanumeric};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// Shared links live this long (one month).
const TTL_SECS: i64 = 30 * 24 * 3600;

pub struct ShareStore {
    conn: Mutex<Connection>,
}

impl ShareStore {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open sqlite at {path}"))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS shares (
                id TEXT PRIMARY KEY,
                app TEXT NOT NULL,
                gpx TEXT NOT NULL,
                params TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            "#,
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn insert(&self, app: &str, gpx: &str, params_json: &str) -> Result<String> {
        let id: String = rand::rng()
            .sample_iter(&Alphanumeric)
            .take(12)
            .map(char::from)
            .collect();
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM shares WHERE created_at < ?1",
            params![now - TTL_SECS],
        )?;
        conn.execute(
            "INSERT INTO shares (id, app, gpx, params, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, app, gpx, params_json, now],
        )?;
        Ok(id)
    }

    fn get(&self, app: &str, id: &str) -> Result<Option<(String, String)>> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT gpx, params FROM shares WHERE id = ?1 AND app = ?2 AND created_at >= ?3",
            params![id, app, now - TTL_SECS],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .context("read share")
    }
}

#[derive(Clone)]
struct ShareState {
    app: &'static str,
    store: Arc<ShareStore>,
}

#[derive(Deserialize)]
struct CreateReq {
    gpx: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Serialize)]
struct CreateResp {
    id: String,
}

#[derive(Serialize)]
struct GetResp {
    gpx: String,
    params: serde_json::Value,
}

async fn create(
    State(st): State<ShareState>,
    Json(req): Json<CreateReq>,
) -> Result<Json<CreateResp>, (StatusCode, String)> {
    if req.gpx.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty gpx".into()));
    }
    let id = st
        .store
        .insert(st.app, &req.gpx, &req.params.to_string())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("share: {e}")))?;
    Ok(Json(CreateResp { id }))
}

async fn fetch(
    State(st): State<ShareState>,
    Path(id): Path<String>,
) -> Result<Json<GetResp>, (StatusCode, String)> {
    let row = st
        .store
        .get(st.app, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("share: {e}")))?;
    match row {
        Some((gpx, params_json)) => {
            let params = serde_json::from_str(&params_json).unwrap_or(serde_json::Value::Null);
            Ok(Json(GetResp { gpx, params }))
        }
        None => Err((StatusCode::NOT_FOUND, "share link expired or unknown".into())),
    }
}

pub fn routes(app: &'static str, store: Arc<ShareStore>) -> Router {
    Router::new()
        .route("/api/share", post(create))
        .route("/api/share/{id}", get(fetch))
        .with_state(ShareState { app, store })
}
