//! SQLite implementation of `Repository`.
//!
//! Every write method opens its own transaction; `Vault` holds a single connection lock inside, which is equivalent to a single writer.

use rusqlite::{params, OptionalExtension};

use dv_core::{
    now_unix, Aid, Bvid, Cid, Danmaku, Dmid, Owner, PageMeta, PageStats, PurgeOutcome, Repository,
    Result, SegmentSource, SegmentState, UpsertStats, VaultError, VideoMeta, VideoSummary,
};

use crate::migrations::db_err;
use crate::vault::Vault;

fn blob_to_hash(blob: Vec<u8>) -> Result<[u8; 32]> {
    blob.try_into()
        .map_err(|_| VaultError::Corrupt("segment.sha256 is not 32 bytes long".into()))
}

impl Repository for Vault {
    fn upsert_video(&self, meta: &VideoMeta) -> Result<()> {
        let now = now_unix();
        let owner_mid = meta.owner.as_ref().map(|o| o.mid as i64);
        let owner_name = meta.owner.as_ref().map(|o| o.name.as_str());

        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction().map_err(db_err)?;

            tx.execute(
                "INSERT INTO video
                   (aid, bvid, title, owner_mid, owner_name, duration_ms, cover_url, pubdate,
                    first_seen_at, last_synced_at, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, 'active')
                 ON CONFLICT(aid) DO UPDATE SET
                   bvid       = excluded.bvid,
                   title      = excluded.title,
                   owner_mid  = excluded.owner_mid,
                   owner_name = excluded.owner_name,
                   duration_ms= excluded.duration_ms,
                   cover_url  = excluded.cover_url,
                   pubdate    = excluded.pubdate,
                   state      = 'active'",
                params![
                    meta.aid.get() as i64,
                    meta.bvid.as_ref().map(|b| b.as_str()),
                    meta.title.as_str(),
                    owner_mid,
                    owner_name,
                    meta.duration_ms as i64,
                    meta.cover_url.as_deref(),
                    meta.pubdate,
                    now,
                ],
            )
            .map_err(db_err)?;

            {
                let mut stmt = tx
                    .prepare_cached(
                        "INSERT INTO page (cid, aid, page, part, duration_ms, declared_count)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(cid) DO UPDATE SET
                           page           = excluded.page,
                           part           = excluded.part,
                           duration_ms    = excluded.duration_ms,
                           declared_count = COALESCE(excluded.declared_count, page.declared_count)",
                    )
                    .map_err(db_err)?;
                for p in &meta.pages {
                    stmt.execute(params![
                        p.cid.get() as i64,
                        meta.aid.get() as i64,
                        p.page as i64,
                        p.part.as_str(),
                        p.duration_ms as i64,
                        p.declared_count.map(|c| c as i64),
                    ])
                    .map_err(db_err)?;
                }
            }

            tx.commit().map_err(db_err)?;
            Ok(())
        })
    }

    fn upsert_danmaku(&self, cid: Cid, items: &[Danmaku], seen_at: i64) -> Result<UpsertStats> {
        if items.is_empty() {
            return Ok(UpsertStats::default());
        }

        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction().map_err(db_err)?;
            let mut inserted = 0u64;

            {
                let mut ins = tx
                    .prepare_cached(
                        "INSERT INTO danmaku
                           (cid, dmid, progress_ms, mode, fontsize, color, mid_hash, content, ctime,
                            weight, pool, attr, extras, first_seen_at, last_seen_at, missing_since)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, NULL)
                         ON CONFLICT(cid, dmid) DO NOTHING",
                    )
                    .map_err(db_err)?;

                for d in items {
                    let changed = ins
                        .execute(params![
                            cid.get() as i64,
                            d.dmid.get() as i64,
                            d.progress_ms,
                            d.mode as i64,
                            d.fontsize as i64,
                            d.color as i64,
                            d.mid_hash.as_str(),
                            d.content.as_str(),
                            d.ctime,
                            d.weight as i64,
                            d.pool as i64,
                            d.attr as i64,
                            d.extras.as_deref(),
                            seen_at,
                        ])
                        .map_err(db_err)?;
                    inserted += changed as u64;
                }
            }

            {
                let mut upd = tx
                    .prepare_cached(
                        "UPDATE danmaku SET last_seen_at = ?1, missing_since = NULL
                         WHERE cid = ?2 AND dmid = ?3",
                    )
                    .map_err(db_err)?;
                for d in items {
                    upd.execute(params![seen_at, cid.get() as i64, d.dmid.get() as i64])
                        .map_err(db_err)?;
                }
            }

            tx.commit().map_err(db_err)?;

            let total = items.len() as u64;
            Ok(UpsertStats { inserted, updated: total.saturating_sub(inserted) })
        })
    }

    fn segment_state(&self, cid: Cid, index: u32) -> Result<Option<SegmentState>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT sha256, byte_len, n_items, source, fetched_at
                     FROM segment WHERE cid = ?1 AND segment_index = ?2",
                    params![cid.get() as i64, index as i64],
                    |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(db_err)?;

            match row {
                None => Ok(None),
                Some((blob, byte_len, n_items, source, fetched_at)) => Ok(Some(SegmentState {
                    index,
                    sha256: blob_to_hash(blob)?,
                    byte_len: byte_len.max(0) as u64,
                    n_items: n_items.max(0) as u32,
                    source: SegmentSource::parse(&source).unwrap_or(SegmentSource::Protobuf),
                    fetched_at,
                })),
            }
        })
    }

    fn put_segment_state(&self, cid: Cid, state: &SegmentState) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO segment (cid, segment_index, sha256, byte_len, n_items, source, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(cid, segment_index) DO UPDATE SET
                   sha256 = excluded.sha256,
                   byte_len = excluded.byte_len,
                   n_items = excluded.n_items,
                   source = excluded.source,
                   fetched_at = excluded.fetched_at",
                params![
                    cid.get() as i64,
                    state.index as i64,
                    &state.sha256[..],
                    state.byte_len as i64,
                    state.n_items as i64,
                    state.source.as_str(),
                    state.fetched_at,
                ],
            )
            .map_err(db_err)?;
            Ok(())
        })
    }

    fn mark_missing(&self, cid: Cid, before: i64) -> Result<u64> {
        self.with_conn(|conn| {
            let changed = conn
                .execute(
                    "UPDATE danmaku SET missing_since = ?1
                     WHERE cid = ?2 AND last_seen_at < ?1 AND missing_since IS NULL",
                    params![before, cid.get() as i64],
                )
                .map_err(db_err)?;
            Ok(changed as u64)
        })
    }

    fn count_missing_before(&self, before: i64) -> Result<u64> {
        self.with_conn(|conn| {
            let count = conn
                .query_row(
                    "SELECT count(*) FROM danmaku WHERE missing_since IS NOT NULL AND missing_since < ?1",
                    params![before],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(db_err)?;
            Ok(count.max(0) as u64)
        })
    }

    fn purge_missing_before(&self, before: i64) -> Result<PurgeOutcome> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction().map_err(db_err)?;

            let affected: Vec<Cid> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT DISTINCT cid FROM danmaku
                         WHERE missing_since IS NOT NULL AND missing_since < ?1",
                    )
                    .map_err(db_err)?;
                let rows = stmt
                    .query_map(params![before], |row| row.get::<_, i64>(0))
                    .map_err(db_err)?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(Cid(row.map_err(db_err)? as u64));
                }
                out
            };

            let deleted = tx
                .execute(
                    "DELETE FROM danmaku WHERE missing_since IS NOT NULL AND missing_since < ?1",
                    params![before],
                )
                .map_err(db_err)?;

            tx.commit().map_err(db_err)?;
            Ok(PurgeOutcome {
                deleted: deleted as u64,
                affected,
            })
        })
    }

    fn page_location(&self, cid: Cid) -> Result<Option<(String, u32)>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT v.bvid, v.aid, p.page
                 FROM page p JOIN video v ON v.aid = p.aid
                 WHERE p.cid = ?1",
                params![cid.get() as i64],
                |row| {
                    let bvid: Option<String> = row.get(0)?;
                    let aid: i64 = row.get(1)?;
                    let page: i64 = row.get(2)?;
                    let folder = bvid.unwrap_or_else(|| format!("av{aid}"));
                    Ok((folder, page as u32))
                },
            )
            .optional()
            .map_err(db_err)
        })
    }

    fn load_danmaku(&self, cid: Cid, include_missing: bool) -> Result<Vec<Danmaku>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT dmid, progress_ms, mode, fontsize, color, mid_hash, content, ctime,
                            weight, pool, attr, extras
                     FROM danmaku WHERE cid = ?1 AND (?2 OR missing_since IS NULL)
                     ORDER BY progress_ms, dmid",
                )
                .map_err(db_err)?;

            let rows = stmt
                .query_map(params![cid.get() as i64, include_missing], |row| {
                    Ok(Danmaku {
                        dmid: Dmid(row.get::<_, i64>(0)? as u64),
                        progress_ms: row.get(1)?,
                        mode: row.get::<_, i64>(2)? as u8,
                        fontsize: row.get::<_, i64>(3)? as u8,
                        color: row.get::<_, i64>(4)? as u32,
                        mid_hash: row.get(5)?,
                        content: row.get(6)?,
                        ctime: row.get(7)?,
                        weight: row.get::<_, i64>(8)? as u32,
                        pool: row.get::<_, i64>(9)? as u8,
                        attr: row.get::<_, i64>(10)? as u32,
                        extras: row.get(11)?,
                    })
                })
                .map_err(db_err)?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(db_err)?);
            }
            Ok(out)
        })
    }

    fn set_page_synced(&self, cid: Cid, at: i64) -> Result<()> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction().map_err(db_err)?;
            tx.execute(
                "UPDATE page SET last_synced_at = ?1 WHERE cid = ?2",
                params![at, cid.get() as i64],
            )
            .map_err(db_err)?;
            tx.execute(
                "UPDATE video SET last_synced_at = ?1
                 WHERE aid = (SELECT aid FROM page WHERE cid = ?2)",
                params![at, cid.get() as i64],
            )
            .map_err(db_err)?;
            tx.commit().map_err(db_err)?;
            Ok(())
        })
    }

    fn page_synced_at(&self, cid: Cid) -> Result<Option<i64>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT last_synced_at FROM page WHERE cid = ?1",
                params![cid.get() as i64],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(db_err)
        })
    }

    fn page_stats(&self, cid: Cid) -> Result<PageStats> {
        self.with_conn(|conn| {
            let (danmaku, missing) = conn
                .query_row(
                    "SELECT count(*), count(missing_since) FROM danmaku WHERE cid = ?1",
                    params![cid.get() as i64],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .map_err(db_err)?;
            let segments = conn
                .query_row(
                    "SELECT count(*) FROM segment WHERE cid = ?1",
                    params![cid.get() as i64],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(db_err)?;

            Ok(PageStats {
                danmaku: danmaku.max(0) as u64,
                missing: missing.max(0) as u64,
                segments: segments.max(0) as u32,
            })
        })
    }

    fn video_by_bvid(&self, bvid: &Bvid) -> Result<Option<VideoMeta>> {
        self.with_conn(|conn| load_video(conn, "bvid = ?1", bvid.as_str()))
    }

    fn video_by_aid(&self, aid: Aid) -> Result<Option<VideoMeta>> {
        self.with_conn(|conn| load_video(conn, "aid = ?1", aid.get() as i64))
    }

    fn list_videos(&self) -> Result<Vec<VideoSummary>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT v.aid, v.bvid, v.title, v.last_synced_at,
                            (SELECT count(*) FROM page p WHERE p.aid = v.aid),
                            (SELECT count(*) FROM danmaku d
                               JOIN page p2 ON p2.cid = d.cid WHERE p2.aid = v.aid),
                            (SELECT count(*) FROM danmaku d
                               JOIN page p3 ON p3.cid = d.cid
                               WHERE p3.aid = v.aid AND d.missing_since IS NOT NULL)
                     FROM video v ORDER BY v.aid",
                )
                .map_err(db_err)?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(VideoSummary {
                        aid: Aid(row.get::<_, i64>(0)? as u64),
                        bvid: row
                            .get::<_, Option<String>>(1)?
                            .and_then(|s| Bvid::parse(&s).ok()),
                        title: row.get(2)?,
                        last_synced_at: row.get(3)?,
                        page_count: row.get::<_, i64>(4)? as u32,
                        danmaku_count: row.get::<_, i64>(5)? as u64,
                        missing_count: row.get::<_, i64>(6)? as u64,
                    })
                })
                .map_err(db_err)?;

            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(db_err)?);
            }
            Ok(out)
        })
    }
}

