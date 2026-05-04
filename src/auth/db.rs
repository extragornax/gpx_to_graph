use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{params, Connection};

pub struct StravaTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub athlete_id: i64,
    pub athlete_name: Option<String>,
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn migrate(&self) -> Result<()> {
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

            CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token);

            CREATE TABLE IF NOT EXISTS strava_tokens (
                user_id       INTEGER PRIMARY KEY,
                access_token  TEXT NOT NULL,
                refresh_token TEXT NOT NULL,
                expires_at    INTEGER NOT NULL,
                athlete_id    INTEGER NOT NULL,
                athlete_name  TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_strava_athlete ON strava_tokens(athlete_id);"
        )?;
        Ok(())
    }

    pub fn create_user(&self, username: &str, password_hash: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
            params![username, password_hash],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<(i64, String)>> {
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

    pub fn get_username(&self, user_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT username FROM users WHERE id = ?1",
            params![user_id],
            |row| row.get(0),
        ) {
            Ok(u) => Ok(Some(u)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn create_session(&self, token: &str, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (token, user_id, expires_at) VALUES (?1, ?2, datetime('now', '+30 days'))",
            params![token, user_id],
        )?;
        Ok(())
    }

    pub fn get_session(&self, token: &str) -> Result<Option<i64>> {
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

    pub fn delete_session(&self, token: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE token = ?1", params![token])?;
        Ok(())
    }

    pub fn cleanup_expired_sessions(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE expires_at < datetime('now')", [])?;
        Ok(())
    }

    // ── Strava tokens ──

    pub fn save_strava_tokens(
        &self, user_id: i64, access_token: &str, refresh_token: &str,
        expires_at: i64, athlete_id: i64, athlete_name: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO strava_tokens (user_id, access_token, refresh_token, expires_at, athlete_id, athlete_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![user_id, access_token, refresh_token, expires_at, athlete_id, athlete_name],
        )?;
        Ok(())
    }

    pub fn get_strava_tokens(&self, user_id: i64) -> Result<Option<StravaTokens>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT access_token, refresh_token, expires_at, athlete_id, athlete_name FROM strava_tokens WHERE user_id = ?1",
            params![user_id],
            |row| Ok(StravaTokens {
                access_token: row.get(0)?, refresh_token: row.get(1)?,
                expires_at: row.get(2)?, athlete_id: row.get(3)?, athlete_name: row.get(4)?,
            }),
        );
        match result {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_strava_tokens_by_athlete(&self, athlete_id: i64) -> Result<Option<(i64, StravaTokens)>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT user_id, access_token, refresh_token, expires_at, athlete_id, athlete_name
             FROM strava_tokens WHERE athlete_id = ?1",
            params![athlete_id],
            |row| Ok((row.get(0)?, StravaTokens {
                access_token: row.get(1)?, refresh_token: row.get(2)?,
                expires_at: row.get(3)?, athlete_id: row.get(4)?, athlete_name: row.get(5)?,
            })),
        );
        match result {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_strava_tokens(&self, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM strava_tokens WHERE user_id = ?1", params![user_id])?;
        Ok(())
    }
}
