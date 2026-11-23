# Agent 接口：Munger Workbench 数据与 CLI 契约

人类使用 GPUI 桌面窗口（`open apps/m3-desktop/artifacts/M3.app`）浏览、调参和记录判断；Alan / Agent 通过 `apps/m3-desktop/m3` CLI 与本地 JSON 文件工作，**不依赖屏幕截图或 Computer Use**。

## 1. CLI 命令一览

在仓库根目录执行，所有命令均输出 JSON 至 `stdout`：

```json
{"schema_version": 1, "ok": true, "data": ...}
```

失败时输出 `{"schema_version": 1, "ok": false, "error": "..."}` 并返回非零退出码。

### 可用子命令

```sh
# 1. 查看工作台整体状态、数据新鲜度、各公司击球状态摘要与同步记录
apps/m3-desktop/m3 status

# 2. 同步一手 SEC EDGAR 财报与 Yahoo 日级股价（支持单标的和仅同步价格）
apps/m3-desktop/m3 sync
apps/m3-desktop/m3 sync --ticker GOOGL
apps/m3-desktop/m3 sync --prices-only

# 3. 读取单家公司的确定性查理计算器报告（含多年财务、TTM、估值、预警与溯源）
apps/m3-desktop/m3 report AAPL

# 4. 读取单家公司的完整人机审讯档案（AI 草稿 + 人类判断与证伪栏位）
apps/m3-desktop/m3 dossier AAPL

# 5. 向观察池添加公司或更新分桶（自动从 SEC tickers 查找 CIK；支持 in/core、watch、out、too_hard）
apps/m3-desktop/m3 add CDNS --bucket watch --circle "EDA / 半导体设计工具" --name "Cadence Design Systems"
apps/m3-desktop/m3 add INTC --bucket too_hard --reason "重资产晶圆代工死磕期，现金流不可预测"

# 6. 决策账本操作（事前记录 WAIT / OUT / TOO_HARD / PAPER_BUY / PAPER_SELL）
apps/m3-desktop/m3 journal list
apps/m3-desktop/m3 journal record --ticker GOOGL --action wait --note "Q3 2026 CapEx 超 OCF，等击球安全边际或 CapEx 见顶"

# 7. 启动 GPUI 桌面工作台
apps/m3-desktop/m3 start

# 8. L0 全美股湖仓（DuckDB + Parquet）筛选与同行对照
apps/m3-desktop/m3 universe status
apps/m3-desktop/m3 universe sync --full --prices-top 400
apps/m3-desktop/m3 universe screen --min-roic 15 --max-sbc-ocf 25 --exclude-financials --limit 50
apps/m3-desktop/m3 universe screen --strike-only --sort fcf_yield --limit 20
apps/m3-desktop/m3 universe peers ADSK --limit 15
```

#### `m3 universe` 子命令契约

- `m3 universe status`：返回 `UniverseSummary` JSON（未同步前返回全 `0` 默认值，不报错）。
- `m3 universe sync [--full] [--prices-top N] [--skip-prices]`：同步 SEC `company_tickers_exchange.json`、`submissions.zip`（SIC 行业分类）与 `companyfacts.zip`，写入 ZSTD Parquet 与 `m3.duckdb`，按**价格方案 1**仅对 Top `N`（默认 `400`，有效范围 `1..=1000`；跳过股价请使用 `--skip-prices`）质量候选池 + 观察池标的同步日级股价。进度日志写往 `stderr`，`stdout` 保持单一 schema v1 JSON（`UniverseSyncReport`）。
- `m3 universe screen [FLAGS]`：在本地 DuckDB 执行毫秒级全美股筛选与排序，返回 `{ "query_ms", "count", "filter", "rows" }`。支持：
  - 数值过滤：`--min-roic N`、`--max-sbc-ocf N`、`--max-share-change-3y N`、`--max-net-debt-to-oe N`（共用精确十进制与科学计数法解析器，先归一化指数再校验 28 位小数精度与 `[-10000, 100000]` 范围，拒绝无法精确表示的小数或溢出）、`--min-years N`（有效范围 `1..=20`）
  - 开关过滤：`--exclude-financials`（默认启用，剔除 `SIC 6000–6799`）、`--include-financials`、`--positive-oe-only`、`--price-pool-only`、`--strike-only`
  - 搜索与排序：`--query TEXT`、`--sort quality|roic|fcf_yield|distance|share_change|sbc_ocf|price|strike|warnings|ticker|name|sic|oe`、`--asc` / `--desc`、`--limit N`（默认 `200`，有效范围 `1..=5000`）