fn load_video(
    conn: &rusqlite::Connection,
    predicate: &str,
    value: impl rusqlite::ToSql,
) -> Result<Option<VideoMeta>> {
    let sql = format!(
        "SELECT aid, bvid, title, owner_mid, owner_name, duration_ms, cover_url, pubdate
         FROM video WHERE {predicate}"
    );

    let head = conn
        .query_row(&sql, params![value], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        })
        .optional()
        .map_err(db_err)?;

    let Some((aid, bvid, title, owner_mid, owner_name, duration_ms, cover_url, pubdate)) = head
    else {
        return Ok(None);
    };

    let mut stmt = conn
        .prepare("SELECT cid, page, part, duration_ms, declared_count FROM page WHERE aid = ?1 ORDER BY page")
        .map_err(db_err)?;
    let rows = stmt
        .query_map(params![aid], |row| {
            Ok(PageMeta {
                cid: Cid(row.get::<_, i64>(0)? as u64),
                page: row.get::<_, i64>(1)? as u32,
                part: row.get(2)?,
                duration_ms: row.get::<_, i64>(3)? as u64,
                declared_count: row.get::<_, Option<i64>>(4)?.map(|c| c as u32),
            })
        })
        .map_err(db_err)?;

    let mut pages = Vec::new();
    for row in rows {
        pages.push(row.map_err(db_err)?);
    }

    Ok(Some(VideoMeta {
        aid: Aid(aid as u64),
        bvid: bvid.and_then(|s| Bvid::parse(&s).ok()),
        title,
        owner: owner_mid.map(|mid| Owner {
            mid: mid as u64,
            name: owner_name.unwrap_or_default(),
        }),
        duration_ms: duration_ms.max(0) as u64,
        pubdate,
        cover_url,
        pages,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::{SegmentSource, VideoRef};

    fn sample_meta() -> VideoMeta {
        VideoMeta {
            aid: Aid(1),
            bvid: Bvid::parse("BV1GJ411x7h7").ok(),
            // CJK title on purpose: the roundtrip test checks that non-ASCII text survives storage.
            title: "测试稿件".into(),
            owner: Some(Owner {
                mid: 9,
                name: "UP".into(),
            }),
            duration_ms: 400_000,
            pubdate: Some(1_700_000_000),
            cover_url: None,
            pages: vec![PageMeta {
                cid: Cid(11),
                page: 1,
                part: "P1".into(),
                duration_ms: 400_000,
                declared_count: None,
            }],
        }
    }

    fn sample_danmaku(dmid: u64, progress_ms: i64) -> Danmaku {
        Danmaku {
            dmid: Dmid(dmid),
            progress_ms,
            mode: 1,
            fontsize: 25,
            color: 16_777_215,
            mid_hash: "abc".into(),
            content: format!("danmaku {dmid}"),
            ctime: 1_700_000_000,
            weight: 5,
            pool: 0,
            attr: 0,
            extras: None,
        }
    }

    #[test]
    fn roundtrips_video_and_pages() {
        let vault = Vault::open_in_memory().unwrap();
        let meta = sample_meta();
        vault.upsert_video(&meta).unwrap();

        let loaded = vault.video_by_aid(Aid(1)).unwrap().unwrap();
        assert_eq!(loaded.title, "测试稿件");
        assert_eq!(loaded.pages.len(), 1);
        assert_eq!(loaded.pages[0].cid.get(), 11);
        assert_eq!(loaded.owner.unwrap().name, "UP");

        let by_bvid = vault
            .video_by_bvid(&Bvid::parse("BV1GJ411x7h7").unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(by_bvid.aid.get(), 1);
        // Idempotent: a repeated upsert must not create duplicate parts
        vault.upsert_video(&meta).unwrap();
        assert_eq!(vault.video_by_aid(Aid(1)).unwrap().unwrap().pages.len(), 1);
    }

    #[test]
    fn upsert_danmaku_is_idempotent() {
        let vault = Vault::open_in_memory().unwrap();
        vault.upsert_video(&sample_meta()).unwrap();

        let batch = vec![sample_danmaku(100, 1000), sample_danmaku(101, 2000)];
        let first = vault.upsert_danmaku(Cid(11), &batch, 5_000).unwrap();
        assert_eq!(
            first,
            UpsertStats {
                inserted: 2,
                updated: 0
            }
        );

        let second = vault.upsert_danmaku(Cid(11), &batch, 6_000).unwrap();
        assert_eq!(
            second,
            UpsertStats {
                inserted: 0,
                updated: 2
            }
        );

        let loaded = vault.load_danmaku(Cid(11), true).unwrap();
        assert_eq!(loaded.len(), 2);
        // Chronological order is stable
        assert_eq!(loaded[0].dmid.get(), 100);
        assert_eq!(loaded[1].dmid.get(), 101);
    }

    #[test]
    fn mark_missing_only_affects_stale_rows() {
        let vault = Vault::open_in_memory().unwrap();
        vault.upsert_video(&sample_meta()).unwrap();
        vault
            .upsert_danmaku(Cid(11), &[sample_danmaku(1, 0)], 1_000)
            .unwrap();
        vault
            .upsert_danmaku(Cid(11), &[sample_danmaku(2, 0)], 3_000)
            .unwrap();

        let marked = vault.mark_missing(Cid(11), 2_000).unwrap();
        assert_eq!(marked, 1);
        // Running it again must not count twice (the tombstone already has a value)
        assert_eq!(vault.mark_missing(Cid(11), 2_000).unwrap(), 0);

        // Tombstones are kept by default but can be excluded when reading
        assert_eq!(vault.load_danmaku(Cid(11), true).unwrap().len(), 2);
        let visible = vault.load_danmaku(Cid(11), false).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].dmid.get(), 2);
    }

    #[test]
    fn segment_state_roundtrip() {
        let vault = Vault::open_in_memory().unwrap();
        vault.upsert_video(&sample_meta()).unwrap();
        assert!(vault.segment_state(Cid(11), 1).unwrap().is_none());

        let state = SegmentState {
            index: 1,
            sha256: [7u8; 32],
            byte_len: 12,
            n_items: 3,
            source: SegmentSource::Protobuf,
            fetched_at: 123,
        };
        vault.put_segment_state(Cid(11), &state).unwrap();
        assert_eq!(vault.segment_state(Cid(11), 1).unwrap().unwrap(), state);
    }

    #[test]
    fn lists_videos_with_counts() {
        let vault = Vault::open_in_memory().unwrap();
        vault.upsert_video(&sample_meta()).unwrap();
        vault
            .upsert_danmaku(Cid(11), &[sample_danmaku(1, 0), sample_danmaku(2, 1)], 10)
            .unwrap();

        let list = vault.list_videos().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].danmaku_count, 2);
        assert_eq!(list[0].page_count, 1);
        assert!(list[0].bvid.is_some());
    }

    #[test]
    fn purge_reports_affected_pages() {
        let vault = Vault::open_in_memory().unwrap();
        vault.upsert_video(&sample_meta()).unwrap();
        vault
            .upsert_danmaku(Cid(11), &[sample_danmaku(1, 0)], 1_000)
            .unwrap();

        // No candidates until a tombstone has been set
        assert_eq!(vault.count_missing_before(2_001).unwrap(), 0);

        vault.mark_missing(Cid(11), 2_000).unwrap();
        assert_eq!(vault.count_missing_before(2_001).unwrap(), 1);

        let outcome = vault.purge_missing_before(2_001).unwrap();
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.affected, vec![Cid(11)]);
        assert!(vault.load_danmaku(Cid(11), true).unwrap().is_empty());

        // After gc, this is what lets the files be redrawn on disk
        assert_eq!(
            vault.page_location(Cid(11)).unwrap(),
            Some(("BV1GJ411x7h7".to_string(), 1))
        );
    }

    #[test]
    fn resolves_by_bvid_and_aid() {
        let vault = Vault::open_in_memory().unwrap();
        vault.upsert_video(&sample_meta()).unwrap();
        let reference = VideoRef::parse_input("BV1GJ411x7h7").unwrap();
        let found = vault
            .video_by_bvid(reference.bvid().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(found.aid.get(), 1);
    }
}
