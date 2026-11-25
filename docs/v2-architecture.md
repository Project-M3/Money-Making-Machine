# Project-M3 v2 架构与里程碑设计：全美股事件驱动投研引擎

> 状态：已完成并交付（`v0.3.0 – v0.4.2`）· 后续 `v0.5 → v1.0` 演进规划请见 [ROADMAP.md](../ROADMAP.md)  
> 目标：把 M3 从“9 只手动自选股体检本”演进为“**全美股确定性底库 + 事件驱动近实时流 + 隐空间跨公司综合 + Alan 分析师 Deep Research**”的 Local-First 投委会工作台。

---

## 1. 核心范式跃迁（Why v2）

v1 验证了确定性 Rust XBRL 计算、桌面工作台与进程内 `EmbeddedAlan` 审讯的基本闭环，但只覆盖手选的 9 只股票——其中明星白马（`AAPL`、`COST`、`SNPS` 等）长期显著高于击球价，暴露出**“人手选股带宽受限”**的根本瓶颈。

查理·芒格集中投资的本质是**决策集中**（只打好球、重仓少做），而**“只能精读几十家公司”只是碳基大脑的带宽约束，并非投资优势**。v2 用机器规模打破研究带宽瓶颈，同时保留两条不可动摇的铁律：

1. **财务算术零 LLM**：营收、毛利、ROIC、严苛股东盈余（`OCF - CapEx - SBC`）、股本稀释率、净负债比与击球价，全部由确定性 Rust 与本地 DuckDB 计算并保留 SHA-256 与 XBRL 溯源。
2. **人类主权不可替代**：Alan 作为股票分析师负责发现、关联综合、排雷与费曼追问；禁止替用户填写 `human_judgment` 或直接给出买卖指令，击球价拍板与模拟/决策账本仍由人类主理人锁定。

### 已对齐的 4 项硬约束（2026-11-23 确认）

| 维度 | 决策 | 工程含义 |
| :--- | :--- | :--- |
| **1. 覆盖范围** | **先吃透全美股** | 基于 SEC EDGAR 免费全量 XBRL 与公告覆盖 ~5,000+ 活跃美股，完整闭环跑通后再评估非美股扩展。 |
| **2. 流式粒度** | **事件驱动的近实时投研流** | 关注 10-K / 10-Q / 8-K / Form 4、候选池新闻与日级价格穿越击球阈值；不做盘中逐笔 tick 与高频盯盘。 |
| **3. 运行形态** | **Local-First 单体 App** | 沿用 `m3-nsh` + `m3` CLI（`crates/m3-core` / `crates/m3-cli` / `apps/m3-nsh`），不搭云端微服务或 24/7 `launchd` 守护进程；依靠 DuckDB 游标实现启动时**断点续传追平（Catch-up Stream）** + 运行期低频增量轮询。 |
| **4. 成本策略** | **极度克制的低成本自动运转 + 价格方案 1（零成本先跑通）** | L0 纯靠 SEC 免费全量财报筛出 Top 300–500 质量候选池（0 Token、0 订阅费），仅对该池拉取免费公开日级股价；L1 近零成本做增量索引与相关性；L2 旗舰模型 Deep Research 严格受手动派单或硬预算上限控制。 |

---

## 2. 三层漏斗与数据流架构（L0 → L1 → L2 → L3）

解决“5,000+ 股票规模 × 芒格深度”矛盾的核心，是让算力与 Token 沿着漏斗逐级收敛：

