# DanmakuVault 设计文档

> 面向 B 站弹幕的本地备份工具（Rust）。本文是可执行的工程方案：包含接口事实、数据模型、
> 同步算法、目录布局、CLI 契约与测试策略，可直接据此拆任务开工。

- 状态：Draft v0.1
- 许可：MIT
- 目标平台：Windows / macOS / Linux（单一静态二进制）

---

## 0. TL;DR

一句话：**把「一段视频的弹幕」当作一个可增量、可校验、可重放的快照集合来管理**，
用 protobuf 分段接口做数据源（完整），用 SQLite 做索引（可检索、可断点），
用「只增不删 + 墓碑」做备份语义（不丢数据），用规范化 XML 做导出（播放器兼容）。

| # | 关键决策 | 结论 | 理由（详见对应章节） |
|---|---|---|---|
| D1 | 弹幕数据源 | **`seg.so` protobuf 分段为主**，XML 接口仅用于兼容导出/兜底 | XML 接口是整包快照且有条数上限，大流量视频会被截断，无法增量（§3.2） |
| D2 | 索引存储 | SQLite（WAL）+ 单写者任务 | 需要按 cid/progress/内容检索，纯文件方案会退化为全量扫描（§5） |
| D3 | 删除语义 | 备份侧**永不自动删除**，只打 `missing_since` 墓碑；`gc` 需显式确认 | 备份工具的第一职责是不丢数据；B 站会吞弹幕（§6.4） |
| D4 | 幂等/断点粒度 | **段级**（segment）+ 段内容 sha256 | 一集 2h 视频约 20 段，段级失败隔离比整视频重传划算（§6.3） |
| D5 | 并发模型 | tokio 多任务 + 全局信号量 + 令牌桶限速 + 单写者通道 | 既要吞吐又要不触发风控，还要避免 `database is locked`（§7） |
| D6 | 导出格式 | XML（兼容）/ JSONL（无损）/ ASS、SRT（可选） | 用户最终是要喂给播放器或剪辑软件（§8） |
| D7 | protobuf 绑定 | `prost` derive 手写 tag，**不用 build.rs 调 protoc** | 免去 CI/用户机器上的 protoc 依赖，字段少且稳定（§12） |
| D8 | 认证 | 匿名可跑，cookie 可选；cookie 只从文件/环境变量读，永不入日志 | 匿名限速更严、部分稿件需登录（§7.4） |

---

## 1. 目标与范围

### 1.1 目标

1. **完整**：单 P 视频的弹幕一条不少（含普通池弹幕；特殊弹幕列为可选能力）。
2. **可增量**：同一视频反复同步，只拉变化的部分，新增弹幕能追上。
3. **可恢复**：进程被杀、断网、磁盘写一半，重启后能继续，不产生半截文件。
4. **可校验**：能回答「这份备份是不是完整的、什么时候抓的、丢了什么」。
5. **可消费**：导出的文件能直接丢进常见播放器 / 弹幕转换工具 / 剪辑软件。

### 1.2 非目标

- 不下载视频画面/音频（只做弹幕；如需整站备份请用其他工具）。
- 不做弹幕发送、点赞、举报等写操作。
- 不做全站爬取、不做「按关键词搜全站弹幕」（需登录+风控，不在 v1）。
- 不做 GUI 播放器。v1 是 CLI；TUI 进度条算在内。

### 1.3 用户故事（验收口径）

| ID | 故事 | 验收标准 |
|---|---|---|
| US-1 | 我粘贴一个 BV 号，就要拿到它的全部弹幕 | `dv add BV1xx && dv sync` 后 `videos/BV1xx/p1/danmaku.xml` 可被本地播放器加载 |
| US-2 | 一个月后再同步，要能拿到新弹幕，且不重复 | 第二次同步 `danmaku_added > 0` 时文件条数增加，`SELECT count(*)` 无重复 dmid |
| US-3 | 同步到一半我按了 Ctrl-C，重跑要能接着来 | 重跑只拉未完成的段；已存在段不重复请求 |
| US-4 | 我要把弹幕转成字幕做二创 | `dv export BV1xx --format ass` 生成可导入剪映/Premiere 的 ASS |
| US-5 | 我要确认备份没坏 | `dv verify --deep` 逐段校验 sha256 并对比条数 |

---

## 2. 领域背景：B 站弹幕的三种形态

理解这三点，整个设计就顺了：

1. **弹幕是「按视频时间轴分布的时间序列」**，服务端按 **6 分钟一段** 切分存储。
   分段号 `segment_index` 从 1 开始，第 n 段覆盖 `[(n-1)*6min, n*6min)`。
   → 天然适合断点续传：段是独立的、可校验的最小单元。
2. **同一条弹幕有稳定的全局 ID（dmid）**，且 `dmid` 在 XML 与 protobuf 两种格式里都存在。
   → 天然适合幂等去重：`(cid, dmid)` 做主键，重复抓取无副作用。
3. **服务端会「吞」弹幕**（审核删除、UP 主清理、风控折叠）。
   → 备份工具必须区分「这次没抓到」和「服务端确实没有了」，且默认不因此删除本地数据。

---

## 3. 接口调研结论

### 3.1 端点清单

| 用途 | 端点 | 认证 | 响应 | 备注 |
|---|---|---|---|---|
| 视频元信息 | `GET api.bilibili.com/x/web-interface/view?bvid=` 或 `?aid=` | 无 | JSON | `data.aid`、`data.cid`、`data.pages[]`、`data.duration` |
| 分 P 列表 | `GET api.bilibili.com/x/player/pagelist?bvid=` | 无 | JSON | `cid`/`page`/`part`/`duration` |
| **弹幕分段** | `GET api.bilibili.com/x/v2/dm/web/seg.so?type=1&oid={cid}&pid={aid}&segment_index={n}` | 可选 cookie | **protobuf** | 主数据源；每段 6min |
| 弹幕元数据 | `GET api.bilibili.com/x/v2/dm/web/view?type=1&oid={cid}&pid={aid}` | 可选 cookie | **protobuf** | `DmWebViewReply`：总条数、段配置、特殊弹幕 |
| 旧整包 XML | `GET api.bilibili.com/x/v1/dm/list.so?oid={cid}` | 可选 cookie | **deflate + XML** | 有上限，见 §3.2 |
| 收藏夹内容 | `GET api.bilibili.com/x/v3/fav/resource/list?media_id=&pn=&ps=20` | **cookie** | JSON | 私密收藏夹必须登录 |
| UP 主投稿 | `GET api.bilibili.com/x/space/wbi/arc/search?mid=&ps=30&pn=1` | **cookie + WBI 签名** | JSON | 需要 `w_rid`/`wts`，签名逻辑独立成模块 |
| 稍后再看 | `GET api.bilibili.com/x/v2/history/toview` | **cookie** | JSON | 批量目标的另一种来源 |

