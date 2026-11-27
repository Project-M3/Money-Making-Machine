# Agent 接口：Munger Workbench 数据与 CLI 契约

人类使用 Longbridge NSH `0.7.0` 原生桌面窗口（`open apps/m3-nsh/artifacts/M3.app` 或 `/Applications/M3.app`）浏览、调参和记录判断；Alan / 外部 Agent 可通过 `m3` CLI（`~/.local/bin/m3`、`apps/m3-nsh/m3` 或 `cargo run -p m3-cli --`）、全能力或只读 MCP 服务器（`m3 mcp`，见 [docs/mcp-server.md](mcp-server.md) 与第一方 Agent Skill [skills/m3/SKILL.md](../skills/m3/SKILL.md)），以及可选的桌面控制 IPC（`m3 app` / `control.sock`）与本地工作台协作，**日常研究不依赖屏幕截图或脆弱的坐标点击**（`m3 app screenshot` 仅在需要视觉核查 UI 时使用）。

## 1. CLI 命令一览

在仓库根目录执行（已安装用户可直接使用 `m3 ...`），所有业务命令均输出 JSON 至 `stdout`：
- 交互终端（TTY）下自动使用多行缩进美化格式；在管道/重定向给脚本或 Agent 时自动输出单行紧凑 JSON（也可用 `--pretty` 或 `--compact`/`--json` 显式覆盖）。
- `m3 --help`、`m3 help <command>`、`m3 <command> --help` 直接在 `stdout` 输出结构化帮助文本并退出码为 `0`；`m3 update` 在交互式 `stderr` 下自动渲染单行实时进度条（不影响 `stdout` 纯净 JSON）。

```json
{"schema_version": 1, "ok": true, "data": ...}
```

失败时输出 `{"schema_version": 1, "ok": false, "error": "..."}` 并返回非零退出码。

### 可用子命令

```sh
# 0. 帮助与本地环境体检（无需已初始化工作区即可执行）
m3 --help
m3 help alan
m3 help app
m3 doctor

# 1. 查看工作台整体状态、数据新鲜度、各公司击球状态摘要与同步记录
m3 status

# 2. 同步一手 SEC EDGAR 财报与 Yahoo 日级股价（支持单标的和仅同步价格）
m3 sync
m3 sync --ticker GOOGL
m3 sync --prices-only

# 3. 读取单家公司的确定性查理计算器报告（含多年财务、TTM、估值、预警与溯源）
m3 report AAPL

# 4. 读取单家公司的完整人机审讯档案（AI 草稿 + 人类判断与证伪栏位）
m3 dossier AAPL

# 5. 向观察池添加公司或更新分桶（自动从 SEC tickers 查找 CIK；支持 core/in、watch、out、too_hard）
m3 add CDNS --bucket watch --circle "EDA / 半导体设计工具" --name "Cadence Design Systems"
m3 add INTC --bucket too_hard --reason "重资产晶圆代工死磕期，现金流不可预测"

# 6. 决策账本操作（事前记录 WAIT / OUT / TOO_HARD / PAPER_BUY / PAPER_SELL）
m3 journal list
m3 journal record --ticker GOOGL --action wait --note "Q3 2026 CapEx 超 OCF，等击球安全边际或 CapEx 见顶"

# 7. 启动 M3 NSH 桌面工作台
m3 start

# 8. L0 全美股湖仓（DuckDB + Parquet）筛选、同行对照与混合语义相似搜索
m3 universe status
m3 universe sync --full --prices-top 400
m3 universe screen --min-roic 15 --max-sbc-ocf 25 --exclude-financials --limit 50
m3 universe screen --strike-only --sort fcf_yield --limit 20
m3 universe peers ADSK --limit 15
m3 universe similar ADSK --ticker --limit 10
m3 universe similar "enterprise engineering software subscription" --min-roic 15 --limit 10

# 9. L1 事件流追平、收件箱与 L1.5 看门狗简报/证伪管理
m3 events catchup --bootstrap --max-ciks 25 --max-docs 12 --max-news-tickers 20
m3 events list --ticker PTC --type sec_8k --limit 20
m3 inbox list --status unread --limit 20
m3 inbox triage <SIGNAL_ID> --status watch --promote-watch
m3 briefing show --period daily
m3 briefing run --period daily --no-llm
m3 triage status --ticker COST
m3 triage run --dry-run --limit 5
m3 falsification COST list

# 10. L2 Alan Deep Research（4 视角并行）、预算门控与商业原型主题图谱
m3 research run ADSK --focus "云转型净留存与 SBC 控制" --trigger cli
m3 research status --ticker ADSK --limit 10
m3 budget status
m3 themes sync
m3 themes list

# 11. 统一 Alan 连接体检、Scoped 审讯对话与会话管理
m3 alan status
m3 alan test
m3 alan ask --scope company:COST "请核实 Costco 的会员护城河并附上 [report:COST]" --lang zh --timeout 120
m3 alan session show --scope company:COST
m3 alan session clear --scope company:COST

# 12. 实时检查与驱动运行中的 M3 桌面窗口（control.sock IPC）
m3 app ping
m3 app state
m3 app navigate --tab universe --ticker COST
m3 app alan --mode split --scope company:COST --pin true
m3 app screenshot /tmp/m3-window.png

# 13. 启动 MCP stdio 服务器（默认 40 工具全能力；加 --read-only 限制为 24 个只读工具）
m3 mcp
m3 mcp --read-only --workspace /path/to/m3-workbench
```