```text
[L0 · 全市场确定性底库 | 0 Token · 0 订阅费 | ~5,000+ 活跃美股]
  全量引导: SEC company_tickers_exchange.json + companyfacts.zip (ETag/Last-Modified)
  增量追平: 仅对新披露 10-K/10-Q 的 CIK 拉取 CIK{cik}.json
  计算引擎: 复用 edgar::parse_companyfacts_bytes + calculator::build_report (多核并行)
  价格方案1: 先用无股价依赖的质量指标排序筛出 Top 300–500 候选池 (+ watchlist)
            再仅对该池限速拉取免费公开日级股价 (market.rs) 计算击球距与 FCF 收益率
        │
        ▼ (仅候选池 + 既有观察池 + 重大财报/价格异动)
[L1 · 事件驱动流与隐空间相关性层 | 近 0 成本 | DuckDB 游标断点续传]
  事件源: 新 10-K/10-Q/8-K/Form 4 (submissions.zip 引导 + 增量 feed)、候选池新闻、击球穿越
  文本切片: 仅切分候选池 10-K 核心章节 (Item 1 / Item 1A / Item 7)、8-K 摘要与新闻 (~1–2 万 chunks)
  混合检索: 嵌入式 DuckDB 内执行「结构化财务 SQL × FTS 全文 × 余弦向量相似度」
            1) Lookalikes（商业原型近邻）: 寻找像 ADSK/COST 但 SBC 更低或估值更合理的长尾公司
            2) Cross-Company Ripple（事件传导）: 新 8-K/新闻在隐空间里波及到候选池哪几家公司
  静默降噪: 规则分 + 低成本轻量过滤；低信噪比事件静默归档，高信号进入简报收件箱
        │
        ▼ (由人类主动派单，或周度硬配额内的高分新发现/击球触发)
[L2 · Alan 股票分析师集群 | Deep Research | 严格成本闸门]
  底座升级: 基于 EmbeddedAlan (alan v0.1.4+ / open_session / spawn_child_session)
  分析师工具: 只读 DuckDB 工具 (全市场筛选、同行分位数、语义近邻、事件流) + 受控一手原文/新闻工具
  并行视角: ① 商业模式与护城河  ② 会计与现金流排雷  ③ 跨公司隐空间/产业链综合  ④ 5 年逆向空头验尸
  硬防线: M3ToolPolicyHook 阻断任何对真值湖仓、human_judgment、watchlist、journal 的写操作
          workbench/budget.json 强制月度与单次 Token 上限
        │
        ▼
[L3 · 人类主理人（投委会主席）| 每周 6–10 小时低焦虑节奏]
  在 M3 桌面工作台分拣 Inbox 信号、发起费曼拷问、锁定 human_judgment 与击球价、记入决策账本
```

### 2.1 L0：全市场确定性底库与价格方案 1

- **合规全量引导（Bootstrap）**：
  - 使用 `https://www.sec.gov/files/company_tickers_exchange.json` 获取美股主流交易所（Nasdaq / NYSE / CBOE）映射与唯一规范 Ticker（同一 CIK 多类别股票选取主交易代码）。
  - 下载 SEC 官方每日批处理归档 `https://www.sec.gov/Archives/edgar/daily-index/xbrl/companyfacts.zip`，配合 `ETag` / `Last-Modified` 条件请求避免重复下载；全请求保留合规 `M3_SEC_USER_AGENT`（见 `src/edgar.rs`）。
  - 流式解压并通过 `rayon` 并行调用现有的 `edgar::parse_companyfacts_bytes` 与 `calculator::build_report`（纯函数、零 I/O），保证全市场指标口径与单股 `m3 report` 100% 一致。
- **零成本价格分层（方案 1）**：
  - 全市场 5,000+ 家公司无需逐一抓取股价即可完成核心质量初筛，因为 **5 年 ROIC 均值与标准差、严苛股东盈余正负、毛利率稳定性、3 年/5 年净股本变化率、SBC/OCF 占比、净负债/严苛股东盈余** 全部仅依赖财报本身。
  - 在 DuckDB 内按质量规则排定 Top 300–500 家 `quality_candidates`（并无条件并入 `workbench/watchlist.json` 现有标的），只对这个收缩后的集合执行节流化的 Yahoo Finance 日级股价同步（失败保留旧缓存，不触发 429）。

### 2.2 L1：断点续传事件流与隐空间相关性