> 所有请求建议统一带 `Referer: https://www.bilibili.com/`、桌面 Chrome 的 `User-Agent`，
> 并维持一个 cookie jar（至少 `buvid3` / `SESSDATA`）。缺少 `buvid3` 时可通过
> `GET api.bilibili.com/x/frontend/finger/spi` 领取后写入 jar。

### 3.2 为什么主用 protobuf 分段而不是 XML

| 维度 | `seg.so`（protobuf） | `list.so`（XML） |
|---|---|---|
| 完整性 | 分段返回该 6min 窗口内的全部弹幕 | 整包返回，但**有条数上限**（常见 3000/6000，随视频规模变化）；爆款视频会被截断 |
| 粒度 | 6min/段，可增量、可断点、可校验 | 全量整包，每次都重传 |
| 压缩 | protobuf 本身紧凑，可再 gzip | deflate（raw，需 `inflate` 无 zlib 头） |
| 解析 | 需要 proto 定义 | 需要 XML + `p` 属性解析 |
| 字段 | 更全（`weight`/`attr`/`pool`/`idStr`） | `p` 属性里字段较少，但含 `uid` 哈希 |

**结论**：数据源统一抽象为 `DanmakuSource` trait（§4.2），默认实现走分段 protobuf；
XML 实现只用于「拿一份和播放器一致的东西」以及 protobuf 接口异常时的兜底。
两者都产出同一个领域模型 `Danmaku`，去重键都是 `dmid`。

### 3.3 弹幕分段 protobuf 结构

```protobuf
syntax = "proto3";

message DanmakuElem {
  int64  id       = 1;   // dmid（弹幕 ID）
  int32  progress = 2;   // 相对视频起点的毫秒偏移
  int32  mode     = 3;   // 1/2/3 滚动，4 底部，5 顶部，6 逆向，7 高级，8 代码，9 BAS
  int32  fontsize = 4;   // 字号
  uint32 color    = 5;   // 十进制 RGB，如 16777215 = 白
  string midHash  = 6;   // 发送者 UID 的哈希（不是明文 UID）
  string content  = 7;   // 弹幕文本
  int64  ctime    = 8;   // 发送时间（unix 秒）
  int32  weight   = 9;   // 权重（影响折叠/展示优先级）
  string action   = 10;  // 动作（特殊弹幕用）
  int32  pool     = 11;  // 0/1 普通池，2 字幕池（BAS）
  string idStr    = 12;  // dmid 的字符串形式
  int32  attr     = 13;  // 位标志
}

message DmSegMobileReply {
  repeated DanmakuElem elems = 1;
}
```

要点：

