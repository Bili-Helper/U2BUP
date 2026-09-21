use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Serialize};
use std::{path::Path, sync::Mutex};

// Versioned document storage keeps the initial schema small; writes are atomic in SQLite.
pub struct Db(Mutex<Connection>);
impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path)?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
            CREATE TABLE IF NOT EXISTS documents(kind TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,PRIMARY KEY(kind,id));
            PRAGMA user_version=1;")?;
        Ok(Self(Mutex::new(c)))
    }
    pub fn put(&self, kind: &str, id: &str, value: &impl Serialize) -> Result<()> {
        self.0.lock().unwrap().execute("INSERT INTO documents VALUES(?1,?2,?3) ON CONFLICT(kind,id) DO UPDATE SET json=excluded.json", params![kind,id,serde_json::to_string(value)?])?;
        Ok(())
    }
    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>> {
        use rusqlite::OptionalExtension;
        let text: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT json FROM documents WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(text.map(|s| serde_json::from_str(&s)).transpose()?)
    }
    pub fn list<T: DeserializeOwned>(&self, kind: &str) -> Result<Vec<T>> {
        let c = self.0.lock().unwrap();
        let mut q = c.prepare("SELECT json FROM documents WHERE kind=?1 ORDER BY rowid DESC")?;
        let rows = q.query_map([kind], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
}