- **Local-First Catch-up 语义**：App 关闭期间不启动后台服务；`m3.duckdb` 的 `sync_watermarks` 表记录 SEC 提交、日级股价与新闻源的最后处理游标。每次启动 `M3.app`（或运行 `m3 events catchup`），自动补齐离线期间产生的全部增量事件；App 打开期间由 Tokio 后台任务按 15–30 分钟低频追平。
- **隐空间（Latent Space）与结构化 SQL 融合**：
  - 只对 Top 300–500 候选池抓取并切分 10-K（Item 1 业务描述、Item 1A 风险因素、Item 7 MD&A）、8-K 摘要及候选池新闻，总 chunk 量控制在 1–2 万条级别。
  - Embedding 仅在新增/变更 chunk 时计算一次（优先本地轻量模型或极低单价 Embedding API），向量直接存入 DuckDB，不引入外部独立向量数据库。
  - 在 ~2 万条切片规模下，DuckDB 内可以直接把**财务硬过滤 SQL**（如 `roic_5y_avg >= 18 AND sbc_to_ocf <= 20`）与**全文/余弦相似度计算**写进同一条查询，实现毫秒级 **Lookalike 搜索** 与 **跨公司事件传导匹配**。

### 2.3 L2：Alan 股票分析师集群与成本/权限门控

- **多会话并行深研**：基于 `crates/m3-core/src/alan_interrogator.rs` 现有的 `EmbeddedAlan` 集成（固定在公开仓库 `alan` `v0.1.4+`），通过 `open_session` / `spawn_child_session` 调度专业化子分析师会话，并行完成商业护城河、会计排雷、隐空间同行对比与 5 年逆向验尸，最终汇总为可演化研报。
- **面向分析的只读工具集**：在现有 `m3_status / m3_report / m3_dossier / m3_update_draft` 基础上，新增只读湖仓查询工具（`m3_universe_query`、`m3_peer_percentiles`、`m3_semantic_neighbors`、`m3_events_for_ticker`）及受控的一手原文/新闻片段抓取工具。
- **预算与红线双闸门**：
  - `workbench/budget.json` 定义月度 Token/费用硬上限、单次 Deep Research 预算上限以及自动触发配额（默认关闭自动唤醒旗舰模型，只允许手动派单或每周有限高分信号名额）。
  - `M3ToolPolicyHook` 继续在工具策略层无条件拦截任何写 `data/munger/`、DuckDB/Parquet、`human_judgment`、`watchlist.json`、`journal.json` 的行为。

---

## 3. 本地湖仓 Schema 与目录边界

坚持 **单二进制嵌入式湖仓**：在 `crates/m3-core` 中以 `bundled` 模式内嵌 Rust `duckdb` crate，既用 DuckDB SQL 原生执行 OLAP、FTS 与相似度检索，也由 DuckDB 直接导出/查询 ZSTD 压缩 Parquet（避免引入整套重型 `arrow` / `parquet` Rust 编译链）。

```text
Money-Making-Machine/
├── data/munger/                         # [Git 忽略] 本地事实缓存与湖仓
│   ├── raw/                             # v1 既有逐标原始 JSON（向后兼容）
│   ├── computed/                        # v1 既有单标 MungerReport 缓存
│   └── lake/                            # v2 新增嵌入式湖仓
│       ├── raw/sec/                     # companyfacts.zip / submissions.zip / company_tickers_exchange.json
│       ├── parquet/
│       │   ├── companies.parquet        # cik, ticker, name, exchange, sic, is_active
│       │   ├── facts_annual.parquet     # 多年期财务与 ROIC/严苛 OE/稀释率/SHA-256/XBRL 溯源
│       │   ├── facts_ttm.parquet        # 最新 TTM 与 YTD 预警输入
│       │   ├── prices_daily.parquet     # 候选池及基准日级行情
│       │   └── filings.parquet          # 10-K/10-Q/8-K/Form 4 元数据与路径
│       └── m3.duckdb                    # 面向 Parquet 的视图 + 游标/事件/切片向量/Inbox/预算表
└── workbench/                           # [Git 跟踪] 人类与 Alan 协作的长期研究资产
    ├── watchlist.json                   # 核心/观察/否决/Too Hard 篮子与估值假设（v1 兼容）
    ├── journal.json                     # 事前冻结决策账本（v1 兼容）
    ├── dossiers/<TICKER>.json           # 审讯档案与 human_judgment（v1 兼容）
    ├── memos/<TICKER>.md                # [v2] 活研究备忘录（Living Research Memo，含一手引用锚点）
    ├── themes.json                      # [v2] 跨公司商业原型、产业链与隐空间主题图谱
    └── budget.json                      # [v2] L1/L2 成本与 Deep Research 配额上限
```