- 字段号请与 [bilibili-API-collect](https://github.com/SocialSisterYi/bilibili-API-collect) 的
  `danmaku/danmaku_proto.md` 对照一次再落地；新增字段（如动画相关）一律靠**忽略未知字段**兼容，
  `prost` 默认就是这样，不要用 `#[prost(...)]` 写死 denylist。
- 同一 `dmid` 在 protobuf 与 XML 里都稳定，可跨通道去重。
- `pool = 2` / `mode = 9` 的 **BAS 弹幕**（高级/字幕弹幕）在 `seg.so` 里通常拿不到，
  需要从 `dm/web/view` 的 `specialDms` 字段单独取。列为 **可选能力**（`--include-special`），
  v1 不做，文档里标注为已知限制。

### 3.4 旧 XML 的 `p` 属性

```xml
<i>
  <chatserver>chat.bilibili.com</chatserver>
  <chatid>1176840</chatid>
  <mission>0</mission>
  <maxlimit>1500</maxlimit>
  <state>0</state>
  <real_name>0</real_name>
  <source>k-v</source>
  <d p="29.35200,1,25,16777215,1565103107,0,f7a4e1a1,1565103107899">前方高能</d>
</i>
```

`p` 字段顺序（8 段，以逗号分隔）：

| # | 名称 | 类型 | 说明 |
|---|---|---|---|
| 1 | `time` | f64 | 秒，5 位小数 |
| 2 | `mode` | int | 同 protobuf `mode` |
| 3 | `size` | int | 字号 |
| 4 | `color` | int | 十进制 RGB |
| 5 | `timestamp` | int | 发送时间 unix 秒 |
| 6 | `pool` | int | 池 |
| 7 | `uid_hash` | string | 发送者 UID 哈希（对应 `midHash`） |
| 8 | `rowID` | string | dmid（对应用 `idStr`/`id`） |

解析注意：

- 文本里可能有逗号、引号、`&`、`<`、emoji；XML 实体要按规范反转义，**不要用字符串切分解析**。
- 部分历史数据 `p` 有第 9 字段，解析时**按需读取前 8 个，多余字段保留到 `extras`** 以免再导出丢失。
- `list.so` 返回的是 **raw deflate**（无 zlib 头），要用 `flate2::Decompress::new(false)`。
- 少数情况下直接返回明文 XML 或 gzip，需要按 magic bytes 嗅探：`0x1f 0x8b` → gzip，
  其他先试 raw deflate，失败再当明文 XML。

---

## 4. 总体架构

### 4.1 分层与依赖方向

```
┌──────────────────────────────────────────────────────────────┐
│ dv-cli        clap 参数解析 / 配置加载 / 进度条 / 退出码      │
└───────────────┬──────────────────────────────────────────────┘
                │
┌───────────────▼──────────────────────────────────────────────┐
│ dv-sync       编排引擎：plan → fetch → merge → persist       │
│               （不关心 HTTP 细节，也不关心 SQL 细节）        │
└───────┬─────────────────────┬──────────────────┬─────────────┘
        │                     │                  │
┌───────▼────────┐  ┌─────────▼────────┐  ┌──────▼──────────┐
│ dv-bili        │  │ dv-store         │  │ dv-export       │
│ HTTP 客户端    │  │ SQLite 仓储      │  │ XML/JSONL/ASS   │
│ 限流/重试/认证 │  │ 文件/blob 存储   │  │ SRT             │
└───────┬────────┘  └─────────┬────────┘  └──────┬──────────┘
        │                     │                  │
┌───────▼─────────────────────▼──────────────────▼──────────┐
│ dv-core      领域模型 / ID newtype / trait 定义 / 错误类型 │
│              Danmaku, VideoMeta, PageMeta, VaultError      │
└────────────────────────────────────────────────────────────┘
```

硬规则：

- **依赖只能向左/向下**：`dv-core` 不依赖任何其他 crate；`dv-sync` 不直接 `use reqwest`，也不直接
  `use rusqlite`。这样换数据源（比如加 A 站）或换存储（比如换 `redb`）不用改引擎。
- `dv-core` 里定义 trait，具体实现在外层，方便测试用 fake 替身。

### 4.2 关键 trait

```rust
// dv-core/src/source.rs
#[async_trait::async_trait]
pub trait DanmakuSource: Send + Sync {
    /// 视频元信息（含分 P）
    async fn video(&self, target: &Target) -> Result<VideoMeta, SourceError>;

    /// 第 idx 段（1-based）。返回原始字节 + 解析后的弹幕 + 内容哈希。
    async fn segment(&self, req: SegmentReq) -> Result<Segment, SourceError>;

    /// 可选：整包 XML（仅兼容导出/兜底）
    async fn legacy_xml(&self, cid: Cid) -> Result<Bytes, SourceError>;
}

// dv-core/src/repo.rs
#[async_trait::async_trait]
pub trait Repository: Send + Sync {
    async fn upsert_video(&self, v: &VideoMeta) -> Result<()>;
    async fn upsert_danmaku(&self, cid: Cid, items: &[Danmaku], seen_at: Timestamp) -> Result<UpsertStats>;
    async fn segment_state(&self, cid: Cid, idx: u32) -> Result<Option<SegmentState>>;
    async fn save_segment_state(&self, cid: Cid, idx: u32, st: &SegmentState) -> Result<()>;
    async fn mark_missing(&self, cid: Cid, before: Timestamp) -> Result<u64>;
    async fn load_page(&self, cid: Cid) -> Result<Vec<Danmaku>>;
}
```

> `Repository` 的异步写法有两种落地方式，见 §5.4（单写者 + `oneshot` 回执）。

---

## 5. 数据模型与存储

### 5.1 领域类型（`dv-core`）

```rust
pub struct Bvid(String);      // 校验 ^BV[0-9A-Za-z]{10}$
pub struct Aid(u64);
pub struct Cid(u64);
pub struct Dmid(u64);

pub enum Target { Bvid(Bvid), Aid(Aid), Ep(i64) }   // ep 预留番剧

pub struct VideoMeta {
    pub aid: Aid, pub bvid: Option<Bvid>, pub title: String,
    pub owner: Option<(u64, String)>,
    pub duration_ms: u64,
    pub pages: Vec<PageMeta>,
}

pub struct PageMeta {
    pub cid: Cid, pub page: u32, pub part: String,
    pub duration_ms: u64,
}

pub struct Danmaku {
    pub dmid: Dmid,
    pub progress_ms: i64,     // 用毫秒整数，别用 f64：避免累加误差和排序坑
    pub mode: u8,
    pub fontsize: u8,
    pub color: u32,
    pub mid_hash: String,
    pub content: String,
    pub ctime: i64,
    pub weight: u32,
    pub pool: u8,
    pub attr: u32,
    pub extras: Option<String>, // 未知/额外字段的原文，保证再导出不丢
}
```

设计取舍：

- **时间用毫秒 `i64`**。XML 里是 `29.35200` 秒，导出时再格式化回 5 位小数；
  `f64` 参与 `Ord`/`Hash` 会带来麻烦，且排序不稳定。
- **ID 用 newtype**，防止把 `aid` 当 `cid` 传（这类 bug 在 B 站工具里非常常见且难查）。
- `content` 保留原样（含 emoji、`[表情]` 之类的 token），不做归一化；
  去重只看 `dmid`，不看内容。

### 5.2 SQLite 表结构

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;   -- 备份可接受；崩溃丢最后一两条可重抓
PRAGMA foreign_keys = ON;

-- 视频（稿件）
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
  state          TEXT    NOT NULL DEFAULT 'active'   -- active|gone|private
);

-- 分 P
CREATE TABLE page (
  cid            INTEGER PRIMARY KEY,
  aid            INTEGER NOT NULL REFERENCES video(aid) ON DELETE CASCADE,
  page           INTEGER NOT NULL,
  part           TEXT,
  duration_ms    INTEGER NOT NULL,
  -- 服务端声明的弹幕总数（来自 dm/web/view），用于完整性对比
  declared_count INTEGER,
  last_synced_at INTEGER,
  UNIQUE (aid, page)
);

-- 弹幕：核心表
CREATE TABLE danmaku (
  cid          INTEGER NOT NULL,
  dmid         INTEGER NOT NULL,
  progress_ms  INTEGER NOT NULL,
  mode         INTEGER NOT NULL,
  fontsize     INTEGER NOT NULL,
  color        INTEGER NOT NULL,
  mid_hash     TEXT,
  content      TEXT    NOT NULL,
  ctime        INTEGER,
  weight       INTEGER,
  pool         INTEGER,
  attr         INTEGER,
  extras       TEXT,
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  missing_since INTEGER,           -- 非 NULL 表示最近一次全量同步没再看到（墓碑）
  PRIMARY KEY (cid, dmid)
) WITHOUT ROWID;

