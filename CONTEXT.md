# M3 芒格价值研究工作台

个人长期价值投资与研究系统中的核心边界定义与当前状态。

## Language

- **核心观察池（Core / Watch）**：处于用户真实能力圈内、商业模式清晰且具备长期跟踪价值的公司（第一版控制在 15–20 家以内）。
- **Too Hard 博物馆**：因商业生态过于复杂、重资产内卷或未来 5–10 年自由现金流不可预测而被明确放弃研究与交易的公司（如 `INTC`、`1810.HK`）。
- **严苛股东盈余（Strict Owner Earnings）**：`OCF - CapEx - SBC`，把股权激励作为真实股东成本直接扣除，不采信剔除 SBC 的 Non-GAAP 口径。
- **击球价（Strike Price）**：依据严苛股东盈余与用户保守估值倍数（默认下沿，如 18–22 倍）或用户手工覆写值确定的入场价格门槛。
- **模拟决策账本（Paper Decision Journal）**：仅用于记录研究结论、事前证伪条件与假设击球基准对比（现金 vs 标普 500 `^GSPC`），**不代表真实资金账户或实盘授权**。

## 2026-11-19 战略转向与现状

1. **历史量化原型归档**：2026 年 10 月验证的 `BTC/USDT` 小时 SMA20 趋势策略（2025 留出期净亏 16.21%）与 1 万元资金费率对冲（2025 扣出入金后净赚约 76 元、2024 合约钱包资金不足）已完成事前审计并归档于 `spikes/nautilus-trend/` 与 `docs/research/`。不得再调用或重启 `m3-paper` 与相关定时采集任务。
2. **新系统落地**：工作区已从早期“高频监控面板”彻底重塑为 **M3 · Munger Workbench**（`crates/m3-core` + `crates/m3-cli` + `apps/m3-nsh`）：
   - Rust 确定性引擎（`crates/m3-core`，macOS 主力 + iOS-ready）：直接抓取 SEC EDGAR 官方 Company Facts XBRL 原始财报 + Yahoo Finance 日级公开股价，计算 5–8 年财务指标、TTM、10-Q 预警与交互式估值。
   - 文件原生工作区：`workbench/`（Git 跟踪观察池、公司审讯档案、模拟决策账本），`data/munger/`（Git 忽略原始 JSON 缓存与计算快照）。
   - Agent 接口：`crates/m3-cli`（`m3` 二进制 / `apps/m3-nsh/m3` 包装脚本）输出 schema v1 JSON，供 Alan 读写与开展费曼问答，绝不依赖屏幕截图或 Computer Use。
   - Longbridge NSH `0.7.0` 桌面工作台（`apps/m3-nsh`）：深黑曜石低焦虑高密度原生界面，提供简报收件箱、全市场雷达、行业研究、能力圈矩阵、公司体检卡（含 Canvas K 线与查理沙盒）、Too Hard 博物馆与决策账本。