#### `m3 universe` 子命令契约

- `m3 universe status`：返回 `UniverseSummary` JSON（未同步前返回全 `0` 默认值，不报错）。
- `m3 universe sync [--full] [--prices-top N] [--skip-prices]`：同步 SEC `company_tickers_exchange.json`、`submissions.zip`（SIC 行业分类）与 `companyfacts.zip`，写入 ZSTD Parquet 与 `m3.duckdb`，按**价格方案 1**仅对 Top `N`（默认 `400`，有效范围 `1..=1000`；跳过股价请使用 `--skip-prices`）质量候选池 + 观察池标的同步日级股价。进度日志写往 `stderr`，`stdout` 保持单一 schema v1 JSON（`UniverseSyncReport`）。
- `m3 universe screen [FLAGS]`：在本地 DuckDB 执行毫秒级全美股筛选与排序，返回 `{ "query_ms", "count", "filter", "rows" }`。支持：
  - 数值过滤：`--min-roic N`、`--max-sbc-ocf N`、`--max-share-change-3y N`、`--max-net-debt-to-oe N`（共用精确十进制与科学计数法解析器，先归一化指数再校验 28 位小数精度与 `[-10000, 100000]` 范围，拒绝无法精确表示的小数或溢出）、`--min-years N`（有效范围 `1..=20`）
  - 开关过滤：`--exclude-financials`（默认启用，剔除 `SIC 6000–6799`）、`--include-financials`、`--positive-oe-only`、`--price-pool-only`、`--strike-only`
  - 搜索与排序：`--query TEXT`、`--sort quality|roic|fcf_yield|distance|share_change|sbc_ocf|price|strike|warnings|ticker|name|sic|oe`、`--asc` / `--desc`、`--limit N`（默认 `200`，有效范围 `1..=5000`）
- `m3 universe peers <TICKER> [--limit N]`：输出目标公司在同 SIC 组内的 `roic_percentile`、`gross_margin_percentile`、`strict_fcf_yield_percentile` 及头部同行列表（`PeerComparison`；`--limit` 默认 `15`，有效范围 `1..=200`）。
- `m3 universe similar <TICKER_OR_QUERY> [--ticker] [--min-roic N] [--max-sbc-ocf N] [--exclude-financials|--include-financials] [--positive-oe-only] [--price-pool-only] [--limit N]`：在本地 DuckDB 先执行确定性财务硬过滤，再对 `doc_chunks` 进行余弦相似度 + BM25 混合检索，返回 `{ "query_ms", "query", "as_ticker", "count", "filter", "hits" }`，每条命中附带综合得分、语义/词法子分与一手原文出处 `evidence`（`--limit` 默认 `20`，有效范围 `1..=200`）。

