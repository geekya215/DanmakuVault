//! Vault directory layout and the SQLite connection.
//!
//! Write model: one `Mutex<Connection>` serializes every write, which is equivalent
//! to a "single writer" but needs no extra tasks/channels. Reads are rare (export,
//! list), so they can share that same connection.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use dv_core::{now_unix, Result, VaultError};

use crate::fsx;
use crate::migrations::{db_err, migrate, schema_version, SCHEMA_VERSION};

/// Fixed paths inside a vault. Every path is assembled here so nothing hard-codes them elsewhere.
#[derive(Clone, Debug)]
pub struct VaultPaths {
    root: PathBuf,
}

impl VaultPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn db_file(&self) -> PathBuf {
        self.root.join("index.sqlite")
    }

    pub fn marker_file(&self) -> PathBuf {
        self.root.join("vault.json")
    }

    pub fn videos_dir(&self) -> PathBuf {
        self.root.join("videos")
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("reports")
    }

    pub fn quarantine_dir(&self) -> PathBuf {
        self.reports_dir().join("quarantine")
    }

    pub fn video_dir(&self, folder: &str) -> PathBuf {
        self.videos_dir().join(folder)
    }

    pub fn page_dir(&self, folder: &str, page: u32) -> PathBuf {
        self.video_dir(folder).join(format!("p{page}"))
    }

    pub fn segments_dir(&self, folder: &str, page: u32) -> PathBuf {
        self.page_dir(folder, page).join("segments")
    }

    pub fn ensure_layout(&self) -> std::io::Result<()> {
        for dir in [
            self.root(),
            &self.videos_dir(),
            &self.blobs_dir(),
            &self.reports_dir(),
        ] {
            fsx::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// `vault.json`: identifies whose database this is and which version, even without the DB.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultMarker {
    pub schema_version: i32,
    pub app_version: String,
    pub created_at: i64,
}

/// Database size summary, used by `init` / `doctor`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VaultStats {
    pub videos: u64,
    pub pages: u64,
    pub danmaku: u64,
    pub missing: u64,
    pub segments: u64,
}

pub struct Vault {
    paths: VaultPaths,
    conn: Mutex<Connection>,
}

impl Vault {
    /// Create (or complete) a vault: make the directories, create the DB, run migrations, write the marker.
    pub fn create(root: impl Into<PathBuf>) -> Result<Self> {
        let paths = VaultPaths::new(root);
        paths.ensure_layout()?;
        let vault = Self::open_paths(paths, false)?;
        vault.ensure_marker()?;
        Ok(vault)
    }

    /// Open an existing vault (does not create directories).
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let paths = VaultPaths::new(root);
        Self::open_paths(paths, true)
    }

    /// In-memory database, for tests only.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(db_err)?;
        let mut conn = conn;
        configure(&conn)?;
        migrate(&mut conn)?;
        Ok(Self {
            paths: VaultPaths::new(std::env::temp_dir().join("dv-memory")),
            conn: Mutex::new(conn),
        })
    }

    fn open_paths(paths: VaultPaths, must_exist: bool) -> Result<Self> {
        if must_exist && !paths.root().exists() {
            return Err(VaultError::Other(format!(
                "vault does not exist: {}",
                paths.root().display()
            )));
        }
        fsx::create_dir_all(paths.root())?;
        let mut conn = Connection::open(paths.db_file()).map_err(db_err)?;
        configure(&conn)?;
        migrate(&mut conn)?;
        Ok(Self {
            paths,
            conn: Mutex::new(conn),
        })
    }

    pub fn paths(&self) -> &VaultPaths {
        &self.paths
    }

    pub fn schema_version(&self) -> Result<i32> {
        let conn = self.lock();
        schema_version(&conn)
    }

    /// Hand the caller exclusive read/write access. Keep write transactions short.
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.lock();
        f(&conn)
    }

    pub fn stats(&self) -> Result<VaultStats> {
        let conn = self.lock();
        let one = |sql: &str| -> Result<u64> {
            conn.query_row(sql, [], |row| row.get::<_, i64>(0))
                .map(|n| n.max(0) as u64)
                .map_err(db_err)
        };
        Ok(VaultStats {
            videos: one("SELECT count(*) FROM video")?,
            pages: one("SELECT count(*) FROM page")?,
            danmaku: one("SELECT count(*) FROM danmaku")?,
            missing: one("SELECT count(*) FROM danmaku WHERE missing_since IS NOT NULL")?,
            segments: one("SELECT count(*) FROM segment")?,
        })
    }

    pub fn read_marker(&self) -> Option<VaultMarker> {
        let raw = std::fs::read_to_string(self.paths.marker_file()).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn ensure_marker(&self) -> Result<()> {
        let path = self.paths.marker_file();
        if path.exists() {
            return Ok(());
        }
        let marker = VaultMarker {
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: now_unix(),
        };
        fsx::write_json_atomic(&path, &marker)?;
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panicked write transaction must not make the whole database unusable: recover the poisoned lock.
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn configure(conn: &Connection) -> Result<()> {
    // journal_mode returns a result row, so this must use query_row rather than pragma_update.
    conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0))
        .map_err(db_err)?;
    conn.execute_batch(
        "PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )
    .map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_makes_layout_and_marker() {
        let root = crate::fsx::scratch_dir("vault-create");
        let _ = std::fs::remove_dir_all(&root);

        let vault = Vault::create(&root).unwrap();
        assert!(vault.paths().db_file().exists());
        assert!(vault.paths().videos_dir().is_dir());
        assert!(vault.paths().reports_dir().is_dir());
        assert_eq!(vault.schema_version().unwrap(), SCHEMA_VERSION);

        let marker = vault.read_marker().unwrap();
        assert_eq!(marker.schema_version, SCHEMA_VERSION);

        let stats = vault.stats().unwrap();
        assert_eq!(stats, VaultStats::default());

        drop(vault);
        // Repeating init should be idempotent
        let again = Vault::create(&root).unwrap();
        assert_eq!(again.schema_version().unwrap(), SCHEMA_VERSION);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_missing_vault_is_an_error() {
        let root = crate::fsx::scratch_dir("vault-absent");
        let _ = std::fs::remove_dir_all(&root);
        assert!(Vault::open(&root).is_err());
    }
}
