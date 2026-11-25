# AGENTS.md — 协作与工程约束

## 1. 项目阶段与定位

Project M3 当前处于 **芒格集中价值研究工作台（Munger Workbench）** 建设与验证阶段（2026-11-19 从早期短周期量化实验正式转向）。最终交付采用 **全 Rust 栈，桌面界面使用 Longbridge NSH `0.7.0`（`apps/m3-nsh`），确定性内核位于 `crates/m3-core`（macOS 主力 + iOS-ready `M3Paths` 抽象，零 UI 框架依赖），命令行入口位于 `crates/m3-cli`（`m3`）**。

- 先读 [README.md](README.md)、[CONTEXT.md](CONTEXT.md)、[docs/architecture.md](docs/architecture.md)、[docs/munger-workflow.md](docs/munger-workflow.md) 与 [map.md](.scratch/quant-validation/map.md)。
- 区分已核验的一手财报事实（`data/munger/`）、AI 跨学科草稿（`dossier.draft_source`）、用户确认的判断（`dossier.human_judgment`）与决策账本记录（`workbench/journal.json`）。
- 原量化原型 `spikes/nautilus-trend/` 与 `docs/research/*` 作为**已归档的否定证据**完整保留，不再继续开发、修改或启动 `m3-paper`。
- 首轮探索窗口三个月，人民币 10,000 元总预算现定位为研究、工具与数据源开销上限；现有电脑及已有 Agent 订阅不重复计入。首轮仅做研究与模拟决策账本（`PAPER RESEARCH FIXTURE`），**不开户、不入金、不接入券商下单接口**。

## 2. 算术与 LLM 严格隔离（核心原则）

- **财务算术零 LLM 原则**：营收、毛利、EBIT、OCF、CapEx、SBC、回购、债务、股本、ROIC、严苛股东盈余、隐含增速、击球价等全部数值计算，必须由确定性 Rust 代码使用 `rust_decimal` 直接从一手官方财报数据（如 SEC EDGAR Company Facts XBRL）解析并计算，保留源数据 SHA-256 与 XBRL 标签溯源。**禁止让 LLM 估算、补全或口算财务报表数字。**
- **Alan / Agent 的职责边界**：LLM 仅读取 Rust 生成的结构化 `MungerReport` 与官方原文（10-K 业务描述、财报电话会记录），执行[芒格审讯工作流](docs/munger-workflow.md)（能力圈三句话过滤、跨学科格栅透视、5 年逆向事前验尸、管理层避重就轻检测）。**禁止输出“建议买入/卖出”或替用户给出仓位指令。**
- **人类主权与费曼防线**：`workbench/dossiers/*.json` 中的 `human_judgment`（`my_stance`、`weight_rationale`、`falsification_metrics`、`feynman_checked_at`）只能由用户本人填写或在与用户费曼问答通过后由用户确认写入；**Agent 不得替用户伪造或代填人类判断。**

## 3. 工程质量与最小实现

- 全 Rust 栈 + Longbridge NSH `0.7.0`（`apps/m3-nsh`、`crates/m3-core`、`crates/m3-cli`），复用已有稳定 crate（`rust_decimal`、`serde`、`serde_json`、`sha2`、`reqwest`），保持 `crates/m3-core` 纯净（零 `nsh` 依赖、iOS-ready 路径抽象）、桌面平台桥接隔离在 `apps/m3-nsh/src/platform_desktop.rs`。
- 所有金额、价格、股本、比率及估值计算使用 `rust_decimal::Decimal`；若在统计辅助中出现精度/舍入边界，必须显式文档化。
- 执行链路**不得依赖 `unwrap()` / `expect()`**；网络请求（SEC EDGAR / Yahoo Finance）必须设置明确超时、Fair Access `User-Agent` 与请求节流，失败时保留既有本地缓存并输出可诊断错误，不得写入空值覆盖历史快照。
- 文件写入采用“写临时文件 + 原子重命名”，确保损坏风险最小化。
- 新增非平凡解析与计算逻辑必须附带单元测试，关键财报计算须经独立脚本（如 Python `Decimal`）与一手原始 XBRL 数据核对一致。

## 4. 数据合规、密钥与真实交易边界

- 密钥不得进入源码、日志或版本库；保持 `.gitignore` 对 `data/`、`.env`、密钥和日志的隔离。
- 遵守外部数据源访问条款与 Fair Access 频率限制（SEC EDGAR 请求须带合规 `User-Agent` 并限制并发与频率；遇到 403/429/451 不绕过限制，如实诊断报错）。
- 当前授权**绝对不包括**券商开户、入金、持有真实交易密钥或发送真实买卖指令；任何实盘相关扩展必须先经用户明确确认。

## 5. 本地决策跟踪

- 决策地图保存在 `.scratch/quant-validation/map.md`，子票保存在 `.scratch/quant-validation/issues/`。
- 研究中发现的否定结果、`Too Hard` 拒绝案例与预设决策账本均永久保留，不得为美化记录而事后删改。
