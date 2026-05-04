use anyhow::{Context, Result};
use rusqlite::Connection;

pub fn init(path: &str) -> Result<Connection> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).context("creating roulette DB parent directory")?;
    }
    let conn = Connection::open(path).context("opening roulette SQLite database")?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA busy_timeout=5000;
         PRAGMA foreign_keys=ON;",
    )
    .context("setting roulette SQLite pragmas")?;
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS roulette_daily (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            city        TEXT    NOT NULL,
            date        TEXT    NOT NULL,
            gpx         TEXT    NOT NULL,
            distance_km REAL   NOT NULL,
            dplus_m     REAL   NOT NULL,
            waypoints   TEXT   NOT NULL,
            created_at  TEXT   NOT NULL DEFAULT (datetime('now')),
            UNIQUE(city, date)
        );

        CREATE TABLE IF NOT EXISTS roulette_avoid_sessions (
            id         TEXT PRIMARY KEY,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS roulette_avoid_segments (
            session_id TEXT    NOT NULL REFERENCES roulette_avoid_sessions(id),
            lat        REAL    NOT NULL,
            lon        REAL    NOT NULL,
            seq        INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_roulette_avoid_seg_session ON roulette_avoid_segments(session_id);
        CREATE INDEX IF NOT EXISTS idx_roulette_daily_lookup ON roulette_daily(city, date);",
    )
    .context("creating roulette schema")?;
    Ok(())
}

pub fn cleanup_old_sessions(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM roulette_avoid_segments WHERE session_id IN (
            SELECT id FROM roulette_avoid_sessions WHERE created_at < datetime('now', '-24 hours')
        )",
        [],
    )?;
    conn.execute(
        "DELETE FROM roulette_avoid_sessions WHERE created_at < datetime('now', '-24 hours')",
        [],
    )?;
    Ok(())
}

pub fn get_daily(conn: &Connection, city: &str, date: &str) -> Result<Option<DailyRow>> {
    let mut stmt = conn.prepare(
        "SELECT gpx, distance_km, dplus_m, waypoints FROM roulette_daily WHERE city = ?1 AND date = ?2",
    )?;
    let row = stmt
        .query_row(rusqlite::params![city, date], |row| {
            Ok(DailyRow {
                gpx: row.get(0)?,
                distance_km: row.get(1)?,
                dplus_m: row.get(2)?,
                waypoints: row.get(3)?,
            })
        })
        .optional()?;
    Ok(row)
}

pub fn insert_daily(
    conn: &Connection,
    city: &str,
    date: &str,
    gpx: &str,
    distance_km: f64,
    dplus_m: f64,
    waypoints: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO roulette_daily (city, date, gpx, distance_km, dplus_m, waypoints)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![city, date, gpx, distance_km, dplus_m, waypoints],
    )?;
    Ok(())
}

pub fn store_avoid_session(conn: &Connection, session_id: &str, points: &[(f64, f64)]) -> Result<()> {
    conn.execute(
        "INSERT INTO roulette_avoid_sessions (id) VALUES (?1)",
        [session_id],
    )?;
    let mut stmt = conn.prepare(
        "INSERT INTO roulette_avoid_segments (session_id, lat, lon, seq) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, (lat, lon)) in points.iter().enumerate() {
        stmt.execute(rusqlite::params![session_id, lat, lon, i as i64])?;
    }
    Ok(())
}

pub fn get_avoid_points(conn: &Connection, session_id: &str) -> Result<Vec<(f64, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT lat, lon FROM roulette_avoid_segments WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map([session_id], |row| {
        Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?))
    })?;
    let mut pts = Vec::new();
    for r in rows {
        pts.push(r?);
    }
    Ok(pts)
}

#[derive(Debug)]
pub struct DailyRow {
    pub gpx: String,
    pub distance_km: f64,
    pub dplus_m: f64,
    pub waypoints: String,
}

trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
