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
```

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

---

## 3. Alan / Agent 最小研究工作流

1. 先运行 `m3 status` 检查数据新鲜度；若需要最新财报或价格，运行 `m3 sync`（不要绕过 Rust 工具自行抓取或估算财务）。
2. 运行 `m3 report <TICKER>` 获取确定性多年 ROIC、严苛股东盈余、股本变化、反向隐含增速和自动红黄灯预警。
3. 运行 `m3 dossier <TICKER>` 或直接读取 `workbench/dossiers/<TICKER>.json`，围绕事实数字更新跨学科草稿与 5 年事前验尸问题，**保留 `human_judgment` 待用户作答**。
4. 引导用户用自己的话完成“费曼防线”答辩（见 [docs/munger-workflow.md](munger-workflow.md)），再由用户确认击球价或决策账本记录。