CREATE INDEX idx_dm_cid_progress ON danmaku (cid, progress_ms);
CREATE INDEX idx_dm_missing      ON danmaku (cid) WHERE missing_since IS NOT NULL;

-- 段状态：断点续传与校验的依据
CREATE TABLE segment (
  cid           INTEGER NOT NULL,
  segment_index INTEGER NOT NULL,
  sha256        BLOB    NOT NULL,  -- 响应体（解压后 protobuf）的哈希
  byte_len      INTEGER NOT NULL,
  n_items       INTEGER NOT NULL,
  source        TEXT    NOT NULL,  -- 'pb' | 'xml'
  fetched_at    INTEGER NOT NULL,
  PRIMARY KEY (cid, segment_index)
) WITHOUT ROWID;

-- 一次同步运行的汇总（用户可回看「上次到底做了什么」）
CREATE TABLE sync_run (
  id              INTEGER PRIMARY KEY,
  started_at      INTEGER NOT NULL,
  finished_at     INTEGER,
  trigger         TEXT    NOT NULL,   -- manual|daemon|add
  videos_ok       INTEGER NOT NULL DEFAULT 0,
  videos_failed   INTEGER NOT NULL DEFAULT 0,
  pages_ok        INTEGER NOT NULL DEFAULT 0,
  pages_failed    INTEGER NOT NULL DEFAULT 0,
  danmaku_added   INTEGER NOT NULL DEFAULT 0,
  danmaku_missing INTEGER NOT NULL DEFAULT 0,
  status          TEXT    NOT NULL,   -- running|ok|partial|failed
  report_path     TEXT
);

CREATE TABLE run_error (
  id     INTEGER PRIMARY KEY,
  run_id INTEGER NOT NULL REFERENCES sync_run(id) ON DELETE CASCADE,
  aid    INTEGER, cid INTEGER, segment_index INTEGER,
  kind   TEXT NOT NULL,       -- http|api|parse|io|rate_limited
  message TEXT NOT NULL,
  at     INTEGER NOT NULL
);

-- 元数据 / schema 版本
CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
```

迁移：用 `PRAGMA user_version` + 一个 `&[Migration]` 数组，启动时顺序应用，事务包裹。
**只加不改**：已发布的表结构不做破坏性变更，废弃列保留。

### 5.3 vault 目录布局

```
<vault>/
├── index.sqlite  (+ -wal, -shm)
├── blobs/                          # 可选：原始段二进制，内容寻址
│   └── ab/cd1234...zst             # sha256 前 2 位做分片目录
├── videos/
│   └── BV1xx411c7mD/
│       ├── meta.json               # 冗余快照，脱离 DB 也能读懂
│       ├── p1/
│       │   ├── danmaku.xml         # 规范化 XML（播放器可直接加载）
│       │   ├── danmaku.jsonl.zst   # 无损内部格式（导出/再处理的真源）
│       │   └── segments/
│       │       ├── 0001.pb.zst     # 原始响应缓存（--save-raw 时）
│       │       └── 0002.pb.zst
│       └── p2/
└── reports/
    └── 20261121T234000Z.json       # 每次 sync 的机器可读报告
```

为什么 DB 和文件**双写**：

- DB 是查询/增量/断点的真源；文件是**人类可读、可搬运、可被其他工具消费**的产物。
- 文件丢失可由 DB 重建（`dv repair`）；DB 丢失可由 `meta.json` + 文件部分重建索引（`dv reindex`）。
- 这只是**冗余**，不是「两套真源」：以 DB 为准，文件是派生物。

### 5.4 写入模型：单写者

SQLite 在 WAL 下支持一写多读，但多任务并发写仍会遇到 `SQLITE_BUSY`。
方案：**所有写操作序列化到一个专用任务**，通过 `mpsc` 提交 `WriteOp`，用 `oneshot` 回执：

```rust
enum WriteOp {
    UpsertVideo(VideoMeta),
    UpsertDanmaku { cid: Cid, items: Vec<Danmaku>, seen_at: Timestamp, ack: oneshot::Sender<UpsertStats> },
    SaveSegment { .., ack: oneshot::Sender<()> },
    FinishRun { .., ack: oneshot::Sender<()> },
}
```

写入批次每 500~2000 条一个事务；读操作走独立的 reader 连接（`r2d2` 或 `deadpool-sqlite` 池）。
若不想引入任务模型，`tokio-rusqlite` 也能达到同样效果（它就是「单连接 + 专用线程」）。

---

## 6. 同步引擎

### 6.1 数据流

```
输入（BV / aid / 收藏夹 / UP 主）
        │  resolve
        ▼
   Vec<SyncTarget { aid, bvid, pages }>
        │  plan（跳过未到期、过滤已完成）
        ▼
   任务队列 ──(并发 = sync.concurrency)──►
        │
        ├─ fetch video meta ──► upsert video/page
        ├─ for each page:
        │     ├─ 决定段数 N = ceil(duration_ms / 360_000)
        │     ├─ for seg in 1..=N:  ├ 命中缓存/未变化 → skip
        │     │                    └ 拉取 → 解析 → 批量 upsert
        │     ├─ 全段成功 → mark_missing(cid, run_start)
        │     └─ 合并 → 原子写 danmaku.xml / jsonl
        └─ 汇总 run 报告
