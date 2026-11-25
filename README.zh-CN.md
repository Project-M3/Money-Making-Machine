<p align="right">
  <a href="./README.md">English</a> | <strong>简体中文</strong>
</p>

# Project M3 — Money · Making · Machine

**像查理·芒格一样投资** —— *学习市场、研究行业、挑选投资标的，并逐步演进至 Agent 与程序化交易，把你的价值投资策略转化为真正的收益。*

[![CI](https://github.com/Project-M3/Money-Making-Machine/actions/workflows/ci.yml/badge.svg)](https://github.com/Project-M3/Money-Making-Machine/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/Project-M3/Money-Making-Machine?label=release)](https://github.com/Project-M3/Money-Making-Machine/releases/latest)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Platform: macOS Apple Silicon](https://img.shields.io/badge/platform-macOS%20Apple%20Silicon-black.svg)](https://github.com/Project-M3/Money-Making-Machine/releases)
[![Website](https://img.shields.io/badge/website-m3.thinksmart.systems-10b981.svg)](https://m3.thinksmart.systems)

Project M3 把查理·芒格的投资纪律工程化为本地优先工具：基于一手 SEC EDGAR XBRL 财报的确定性 Rust 会计核算、内置 **Alan** 跨学科逆向审讯，以及分阶段演进的自主 Agent 与受控程序化执行闭环。

> **当前阶段说明（`v0.4.x`）**：目前 M3 仍专注于**本地投研、全市场湖仓与模拟决策账本（`PAPER RESEARCH`）**，尚未接入实盘券商接口。后续 `v0.5 → v1.0` 如何逐步补齐开箱 BYOK、`m3 mcp`、可编程价值策略引擎、内置模拟盘与受确定性 Rust `RiskEngine` 保护的券商执行，详见 [ROADMAP.md](./ROADMAP.md)。

---

## 核心能力

- **确定性查理计算器（`m3-core`）**：基于 `rust_decimal` 定点十进制算术计算多年 ROIC、严苛股东盈余（`OCF - CapEx - SBC`）、3 年股本变化率、反向隐含增速、击球价与自动财务排雷预警，杜绝浮点漂移与 LLM 幻觉。
- **三层本地优先研究湖仓**：
  - **L0 全美股湖仓**：批量解析 SEC EDGAR `companyfacts.zip` 与 `submissions.zip`，沉淀为不可变 ZSTD Parquet 代际目录与嵌入式 `m3.duckdb`，支持毫秒级多维筛选与同 SIC 同行分位对照。
  - **L1 事件简报收件箱**：零旗舰 Token 消耗监控 `8-K`、`10-K`、`10-Q` 与新闻 RSS，并通过本地 256 维语义/BM25 混合索引生成带一手原文摘录的 `Discovery`、`Ripple`、`Strike`、`Red Flag` 卡片。
  - **L2 Alan 深度研究**：由内置 Alan 执行的 4 视角并行深研（护城河、会计排雷、隐空间同行、5 年事前验尸），受月度 USD / Token 硬上限与周度配额保护，并永久保护人类手写栏位（`human_judgment` 与 `## Human Notes`）不被覆盖。
- **macOS 原生桌面工作台（`apps/m3-nsh`）**：基于 Rust 与 Longbridge NSH `0.7.0` 构建，支持中英双语即时切换、能力圈门控击球区（`Core / Watch / Out / Too Hard`）、行业研究工作台（Industry Research Studio）、Canvas 交互式 K 线与 ROIC × FCF 散点图、`⌘K` 命令面板、macOS 风格存储管理以及带实时进度条的 SHA-256 原子自更新。
- **Agent 原生 JSON CLI（`m3`）**：完整覆盖工作台、湖仓、存储清理与自更新能力；交互终端下自动格式化缩进 JSON，管道模式下输出单行紧凑 schema v1 JSON（`--pretty` / `--compact` 可随时显式指定）。

---

## 快速安装（macOS Apple Silicon）

安装 `/Applications/M3.app` 并自动建立 `~/.local/bin/m3` CLI 软链：

```bash
# 安装最新稳定版（stable）
curl -fsSL https://m3.thinksmart.systems/install.sh | bash

# 或安装跟随 main 合并自动构建的 rolling beta
curl -fsSL https://m3.thinksmart.systems/install.sh | bash -s -- --channel beta
```

也可直接从 [GitHub Releases](https://github.com/Project-M3/Money-Making-Machine/releases) 下载预编译安装包与校验清单。

---

## CLI 快速上手

```bash
# 查看分组帮助与子命令用法
m3 --help
m3 help universe

# 一键体检二进制、工作区、DuckDB 湖仓与存储占用
m3 doctor

# 初始化或增量同步 L0 全美股 DuckDB/Parquet 湖仓
m3 universe sync --full --prices-top 400
m3 universe screen --min-roic 15 --max-sbc-ocf 25 --limit 25

# 查看 L1 简报收件箱并向能力圈添加公司
m3 inbox list --status unread --limit 20
m3 add MCO --bucket core --circle "Credit Ratings & Analytics"
m3 status

# 查看本地存储分布并安全清理可重建缓存（绝不删除个人研报与观察池）
m3 storage status
m3 storage clean sec_bulk_archives

# 检查更新与切换发布通道
m3 update --check
```

---

## 仓库结构

| 路径 | 说明 |
| :--- | :--- |
| [`crates/m3-core`](./crates/m3-core) | 零 UI 确定性核心：SEC EDGAR 同步、`rust_decimal` 查理计算器、DuckDB/Parquet 湖仓（`L0`/`L1`/`L2`）、行业研究、存储管理、中英双语 i18n 与自更新引擎 |
| [`crates/m3-cli`](./crates/m3-cli) | 结构化 schema v1 JSON 命令行工具（`m3`），含分组帮助、`doctor`、`storage`、`industry` 与 `stderr` 更新进度条 |
| [`apps/m3-nsh`](./apps/m3-nsh) | 基于 Longbridge NSH `0.7.0` 的 macOS 原生桌面端，内置打包 `m3` CLI |
| [`website`](./website) | 基于 Vite + TypeScript 的官网与交互式工作台演示（`m3.thinksmart.systems`） |
| [`docs`](./docs) | 架构设计、芒格人机审讯工作流与 Agent/CLI 数据契约 |
| [`scripts`](./scripts) | macOS 应用组装、一键安装脚本（`install.sh`）、发版与清单生成脚本 |

---

## 本地开发

需要 macOS Apple Silicon、`Rust 1.98.1`（含 `rustfmt` / `clippy`）以及 `Node.js` + `pnpm`（用于 `website/`）：

```bash
# 格式检查与零警告 Clippy 检查（CI 同时强制要求生产代码零 .unwrap()/.expect()）
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings

# 运行全部工作区单元测试
cargo test --workspace --locked -j 4

# 构建本地 macOS 应用包（apps/m3-nsh/artifacts/M3.app）
./apps/m3-nsh/build.sh

# 构建官网静态资源
pnpm --dir website install && pnpm --dir website build
```

完整协作规范与发版检查请阅读 [CONTRIBUTING.md](./CONTRIBUTING.md)。

---

## 演进路线图（`v0.4.x → v1.0`）

| 里程碑 | 核心目标 | 状态 |
| :--- | :--- | :--- |
| `v0.4.x` | 确定性查理计算器、6,000+ 全美股 DuckDB/Parquet 湖仓、L1 收件箱、行业研究工作台、NSH 0.7 macOS 应用与 `m3` CLI | **已交付** |
| `v0.5` | App 内置零门槛 BYOK 模型配置、新用户引导与可选经典样本、后台渐进同步 L0、`m3 mcp` 与 Agent Skill | **下一步** |
| `v0.6` | 事件驱动主动研判 Agent、证伪监控、交互式费曼追问教练、港股数据与多市场扩展 | **规划中** |
| `v0.7` | 可编程价值策略引擎、基于湖仓的防前视历史回测、统一 `BrokerAdapter` 与零门槛内置 `Paper Broker` | **规划中** |
| `v0.8 – v1.0` | 硬编码 Rust `RiskEngine` 风控闸门、**长桥 OpenAPI**（美/港）+ **Alpaca**（美股）适配与授权资金池内自主组合执行 | **规划中** |

完整架构设计与不可动摇的安全不变量详见 [ROADMAP.md](./ROADMAP.md)。

---

## 文档导航

- [产品与架构演进路线图（`v0.5 → v1.0`）](./ROADMAP.md)
- [NSH 桌面工作台说明](./apps/m3-nsh/README.md)
- [Agent 与 CLI 数据契约](./docs/agent-interface.md)
- [系统架构说明](./docs/architecture.md)
- [V2 湖仓与事件流设计（已交付基线）](./docs/v2-architecture.md)
- [芒格人机审讯工作流](./docs/munger-workflow.md)
- [更新日志](./CHANGELOG.md) · [贡献指南](./CONTRIBUTING.md) · [安全策略](./SECURITY.md)

---

## 免责声明

当前版本（`v0.4.x`）仅作为本地研究、财报核验与模拟决策记录工具（`PAPER RESEARCH`），**不连接任何券商账户、不保存交易密钥、不执行实盘下单**。规划于 `v0.8+` 上线的程序化执行与自主组合能力仅在用户显式启用与授权后生效，并始终受确定性 Rust 风控引擎约束；软件任何输出均不构成投资建议。

## 开源协议

基于 [Apache License, Version 2.0](./LICENSE) 开源。
