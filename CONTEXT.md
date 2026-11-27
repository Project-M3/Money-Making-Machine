# M3 芒格价值研究工作台

个人长期价值投资与研究系统中的核心边界定义与当前状态。

## Language

- **能力圈标准分桶（`WATCH → IN → OUT → TOO HARD`）**：
  - `WATCH`（待审讯）：待完成财报体检与费曼答辩的观察名单；
  - `IN`（核心击球）：处于用户真实能力圈内、商业模式清晰且具备长期跟踪价值的精选公司（第一版控制在 15–20 家以内）；
  - `OUT`（看懂否决）：商业逻辑看懂但因护城河、文化或资本配置不够好而明确否决的公司；
  - `TOO HARD`（能力圈外 / Too Hard 博物馆）：因商业生态过于复杂、重资产内卷或未来 5–10 年自由现金流不可预测而被明确放弃研究与交易的公司（如 `INTC`、`1810.HK`）。
- **严苛股东盈余（Strict Owner Earnings）**：`OCF - CapEx - SBC`，把股权激励作为真实股东成本直接扣除，不采信剔除 SBC 的 Non-GAAP 口径。
- **击球价（Strike Price）**：依据严苛股东盈余与用户保守估值倍数（默认下沿，如 18–22 倍）或用户手工覆写值确定的入场价格门槛。
- **模拟决策账本（Paper Decision Journal）**：仅用于记录研究结论、事前证伪条件与假设击球基准对比（现金 vs 标普 500 `^GSPC`），**不代表真实资金账户或实盘授权**。

## 当前基线与演进路线（`v0.6.0` 已交付 → `v0.6.5–v1.0`）

1. **历史量化原型归档**：2026 年 10 月验证的 `BTC/USDT` 小时 SMA20 趋势策略（2025 留出期净亏 16.21%）与 1 万元资金费率对冲（2025 扣出入金后净赚约 76 元、2024 合约钱包资金不足）已完成事前审计并归档于 `spikes/nautilus-trend/` 与 `docs/research/`。不得再调用或重启 `m3-paper` 与相关定时采集任务。
2. **研究工作台与零空白屏响应基线已落地（`v0.5.1`，`e23259f`）**：已建成全 Rust 栈（`crates/m3-core` + `crates/m3-cli` + `apps/m3-nsh`）、SEC EDGAR XBRL 全美股 DuckDB/Parquet 湖仓（`L0`/`L1`/`L2` 与渐进式启动）、约 4x 加速的 `search_similar`、首帧无空白屏水合与异步 Off-Thread Research Desk、无损移出能力圈、标准 `WATCH → IN → OUT → TOO HARD` 排序、应用内 5 家服务商 **Alan** 配置与四视角逆向审讯、只读 `m3 mcp` 服务，以及烟熏黑毛玻璃 Longbridge NSH `0.7.0` macOS 桌面工作台。
3. **`v0.6.0 — Proactive Alan` 已交付（见 [docs/v0.6-proactive-alan.md](docs/v0.6-proactive-alan.md)）**：
   - **核心模块（`crates/m3-core`）**：落地 `feynman.rs`（三轮苏格拉底费曼教练、`WATCH → IN` 毕业软门控与答辩归档）与 `proactive.rs`（L1.5 能力圈公告看门狗与平静日/周报），并在 `budget.rs` 加入日配额 + 月度硬上限双闸门、`store.rs` 加入 `Dossier` / `falsification_metrics` 原子持久化。
   - **存储与湖仓**：在嵌入式 DuckDB（`m3.duckdb`）新增 `signal_triage` 与 `circle_briefings` 表，与 Workbench JSON / Markdown 存储（`workbench/dossiers/<TICKER>.json`、`workbench/budget.json`、`workbench/memos/<TICKER>-feynman.md`、`workbench/industries/<slug>/feynman-defense.md`）协同，严格隔离人类主权字段与 AI 产物。
   - **CLI 与只读 MCP**：`crates/m3-cli` 扩展 `m3 briefing`、`m3 triage` 与 `m3 falsification` 子命令，并在 `m3 mcp` 中暴露对应的只读简报与分拣查询工具。
   - **NSH `0.7.0` 桌面 IA 与单进程保活**：`⌘1` 升级为分段式 `简报 | 事件流` 信息架构；提供 `falsification_metrics` 原生 CRUD 编辑器与珊瑚色 `FalsificationAlert` 一键触发四视角深研；内置兼容 macOS 原生听写的三轮费曼教练与 `WATCH → IN` 软门控弹窗；通过 `AppOptions::keep_alive_on_close` + macOS Menu Bar `SetTray` 实现单进程低频后台刷新，避免 DuckDB 跨进程锁冲突。
4. **`v0.6.5 — Unified Alan` 架构增量（下一里程碑，见 [docs/v0.6.5-unified-alan.md](docs/v0.6.5-unified-alan.md)）**：
   - **App-Shell 双画布（`Closed | Split | Immersive`）**：全局与总览标签页默认 `Closed`，`Company Desk` 与 `Industry Studio` 记住用户的 `Split` 偏好并在 `Split` 打开或窄窗时自动收起左侧导航；统一顶栏 Alan 开关与 `⌘L` / `⇧⌘L` / `Esc` 快捷键节律。
   - **统一 `AlanScope` 与入口收口**：将公司与行业的所有交互式 Alan 对话、费曼答辩统一到同一 Alan Canvas（`审讯对话 | 费曼答辩`），支持 `Company` / `Industry` / `Briefing` / `Portfolio` 四类作用域、`📌` 上下文锁定、确定性内联 M3 证据卡与默认单行可折叠 Turing Tape；退役 Company Desk Tab 2（4 视角深研折叠入 Living Memo 并保留异步触发），并移除 Industry 底部独立聊天框与内联费曼区。
   - **只读 `/mnt/m3` 扩展与会话持久化**：扩展 `/mnt/m3` 只读挂载树（`industries/`、`memos/`、`briefings/latest.json`、`events/recent.json`），新增有界原子会话持久化（`workbench/alan_sessions/<scope_key>.json`）；`crates/m3-core/Cargo.toml` 中的 `alan` 依赖保持 `v0.1.4`，同时在 `/mnt/m3` 与 `FileServer` 契约上保持对后续 `alan9` 的前向兼容。
5. **后续演进路线（`v0.7 → v1.0`）**：按 [ROADMAP.md](ROADMAP.md) 先落地可编程价值策略引擎与内置 `Paper Broker`（`v0.7`），将港股 `HKEX` 公告与多市场多币种会计顺延至 `v0.7.5` / `v0.7` 之后随券商适配推进，随后演进至基于确定性 Rust `RiskEngine` 硬护栏的券商适配与自主组合管理（`v0.8–v1.0`）。