- `m3 universe peers <TICKER> [--limit N]`：输出目标公司在同 SIC 组内的 `roic_percentile`、`gross_margin_percentile`、`strict_fcf_yield_percentile` 及头部同行列表（`PeerComparison`；`--limit` 默认 `15`，有效范围 `1..=200`）。

---

## 2. 结构化事实来源与权限边界

| 路径 | 跟踪状态 | 写入方 | 用途与规则 |
| :--- | :--- | :--- | :--- |
| `workbench/watchlist.json` | Git 跟踪 | 人类 / `m3 add` / UI 沙盒 | 标的列表、分桶（`core`(`IN`) / `watch` / `out`(`OUT`) / `too_hard`）、数据来源与估值倍数假设。 |
| `workbench/dossiers/<TICKER>.json` | Git 跟踪 | Alan（仅草稿区）/ 人类（`human_judgment`） | 三句话业务、跨学科格栅、事前验尸由 Alan 起草；**`human_judgment` 仅限用户本人填写或费曼答辩确认后写入，Agent 严禁代填。** |
| `workbench/journal.json` | Git 跟踪 | `m3 journal record` / 人类 | 模拟决策账本（100% 现金起步 + 标普 500 对照），事前冻结记录不可篡改历史条目。 |
| `data/munger/raw/sec/*.json` | Git 忽略 | `m3 sync` / GUI 同步 | SEC EDGAR 一手原始 XBRL JSON，带 SHA-256 校验。 |
| `data/munger/raw/prices/*.json` | Git 忽略 | `m3 sync` / GUI 同步 | Yahoo Finance 日级股价快照（含 `^GSPC`）。 |
| `data/munger/computed/<TICKER>.json` | Git 忽略 | Rust 计算引擎 | 确定性 `MungerReport`，所有金额与比率以十进制字符串保存，是桌面 UI 与 `m3 report` 的事实来源。 |
| `data/munger/lake/raw/sec/*` | Git 忽略 | `m3 universe sync` / UI 同步 | SEC 批量归档缓存（`company_tickers_exchange.json`、`submissions.zip`、`companyfacts.zip`）。 |
| `data/munger/lake/parquet/` | Git 忽略 | `m3 universe sync` / UI 同步 | 不可变代际目录（`gen-*/`）与原子 `manifest.json` 发布的 ZSTD 压缩 Parquet 事实表（`companies`、`facts_annual`、`facts_ttm`、`prices_daily`），保留 `raw_sha256` 与 XBRL 溯源。 |
| `data/munger/lake/m3.duckdb` | Git 忽略 | `m3 universe sync` / UI 同步 | 嵌入式 DuckDB 湖仓（`universe_snapshot`、`sync_watermarks`、`v_*` 视图），由 `m3 universe status/screen/peers` 只读查询。 |

---

## 3. Alan / Agent 最小研究工作流

1. 先运行 `m3 status` 检查观察池数据新鲜度；若要从全美股中发掘长尾候选，运行 `m3 universe status` 或 `m3 universe screen` / `m3 universe peers <TICKER>`（需要最新全市场底库时运行 `m3 universe sync`）。
2. 对筛选出的高质量候选公司，可用 `m3 add <TICKER> --bucket watch --circle "..."` 加入观察池，再用 `m3 sync --ticker <TICKER>` 拉取完整日级历史行情。
3. 运行 `m3 report <TICKER>` 获取确定性多年 ROIC、严苛股东盈余、股本变化、反向隐含增速和自动红黄灯预警（不要绕过 Rust 工具自行抓取或估算财务）。
4. 运行 `m3 dossier <TICKER>` 或直接读取 `workbench/dossiers/<TICKER>.json`，围绕事实数字更新跨学科草稿与 5 年事前验尸问题，**保留 `human_judgment` 待用户作答**。
5. 引导用户用自己的话完成“费曼防线”答辩（见 [docs/munger-workflow.md](munger-workflow.md)），再由用户确认击球价或决策账本记录。