#### `m3 events`、`m3 inbox`、`m3 briefing`、`m3 triage` 与 `m3 falsification` 子命令契约

- `m3 events catchup [--bootstrap] [--skip-sec] [--skip-news] [--skip-docs] [--max-ciks N] [--max-docs N] [--max-news-tickers N]`：增量拉取 SEC EDGAR `submissions`（首次 `--bootstrap` 可利用本地 `submissions.zip` 缓存与官方 Fair-Access 请求头）、10-K/10-Q/8-K 原文片段与候选公司新闻 RSS，构建本地 256 维嵌入索引 `doc_chunks` 并自动生成 `discovery` / `ripple` / `strike` / `red_flag` 信号写入 `signal_inbox`。全程 `flagship_tokens_used == 0`，进度写往 `stderr`，`stdout` 输出 `EventCatchupReport` JSON。
- `m3 events list [--ticker SYM] [--type TYPE] [--unprocessed-only] [--limit N]`：查询本地 DuckDB `events` 表，返回 `{ "count", "events" }`（未同步湖仓时返回 `count: 0`）。
- `m3 inbox list [--status unread|all|watch|too_hard|researching|archived] [--kind discovery|ripple|strike|red_flag] [--limit N]`：返回 `{ "status_filter", "kind_filter", "summary", "count", "signals" }`，每条 `SignalInboxRow` 包含可溯源 `evidence` 卡片（`doc_type`、`section`、`filing_date`、`quote`、`source_url`、`similarity`）。
- `m3 inbox triage <SIGNAL_ID> --status unread|researching|watch|too_hard|archived [--promote-watch]`：更新信号分拣状态；传入 `--promote-watch` 时同步将 `primary_ticker` 加入 `workbench/watchlist.json`（`WATCH` 篮子）并刷新 DuckDB `in_watchlist` 标记。不会触碰任何 `human_judgment` 栏位。
- `m3 briefing [show] [--period daily|weekly]` / `m3 briefing run [--period daily|weekly] [--no-llm]` / `m3 briefing list [--period daily|weekly] [--limit N]`：生成或读取平静日/周报（确定性 3 段骨架 + 可选 Alan 总结，平静期 0 Token）。
- `m3 triage [status] [--ticker SYM] [--verdict noise|material|falsification_alert] [--limit N]` / `m3 triage run [--limit N] [--dry-run]`：查询或执行 L1.5 能力圈公告看门狗分拣（受每日配额与月度预算门控）。
- `m3 falsification <TICKER> [list | add <TEXT> | remove <INDEX>]`：显式管理 `Dossier.human_judgment.falsification_metrics`（不会自动签署 `feynman_checked_at` 或修改 `my_stance`）。

#### `m3 research`、`m3 budget` 与 `m3 themes` 子命令契约

- `m3 research run <TICKER> [--focus "..."] [--trigger manual|inbox|cli]`：
  - 先读取 `workbench/budget.json` 与 `m3.duckdb` 当月累计消耗并执行 `evaluate_budget_gate` **硬预算预检**；若超限或月/次配额为 `0`，在启动 **Alan** 或发起任何 LLM 请求前立即阻断，记录 `blocked_budget` 审计行并返回 `ok: false`（附带结构化 `DeepResearchResult`）。
  - 预算放行后，清理过期僵死 `running` 记录（`DEFAULT_STALE_RESEARCH_RUN_MS = 15 分钟`），启动受 `M3ToolPolicyHook` 保护的 Alan 并并行调度 4 个只读子分析师会话（`护城河`、`会计排雷`、`隐空间同行`、`5年逆向验尸`），写入 `workbench/memos/<TICKER>.md`（保留既有的 `## Human Notes` 人类手写区）并记录 `research_runs` 审计行。
