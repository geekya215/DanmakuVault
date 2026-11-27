# DanmakuVault

[![CI](https://github.com/geekya215/DanmakuVault/actions/workflows/ci.yml/badge.svg)](https://github.com/geekya215/DanmakuVault/actions/workflows/ci.yml)

Back up every danmaku (bullet comment) of a Bilibili video, and export it as XML that players load directly.

```bash
dv init && dv add BV1GJ411x7h7 && dv sync
```

- **Complete** — reads the 6-minute protobuf segments, not the capped bulk XML endpoint.
- **Resumable** — an interrupted run refetches only the segments it is missing.
- **Non-destructive** — comments removed upstream are tombstoned, never dropped.
- **Verifiable** — `dv verify` cross-checks the database, the files, and upstream.

## Usage

```bash
dv init                 # create vault, database and default config
dv add BV1GJ411x7h7     # register a video (metadata only)
dv sync                 # fetch danmaku
dv export BV1GJ411x7h7  # write XML into the vault
```

Output lands in `<vault>/videos/BV1GJ411x7h7/p1/danmaku.xml`.

| Command | Description |
|---|---|
| `dv init [--root DIR] [--force]` | Create the vault (idempotent) |
| `dv add <ID>...` | Register videos and their parts |
| `dv sync [ID]... [--force] [--dry-run]` | Fetch danmaku; all registered videos when no ID is given |
| `dv list` | List backed-up videos |
| `dv show <ID>` | Parts, segments, counts, sync state |
| `dv export <ID> [--page N\|all] [--format xml\|jsonl] [--out DIR]` | Export danmaku |
| `dv verify [ID] [--deep]` | Check integrity; `--deep` refetches from upstream |
| `dv gc [--older-than-days 30] [--yes]` | Purge old tombstones (dry run unless `--yes`) |
| `dv reindex` | Rebuild the index from `videos/` |
| `dv doctor` | Environment check |

`<ID>` accepts a BV id, an av id, a numeric aid, or a video URL. Short links (`b23.tv`) must be opened in a browser first.

Global flags: `--vault DIR`, `--config FILE`, `--json`, `-v/-vv/-vvv`.
Exit codes: `0` ok · `1` partial failure · `2` bad argument · `3` login required · `4` network down · `5` verification failed.

## Layout

```
<vault>/
├── index.sqlite            # videos, parts, danmaku, segment state
├── vault.json              # format version, creation time
├── videos/<id>/p<N>/       # danmaku.xml (players), danmaku.jsonl (lossless)
└── reports/                # one JSON report per sync
```

The database indexes, the files are the product: `dv export` regenerates files, `dv reindex` rebuilds the index.

## Configuration

```toml
[net]
qps = 2.0                       # requests per second
burst = 4
timeout_secs = 20
max_retries = 5

[sync]
probe_extra_segment = true      # probe one more segment past the computed count
refresh_interval_secs = 21600   # skip parts synced within 6 hours

[export]
write_jsonl = true
```

`concurrency`, `save_raw_segments`, `retain_missing` and `default_format` are reserved and have no effect yet.
Cookies go in the `DV_COOKIE` environment variable, never in the config file.

## How it stays honest

`dv verify` re-renders the database as XML and diffs it against the file on disk; `--deep` refetches every segment and compares SHA-256.

Tombstones are written only after **every** segment of a part was fetched successfully in the same run, so "not fetched" is never mistaken for "deleted upstream". A part that was never fully synced uses the resume path; the second sync does a full refetch. Tombstones survive until `dv gc --yes`.

## Limitations

- Single video, manual backup: no favorites, uploader feeds or watch-later.
- Danmaku only; no video download, no ASS/SRT export.
- BAS and subtitle danmaku (`pool=2`, `mode=9`) are not fetched.
- Segment payloads drift on popular videos, so `verify --deep` reporting changes is expected.
- `reindex` cannot restore segment state; the next `sync` refetches all segments.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT. Danmaku content belongs to its authors and to Bilibili.

Personal backups only; 2 QPS by default; cookies are never logged or written into reports.
