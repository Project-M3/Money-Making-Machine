# Money-Making-Machine（Project M3）

> 官网：<https://m3.thinksmart.systems> · 开源协议：[Apache-2.0](LICENSE) · 最新稳定版：`v0.3.0`

一个面向个人长期价值投资的**人机混动芒格研究与决策工作台**（Munger Focus Value Research & Decision Workbench）。

核心目标：依托自有长期资金、个人真实能力圈与“坐在屁股上等待”的低焦虑节奏，以全美股 DuckDB/Parquet 确定性湖仓扩大观察边界，以事件流与隐空间相似发现替代无效盯盘，以可核验的一手财报算术和 `EmbeddedAlan` 跨学科逆向审讯辅助思考——并把全部决定权严格保留在人类手中。

---

## 为什么从短期量化转向芒格价值研究？（实证归档）

2026 年 10 月本仓库曾按严格的事前登记与独立审计流程验证两条小资金短周期/套利路线，均已获得明确的可执行否定结论并完整归档（保留于 `spikes/nautilus-trend/` 与 `docs/research/`）：

- **小时级趋势交易被否定**（[sma20-history-review.md](docs/research/sma20-history-review.md)）：BTC/USDT 1h SMA20 在 2025 留出期产生 525 轮买卖，标准成本下净亏 16.21%、最大回撤 16.80%，全面劣于同仓位买入持有。
- **小资金资金费率对冲不经济**（[funding-carry-review.md](docs/research/funding-carry-review.md)）：1 万元本金保守成本下 2025 全年净收益约 76 元，双倍综合成本下转负，且存在独立合约钱包资金约束。
- **24/7 常驻行情采集与个人生活方式错配**（[paper-soak-review.md](docs/research/paper-soak-review.md)）：笔记本休眠直接造成 11 小时采集缺口。

2026-11-19，用户确认转向**查理·芒格集中价值投资范式**：季报级/事件级低频体检、能力圈 15–20 家深度跟踪、预设击球价、只用自有资金（无赎回压力与季度排名考核），彻底匹配每周 6–10 小时投入、无需盯盘的生活方式。

---

## 当前方向与约束（2026-11-19 确认）

- **全 Rust 栈 + Longbridge NSH `0.7.0`**：沿用 Rust 1.98.1 + Longbridge NSH `0.7.0`（`apps/m3-nsh`）构建 macOS 原生桌面工作台，并将确定性内核（`crates/m3-core`，macOS 主力 + iOS-ready 路径抽象，零 UI 框架依赖）与 JSON CLI（`crates/m3-cli` / `m3`）拆分为清晰边界（[apps/m3-nsh/README.md](apps/m3-nsh/README.md)）。
- **算术与推理严格分离**：所有财务指标（ROIC、严苛股东盈余 `OCF - CapEx - SBC`、股本缩减/稀释率、反向隐含增速、击球价）必须由确定性 Rust 代码直接从一手官方数据源（美股首选 SEC EDGAR Company Facts XBRL）计算并附 SHA-256 溯源；**绝不让 LLM 估算或编造财报数字**。
- **第一版范围**：先做 **15–20 家能力圈深度观察台 + 查理计算器 + Alan 逆向审讯**，把全市场 5000 股筛选器留待二期。
- **首批能力圈观察池**：美股（SEC EDGAR 一手数据）先行覆盖 `AAPL`、`GOOGL`、`ADSK`、`PTC`、`SNPS`、`COST` 与 `Too Hard` 样本 `INTC`；非美股 `DSY.PA`（达索，欧元实时报价+待录 IFRS 财报）与 `1810.HK`（小米，`Too Hard` 样本）纳入统一结构。
- **预算与研究边界**：首轮三个月、人民币 10,000 元总预算定位为研发与数据源费用上限；**首轮纯研究 + 模拟决策账本（`PAPER RESEARCH FIXTURE`），不开户、不入金、不执行真实下单**。
- **费曼学习与人类主权**：Alan 只提供证据、跨学科审讯与反方压力测试，禁止替用户输出“买入/卖出建议”；`human_judgment`（论点权重、证伪指标、击球价拍板、费曼复述）必须由用户本人完成。

---

## 快速安装与双通道自动更新（macOS）

```sh
# 1. 一键安装最新公开正式版（写入 /Applications/M3.app，建立 ~/.local/bin/m3 软链）
curl -fsSL https://m3.thinksmart.systems/install.sh | bash

# 或安装跟随 main 合并自动构建的 rolling beta（支持传入 --channel beta）
curl -fsSL https://m3.thinksmart.systems/install.sh | bash -s -- --channel beta

# 若在本地仓库内执行，也可直接运行脚本自动绑定当前工作区：
./scripts/install.sh --channel stable

# 2. 日常切换通道与自更新（命令行 & M3 NSH 桌面端均支持）
m3 update channel beta     # 切到 beta（或 stable）
m3 update --check          # 检查是否有新版本
m3 update                  # 下载校验 SHA-256 并原子替换 /Applications/M3.app
```

- 持久化工作区与通道状态保存在 `~/Library/Application Support/M3/`（`config.json` 与 `updater/`），从 Finder 打开 `/Applications/M3.app` 也可直接定位 `workbench/` 与 `data/munger/`。

---

## 文档入口

- [NSH 桌面工作台与 CLI](apps/m3-nsh/README.md)：Munger Workbench 桌面界面（Inbox、Universe、Industry Research、Circle Overview、Themes & Journal、Research Desk）与 `m3` JSON CLI 说明
- [Agent 数据与控制接口](docs/agent-interface.md)：结构化命令（`status / sync / report / dossier / add / journal / universe / events / inbox / research / budget / themes`）与文件契约
- [芒格人机审讯工作流](docs/munger-workflow.md)：能力圈过滤、跨学科格栅、事前验尸与费曼答辩协议
- [技术架构方向](docs/architecture.md)：四层架构、`workbench/` 与 `data/munger/` 目录边界
- [v2 演进架构与里程碑设计](docs/v2-architecture.md)：全美股 DuckDB/Parquet 湖仓、事件驱动流、隐空间综合与 Alan Deep Research 蓝图（设计阶段）
- [AGENTS.md](AGENTS.md)：工程纪律、算术与 LLM 隔离、研究与安全边界
- [决策地图](.scratch/quant-validation/map.md)：战略转向记录与归档研究索引
- 历史归档（已冻结）：[SMA20 历史筛选](docs/research/sma20-history-review.md) · [资金费率测算](docs/research/funding-carry-review.md) · [首次 GUI 模拟复盘](docs/research/paper-soak-review.md)