```

### 6.2 段数的不确定性

`ceil(duration/6min)` 是**下界**：部分视频的实际弹幕段数会比这个多（弹幕超过视频时长、
或服务端分段策略差异）。处理方式：

1. 优先用 `dm/web/view` 返回的段配置（`dmSge.total`）拿准确段数；
2. 拿不到时按 `ceil(duration/6min)` 拉，**再多探 1 段**；
3. 探测段返回空 → 停止；返回非空 → 继续探下一段（设上限，如 `N + 5`）。

这个「多探一段」的容错很重要：少一段就是几百条弹幕的静默丢失。

### 6.3 幂等与断点

三条规则保证「随便重跑」：

1. **去重键** `(cid, dmid)`，`INSERT ... ON CONFLICT(cid, dmid) DO UPDATE SET last_seen_at=?, missing_since=NULL`。
   重抓同一段不会产生重复行，也不会改变已有内容（只刷新"最近看到"时间）。
2. **段级跳过**：`segment` 表里已有 `(cid, idx)` 且 `--force` 未给 → 跳过。
   `--refresh` 时则先发请求，用响应体 sha256 与记录比较：**相同就只更新 `fetched_at`**，
   避免了无谓的解析和写入（大视频省很多 CPU）。
3. **崩溃安全**：文件写入一律 临时文件 → `fsync` → `rename` 覆盖；
   DB 事务提交才算完成。任何一个环节中断，重跑都从「最后的完整状态」继续。

> 注意：`--refresh` 语义是「确认服务端有没有变化」，不是「强制重写文件」。
> 区别体现在 `run` 报告里：`segments_unchanged` vs `segments_refetched`。

### 6.4 删除/消失语义

备份工具最危险的默认行为是「同步 = 镜像」。这里明确：

- 一次 page 的**全部段**都成功拉取后，才执行：
  ```sql
  UPDATE danmaku SET missing_since = :run_start
   WHERE cid = :cid AND last_seen_at < :run_start AND missing_since IS NULL;
  ```
- `missing_since` 非空 = 「上次全量没看到它」。**数据仍在**，导出默认包含（可 `--exclude-missing`）。
- 部分段失败时**绝不**执行上面的语句（否则会把没抓到的段误标为消失）。
- 真正的物理删除只能由 `dv gc --older-than 30d --yes` 触发，且先输出将删除的行数与样例。

### 6.5 失败隔离

| 失败类型 | 处理 |
|---|---|
| 单段 5xx / 超时 | 重试（指数退避）；仍失败 → 记 `run_error`，继续下一段 |
| 单段解析失败 | 不重试；记错误 + 保存原始字节到 `reports/quarantine/` 便于复现 |
| 视频级 API 错误（-404 稿件不存在） | 标 `video.state='gone'`，下次默认跳过，`--force` 可重试 |
| `-412` 风控 | **全局熔断**：停止所有请求，休眠（默认 10min，指数递增），恢复后从断点继续 |
| 磁盘满 / 权限 | 立刻中止整个 run，保留已完成部分，退出码非 0 |

全局熔断用 `Arc<AtomicBool>` + `tokio::sync::Notify`：任一任务触发后，其他任务在发请求前检查并挂起。

---

## 7. 网络层

### 7.1 客户端构造

```rust
let client = reqwest::Client::builder()
    .user_agent(cfg.net.user_agent)
    .timeout(Duration::from_secs(cfg.net.timeout_secs))
    .connect_timeout(Duration::from_secs(5))
    .cookie_provider(jar.clone())       // 持久化的 cookie jar
    .gzip(true)                          // 注意：deflate 不要开，list.so 是 raw deflate 需自己处理
    .http2_adaptive_window(true)
    .build()?;
```

每类请求加默认 header：`Referer`、`Origin`（部分接口需要）、`Accept`。

### 7.2 限速

- **全局限速**：令牌桶，默认 `qps = 2.0`、`burst = 4`（保守值，可通过配置放宽到 5~10）。
  手写 30 行即可，不必引 `governor`：

  ```rust
  struct RateLimiter { inner: Mutex<Bucket> }  // tokens: f64, last: Instant
  impl RateLimiter {
      async fn acquire(&self) { /* 计算需等待时长 -> tokio::time::sleep */ }
  }
  ```

- **并发上限**：`Semaphore::new(cfg.sync.concurrency)`（默认 4）。
  并发 ≠ QPS：并发控制在途请求数，QPS 控制在单位时间请求数，两者都要有。
- **抖动**：同一批段的请求间隔加 `±20%` 随机，避免整点齐发。

### 7.3 重试

```rust
RetryPolicy {
    max_attempts: 5,
    base_delay: 500ms,
    max_delay: 30s,
    jitter: 0.3,
    retry_on: [Connect, Timeout, Status(429), Status(500..=599), Api(-412), Api(-509)],
}
```

- `-412`（风控）单独处理：延迟 ≥ 60s，且计入全局熔断计数；连续 3 次 → 熔断。
- `-509`（带宽超限）：延迟 5min 级。
- 业务错误（`-404`、`62002` 稿件不可见）**不重试**。
- 用 `tokio::time::pause()` 做时间可控的单元测试。

### 7.4 认证与安全

- cookie 三个来源，优先级：`--cookie` 参数 > 环境变量 `DV_COOKIE` > 配置文件 `~/.config/danmaku-vault/credentials.toml`。
- 文件权限在 Unix 上设为 `0600`；Windows 上至少不放在共享目录。
- **绝不**把 cookie 写进日志、报错、报告 JSON；`Debug` 实现要手动脱敏。
- 诊断命令 `dv doctor` 只报告「cookie 是否存在、是否过期（调用 `/x/web-interface/nav` 判断）」，不打印值。
- 匿名模式可用但更易触发风控；文档里明确建议登录。

### 7.5 WBI 签名（仅批量来源需要）

`/x/space/wbi/arc/search` 这类接口需要 `w_rid` + `wts`：

1. 拉 `api.bilibili.com/x/web-interface/nav`，取 `wbi_img.img_url` / `sub_url` 的文件名；
2. 拼成 64 字符 `imgKey + subKey`，按固定置换表取前 32 位得 `mixin_key`；
3. 参数按 key 升序拼接 + `wts`，`sha256(query + mixin_key)` 得 `w_rid`。

放在 `dv-bili/src/wbi.rs`，纯函数、可单测（给定输入断言输出）。密钥每日轮换，进程内缓存 1 小时。

---

## 8. 导出

### 8.1 格式矩阵

| 格式 | 用途 | 有损？ |
|---|---|---|
| `xml` | 喂给播放器 / DanmakuFactory 等 | 少（`extras` 之外的字段都能还原） |
| `jsonl` | 内部真源、再做二次处理 | 无损（含全部字段 + 抓取元信息） |
| `ass` | 剪辑/压制，烧进视频 | 有损（布局要算轨道） |
| `srt` | 只当字幕看 | 严重有损（无颜色/模式） |

### 8.2 规范 XML 输出

```xml
<?xml version="1.0" encoding="UTF-8"?>
<i>
  <chatserver>chat.bilibili.com</chatserver>
  <chatid>1176840</chatid>
  <mission>0</mission>
  <maxlimit>0</maxlimit>
  <state>0</state>
  <real_name>0</real_name>
  <source>k-v</source>
  <d p="29.35200,1,25,16777215,1565103107,0,f7a4e1a1,1565103107899">前方高能</d>
