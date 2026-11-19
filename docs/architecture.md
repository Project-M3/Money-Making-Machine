# 技术架构方向（Munger Workbench v1）

## 1. 核心原则

- **全 Rust 栈 + GPUI 0.2.2**：单一二进制 `m3-desktop` 同时承载桌面 GPUI 工作台与结构化 JSON CLI（通过 `apps/m3-desktop/m3` 入口脚本调用）。保持最小实现单元，不预建多 crate 平台或后台常驻守护进程（不需要 24/7 运行，也不安装 `launchd`）。
- **算术与定性彻底分层**：财务算术全部在本地 Rust 确定性完成并保留 SHA-256 源校验；LLM / Alan 只在档案层工作，协助能力圈与跨学科逆向思考，绝不充当财务数据来源。
- **低焦虑按需同步**：数据按季报（10-K / 10-Q）和每日一次（或手动 `⌘R` / `m3 sync`）节奏更新，无实时逐笔行情、无闪烁 K 线图、无自动交易接口。

---

## 2. 四层架构设计

| 层级 | 模块 / 载体 | 核心职责 | 严格边界 |
| :--- | :--- | :--- | :--- |
| **1. 一手事实数据层** | `src/edgar.rs` · `src/market.rs` | 拉取 SEC EDGAR Company Facts XBRL（带合规 `User-Agent` 与节流）及 Yahoo Finance 公开日级股价（含基准 `^GSPC`）；缓存原始 JSON 与 SHA-256。 | 不伪造缺失财报；非美股在未录入 `workbench/manual_financials/` 前明确标记 `NoFinancials`；网络失败保留旧缓存。 |
| **2. 确定性查理计算器** | `src/calculator.rs` | 使用 `rust_decimal` 计算 `ROIC`（5Y 均值与标准差）、毛利率稳定性、严苛股东盈余（`OCF - CapEx - SBC`）、3Y/5Y 净股本变化、净负债/盈余比、FY 与 TTM 双口径估值、击球价、反向隐含增速及自动预警灯（如 `CapEx > OCF`、`SBC > 25% OCF`）。 | 纯函数、零 I/O、零 LLM；`invested_capital <= 0` 时不强行除零，诚实给出总资本回报率辅助列并标明原因。 |
| **3. 人机混动审讯档案** | `workbench/dossiers/*.json` + Alan | 持久化三句话生意理解、跨学科格栅（微观经济/生态位/心理学与激励/资本配置）、5 年事前验尸与开放质疑；显式区分 `agent_draft` 与 `human_judgment`。 | AI 禁止填写 `human_judgment`、禁止给出“买入/卖出建议”；由用户通过费曼复述完成最终判词。 |
| **4. 反焦虑 GPUI 工作台** | `src/ui.rs` + `src/cli.rs` | 人类通过 GPUI 四个主视图查看观察池、公司体检卡（含交互式倍/回报/增速沙盒）、Too Hard 博物馆与决策账本；Alan 通过 `m3` JSON CLI 读写事实并更新档案草稿。 | 无交易密钥、无券商下单接口、不以截图操作代替结构化文件。 |

---

## 3. 目录与数据所有权划分

```text
Money-Making-Machine/
├── apps/m3-desktop/           # Rust + GPUI 0.2.2 单体工作台与 CLI
│   ├── src/
│   │   ├── main.rs            # 应用入口、CLI 路由与 GPUI 窗口初始化
│   │   ├── model.rs           # 域模型与十进制序列化契约
│   │   ├── edgar.rs           # SEC EDGAR XBRL 提取与 10-Q TTM 拼接
│   │   ├── market.rs          # 低频公开股价抓取（Yahoo v8 chart）
│   │   ├── calculator.rs      # 确定性查理计算器与预警引擎
│   │   ├── store.rs           # 原子持久化与全量同步协调
│   │   ├── cli.rs             # 结构化 JSON CLI 处理
│   │   └── ui.rs              # 4 视图暖纸桌面工作台与交互沙盒
│   ├── build.sh               # 构建并打包 M3.app（不再构建 m3-paper）
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
