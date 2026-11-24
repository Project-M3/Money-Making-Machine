# M3 GPUI 芒格价值研究工作台

低焦虑、高密度的 macOS 原生桌面研究工作台，基于 Rust 1.98.1 + GPUI 0.2.2 构建。采用与黑底白字像素风 `M3` 应用图标一脉相承的**深黑曜石精度仪表盘**设计：原生透明沉浸式标题栏（Immersive Titlebar）、左侧半透明毛玻璃边栏（`NSVisualEffectView` Vibrancy）、像素风 `M3` 徽标与 `TTM / FY` 分段控制器。由确定性本地 Rust 引擎从 SEC EDGAR 官方 XBRL 财报与 Yahoo Finance 日级公开行情计算 5–8 年 ROIC、严苛股东盈余（`OCF - CapEx - SBC`）、股本缩减/稀释率、反向隐含增速与击球价，配合 Alan 的跨学科审讯档案与费曼答辩记录使用。

```sh
# 本机正常安装到 /Applications/M3.app + ~/.local/bin/m3（从私有 GitHub Release 下载并绑定工作区）
../../scripts/install.sh --channel beta

# 或本地开发构建（若应用已打开请先退出）
./build.sh
open artifacts/M3.app
# 也可将本地构建包直接安装到 /Applications/M3.app
../../scripts/install.sh --local --channel beta
```

### 版本、工作区绑定与双通道自更新

- **工作区定位优先级**：`M3_REPO_ROOT` 环境变量 > 当前工作目录向上查找 `workbench/watchlist.json` > `~/Library/Application Support/M3/config.json`（`m3 workspace set <仓库路径>`）> 开发态编译期路径回退。
- **发布通道（`stable` / `beta`）**：
  - `beta`：每次合并到 `main`（改动应用代码/构建脚本）后由 GitHub Actions 自动构建发布 `beta` 滚动预发布（版本号 `<next>-beta.<YYYYMMDD>.<run>`）；
  - `stable`：推送 `vX.Y.Z` tag（可用 `./scripts/release.sh stable X.Y.Z --push`）触发正式版发布；
  - 若配置了 Apple Developer ID 签名与公证 Secrets（`MACOS_CERT_P12_BASE64`、`MACOS_CERT_P12_PASSWORD`、`APPLE_TEAM_ID`、`ASC_API_KEY_P8`、`ASC_API_KEY_ID`、`ASC_API_ISSUER_ID`），CI 会自动切换为严格签名 + 公证模式；未配置时自动降级为 ad-hoc 签名并如实在 `latest.json` 中标注 `signed: false, notarized: false`。
- **桌面端交互**：启动时低频静默检查更新（`beta` TTL 1h / `stable` TTL 6h）；右上角可点击更新药丸一键下载校验并重启应用，底部状态栏 `vX.Y.Z · channel` 与命令面板 `⌘K` 也提供检查更新与切换通道入口。
- **CLI 命令**：`m3 -V`、`m3 --version`、`m3 version`、`m3 workspace [show|set <PATH>]`、`m3 update [--check] [--channel stable|beta] [--version VER] [--force]`、`m3 update channel [stable|beta]`。

CLI 包装脚本 `./m3`（仓库根目录运行 `apps/m3-desktop/m3`，或安装后的 `m3`）提供面向 Alan 与自动化的结构化 JSON 接口（含 `m3 universe status|sync|screen|peers|similar`、`m3 events catchup|list` 与 `m3 inbox list|triage`），详见 [Agent 接口](../../docs/agent-interface.md)。

## 核心工作区视图

1. **简报收件箱（`⌘1`，默认启动视图）**：L1 事件驱动研报分拣台（`0 Flagship Token`），展示全美股覆盖、语义索引片段数、未读信号与「追平事件流」后台任务；支持按「未读 / 全部 / 新浮现 / 隐空间传导 / 击球与红灯 / 已归档」过滤，右侧呈现**一手原文出处证据卡片**与 Linear 风格快捷键分拣（`j`/`k` 移动、`↵` 打开档案、`w` 入观察池、`x` Too Hard、`e` 归档、`u` 标为未读、`d` 交由 Alan 审讯、`r` 追平事件流）。
2. **全市场雷达（`⌘2` / `u`）**：基于嵌入式 DuckDB + ZSTD Parquet（`data/munger/lake/`）的 L0/L1 全美股筛选台，支持 SEC `companyfacts.zip` 批量解析、SIC 6000–6799 金融业过滤、价格方案 1（Top 400 质量候选池 + 观察池日级报价）、**ROIC × 严苛 FCF 收益率**二维散点图、**混合语义相似搜索（Lookalike）** 与详情抽屉「找相似公司」按钮、5 年下钻与同行分位数对照，以及一键加入观察池（`WATCH`）。
3. **篮子总览（`⌘3`）**：核心（`IN`）、待审讯（`WATCH`）、否决（`OUT`）与能力圈外 / **Too Hard 博物馆**（`TOO HARD`，支持按 `4` 快速聚焦拒绝标的及警示原因）四篮子矩阵与看板视图，展示现价、击球价、距击球价安全边际 %、5 年 ROIC 均值、严苛 FCF 收益率、3 年股本变化与自动预警状态。点击任意公司直接进入体检卡。
4. **决策账本（`⌘4`）**：事前冻结记录 `TOO_HARD`、`WAIT`、`PAPER_BUY`、`PAPER_SELL` 决策，长期对照 `100% 现金` 与 `标普 500 (^GSPC)` 机会成本（纯研究模拟夹具，无任何实盘连接）。
5. **公司体检卡 & Alan 审讯台（点击标的 / `⌘K`）**：按 FY / TTM 口径切换展示 4 个芒格核心支柱卡片、现金流/CapEx/SBC 比例图、交互式“查理计算器沙盒”、多年财务与 SHA-256 溯源表，以及进程内 `EmbeddedAlan` 跨学科审讯工作台（保留 AI 草稿与人类主权 `human_judgment` 严格分离）。

## 数据与验证

- `cargo test --locked -j 4` 运行 `edgar`、`calculator`、`store`、`lake`、`universe`、`cli`、`alan_interrogator` 与 `ui` 单元测试。
- `alan-*` 依赖在 [Cargo.toml](Cargo.toml) 中固定指向公开仓库 `https://github.com/realmorrisliu/alan.git` 的 `v0.1.4` tag；如需与本地 Alan 源码联调，可通过 `.cargo/config.toml` 或 `[patch."https://github.com/realmorrisliu/alan.git"]` 临时覆写，避免将本机绝对路径提交进仓库。
- 一手原始财报、行情缓存与 L0 全美股 DuckDB/Parquet 湖仓写入仓库 `data/munger/`（Git 忽略）；观察池、审讯档案与模拟账本保存在 `workbench/`（Git 跟踪）。
- 本工作台不包含券商登录、实盘交易密钥或自动下单功能，也不再运行已归档的 `m3-paper` 进程。