</i>
```

- 按 `progress_ms` 升序，同时间按 `dmid` 升序 → **输出确定性**（便于 diff 和内容哈希）。
- `maxlimit` 写 0 表示「不限制」，避免播放器二次截断。
- 文本转义：`&` `<` `>` `"` `'`；同时**不要**转义成数字实体（部分老播放器不认）。
- 文件头写 `<!-- generated by DanmakuVault at ... -->`？**不写**——保持与官方 XML 字节级兼容，
  元信息放 `meta.json` 和 jsonl。

### 8.3 ASS 渲染（可选，v2）

1. **常量**：`PlayResX=1920`、`PlayResY=1080`、字号按 `fontsize/25 * 48` 缩放。
2. **轨道分配**：维护 `lanes: Vec<Instant>`（每条轨道上「最后一个弹幕的结束时刻」）。
   对按时间排序的弹幕，找第一条 `lane_free <= start` 的轨道；都占用则取最早空闲的 。
3. **滚动弹幕**：`\move(1920+w/2, y, -w/2, y)`，时长 `8s`（可配），
   y 按轨道号算，`\c&HBBGGRR&` 注意 BGR 字节序转换。
4. **顶部/底部固定**：`\pos`，时长 `4s`，多条同样占轨道。
5. **去重叠**：同一时刻轨道用满时，降级为「静默丢弃并计数」，报告里输出 `ass_dropped`。

### 8.4 导出确定性

同一份数据导两次，字节必须完全相同（除时间戳外）。这靠：
- 稳定排序（见 8.2）；
- 不写当前时间到内容文件（写进 sidecar）；
- ASS 里 `ScriptType`/`PlayRes` 等固定值。

这条是 `dv verify` 能对比「导出文件 vs DB 重算」的前提。

---

## 9. CLI 与配置

### 9.1 命令面

```
dv 0.1.0 — B 站弹幕备份工具

USAGE: dv <COMMAND>

  init                        初始化 vault（建库、建目录）
  add <ID>...                 登记待备份目标（BV/av/URL/收藏夹 ID）
  sync [ID]...                同步（默认全部待同步目标）
  list [--json]               列出已备份视频
  show <ID>                   查看某视频的同步状态、段状态、条数
  export <ID> [--page all]    导出（xml|jsonl|ass|srt）
  verify [--deep]             校验（浅：DB 一致性；深：逐段 sha256 + 条数）
  gc [--dry-run]              清理墓碑/孤立 blob（需 --yes 才真删）
  daemon [--interval 6h]      常驻增量同步
  reindex                     从 videos/ 下的文件重建索引
  doctor                      环境自检（网络/认证/磁盘/权限）
```

### 9.2 全局参数

| 参数 | 说明 |
|---|---|
| `--vault <DIR>` | 覆盖 vault 根目录 |
| `--config <FILE>` | 覆盖配置文件 |
| `--cookie <STR>` | 临时 cookie（慎用，会进 shell history） |
| `--json` | 机器可读输出（进度信息走 stderr） |
| `-v/-q` | 日志级别 |
| `--no-progress` | 关闭进度条（CI 用） |
| `--dry-run` | 只打印计划不发请求 |

### 9.3 退出码

| 码 | 含义 |
|---|---|
| 0 | 全部成功 |
| 1 | 部分失败（有 `run_error`） |
| 2 | 参数/配置错误 |
| 3 | 认证失败（cookie 无效/未登录） |
| 4 | 网络整体不可用（连接级失败） |
| 5 | 数据损坏（`verify` 失败） |
| 130 | 被 SIGINT 中断（已完成部分保留） |

### 9.4 配置文件（TOML）

```toml
[vault]
root = "~/.local/share/danmaku-vault"

[net]
qps = 2.0
burst = 4
timeout_secs = 20
max_retries = 5
user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0 Safari/537.36"
# proxy = "socks5://127.0.0.1:1080"

[auth]
# cookie_file = "~/.config/danmaku-vault/cookies.txt"

[sync]
concurrency = 4
refresh_interval = "6h"      # 小于此时长视为新鲜，跳过
save_raw_segments = true
include_special = false      # BAS/字幕弹幕，v2
retain_missing = true

[export]
default_format = "xml"
write_jsonl = true
ass_font = "Microsoft YaHei"
ass_duration_secs = 8
```

配置分层（后者覆盖前者）：内置默认 → 配置文件 → 环境变量（`DV_*`）→ CLI 参数。
用 `figment` 或手写 `serde` + 合并逻辑都行；建议手写，减少依赖。

---

## 10. 错误处理与可观测性

### 10.1 错误类型

```rust
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("network: {0}")]        Network(#[from] reqwest::Error),
    #[error("api {code}: {msg}")]   Api { code: i32, msg: String },
    #[error("parse: {0}")]          Parse(String),
    #[error("io: {0}")]             Io(#[from] std::io::Error),
    #[error("db: {0}")]             Db(#[from] rusqlite::Error),
    #[error("auth required")]       Auth,
    #[error("rate limited")]        RateLimited,
    #[error("corrupt: {0}")]        Corrupt(String),
}
```

API 错误码映射表（`code → 语义 → 是否可重试`）集中在一个 `fn classify(code) -> ApiClass` 里，
不要散落在各处 `if code == -412`。

### 10.2 日志与进度

