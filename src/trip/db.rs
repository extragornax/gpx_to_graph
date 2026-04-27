use rusqlite::{params, Connection};
use std::sync::Mutex;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TripSummary {
    pub id: i64,
    pub name: String,
    pub total_km: f64,
    pub total_gain: f64,
    pub num_days: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TripDetail {
    pub id: i64,
    pub name: String,
    pub points_json: String,
    pub boundaries: Vec<usize>,
    pub created_at: String,
}

impl Db {
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
                password_hash TEXT NOT NULL,
                created_at    TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS sessions (
                token      TEXT PRIMARY KEY,
                user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                expires_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS trips (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                name         TEXT NOT NULL,
                gpx_data     TEXT NOT NULL,
                points_json  TEXT NOT NULL,
                boundaries   TEXT NOT NULL DEFAULT '[]',
                created_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_trips_user ON trips(user_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token);",
        )?;
        Ok(())
    }

    // ── Users ──

    pub fn create_user(&self, username: &str, password_hash: &str) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
            params![username, password_hash],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_user_by_username(&self, username: &str) -> anyhow::Result<Option<(i64, String)>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT id, password_hash FROM users WHERE username = ?1",
            params![username],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ) {
            Ok(u) => Ok(Some(u)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_user_by_id(&self, id: i64) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT username FROM users WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ) {
            Ok(u) => Ok(Some(u)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // ── Sessions ──

    pub fn create_session(&self, token: &str, user_id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, datetime('now', '+30 days'))",
            params![token, user_id],
        )?;
        Ok(())
    }

    pub fn get_session(&self, token: &str) -> anyhow::Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT user_id FROM sessions WHERE token = ?1 AND expires_at > datetime('now')",
            params![token],
            |row| row.get(0),
        ) {
            Ok(uid) => Ok(Some(uid)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_session(&self, token: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
        Ok(())
    }

    // ── Trips ──

    pub fn create_trip(
        &self,
        user_id: i64,
        name: &str,
        gpx_data: &str,
        points_json: &str,
        boundaries: &str,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO trips (user_id, name, gpx_data, points_json, boundaries) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user_id, name, gpx_data, points_json, boundaries],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_trips(&self, user_id: i64) -> anyhow::Result<Vec<TripSummary>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, points_json, boundaries, created_at
             FROM trips WHERE user_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut trips = Vec::new();
        for (id, name, points_json, boundaries_json, created_at) in rows {
            let (total_km, total_gain) = compute_trip_stats(&points_json);
            let boundaries: Vec<usize> =
                serde_json::from_str(&boundaries_json).unwrap_or_default();
            trips.push(TripSummary {
                id,
                name,
                total_km,
                total_gain,
                num_days: boundaries.len() + 1,
                created_at,
            });
        }
        Ok(trips)
    }

    pub fn get_trip(&self, user_id: i64, trip_id: i64) -> anyhow::Result<Option<TripDetail>> {
        let conn = self.conn.lock().unwrap();
        let row = conn.query_row(
            "SELECT id, name, points_json, boundaries, created_at FROM trips WHERE id = ?1 AND user_id = ?2",
            params![trip_id, user_id],
            |row| Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            )),
        );
        match row {
            Ok((id, name, points_json, boundaries_json, created_at)) => {
                let boundaries: Vec<usize> =
                    serde_json::from_str(&boundaries_json).unwrap_or_default();
                Ok(Some(TripDetail {
                    id,
                    name,
                    points_json,
                    boundaries,
                    created_at,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_trip_for_gpx(
        &self,
        user_id: i64,
        trip_id: i64,
    ) -> anyhow::Result<Option<(String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT points_json, boundaries, name FROM trips WHERE id = ?1 AND user_id = ?2",
            params![trip_id, user_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ) {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn update_trip_name(
        &self,
        user_id: i64,
        trip_id: i64,
        name: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE trips SET name = ?3 WHERE id = ?1 AND user_id = ?2",
            params![trip_id, user_id, name],
        )?;
        Ok(n > 0)
    }

    pub fn update_boundaries(
        &self,
        user_id: i64,
        trip_id: i64,
        boundaries: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE trips SET boundaries = ?3 WHERE id = ?1 AND user_id = ?2",
            params![trip_id, user_id, boundaries],
        )?;
        Ok(n > 0)
    }

    pub fn delete_trip(&self, user_id: i64, trip_id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM trips WHERE id = ?1 AND user_id = ?2",
            params![trip_id, user_id],
        )?;
        Ok(n > 0)
    }
}

fn compute_trip_stats(points_json: &str) -> (f64, f64) {
    #[derive(serde::Deserialize)]
    struct Pt {
        km: f64,
        ele: f64,
    }
    let pts: Vec<Pt> = serde_json::from_str(points_json).unwrap_or_default();
    if pts.is_empty() {
        return (0.0, 0.0);
    }
    let total_km = pts.last().map(|p| p.km).unwrap_or(0.0);
    let mut gain = 0.0;
    for w in pts.windows(2) {
        let d = w[1].ele - w[0].ele;
        if d > 0.0 {
            gain += d;
        }
    }
    (total_km, gain)
}