- `m3 research status [--ticker SYM] [--limit N]`：查询 `m3.duckdb` 中的 `research_runs` 历史记录（`--limit` 默认 `20`，范围 `1..=500`）及当前月度预算状态；当 `m3.duckdb` 存在且 RW 锁空闲时机会性地调用 `reclaim_stale_running_runs` 将启动超 `15 分钟` 的僵死 `running` 记录回收为 `failed`（若桌面端正持有写锁，则优雅降级为只读查询不报错），并在输出中返回 `reclaimed_stale_runs`。
- `m3 budget status`：返回 `BudgetStatusReport`（`workbench/budget.json` 配额配置 + 当月 Token / USD 消耗 + 剩余额度 + 周度自动触发剩余配额）。
- `m3 themes sync` / `m3 themes list`：`sync` 基于 `universe_snapshot` 与 `doc_chunks` 确定性聚合 SIC + 隐空间商业原型簇并原子写入 `workbench/themes.json`（保留人类编辑的 `user_notes`）；`list` 读取 `workbench/themes.json`。

#### `m3 alan` 子命令契约（无需/需要工作区按子命令区分）

- `m3 alan [status]`（独立可用，无需已初始化工作区）：返回 Alan 共享连接存储中的非密钥配置摘要（`default_profile`、`active_profile`、`profiles`，其中密钥仅展示脱敏 `masked_hint`）。
- `m3 alan test [--profile <ID>]`（独立可用）：调用 `alan_connection::test_alan_connection` 对默认或指定连接 Profile 发起真实探测，返回 `AlanConnectionTestResult`（`profile_id`、`provider`、`model`、`base_url`、`status_code`、`latency_ms`、`message`）。即使在已有 Tokio 运行时上下文内调用，也会通过独立 OS 线程隔离执行当前线程运行时，避免嵌套运行时 Panic。
- `m3 alan ask --scope <SCOPE> "<QUESTION>" [--lang zh|en] [--timeout <SECS>]`：
  - 支持四类规范 `AlanScope`：`company:<TICKER>`、`industry:<SLUG>`、`briefing:latest`（或 `briefing:<ID>`）、`portfolio`。
  - `--timeout` 范围 `1..=600` 秒（默认 `120`）。
  - 内部调用 `ask_scoped_blocking`：加载已有作用域会话，启动带 `/mnt/m3` 只读文件工具与 `M3ToolPolicyHook` 的 Alan，按当前 `submission_id` 精确收集文本与 `TuringTapeEntry`，解析 `[report:TICKER]` / `[xbrl:TICKER]` / `[dossier:TICKER]` 令牌并实时解析 `InlineEvidenceCard`（优先读取实时 `watchlist.json` 的 `bucket` 与 `name`），原子追加并持久化至 `workbench/alan_sessions/<scope_key>.json`（保留最近 30 轮），最终输出完整 `ScopedAlanReply` JSON（`scope`、`scope_key`、`prose`、`turing_tape`、`evidence_tokens`、`evidence_cards`、`session_path`、`input_tokens`、`output_tokens`、`elapsed_ms`）。
- `m3 alan session show --scope <SCOPE>` / `m3 alan session clear --scope <SCOPE>`：读取（返回 `session_path`、`exists`、`message_count`、`session`）或清空指定作用域的持久化会话文件（返回 `scope`、`scope_key`、`session_path`、`cleared`），不影响任何 Dossier 或 `human_judgment`。

#### `m3 app` 子命令契约（桌面应用实时控制 IPC）

所有 `m3 app` 子命令通过本地 Unix Domain Socket（默认 `<M3Paths::config_dir>/control.sock`，可用 `M3_CONTROL_SOCKET` 覆盖，权限 `0600`）与运行中的 `M3.app` 通信，支持 `--timeout <SECS>`（`1..=120`，默认 `5`），返回 `AppControlResponse` JSON（含实时 `AppControlState` 快照）：

