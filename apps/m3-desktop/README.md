# M3 GPUI 芒格价值研究工作台

低焦虑、高密度的 macOS 原生桌面研究工作台，基于 Rust 1.98.1 + GPUI 0.2.2 构建。采用与黑底白字像素风 `M3` 应用图标一脉相承的**深黑曜石精度仪表盘**设计：原生透明沉浸式标题栏（Immersive Titlebar）、左侧半透明毛玻璃边栏（`NSVisualEffectView` Vibrancy）、像素风 `M3` 徽标与 `TTM / FY` 分段控制器。由确定性本地 Rust 引擎从 SEC EDGAR 官方 XBRL 财报与 Yahoo Finance 日级公开行情计算 5–8 年 ROIC、严苛股东盈余（`OCF - CapEx - SBC`）、股本缩减/稀释率、反向隐含增速与击球价，配合 Alan 的跨学科审讯档案与费曼答辩记录使用。

```sh
# 从 apps/m3-desktop 目录构建 M3.app（若应用已打开请先退出）
./build.sh
open artifacts/M3.app
```

CLI 包装脚本 `./m3`（仓库根目录运行 `apps/m3-desktop/m3`）提供面向 Alan 与自动化的结构化 JSON 接口，详见 [Agent 接口](../../docs/agent-interface.md)。

## 四个核心工作区视图

1. **观察池（`⌘1`）**：核心（`core`）与观察（`watch`）公司矩阵，展示现价、击球价、距击球价安全边际 %、5 年 ROIC 均值、严苛 FCF 收益率、3 年股本变化与自动预警状态。点击任意公司直接进入体检卡。
2. **公司体检卡（`⌘2`）**：按 FY / TTM 口径切换展示 4 个芒格核心支柱卡片、现金流/CapEx/SBC 比例图、交互式“查理计算器沙盒”（调节倍数/目标回报/预期增速并保存击球假设）、多年财务与 SHA-256 溯源表，以及 AI 草稿与人类判断分离的审讯档案。
3. **Too Hard 博物馆（`⌘3`）**：集中保存明确拒绝的标的（如 `INTC`、`1810.HK`）及其拒绝原因与真实财务警示，克制“怕错过（FOMO）”情绪。
4. **决策账本（`⌘4`）**：事前冻结记录 `TOO_HARD`、`WAIT`、`PAPER_BUY`、`PAPER_SELL` 决策，长期对照 `100% 现金` 与 `标普 500 (^GSPC)` 机会成本（纯研究模拟夹具，无任何实盘连接）。

## 数据与验证

- `cargo test --locked -j 4` 运行 `edgar`、`calculator`、`store`、`cli`、`alan_interrogator` 与 `ui` 单元测试。
- `alan-*` 依赖在 [Cargo.toml](Cargo.toml) 中固定指向公开仓库 `https://github.com/realmorrisliu/alan.git` 的 `v0.1.4` tag；如需与本地 Alan 源码联调，可通过 `.cargo/config.toml` 或 `[patch."https://github.com/realmorrisliu/alan.git"]` 临时覆写，避免将本机绝对路径提交进仓库。
- 一手原始财报与行情缓存写入仓库 `data/munger/`（Git 忽略）；观察池、审讯档案与模拟账本保存在 `workbench/`（Git 跟踪）。
- 本工作台不包含券商登录、实盘交易密钥或自动下单功能，也不再运行已归档的 `m3-paper` 进程。
