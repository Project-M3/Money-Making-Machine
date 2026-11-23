# M3 GPUI 芒格价值研究工作台

低焦虑、高密度的 macOS 原生桌面研究工作台，基于 Rust 1.98.1 + GPUI 0.2.2 构建。采用与黑底白字像素风 `M3` 应用图标一脉相承的**深黑曜石精度仪表盘**设计：原生透明沉浸式标题栏（Immersive Titlebar）、左侧半透明毛玻璃边栏（`NSVisualEffectView` Vibrancy）、像素风 `M3` 徽标与 `TTM / FY` 分段控制器。由确定性本地 Rust 引擎从 SEC EDGAR 官方 XBRL 财报与 Yahoo Finance 日级公开行情计算 5–8 年 ROIC、严苛股东盈余（`OCF - CapEx - SBC`）、股本缩减/稀释率、反向隐含增速与击球价，配合 Alan 的跨学科审讯档案与费曼答辩记录使用。

```sh
# 从 apps/m3-desktop 目录构建 M3.app（若应用已打开请先退出）
./build.sh
open artifacts/M3.app
```

CLI 包装脚本 `./m3`（仓库根目录运行 `apps/m3-desktop/m3`）提供面向 Alan 与自动化的结构化 JSON 接口（含 `m3 universe status|sync|screen|peers` 全美股湖仓子命令），详见 [Agent 接口](../../docs/agent-interface.md)。

## 核心工作区视图

1. **篮子总览（`⌘1`）**：核心（`IN`）、待审讯（`WATCH`）、否决（`OUT`）与能力圈外 / **Too Hard 博物馆**（`TOO HARD`，支持按 `4` 快速聚焦拒绝标的及警示原因）四篮子矩阵与看板视图，展示现价、击球价、距击球价安全边际 %、5 年 ROIC 均值、严苛 FCF 收益率、3 年股本变化与自动预警状态。点击任意公司直接进入体检卡。
2. **财报与 K 线 / 公司体检卡（`⌘2`）**：按 FY / TTM 口径切换展示 4 个芒格核心支柱卡片、现金流/CapEx/SBC 比例图、交互式“查理计算器沙盒”（调节倍数/目标回报/预期增速并保存击球假设）以及多年财务与 SHA-256 溯源表。
3. **Alan 审讯台（`⌘3`）**：进程内 `EmbeddedAlan` 跨学科审讯工作台，实时展示 Turing Tape 与可跳转的 Acme 一手财报/XBRL 证据标签，保留 AI 草稿与人类主权 `human_judgment` 严格分离。
4. **决策账本（`⌘4`）**：事前冻结记录 `TOO_HARD`、`WAIT`、`PAPER_BUY`、`PAPER_SELL` 决策，长期对照 `100% 现金` 与 `标普 500 (^GSPC)` 机会成本（纯研究模拟夹具，无任何实盘连接）。
5. **全市场雷达（`⌘5` / `u`）**：基于嵌入式 DuckDB + ZSTD Parquet（`data/munger/lake/`）的 L0 全美股筛选台，支持 SEC `companyfacts.zip` 批量解析、SIC 6000–6799 金融业过滤、价格方案 1（Top 400 质量候选池 + 观察池日级报价）、**ROIC × 严苛 FCF 收益率**二维散点图、5 年下钻与同行分位数对照，以及一键加入观察池（`WATCH`）。

## 数据与验证

- `cargo test --locked -j 4` 运行 `edgar`、`calculator`、`store`、`lake`、`universe`、`cli`、`alan_interrogator` 与 `ui` 单元测试。
- `alan-*` 依赖在 [Cargo.toml](Cargo.toml) 中固定指向公开仓库 `https://github.com/realmorrisliu/alan.git` 的 `v0.1.4` tag；如需与本地 Alan 源码联调，可通过 `.cargo/config.toml` 或 `[patch."https://github.com/realmorrisliu/alan.git"]` 临时覆写，避免将本机绝对路径提交进仓库。
- 一手原始财报、行情缓存与 L0 全美股 DuckDB/Parquet 湖仓写入仓库 `data/munger/`（Git 忽略）；观察池、审讯档案与模拟账本保存在 `workbench/`（Git 跟踪）。
- 本工作台不包含券商登录、实盘交易密钥或自动下单功能，也不再运行已归档的 `m3-paper` 进程。