- `tracing` + `tracing-subscriber`（`EnvFilter`，`RUST_LOG` 可调）。
- 每个请求一个 span：`bili.request{endpoint, cid, seg}`；run 一个 span：`sync.run{id}`。
- 进度条用 `indicatif`：`{spinner} 视频 3/50 · 段 12/421 · 新增 1.2k · 失败 0`。
- `--json` 模式下进度/日志走 **stderr**，stdout 只留结构化结果，方便管道。

### 10.3 运行报告

每次 sync 写 `reports/<ts>.json`：

```json
{
  "run_id": 12,
  "started_at": "2026-11-21T15:40:00Z",
  "finished_at": "2026-11-21T15:42:11Z",
  "trigger": "manual",
  "totals": { "videos": 3, "pages": 4, "segments": 61, "segments_unchanged": 12,
              "danmaku_added": 1841, "danmaku_missing": 7 },
  "errors": [ { "cid": 1176840, "segment_index": 7, "kind": "http", "message": "503" } ]
}
```

---

## 11. 测试策略

| 层次 | 手段 | 覆盖点 |
|---|---|---|
| 单元 | `#[test]` | `p` 属性解析/生成、deflate 嗅探、dmid 解析、WBI 签名（固定向量） |
| Golden | `insta` + `fixtures/` | 真实抓取的 `seg_*.bin` → 期望 jsonl；历史 XML → 期望模型 |
| HTTP mock | `wiremock` 或 `httpmock` | 412/429/503 重试次数与退避；`--force` 与缓存命中 |
| 时间 | `tokio::time::pause()` | 退避、限速、熔断，不真睡 |
| 存储 | `tempfile` + 内存/文件 DB | 迁移 v0→vN、并发写、崩溃点重启 |
| 属性 | `proptest` | `Danmaku ↔ p 属性` 往返；任意文本的 XML 转义往返 |
| 端到端 | `tests/e2e.rs`（`#[ignore]`，需 `DV_TEST_BVID`） | 真网抓一个小视频，断言条数 > 0 且导出可解析 |

夹具策略：把 2~3 个有代表性的视频的**原始响应**（protobuf、deflate XML、JSON）存进
`fixtures/`，体积控制在 1MB 内。所有解析测试离线跑，CI 不依赖网络。

---

## 12. 依赖清单

已核对 crates.io 当前稳定版本（2026-11）：

| crate | 版本 | 用途 | 备注 |
|---|---|---|---|
| `tokio` | 1.x | 异步运行时 | `features = ["rt-multi-thread","macros","signal","time","sync","fs"]` |
| `reqwest` | 0.13 | HTTP | 默认 TLS 已切 rustls；开 `cookies`、`gzip`，**不要**开 `deflate`（raw deflate 自己处理） |
| `prost` | 0.14 | protobuf | 只用 derive，无 `build.rs` |
| `flate2` | 1.x | raw deflate / gzip | `Decompress::new(false)` |
| `quick-xml` | 0.3x | XML 读写 | 用 `Reader` 流式解析，别整树构建 |
| `serde`/`serde_json` | 1.x | JSONL、报告 | |
| `rusqlite` | 0.3x | SQLite | `features = ["bundled"]` 免系统依赖 |
| `clap` | 4.x | CLI | derive API |
| `thiserror`/`anyhow` | 1.x | 错误 | 库里 `thiserror`，bin 里 `anyhow` |
| `tracing`/`tracing-subscriber` | 0.1/0.3 | 日志 | |
| `indicatif` | 0.17 | 进度条 | |
| `directories` | 5.x | 平台目录 | config/data 路径 |
| `sha2`/`hex` | 0.10/0.4 | 哈希 | 段校验、内容寻址 |
| `rand` | 0.8 | 抖动 | |
| `tokio-util` | 0.7 | `CancellationToken` | Ctrl-C 优雅退出 |
| `bytes` | 1.x | 零拷贝响应体 | |
| `insta`/`wiremock`/`proptest`/`tempfile` | latest | 测试 | dev-dependencies |

刻意**不用**：`sqlx`（编译期检查换来 async 复杂度和宏编译开销，这里不需要）、
`tonic`（不需要 gRPC）、`governor`（30 行能写完）、`reqwest` 的 `blocking`。

---

## 13. 打包与 CI

- `cargo-dist` 产 `x86_64-pc-windows-msvc` / `aarch64-apple-darwin` / `x86_64-unknown-linux-musl`。
- GitHub Actions：
  - `fmt` + `clippy -D warnings` + `test`（三平台矩阵）
  - `cargo-deny`（许可/漏洞）、`cargo-audit`
  - 发布时校验 musl 静态链接可跑
- MSRV 写死在 `rust-toolchain.toml`，并在 CI 里跑一次 `cargo +1.85 check`。
- 单二进制体积预期 8~12MB（`lto = "thin"`、`strip = true`、`panic = "abort"` 可选）。

---

## 14. 里程碑

| 阶段 | 内容 | 完成判据 |
|---|---|---|
| M0 | workspace 骨架、CI、`dv init`、配置加载 | `cargo test` 绿；`dv init` 建出正确目录与空库 |
| M1 | `dv-bili`：view + seg.so + 限流/重试；`dv add` + `dv sync` 单视频 | 能抓 BV1xx 全部段并落 DB |
| M2 | `dv-store`：表结构、单写者、迁移；`dv export --format xml/jsonl` | XML 能被本地播放器加载；重复同步 0 新增 |
| M3 | 段级断点、墓碑、`dv verify`、运行报告 | 中途 kill 后重跑只补未完成段 |
| M4 | 批量目标（收藏夹/UP 主含 WBI）、`daemon`、`--json` | 100 个视频批量跑完不触发熔断 |
| M5 | `ass`/`srt` 导出、`gc`、`reindex`、`doctor` | 导出的 ASS 在 PotPlayer/剪映里正常显示 |
| M6 | 发布（cargo-dist）、文档站、已知限制清单 | 三平台二进制可用 |

建议先做 M1 的**端到端最小闭环**（抓一个视频 → 落一份 XML），拿到真实数据后再定表结构和字段，
避免在假数据上过度设计。

---

