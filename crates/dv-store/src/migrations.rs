//! schema 迁移：`PRAGMA user_version` 递进，只加不改。

use rusqlite::Connection;

use dv_core::{Result, VaultError};

/// 当前程序支持的 schema 版本。
pub const SCHEMA_VERSION: i32 = 1;

/// v1：视频 / 分 P / 弹幕 / 分段状态。
const M001: &str = r#"
CREATE TABLE video (
  aid            INTEGER PRIMARY KEY,
  bvid           TEXT    UNIQUE,
  title          TEXT    NOT NULL,
  owner_mid      INTEGER,
  owner_name     TEXT,
  duration_ms    INTEGER NOT NULL,
  cover_url      TEXT,
  pubdate        INTEGER,
  first_seen_at  INTEGER NOT NULL,
  last_synced_at INTEGER,
  state          TEXT    NOT NULL DEFAULT 'active'
);

CREATE TABLE page (
  cid            INTEGER PRIMARY KEY,
  aid            INTEGER NOT NULL REFERENCES video(aid) ON DELETE CASCADE,
  page           INTEGER NOT NULL,
  part           TEXT    NOT NULL DEFAULT '',
  duration_ms    INTEGER NOT NULL,
  declared_count INTEGER,
  last_synced_at INTEGER,
  UNIQUE (aid, page)
);

CREATE TABLE danmaku (
  cid           INTEGER NOT NULL REFERENCES page(cid) ON DELETE CASCADE,
  dmid          INTEGER NOT NULL,
  progress_ms   INTEGER NOT NULL,
  mode          INTEGER NOT NULL DEFAULT 1,
  fontsize      INTEGER NOT NULL DEFAULT 25,
  color         INTEGER NOT NULL DEFAULT 16777215,
  mid_hash      TEXT    NOT NULL DEFAULT '',
  content       TEXT    NOT NULL,
  ctime         INTEGER NOT NULL DEFAULT 0,
  weight        INTEGER NOT NULL DEFAULT 0,
  pool          INTEGER NOT NULL DEFAULT 0,
  attr          INTEGER NOT NULL DEFAULT 0,
  extras        TEXT,
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  missing_since INTEGER,
  PRIMARY KEY (cid, dmid)
) WITHOUT ROWID;

CREATE INDEX idx_dm_cid_progress ON danmaku (cid, progress_ms);
CREATE INDEX idx_dm_missing ON danmaku (cid) WHERE missing_since IS NOT NULL;

CREATE TABLE segment (
  cid           INTEGER NOT NULL REFERENCES page(cid) ON DELETE CASCADE,
  segment_index INTEGER NOT NULL,
  sha256        BLOB    NOT NULL,
  byte_len      INTEGER NOT NULL,
  n_items       INTEGER NOT NULL,
  source        TEXT    NOT NULL,
  fetched_at    INTEGER NOT NULL,
  PRIMARY KEY (cid, segment_index)
) WITHOUT ROWID;

CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
"#;

pub(crate) fn db_err(err: rusqlite::Error) -> VaultError {
    VaultError::Db(err.to_string())
}

pub fn schema_version(conn: &Connection) -> Result<i32> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_err)
}

/// 把库升到 `SCHEMA_VERSION`；已是最新则什么都不做。
pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current = schema_version(conn)?;
    if current > SCHEMA_VERSION {
        return Err(VaultError::Corrupt(format!(
            "库 schema 版本 {current} 高于本程序支持的 {SCHEMA_VERSION}，请升级 DanmakuVault"
        )));
    }
    if current == SCHEMA_VERSION {
        return Ok(());
    }

    let tx = conn.transaction().map_err(db_err)?;
    if current < 1 {
        tx.execute_batch(M001).map_err(db_err)?;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_from_empty_and_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 0);
        migrate(&mut conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        // 再跑一次不应报错、不应重复建表
        migrate(&mut conn).unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for expected in ["danmaku", "kv", "page", "segment", "video"] {
            assert!(
                tables.iter().any(|t| t == expected),
                "missing table {expected}"
            );
        }
    }

    #[test]
    fn refuses_newer_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(migrate(&mut conn).is_err());
    }
}