- `m3 app ping [--timeout <SECS>]`：探测桌面端 IPC 存活状态（返回 `message: "pong"` 与最新 `state`）。
- `m3 app state [--timeout <SECS>]`：获取当前桌面端完整状态快照（`active_tab`、`company_open`、`selected_ticker`、`selected_industry_slug`、`sidebar_collapsed`、`palette_open`、`alan_stage_mode`、`alan_canvas_tab`、`alan_scope`、`alan_context_pinned`、`alan_streaming`、`alan_message_count`、`last_alan_preview`、`workspace_root`）。
- `m3 app navigate [--tab inbox|universe|industry|baskets|journal|settings] [--ticker <T>] [--industry <SLUG>] [--timeout <SECS>]`：切换工作台标签、打开指定公司详情（传入 `--ticker ""` 可关闭公司覆盖层）或选中行业研究档案。
- `m3 app alan [--mode closed|split|immersive] [--tab interrogate|feynman] [--scope <SCOPE>] [--pin true|false] [--anchor <LABEL>] [--prompt <TEXT>] [--submit] [--timeout <SECS>]`：切换桌面端统一 Alan Canvas 模式与标签、绑定作用域、锁定/解锁 `📌` 上下文图钉、设置锚点/预填输入或直接提交提问。
- `m3 app action <ACTION> [--ticker <T>] [--timeout <SECS>]`：触发桌面端动作，支持的 `ACTION` 包括：`toggle_sidebar`、`open_palette`、`close_palette`、`close_company`、`sync`、`refresh_briefing`、`reset_alan`、`deep_research`（可带 `--ticker`）。
- `m3 app screenshot <PATH> [--timeout <SECS>]`：自动创建父目录并抓取运行中的 `M3.app` 主窗口截图保存为 PNG（返回 `screenshot_path` 与 `state`）。

#### `m3 industry`、`m3 storage`、`m3 mcp` 与 `m3 doctor` 子命令契约

- `m3 industry list` / `m3 industry show <SLUG>`：列出或读取 `workbench/industries/` 中的行业研究档案（`IndustryProfile` JSON 与 `workbench/industries/<SLUG>.md` 行业 Primer 原文，保留 `## Human Notes`）。
- `m3 storage status` / `m3 storage clean <CATEGORY|all>`：扫描本地 5 类存储占用（`sec_bulk_archives`、`sec_filings_cache`、`duckdb_lakehouse`、`quotes_and_cache` 与受保护的 `personal_workbench`），支持单独或 `all` 清理可重建缓存；`personal_workbench` 永拒绝删除。
- `m3 mcp [--workspace <PATH>] [--read-only]`：在 `stdin`/`stdout` 运行行分隔 JSON-RPC 2.0 MCP 服务。默认暴露全部 **40** 个工具（24 个只读 + 16 个受控动作/Alan/桌面控制工具，均标注准确 `readOnlyHint` 与 `destructiveHint: false`）；传入 `--read-only` 时仅向客户端暴露 24 个只读工具并拒绝一切非只读工具调用（返回 `-32602`）。详情见 [docs/mcp-server.md](mcp-server.md)。
- `m3 doctor`：无需已初始化工作区即可执行，输出二进制版本/通道、配置文件、**`alan_connection` 健康检查**（`configured`、`default_profile`、`active_profile`、`profile_count`、`remediation`）、工作区计数、DuckDB 湖仓、**`stale_research_runs` 僵死任务检查**（`data.workspace.running_research_runs` 与 `stale_running_research_runs`）、存储占用与 `/Applications/M3.app` / `gh` / `alan` 可用性体检结果。

---

## 2. `/mnt/m3` 只读虚拟挂载契约（`m3_fs_list` / `m3_fs_read`）

