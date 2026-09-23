use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{de::DeserializeOwned, Serialize};
use std::{path::Path, sync::Mutex};

// Schema versions:
//   1 – original: documents(kind,id,json)
//   2 – added: libraries(id,json), assets_v2(id,library_id,source_path,…,json)
const CURRENT_VERSION: u32 = 2;

pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let c = Connection::open(path)?;
        c.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )?;
        // Bootstrap v1 table if fresh database.
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS documents(
                kind TEXT NOT NULL,
                id   TEXT NOT NULL,
                json TEXT NOT NULL,
                PRIMARY KEY(kind,id)
             );",
        )?;
        let version: u32 = c
            .query_row("SELECT user_version FROM pragma_user_version", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        if version < 1 {
            c.execute_batch("PRAGMA user_version=1;")?;
        }
        if version < 2 {
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS libraries(
                    id   TEXT PRIMARY KEY,
                    json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS assets_v2(
                    id           TEXT PRIMARY KEY,
                    library_id   TEXT NOT NULL REFERENCES libraries(id),
                    source_path  TEXT NOT NULL,
                    content_hash TEXT,
                    pub_title    TEXT,
                    custom_tags  TEXT,
                    modified_ms  INTEGER NOT NULL DEFAULT 0,
                    file_size    INTEGER NOT NULL DEFAULT 0,
                    json         TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_assets_library  ON assets_v2(library_id);
                 CREATE INDEX IF NOT EXISTS idx_assets_hash     ON assets_v2(content_hash) WHERE content_hash IS NOT NULL;
                 CREATE INDEX IF NOT EXISTS idx_assets_path     ON assets_v2(source_path);
                 CREATE INDEX IF NOT EXISTS idx_assets_modified ON assets_v2(modified_ms);
                 PRAGMA user_version=2;",
            )
            .context("数据库迁移至 v2 失败")?;
        }
        Ok(Self(Mutex::new(c)))
    }

    /// Current on-disk schema version.
    pub fn schema_version(&self) -> u32 {
        self.0
            .lock()
            .unwrap()
            .query_row("SELECT user_version FROM pragma_user_version", [], |r| {
                r.get(0)
            })
            .unwrap_or(0)
    }

    // ── documents table (generic key-value store, unchanged) ──────────────

    pub fn put(&self, kind: &str, id: &str, value: &impl Serialize) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO documents VALUES(?1,?2,?3) \
             ON CONFLICT(kind,id) DO UPDATE SET json=excluded.json",
            params![kind, id, serde_json::to_string(value)?],
        )?;
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

    pub fn delete(&self, kind: &str, id: &str) -> Result<bool> {
        let n = self.0.lock().unwrap().execute(
            "DELETE FROM documents WHERE kind=?1 AND id=?2",
            params![kind, id],
        )?;
        Ok(n > 0)
    }

    // ── libraries table ───────────────────────────────────────────────────

    pub fn library_put(&self, id: &str, value: &impl Serialize) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO libraries VALUES(?1,?2) \
             ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            params![id, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    pub fn library_get<T: DeserializeOwned>(&self, id: &str) -> Result<Option<T>> {
        use rusqlite::OptionalExtension;
        let text: Option<String> = self
            .0
            .lock()
            .unwrap()
            .query_row("SELECT json FROM libraries WHERE id=?1", params![id], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(text.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn library_list<T: DeserializeOwned>(&self) -> Result<Vec<T>> {
        let c = self.0.lock().unwrap();
        let mut q = c.prepare("SELECT json FROM libraries ORDER BY rowid ASC")?;
        let rows = q.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn library_delete(&self, id: &str) -> Result<bool> {
        let n = self
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM libraries WHERE id=?1", params![id])?;
        Ok(n > 0)
    }

    // ── assets_v2 table ───────────────────────────────────────────────────

    pub fn asset_put(&self, a: &crate::model::AssetRecord) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO assets_v2(id,library_id,source_path,content_hash,pub_title,custom_tags,modified_ms,file_size,json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO UPDATE SET
               library_id=excluded.library_id, source_path=excluded.source_path,
               content_hash=excluded.content_hash, pub_title=excluded.pub_title,
               custom_tags=excluded.custom_tags, modified_ms=excluded.modified_ms,
               file_size=excluded.file_size, json=excluded.json",
            params![
                a.id, a.library_id, a.source_path, a.content_hash,
                a.pub_title, a.custom_tags_json,
                a.modified_ms as i64, a.file_size as i64,
                serde_json::to_string(&a.asset)?
            ],
        )?;
        Ok(())
    }

    pub fn asset_get(&self, id: &str) -> Result<Option<crate::model::AssetRecord>> {
        use rusqlite::OptionalExtension;
        let row: Option<(String, String, String, Option<String>, Option<String>, Option<String>, i64, i64, String)> = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT id,library_id,source_path,content_hash,pub_title,custom_tags,modified_ms,file_size,json
                 FROM assets_v2 WHERE id=?1",
                params![id],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
            )
            .optional()?;
        row.map(|(id, lib, path, hash, title, tags, ms, sz, json)| {
            Ok(crate::model::AssetRecord {
                id,
                library_id: lib,
                source_path: path,
                content_hash: hash,
                pub_title: title,
                custom_tags_json: tags,
                modified_ms: ms as u64,
                file_size: sz as u64,
                asset: serde_json::from_str(&json)?,
            })
        })
        .transpose()
    }

    pub fn asset_list_for_library(
        &self,
        library_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::model::AssetRecord>> {
        let c = self.0.lock().unwrap();
        let mut q = c.prepare(
            "SELECT id,library_id,source_path,content_hash,pub_title,custom_tags,modified_ms,file_size,json
             FROM assets_v2 WHERE library_id=?1
             ORDER BY modified_ms DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = q.query_map(params![library_id, limit as i64, offset as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
            ))
        })?;
        rows.map(|r| {
            let (id, lib, path, hash, title, tags, ms, sz, json) = r?;
            Ok(crate::model::AssetRecord {
                id,
                library_id: lib,
                source_path: path,
                content_hash: hash,
                pub_title: title,
                custom_tags_json: tags,
                modified_ms: ms as u64,
                file_size: sz as u64,
                asset: serde_json::from_str(&json)?,
            })
        })
        .collect()
    }

    pub fn asset_count_for_library(&self, library_id: &str) -> Result<usize> {
        let n: i64 = self.0.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM assets_v2 WHERE library_id=?1",
            params![library_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn asset_delete_for_library(&self, library_id: &str) -> Result<usize> {
        let n = self.0.lock().unwrap().execute(
            "DELETE FROM assets_v2 WHERE library_id=?1",
            params![library_id],
        )?;
        Ok(n)
    }

    pub fn asset_find_by_hash(
        &self,
        library_id: &str,
        hash: &str,
    ) -> Result<Option<crate::model::AssetRecord>> {
        use rusqlite::OptionalExtension;
        let row: Option<(String,String,String,Option<String>,Option<String>,Option<String>,i64,i64,String)> = self
            .0.lock().unwrap()
            .query_row(
                "SELECT id,library_id,source_path,content_hash,pub_title,custom_tags,modified_ms,file_size,json
                 FROM assets_v2 WHERE library_id=?1 AND content_hash=?2 LIMIT 1",
                params![library_id, hash],
                |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)),
            )
            .optional()?;
        row.map(|(id, lib, path, hash, title, tags, ms, sz, json)| {
            Ok(crate::model::AssetRecord {
                id,
                library_id: lib,
                source_path: path,
                content_hash: Some(hash.unwrap_or_default()),
                pub_title: title,
                custom_tags_json: tags,
                modified_ms: ms as u64,
                file_size: sz as u64,
                asset: serde_json::from_str(&json)?,
            })
        })
        .transpose()
    }

    /// Total asset count across all libraries (for migration progress).
    pub fn asset_total_count(&self) -> Result<usize> {
        let n: i64 =
            self.0
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM assets_v2", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn _schema_version_is_current(&self) -> bool {
        self.schema_version() >= CURRENT_VERSION
    }
}