### `m3.duckdb` 核心表与视图设计

| 对象 | 类型 | 核心字段 | 用途 |
| :--- | :--- | :--- | :--- |
| `v_companies` / `v_facts_*` / `v_prices_daily` | Parquet 视图 | 直接映射 `parquet/*.parquet` | 零冗余存储，支撑毫秒级全市场指标与同行分位数聚合。 |
| `universe_snapshot` | 物化表 | `cik`, `ticker`, `sic`, `is_financial`, `quality_score`, `in_price_pool`, `latest_price`, `strike_price`, `distance_to_strike_pct`, `roic_5y_avg`, `strict_fcf_yield`, `warnings_count` | 驱动全市场雷达 UI 与 `m3 universe screen` 的主宽表。 |
| `sync_watermarks` | 状态表 | `source_key`, `cursor`, `etag`, `last_modified`, `synced_at_ms` | Local-First 断点续传游标。 |
| `events` | 增量表 | `event_id`, `source`, `cik`, `ticker`, `event_type`, `occurred_at_ms`, `title`, `summary`, `url`, `processed_mask` | 记录每条 Filing / Form 4 / 新闻 / 价格异动事件。 |
| `doc_chunks` | 语义表 | `chunk_id`, `cik`, `ticker`, `doc_type`, `period_end`, `section`, `content`, `embedding FLOAT[N]` | 仅覆盖 Top 300–500 候选池文本切片，支撑 FTS + 余弦混合查询。 |
| `signal_inbox` | 收件箱表 | `signal_id`, `kind` (`discovery`/`ripple`/`strike`/`red_flag`), `score`, `tickers_json`, `evidence_json`, `status` (`unread`/`researching`/`watch`/`too_hard`/`archived`) | 首页 Inbox 卡片数据源与分拣状态机。 |
| `research_runs` | 审计表 | `run_id`, `ticker`, `trigger`, `model`, `input_tokens`, `output_tokens`, `cost_usd`, `started_at_ms`, `status`, `memo_path` | 追踪每次 Deep Research 耗时、成本与预算扣除。 |

---

## 4. 产品与 UI/UX 重构蓝图（4 个核心视图）

沿用当前深黑曜石（Obsidian Precision Instrument）视觉体系与 `src/ui/` 模块化结构，把交互重心从“逐个点开固定自选股”重塑为“**系统推高信噪比信号 + 探索全市场长尾 + 审阅活研报**”：