内置在 `crates/m3-core/src/alan_interrogator.rs` 中的 `M3WorkbenchFs` 通过 `alan-ap` `FileServer` 向 Alan 暴露严格只读（`Access::ReadOnly`）的 `/mnt/m3` 虚拟文件树，并注册了两个由 `Arc<M3WorkbenchFs>` 驱动、受 `M3ToolPolicyHook` 管控的一等文件工具：

| 工具 | 参数 Schema | 约束与返回 |
| :--- | :--- | :--- |
| `m3_fs_list` | `{"path"?: string}`（默认 `"/mnt/m3"`） | 只读列出虚拟目录条目，返回 `{"ok": true, "read_only": true, "path", "count", "entries"}`。 |
| `m3_fs_read` | `{"path": string (required), "offset"?: integer (>=0, default 0), "max_bytes"?: integer (1..=65536, default 65536)}` | 有界只读文件，单次读取上限 **`M3_FS_MAX_READ_BYTES = 64 KiB`（65536 字节）**，返回 `{"ok": true, "read_only": true, "path", "offset", "bytes_read", "total_bytes", "truncated", "content"}`。 |

可读取的虚拟路径如下：
- `/mnt/m3/watchlist.json`（实时观察池、`core`/`watch`/`out`/`too_hard` 分桶与估值假设）
- `/mnt/m3/journal.json`（决策账本与基准）
- `/mnt/m3/computed/<TICKER>.json`（确定性 `MungerReport`）
- `/mnt/m3/dossiers/<TICKER>.json`（公司审讯档案）
- `/mnt/m3/industries/index.json` 与 `/mnt/m3/industries/<slug>.json`
- `/mnt/m3/memos/<TICKER>.md` 与 `/mnt/m3/memos/<TICKER>-feynman.md`
- `/mnt/m3/briefings/latest.json`（DuckDB 最新 `CircleBriefing`，空库时优雅降级）
- `/mnt/m3/events/recent.json`（近期 L1/L1.5 事件记录）

`M3ToolPolicyHook` 仅放行路径前缀为 `/mnt/m3`（`m3_fs_list`）或 `/mnt/m3/...`（`m3_fs_read`）的只读读取，并严格拦截任何写入、越界宿主路径探测或修改受保护人类主权文件的行为。

---

## 3. 结构化事实来源与权限边界

