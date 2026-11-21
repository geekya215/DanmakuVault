# DanmakuVault

[![CI](https://github.com/geekya215/DanmakuVault/actions/workflows/ci.yml/badge.svg)](https://github.com/geekya215/DanmakuVault/actions/workflows/ci.yml)

把 B 站视频弹幕**完整、可校验、可增量**地备份到本地，并导出成播放器能直接加载的 XML。

纯 CLI、单文件、跨平台。当前版本只做「单视频手动备份」——不爬收藏夹、不追 UP 主、不下载视频。

```
dv add BV1GJ411x7h7
dv sync
dv export BV1GJ411x7h7
```

---

## 特性

| 能力 | 说明 |
|---|---|
| 完整抓取 | 走 protobuf 分段接口（6 分钟一段），而不是有条数上限的整包 XML |
| 断点续传 | 段级记录 + 内容 sha256；中断后重跑只补没抓完的段 |
| 幂等 | 去重键 `(cid, dmid)`，重复同步不会产生重复行 |
| 不丢数据 | 服务端删掉的弹幕只打墓碑，默认不物理删除；`gc` 必须显式 `--yes` |
| 可校验 | `dv verify` 用「库重新渲染 vs 磁盘文件」逐字节比对；`--deep` 逐段比对上游 |
| 确定性导出 | 同一份数据导两次字节完全相同，方便 diff 与校验 |
| 崩溃安全 | 文件写入一律 临时文件 → fsync → rename |
| 限速与退避 | 全局令牌桶 + 指数退避；`-412` 风控单独加长等待 |

## 快速开始

需要 Rust 1.85+（`rust-toolchain.toml` 里写的是 stable）。

```powershell
# 编译
cargo build --release
# 之后的命令都在 target\release\dv.exe（Linux/macOS 是 target/release/dv）
```

```powershell
dv init                     # 建 vault、建库、写默认配置
dv add BV1GJ411x7h7         # 登记视频（只拉元信息）
dv sync                     # 抓弹幕
dv verify                   # 校验库与文件是否一致
dv export BV1GJ411x7h7      # 导出 XML（默认写回 vault 内）
```

首次使用建议跑一次 `dv doctor`：

```
✓ 程序版本       dv 0.1.0
✓ vault 目录     C:\Users\me\AppData\Local\danmaku-vault\vault
✓ 目录可写       可写
✓ 数据库 schema  v1
✓ 数据规模       视频 1 · 分P 1 · 弹幕 1406（墓碑 0） · 分段 1
✓ 配置文件       C:\Users\me\AppData\Roaming\danmaku-vault\config.toml
✓ 登录凭据       未提供（匿名模式，限速更严）
✓ 运行参数       qps 2.0 · 并发 4 · 超时 20s
```

## 命令

| 命令 | 作用 |
|---|---|
| `dv init [--root DIR] [--force]` | 初始化（幂等，可重复执行） |
| `dv add <ID>...` | 登记视频：解析元信息与分 P |
| `dv sync [ID]... [--force] [--dry-run]` | 抓弹幕；省略 ID 时同步库里全部 |
| `dv list` | 列出已备份视频 |
| `dv show <ID>` | 单个视频的分 P / 段 / 条数 / 同步状态 |
| `dv export <ID> [--page all\|N] [--format xml\|jsonl] [--out DIR] [--exclude-missing]` | 导出 |
| `dv verify [ID] [--deep]` | 校验；`--deep` 需联网重抓比对 |
| `dv gc [--older-than-days 30] [--dry-run] [--yes]` | 清理长期墓碑 |
| `dv reindex` | 库丢了以后从 `videos/` 重建索引 |
| `dv doctor` | 环境自检 |

`<ID>` 接受四种写法：`BV1GJ411x7h7`、`av80433022`、`80433022`、
`https://www.bilibili.com/video/BV1GJ411x7h7/?p=2`。短链接（`b23.tv`）需要联网展开，请先手动打开复制。

全局参数：`--vault DIR`、`--config FILE`、`--json`、`-v/-vv/-vvv`。
`--json` 时 stdout 只输出结构化结果，日志与进度走 stderr，方便管道。

### 退出码

| 码 | 含义 |
|---|---|
| 0 | 成功 |
| 1 | 部分失败（有分 P 或视频没抓完） |
| 2 | 参数 / ID 错误 |
| 3 | 需要登录（cookie 失效） |
| 4 | 网络整体不可用 |
| 5 | 数据损坏（`verify` 发现库与文件不一致） |

## vault 目录结构

```
<vault>/
├── index.sqlite                 # 索引：视频、分P、弹幕、分段状态
├── vault.json                   # 版本与创建时间
├── videos/
│   └── BV1GJ411x7h7/
│       ├── meta.json            # 元信息快照，脱离数据库也能读懂
│       └── p1/
│           ├── danmaku.xml      # 播放器可直接加载
│           └── danmaku.jsonl    # 无损格式（字段与库一一对应）
├── reports/
│   └── 20261122T002938123Z.json # 每次 sync 的机器可读报告
└── blobs/                       # 预留：原始分段缓存
```

**数据库是索引，文件是产物**：`index.sqlite` 负责查询/增量/断点，文件负责被别的工具消费。
两者都能互相恢复——文件丢了 `dv export` 重生成，库丢了 `dv reindex` 重建。

## 配置

`dv init` 会写一份默认配置；路径可用 `--config` 或 `DV_CONFIG` 覆盖。

```toml
[vault]
root = "C:\\Users\\me\\AppData\\Local\\danmaku-vault\\vault"

[net]
qps = 2.0            # 全局每秒请求数
burst = 4
timeout_secs = 20
max_retries = 5
user_agent = "Mozilla/5.0 ..."
# proxy = "http://127.0.0.1:7890"

[sync]
concurrency = 4
save_raw_segments = false
retain_missing = true
probe_extra_segment = true
refresh_interval_secs = 21600   # 6h 内同步过的分P 跳过

[export]
default_format = "xml"
write_jsonl = true
```

认证是可选的：没有 cookie 也能抓公开视频，但更容易触发风控。
需要登录时用环境变量传，**不要写进配置文件**：

```powershell
$env:DV_COOKIE = "SESSDATA=xxxxx; bili_jct=xxxxx"
```

## 设计要点

完整设计见 [docs/design.md](docs/design.md)。三条最关键的决定：

1. **主用 `seg.so` 的 protobuf 分段，而不是 `list.so` 的整包 XML。**
   整包接口有条数上限（常见 3000/6000），爆款视频会被**静默截断**——这是备份工具最不能接受的失败模式。
2. **墓碑而不是镜像。** 一次分 P 的**每一段**都成功抓完后，才把本轮没见到的弹幕标为 `missing_since`；
   有段失败就绝不标记（否则会把"没抓到"误判成"服务端删了"）。
3. **两种同步模式由「这个分 P 是否完整同步过」决定：**
   从没成功过 → 断点续传（跳过已有段）；成功过 → 全量重抓、比对哈希、更新墓碑。

## 已知限制

- **只做单视频手动备份**：不支持收藏夹 / UP 主投稿 / 稍后再看（这些需要额外接口与 WBI 签名）。
- **不做视频下载**：只备份弹幕。
- **不导出 ASS/SRT**：字幕渲染不在当前范围内。
- **BAS / 字幕弹幕**（`pool=2`、`mode=9`）拿不到：它们在 `dm/web/view` 的特殊字段里，尚未实现。
- **分段内容会变**：同一段在不同时间抓到的条数可能不同（服务端清理垃圾弹幕、热门视频的选择性返回），
  所以 `verify --deep` 报「上游已变化」是正常现象，不是损坏。
- **`reindex` 恢复不了分段状态**：sha256 和「完整同步过」的标记只存在于数据库里，
  重建后第一次 `sync` 会把所有分段重抓一遍（这正好是一次核对）。

## 开发

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
```

- MSRV 是 **Rust 1.88**（`Cargo.toml` 的 `rust-version`，CI 里有专门任务验证）。
- CI（[ci.yml](.github/workflows/ci.yml)）在 Ubuntu / Windows / macOS 上跑 fmt、clippy（`-D warnings`）、
  全量测试，外加 `dv init && dv doctor` 冒烟；推 `v*` tag 会触发 [release.yml](.github/workflows/release.yml)
  产出三个平台的二进制并附 `SHA256SUMS.txt`。
- 需要 MSVC 工具链（`rusqlite` 的 bundled SQLite 要编译 C）。
- Windows + MSVC 14.44 编 C 源码时带 `/Z7` 会失败（`D8050`），
  `Cargo.toml` 里已对**依赖**关掉 debuginfo（`[profile.dev.package."*"] debug = false`），
  自己的代码仍有调试信息。
- 联网测试不放在 `cargo test` 里：解析、限速、重试、SQL 往返都用离线夹具与替身覆盖。

## 合规

- 仅用于**个人本地备份**，不要二次分发或公开镜像。
- 默认 2 QPS，不做任何绕过权限的功能。
- cookie 属于敏感凭据：不写日志、不进报告、不随备份文件分发。
- 弹幕内容版权归原作者与平台所有。

## License

MIT