| 快捷键 | 视图 | 定位与核心交互 |
| :--- | :--- | :--- |
| `⌘1` | **1 简报收件箱 (`Inbox`)**（新默认首页） | 顶部显示全市场覆盖数、最近追平时间、本月 AI 预算水位。主列表分三类卡片：**新浮现候选** / **隐空间跨公司传导** / **击球与红灯**。支持 Linear 风格键盘单键分拣：`d` 派 Alan 跑 Deep Research、`w` 收入观察池、`x` 入 Too Hard、`e` 归档。 |
| `⌘2` | **2 全市场雷达 (`Universe`)** | DuckDB 毫秒级全美股探索：顶部可组合芒格硬滤镜（`ROIC 5Y 均值`、`严苛 OE > 0`、`SBC/OCF 上限`、`净负债倍数`、`剔除金融业`）+ **混合语义搜索框**（例如“类似 ADSK 的工业订阅软件”）。支持高密度榜单与「ROIC × 严苛 FCF 收益率」二维散点图，点击任意未关注公司即可下钻财务底表并一键加入观察池。 |
| `⌘3` | **3 活研报与分析师台 (`Research Desk`)** | 合并升级原公司体检卡与 Alan 审讯台：左侧保持确定性财务支柱、击球带 K 线图、全市场同行分位数与 XBRL 溯源；主区展示持续演化的 **Living Research Memo**、**跨公司隐空间关联卡**与可派单的 **Alan Deep Research 控制台**（Turing Tape 实时展示多子分析师进度与 Acme 证据跳转）；底部/右侧独立保留人类主权区 `human_judgment`。 |
| `⌘4` | **4 主题图谱与决策账本 (`Themes & Journal`)** | 上半区按商业原型/产业链/技术冲击变量聚合相关标的群组，跟踪跨公司联动；下半区延续 v1 事前冻结的模拟决策账本（对比 100% 现金与 `^GSPC` 机会成本）。 |

---

## 5. CLI 与 Agent 接口扩展（向后兼容 v1）

保留现有 `m3 status / sync / report / dossier / add / journal / start` 全部行为与 schema v1 输出不变，按里程碑增量扩展结构化 JSON 子命令：

- **Milestone A**：
  - `m3 universe sync [--full] [--prices-top 400]` — 下载/增量更新 SEC 全量包并构建 Parquet + DuckDB 宽表，按方案 1 仅同步 Top N 质量候选与观察池股价。
  - `m3 universe screen [--min-roic 15] [--max-sbc-ocf 25] [--exclude-financials] [--limit 50]` — 毫秒级确定性全市场筛选。
  - `m3 universe peers <TICKER>` — 基于 SIC 与财务结构输出全市场同行分位数对照。
- **Milestone B**：
  - `m3 events catchup` — 按游标追平最新 SEC Filing / Form 4 / 候选池新闻与击球穿越信号。
  - `m3 inbox list [--status unread]` — 输出高信噪比简报卡片队列。
  - `m3 universe similar <TICKER|QUERY>` — 跑通财务硬条件 × 语义向量的混合相关性检索。
- **Milestone C**：
  - `m3 research run <TICKER> [--focus "..."]` / `m3 research status` — 触发或查询受预算管控的 Alan Deep Research 任务。
  - `m3 budget status` — 查看本月 L1/L2 Token 消耗与剩余配额。

---

## 6. 三阶段里程碑与硬验收标准

### Milestone A — L0 全美股湖仓底座 + 全市场雷达（下一步立即启动）

- **工程范围**：
  - 在 `crates/m3-core/Cargo.toml` 引入 `duckdb`（`bundled`）、`zip`、`rayon`；
  - 新增 `src/lake.rs`（Parquet 导出、DuckDB 连接与表/视图迁移、游标读写）与 `src/universe.rs`（SEC `company_tickers_exchange.json` + `companyfacts.zip` 批量解析、金融业 SIC 6000–6799 标记、方案 1 Top 300–500 质量分层与限速批量股价同步）；
  - 扩展 `crates/m3-cli` 支持 `m3 universe sync / screen / peers`；
  - 在 `apps/m3-nsh/src/views/` 新增 `Universe` 视图并保留现有视图完整可用。
- **硬验收标准**：
  1. 全美股 SEC 批量摄入在本地稳定跑通，明确记录成功解析数与跳过诊断（如缺失 XBRL 字段或非标准财年），全链路零 `unwrap()` panic；
  2. DuckDB 对 5,000+ 家美股的筛选与排序查询耗时 `< 100ms`；
  3. 方案 1 质量分层产出可审计的 Top 300–500 候选池并完成日级股价抓取与击球距计算，触发限流时优雅退避并保留旧缓存；
  4. `Universe` 视图能真实浮现至少 **10+ 家不在原 9 股名单内的高 ROIC 候选公司**，支持直接查看 5 年确定性指标与一键加入观察池；
  5. 对 `AAPL`、`ADSK`、`GOOGL` 等已知标的，DuckDB 湖仓中导出的数值与现有 `m3 report`（及独立 `Decimal` 校验）精确一致。