## 15. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| 接口/字段变更 | 解析失败 | 未知字段忽略；`fixtures` 回归；`doctor` 里做一次存活探测 |
| 风控（-412） | 批量任务中断 | 保守 QPS、全局熔断、断点续传、建议登录 |
| XML 接口上限 | 导出少于实际 | 主走分段接口；导出条数与 `declared_count` 对比并告警 |
| WBI 密钥轮换 | 批量来源失效 | 每次启动刷新、缓存 1h、失败自动重取 |
| cookie 过期 | 私有稿件抓不到 | `dv doctor` 主动检测；错误信息明确提示重新登录 |
| 磁盘增长 | 空间不足 | `save_raw_segments` 默认只留最近一次；`gc` 清理墓碑；报告里统计体积 |
| 用户误删 | 数据丢失 | `gc` 必须 `--yes`；删除前打印样例；`reindex` 可部分恢复 |

---

## 16. 合规与伦理

- 只做**个人用途的本地备份**，不做二次分发、不做公开镜像。
- 遵守合理请求频率（默认 2 QPS），不绕过付费/权限限制，不抓取需要更高权限的内容。
- cookie 属于敏感凭据：不写日志、不进报告、不随备份文件分发（`meta.json` 里不包含任何凭据）。
- 弹幕内容版权属原作者与平台；导出文件供个人观看/研究，文档里明示。

---

## 17. 待确认（影响后续设计）

1. **范围**：只要「单视频手动备份」，还是也要「整个收藏夹/UP 主全量」？
   后者会显著增加 §7.5（WBI）和 §6.1（plan 阶段）的工作量。
2. **形态**：纯 CLI 够用，还是需要 TUI/本地 Web 界面（浏览 + 搜索 + 导出）？
3. **下游**：导出主要给播放器看，还是要烧进视频（决定 ASS 的优先级和渲染质量要求）？
4. **平台**：Windows 是主要目标吗？（影响 cookie 存储与路径策略的优先级）
5. **增量发现**：是否需要「订阅某个 UP 主，新投稿自动备份」，还是手动给链接就行？

---

## 18. 实现状态（v0.1，2026-11-22）

范围已按实际需求收敛为：**单视频手动备份 · 纯 CLI · 不烧字幕**。
下面的偏差是落地时的真实选择，与上文设计不一致处以本节为准。

| 里程碑 | 状态 | 说明 |
|---|---|---|
| M0 骨架 / `init` / `doctor` | ✅ | workspace + 4 个 crate |
| M1 `add` / `sync` | ✅ | protobuf 分段、限速、退避、断点续传 |
| M2 导出 XML / JSONL | ✅ | 确定性输出 + 原子写 |
| M3 `verify` / 运行报告 | ✅ | 浅校验逐字节比对，`--deep` 比对上游 sha256 |
| M4 批量目标 / `daemon` | ❌ 不做 | 收藏夹、UP 主、WBI 签名、定时任务全部移出范围 |
| M5 `gc` / `reindex` | ✅ | `gc` 删除后会重绘受影响分 P 的文件 |
| M5 `ass` / `srt` 导出 | ❌ 不做 | 不烧字幕，格式只保留 XML 与 JSONL |
| M6 打包分发 | ⏳ 未做 | 目前只有 `cargo build --release` |

### 18.1 与上文设计的差异

1. **没有独立的 `dv-sync` crate。** 编排逻辑（200 行左右）放在 `dv-cli/src/commands/sync.rs`；
   `dv-export` 反而独立成 crate，因为它有独立的测试价值（转义、确定性、往返）。
2. **`reqwest` 用 0.12 + `rustls-tls`（ring）**，而不是 0.13 的默认 `aws-lc-rs`：
   后者在 Windows 上需要 cmake 与 NASM，会给贡献者和 CI 增加无谓的门槛。
3. **没有 `sync_run` / `run_error` 表。** 每次运行的报告落在 `reports/<utc-ms>.json`，
   结构见 `RunReport`。个人备份场景下，文件比表更好查、更好带走。
4. **`dm/web/view` 未接入**，`declared_count` 恒为 `None`；段数靠「时长 ÷ 6min」推算，
   再多探一段（`probe_extra_segment`）。少一段就是几百条弹幕的静默丢失，所以探测不能省。
5. **空分段不写 `segment` 记录。** 否则「记录段数」会大于「有数据的段数」，
   让 `verify` 和 `show` 的统计自相矛盾。代价是探测段每次都要重问一次（304/空响应，极廉价）。
6. **`verify` 用「是否完整同步过」判断完整性**，而不是比对段数——稀疏弹幕的视频里，
   某段为空是正常的，按段数比会误报。
7. **`gc` 是「删行 + 重绘文件」的原子动作**，否则下一次 `verify` 立刻报文件漂移。
8. **`reindex` 只能恢复 `meta.json` + `danmaku.jsonl`**，分段 sha256 与「已完整同步」标记无法恢复，
   所以重建后第一次 `sync` 会全量重抓（相当于一次免费核对）。
9. **Windows 构建折中**：MSVC 14.44 编译 C 源码时带 `/Z7` 会失败（`D8050: cannot execute c1.dll`），
   `Cargo.toml` 里对**依赖包**关掉 debuginfo，自己写的第一方 crate 仍保留调试信息。

### 18.2 实测结论（用 BV1GJ411x7h7 验证）

- `view` 返回 `aid=80433022 / cid=137649199 / 213s`，单 P，名义 1 段 + 探测 1 段（空）。
- 首轮抓到 **1406 条**；对同一段反复抓取时，条数会在 1405~1409 之间波动
  （热门视频的服务端清理与选择性返回），因此：
  - `verify --deep` 报「上游已变化」是**正常**的，不代表备份损坏；
  - 墓碑数量会随热门视频的清理动作小幅波动。
- `list.so` 确实返回 raw deflate，且当前 `p` 属性是 **9 段**（第 9 段为 `weight`），
  与本设计 §3.4 的推断一致。
- 对不存在的分段，`seg.so` 返回 **HTTP 304** 而不是空 200 —— 客户端必须把它当空内容，
  否则会误判为网络错误并触发 5 次退避重试（实测踩过这个坑）。

