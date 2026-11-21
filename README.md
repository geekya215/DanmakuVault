# DanmakuVault

[![CI](https://github.com/geekya215/DanmakuVault/actions/workflows/ci.yml/badge.svg)](https://github.com/geekya215/DanmakuVault/actions/workflows/ci.yml)

把 B 站视频弹幕完整备份到本地，导出成播放器能直接加载的 XML。

```bash
dv init && dv add BV1GJ411x7h7 && dv sync
```

- **完整**：走 6 分钟一段的 protobuf 接口，不受整包 XML 的条数上限影响
- **可续传**：段级记录 + 内容哈希，中断重跑只补没抓完的段
- **不丢数据**：服务端删掉的弹幕只打墓碑，默认不物理删除
- **可校验**：库、磁盘文件、上游三方对比

## 安装

需要 Rust 1.88+。

```bash
cargo install --path crates/dv-cli    # 装到 ~/.cargo/bin/dv
```

或者只编译：

```bash
cargo build --release                 # 产物 target/release/dv（Windows 为 dv.exe）
```

预编译二进制还没发布；推 `v*` tag 时 CI 会构建三平台产物并附 `SHA256SUMS.txt`。

## 快速开始

```bash
dv init                     # 建库、建目录、写默认配置
dv add BV1GJ411x7h7         # 登记视频：只取元信息，不抓弹幕
dv sync                     # 抓弹幕
dv export BV1GJ411x7h7      # 导出 XML（默认写回 vault 内）
```

跑完在这里拿到结果：

```
<vault>/videos/BV1GJ411x7h7/p1/danmaku.xml
```

`dv list` 看已备份的内容，`dv show <ID>` 看分 P 与同步状态，`dv doctor` 排查环境问题。

## 命令

| 命令 | 说明 |
|---|---|
| `dv init [--root DIR] [--force]` | 初始化 vault（幂等，可重复执行） |
| `dv add <ID>...` | 登记视频，解析元信息与分 P |
| `dv sync [ID]... [--force] [--dry-run]` | 抓弹幕；省略 ID 时同步库里全部 |
| `dv list` | 列出已备份视频 |
| `dv show <ID>` | 分 P、分段、条数、同步状态 |
| `dv export <ID> [--page N\|all] [--format xml\|jsonl] [--out DIR]` | 导出 |
| `dv verify [ID] [--deep]` | 校验；`--deep` 需联网重抓比对 |
| `dv gc [--older-than-days 30] [--yes]` | 清理长期墓碑（默认只预演） |
| `dv reindex` | 库丢失后从 `videos/` 重建索引 |
| `dv doctor` | 环境自检 |

`<ID>` 接受 `BV1GJ411x7h7`、`av80433022`、`80433022` 和视频页 URL。
短链接（`b23.tv`）无法离线解析，请先在浏览器打开并复制完整链接。

全局参数：`--vault DIR`、`--config FILE`、`--json`、`-v/-vv/-vvv`。
`--json` 时 stdout 只输出结构化结果，日志与进度走 stderr。

退出码：`0` 成功 · `1` 部分失败 · `2` 参数或 ID 错误 · `3` 需要登录 · `4` 网络不可用 · `5` 校验未通过。

## 数据放在哪

```
<vault>/
├── index.sqlite              # 索引：视频、分P、弹幕、分段状态
├── vault.json                # 版本与创建时间
├── videos/
│   └── BV1GJ411x7h7/
│       ├── meta.json         # 元信息快照
│       └── p1/
│           ├── danmaku.xml   # 给播放器
│           └── danmaku.jsonl # 无损格式，字段与库一一对应
└── reports/                  # 每次 sync 的报告
```

数据库是索引，文件是产物：查询与增量靠 `index.sqlite`，消费靠 `videos/`。
两边能互相恢复——文件丢了 `dv export` 重生成，库丢了 `dv reindex` 重建。

## 配置

`dv init` 会写一份默认配置（Windows 在 `%APPDATA%\danmaku-vault\config.toml`），
也可以用 `--config` 或 `DV_CONFIG` 指定其他位置。

```toml
[net]
qps = 2.0                        # 全局每秒请求数，默认很保守
burst = 4
timeout_secs = 20
max_retries = 5
# proxy = "http://127.0.0.1:7890"

[sync]
probe_extra_segment = true       # 按时长算完段数后再多探一段
refresh_interval_secs = 21600    # 6 小时内同步过的分P 跳过

[export]
write_jsonl = true               # 导出 XML 时同时写一份无损 JSONL
```

`concurrency`、`save_raw_segments`、`retain_missing`、`default_format` 是预留字段，当前版本尚未生效。

需要登录时用环境变量传 cookie，不要写进配置文件：

```powershell
$env:DV_COOKIE = "SESSDATA=xxxxx; bili_jct=xxxxx"
```

匿名也能抓公开视频，只是更容易触发风控。

## 校验与备份语义

`dv verify` 把库重新渲染成 XML，与磁盘文件逐字节比对——能发现手工改动、写入中断、磁盘损坏。
加 `--deep` 则逐段重新拉取并比对 sha256，报告哪些分段在上游变了（变更本身是正常现象，不判对错）。

「消失」的处理是这个工具与「同步即镜像」最大的区别：

- 一个分 P 的**每一段**都成功抓完，才把本轮没见到的弹幕标记为 `missing_since`
- 有段失败就绝不标记，否则会把「没抓到」误判成「服务端删了」
- 墓碑默认保留并可导出，`dv export --exclude-missing` 可以排除它们；真要删除必须 `dv gc --yes`

同一个分 P 第二次同步会走全量重抓（比对哈希、更新墓碑），只有「从未完整同步过」才走断点续传。

## 已知限制

- 只做单视频手动备份，不支持收藏夹 / UP 主投稿 / 稍后再看
- 只备份弹幕，不下载视频
- 不导出 ASS / SRT
- BAS 与字幕弹幕（`pool=2`、`mode=9`）暂不支持
- 热门视频的分段内容会变（清理垃圾弹幕、选择性返回），因此 `verify --deep` 报「上游已变化」是正常现象
- `reindex` 恢复不了分段状态：sha256 与「已完整同步」标记只存在库里，重建后首次 `sync` 会把分段重抓一遍

## 开发

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

MSRV 是 **1.88**，CI 有独立任务验证，不是写在文档里的口号。CI 在 Ubuntu / Windows / macOS 上跑上述命令，
外加 `dv init && dv doctor` 冒烟；推 `v*` tag 触发发布。

编译需要 C 工具链（`rusqlite` 的 bundled SQLite）。Windows 上 MSVC 14.44 编 C 源码带 `/Z7` 会挂在
`D8050`，`Cargo.toml` 里已对依赖关闭 debuginfo。

## 许可

MIT。弹幕内容版权归原作者与平台所有。

- 仅限个人本地备份，不要二次分发或公开镜像
- 默认 2 QPS，不做任何绕过权限的功能
- cookie 是敏感凭据：不写日志、不进报告、不随备份文件分发