### Milestone B — L1 断点续传事件流 + 隐空间相关性 + Inbox 首页

- **工程范围**：
  - 接入 `submissions.zip` 历史引导与增量 Filing（10-K/10-Q/8-K/Form 4）+ 候选池新闻事件抓取，落盘至 `events` 与 `sync_watermarks`；
  - 对 Top 300–500 候选池做 10-K/8-K/新闻文本切片、Embedding 索引与 DuckDB 混合相似度检索，生成 Lookalikes 与 Cross-Company Ripple 信号；
  - 将默认首页切换为 `Inbox`（保留原有视图入口），上线规则打分与键盘快速分拣。
- **硬验收标准**：
  1. 关闭 App 一段时间后重新打开，能严格从 `sync_watermarks` 增量追平离线期间新披露公告与股价异动，不重复全量拉取；
  2. 混合查询能针对真实测试事件/公司返回带一手原文出处（Filing / 日期 / 章节）的隐空间近邻与跨公司传导列表；
  3. L0 与 L1 全链路保持 **0 旗舰模型 Token 消耗**。

### Milestone C — L2 Alan 股票分析师集群 + Deep Research + 活研报

- **工程范围**：
  - 在 `src/alan_interrogator.rs` 中为 `EmbeddedAlan` 挂载只读 DuckDB 工具与受控原文/新闻检索工具，并通过 `open_session` / `spawn_child_session` 实现多视角并行深研编排；
  - 落地 `workbench/budget.json` 与 `research_runs` 硬预算闸门，落盘 `workbench/memos/<TICKER>.md` 与 `workbench/themes.json`；
  - 将单公司页升级为 `Research Desk`，增加 `Themes` 图谱联动视图。
- **硬验收标准**：
  1. 从 `Inbox` 或 `Universe` 一键派发 Deep Research 任务后，UI 实时流式展示多子分析师 Turing Tape 过程与 Acme 证据标签，并产出结构化 Cited Memo；
  2. `M3ToolPolicyHook` 单元测试与集成测试证明 Alan 无法写入 DuckDB/Parquet、`human_judgment`、`watchlist.json` 或 `journal.json`；
  3. 当测试配置触发月度或单次预算上限时，调度器在发起 LLM 请求前硬拦截并给出清晰状态提示。

---

## 7. 已知坑点与明确非目标（Non-Goals）

- **金融股口径污染防护**：银行、保险与租赁/消费金融公司（`SIC 6000–6799`）的资产负债表内含存款与自营负债，套用标准 `OCF - CapEx` 与 `投入资本 = 债务 + 权益 - 现金` 会严重失真。L0 在 `companies.parquet` 与筛选器中必须显式标记 `is_financial`，默认从通用质量榜剔除（留待专项金融指标规则或人工标注）。
- **SEC 批量下载与多 ticker 映射**：同一个 CIK 可能对应优先股、认股权证或多类股 ticker，映射表需确定性选出唯一主代码并记录映射依据，避免计算重复。
- **SEC Fair Access 与 Yahoo 节流礼仪**：严禁并发轰炸或绕过限流（403/429/451 如实报错并保留本地缓存）。
- **v2 阶段边界说明（与 `v0.7–v1.0` 的衔接）**：v2（`v0.3 – v0.4.2`）刻意不建云端后端、不做盘中高频盯盘、不接入券商实盘下单，以确保“确定性湖仓 + 事件流 + 逆向投研”先行跑通；面向价值投资的**可编程策略引擎、Paper Broker 与受确定性 Rust `RiskEngine` 硬门控的券商适配/自主组合管理**已规划于 [ROADMAP.md](../ROADMAP.md) 的 `v0.7 → v1.0` 阶段分步演进。