| 路径 | 跟踪状态 | 写入方 | 用途与规则 |
| :--- | :--- | :--- | :--- |
| `workbench/watchlist.json` | Git 跟踪 | 人类 / `m3 add` / `m3_watchlist_add` / UI 沙盒 | 标的列表、规范分桶（`core` / `watch` / `out` / `too_hard`，CLI/MCP 别名 `in` 映射为 `core`）、数据来源与估值倍数假设。 |
| `workbench/dossiers/<TICKER>.json` | Git 跟踪 | Alan（仅草稿区）/ 人类（`human_judgment`） | 三句话业务、跨学科格栅、事前验尸由 Alan 起草；**`human_judgment`（尤其 `my_stance`、`weight_rationale`、`feynman_checked_at`）仅限用户本人填写或费曼答辩确认后写入，Agent 严禁代签。** |
| `workbench/memos/<TICKER>.md` | Git 跟踪 | `m3 research run` / Research Desk | 4 视角并行深研生成的活研究备忘录（含确定性指标快照与原文引用）；**`## Human Notes` 标题及以下人类笔记永不被覆盖。** |
| `workbench/alan_sessions/<scope_key>.json` | Git 忽略/工作区会话 | `alan_session.rs`（原子写入） | 各 `AlanScope`（`company-COST`、`industry-retail`、`briefing-latest`、`portfolio`）的有界会话历史（最多保留 30 轮）；永不改写 `Dossier.human_judgment`。 |
| `workbench/budget.json` | Git 跟踪 | 人类（Alan 工具禁写） | L2 Deep Research 月度/单次 USD 与 Token 硬上限及每日主动看门狗配额。 |
| `workbench/themes.json` | Git 跟踪 | `m3 themes sync` / UI（保留 `user_notes`） | 跨公司商业原型聚类图谱；Alan 工具禁止改写。 |
| `workbench/journal.json` | Git 跟踪 | `m3 journal record` / `m3_journal_record` / 人类 | 模拟决策账本（100% 现金起步 + 标普 500 对照），事前冻结记录不可篡改历史条目。 |
| `data/munger/raw/sec/*.json` | Git 忽略 | `m3 sync` / GUI 同步 | SEC EDGAR 一手原始 XBRL JSON，带 SHA-256 校验。 |
| `data/munger/raw/prices/*.json` | Git 忽略 | `m3 sync` / GUI 同步 | Yahoo Finance 日级股价快照（含 `^GSPC`）。 |
| `data/munger/computed/<TICKER>.json` | Git 忽略 | Rust 计算引擎 | 确定性 `MungerReport`，所有金额与比率以十进制字符串保存，是桌面 UI 与 `m3 report` 的事实来源（注：`InlineEvidenceCard` 会优先用实时 `watchlist.json` 的 `bucket` 覆盖缓存报告中的旧值）。 |
| `data/munger/lake/raw/sec/*` | Git 忽略 | `m3 universe sync` / UI 同步 | SEC 批量归档缓存（`company_tickers_exchange.json`、`submissions.zip`、`companyfacts.zip`）。 |
| `data/munger/lake/parquet/` | Git 忽略 | `m3 universe sync` / UI 同步 | 不可变代际目录（`gen-*/`）与原子 `manifest.json` 发布的 ZSTD 压缩 Parquet 事实表（`companies`、`facts_annual`、`facts_ttm`、`prices_daily`），保留 `raw_sha256` 与 XBRL 溯源。 |
| `data/munger/lake/m3.duckdb` | Git 忽略 | `m3 universe sync` / `events catchup` / `research run` | 嵌入式 DuckDB 湖仓（`universe_snapshot`、`sync_watermarks`、`events`、`doc_chunks`、`signal_inbox`、`signal_triage`、`circle_briefings`、`research_runs`、`v_*` 视图）；Alan 审讯工具仅可执行只读查询。 |

---

## 4. Alan / Agent 最小研究工作流

1. 先运行 `m3 doctor`（或 `m3_doctor`）与 `m3 alan test` 检查环境、湖仓与 Alan 连接健康度，再运行 `m3 status` 检查观察池数据新鲜度；若要从全美股中发掘长尾候选，运行 `m3 universe screen` / `m3 universe peers <TICKER>` / `m3 universe similar`。
2. 对筛选出的高质量候选公司，用 `m3 add <TICKER> --bucket watch --circle "..."`（或 `m3_watchlist_add`）加入观察池，再用 `m3 sync --ticker <TICKER>`（或 `m3_sync`）拉取完整财报与日级历史行情。
3. 运行 `m3 report <TICKER>`（或 `m3_report`）获取确定性多年 ROIC、严苛股东盈余、股本变化、反向隐含增速和自动红黄灯预警（不要绕过 Rust 工具自行抓取或估算财务）。
4. 通过 `m3 alan ask --scope company:<TICKER> "..."`（或 `m3_alan_ask`）在作用域内审讯公司档案，或用 `m3 research run <TICKER>`（受预算门控）产出 4 视角活研报；审讯结果会附带实时核验的 `[report:TICKER]` `InlineEvidenceCard`，同时保留 `human_judgment` 与 `## Human Notes` 待人类主理人作答。
5. 引导用户用自己的话完成 3 轮“费曼防线”答辩（可用 `m3_feynman_round` 或桌面端 `费曼答辩` 面板，见 [docs/munger-workflow.md](munger-workflow.md)），由用户亲自确认击球价、签字 `feynman_checked_at` 或通过 `m3 journal record` 记录决策。
