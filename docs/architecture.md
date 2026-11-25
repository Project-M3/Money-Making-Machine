# 技术架构方向（Munger Workbench）

## 1. 核心原则

- **全 Rust 栈 + Longbridge NSH `0.7.0`**：工作区拆分为确定性核心 `crates/m3-core`（macOS 主力 + iOS-ready `M3Paths` 抽象，零 UI 框架依赖）、独立 JSON CLI `crates/m3-cli`（`m3`）与 Longbridge NSH `0.7.0` 原生桌面壳 `apps/m3-nsh`（双模入口同时兼容 `m3-cli` 参数；桌面系统桥接隔离于 `platform_desktop.rs`）。不预建多 crate 云端微服务或后台常驻守护进程（不需要 24/7 运行，也不安装 `launchd`）。
- **算术与定性彻底分层**：财务算术全部在本地 Rust 确定性完成并保留 SHA-256 源校验；LLM / Alan 只在档案层工作，协助能力圈与跨学科逆向思考，绝不充当财务数据来源。
- **低焦虑按需同步**：数据按季报（10-K / 10-Q）和每日一次（或手动 `⌘R` / `m3 sync`）节奏更新，无实时逐笔行情、无闪烁交易终端、无自动交易接口。

---

## 2. 四层架构设计

| 层级 | 模块 / 载体 | 核心职责 | 严格边界 |
| :--- | :--- | :--- | :--- |
| **1. 一手事实数据层** | `crates/m3-core/src/{edgar,market}.rs` | 拉取 SEC EDGAR Company Facts XBRL（带合规 `User-Agent` 与节流）及 Yahoo Finance 公开日级股价（含基准 `^GSPC`）；缓存原始 JSON 与 SHA-256。 | 不伪造缺失财报；非美股在未录入 `workbench/manual_financials/` 前明确标记 `NoFinancials`；网络失败保留旧缓存。 |
| **2. 确定性查理计算器** | `crates/m3-core/src/calculator.rs` | 使用 `rust_decimal` 计算 `ROIC`（5Y 均值与标准差）、毛利率稳定性、严苛股东盈余（`OCF - CapEx - SBC`）、3Y/5Y 净股本变化、净负债/盈余比、FY 与 TTM 双口径估值、击球价、反向隐含增速及自动预警灯（如 `CapEx > OCF`、`SBC > 25% OCF`）。 | 纯函数、零 I/O、零 LLM；`invested_capital <= 0` 时不强行除零，诚实给出总资本回报率辅助列并标明原因。 |
| **3. 人机混动审讯档案** | `workbench/dossiers/*.json` + Alan | 持久化三句话生意理解、跨学科格栅（微观经济/生态位/心理学与激励/资本配置）、5 年事前验尸与开放质疑；显式区分 `agent_draft` 与 `human_judgment`。 | AI 禁止填写 `human_judgment`、禁止给出“买入/卖出建议”；由用户通过费曼复述完成最终判词。 |
| **4. 反焦虑 NSH 桌面工作台与 CLI** | `apps/m3-nsh/src/` + `crates/m3-cli/src/` | 人类通过 NSH `0.7.0` 原生桌面工作台查看简报收件箱、全市场雷达、行业研究、能力圈矩阵、公司体检卡（含 Canvas K 线与查理沙盒）、Too Hard 博物馆与决策账本；Alan 通过 `m3` JSON CLI 读写事实并更新档案草稿。 | 无交易密钥、无券商下单接口、不以截图操作代替结构化文件。 |

---

## 3. 目录与数据所有权划分

```text
Money-Making-Machine/
├── crates/m3-core/            # 确定性内核（macOS 主力 + iOS-ready，零 UI 框架依赖）
│   └── src/
│       ├── model.rs           # 域模型与十进制序列化契约
│       ├── edgar.rs           # SEC EDGAR XBRL 提取与 10-Q TTM 拼接
│       ├── market.rs          # 低频公开股价抓取（Yahoo v8 chart）
│       ├── calculator.rs      # 确定性查理计算器与预警引擎
│       ├── store.rs           # 原子持久化与全量同步协调
│       ├── lake*.rs           # L0/L1/L2 DuckDB + ZSTD Parquet 湖仓
│       └── alan_interrogator.rs # 内置 Alan 审讯台与红线策略钩子
├── crates/m3-cli/             # 独立结构化 JSON CLI（`m3` 二进制）
├── apps/m3-nsh/               # Rust + Longbridge NSH 0.7.0 原生桌面工作台
│   ├── src/
│   │   ├── main.rs            # 应用入口、CLI 路由与 NSH App 启动
│   │   ├── platform_desktop.rs # macOS 菜单栏 / 文件对话框 / 剪贴板 / 重启桥接
│   │   ├── stores/            # 响应式 Signal 状态仓库与 Tokio 桥接
│   │   └── views/             # Inbox / Universe / Industry / Baskets / Journal / Company / Settings
│   ├── build.sh               # 本地打包 M3.app（含 m3-nsh + m3 CLI）
│   └── m3                     # CLI 包装脚本
├── workbench/                 # [Git 跟踪] 人类与 Alan 协作的长期研究资产
│   ├── watchlist.json         # 核心观察池 / 观察名单 / Too Hard 分桶与估值假设
│   ├── journal.json           # 事前冻结决策账本（对比现金与标普 500）
│   ├── manual_financials/     # 可选：非美股手工核验财报 JSON
│   └── dossiers/<TICKER>.json # 单公司跨学科审讯档案与人类判断卡
├── data/munger/               # [Git 忽略] 原始与派生运行时数据
│   ├── raw/sec/               # SEC CIK 映射与原始 Company Facts JSON
│   ├── raw/prices/            # 股价与 ^GSPC 最新快照
│   ├── computed/<TICKER>.json # Rust 计算器输出的 MungerReport
│   └── sync-status.json       # 最近一次同步状态与逐标的诊断
└── spikes/nautilus-trend/     # [历史归档] 2026-10 短周期量化验证原型与否定证据
```
