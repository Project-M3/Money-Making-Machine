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

CLI 包装脚本 `./m3`（仓库根目录运行 `apps/m3-desktop/m3`，或安装后的 `m3`）提供面向 Alan 与自动化的结构化 JSON 接口（含 `m3 universe status|sync|screen|peers|similar`、`m3 events catchup|list`、`m3 inbox list|triage`、`m3 research run|status`、`m3 budget status` 与 `m3 themes list|sync`），详见 [Agent 接口](../../docs/agent-interface.md)。

## 核心工作区视图

1. **简报收件箱（`⌘1`，默认启动视图）**：L1 事件驱动研报分拣台（`0 Flagship Token`），展示全美股覆盖、语义索引片段数、未读信号、**本月 AI 预算水位（`$X.XX / $10.00`）** 与「追平事件流」后台任务；支持按「未读 / 全部 / 新浮现 / 隐空间传导 / 击球与红灯 / 已归档」过滤，右侧呈现**一手原文出处证据卡片**与 Linear 风格快捷键分拣（`j`/`k` 移动、`↵` 打开档案、`w` 入观察池、`x` Too Hard、`e` 归档、`u` 标为未读、`d` 入观察池并派 Alan 跑 Deep Research、`r` 追平事件流）。
2. **全市场雷达（`⌘2` / `u`）**：基于嵌入式 DuckDB + ZSTD Parquet（`data/munger/lake/`）的 L0/L1 全美股筛选台，支持 SEC `companyfacts.zip` 批量解析、SIC 6000–6799 金融业过滤、价格方案 1（Top 400 质量候选池 + 观察池日级报价）、**ROIC × 严苛 FCF 收益率**二维散点图、**混合语义相似搜索（Lookalike）** 与详情抽屉「找相似公司」、5 年下钻与同行分位数对照，以及一键入观察池或「入池并派 Alan 深研」。
3. **篮子总览（`⌘3`）**：核心（`IN`）、待审讯（`WATCH`）、否决（`OUT`）与能力圈外 / **Too Hard 博物馆**（`TOO HARD`，支持按 `4` 快速聚焦拒绝标的及警示原因）四篮子矩阵与看板视图，展示现价、击球价、距击球价安全边际 %、5 年 ROIC 均值、严苛 FCF 收益率、3 年股本变化与自动预警状态。点击任意公司直接进入 Research Desk。
4. **主题与账本（`⌘4`）**：上半区展示基于 `workbench/themes.json` 的商业原型主题图谱（支持 `s` /「同步主题图谱」刷新并保留人类 `user_notes`，点击标的胶囊直达 Research Desk / 全市场雷达）；下半区延续事前冻结模拟决策账本（`TOO_HARD` / `WAIT` / `PAPER_BUY` / `PAPER_SELL`，长期对照 `100% 现金` 与 `^GSPC` 机会成本）。
5. **Research Desk（`1 财报与活研报` / `2 Deep Research & Alan` / `3 XBRL 底表`）**：在保留确定性 K 线、查理沙盒、ROIC/现金流支柱、XBRL 溯源与人类主权 `human_judgment` 的同时，集成 **Living Research Memo（`workbench/memos/<TICKER>.md`）**、跨公司同行分位与 Lookalike 对照条、最近 `research_runs` 审计记录、受预算硬门控的 **「Deep Research · 4 视角并行」**（`护城河` / `会计排雷` / `隐空间同行` / `5年逆向验尸`）实时 Turing Tape 流式执行，以及交互式 Alan 费曼审讯对话台；命令面板 `⌘K` 同步提供 `Deep Research`、`Budget Status` 与 `Sync Themes` 快捷入口。

## 数据与验证

- `cargo test --locked -j 4` 运行 `edgar`、`calculator`、`store`、`lake`、`lake_l1`、`lake_l2`、`budget`、`research`、`universe`、`cli`、`alan_interrogator` 与 `ui` 单元测试。
- `alan-*` 依赖在 [Cargo.toml](Cargo.toml) 中固定指向公开仓库 `https://github.com/realmorrisliu/alan.git` 的 `v0.1.4` tag；如需与本地 Alan 源码联调，可通过 `.cargo/config.toml` 或 `[patch."https://github.com/realmorrisliu/alan.git"]` 临时覆写，避免将本机绝对路径提交进仓库。
- 一手原始财报、行情缓存与 L0 全美股 DuckDB/Parquet 湖仓写入仓库 `data/munger/`（Git 忽略）；观察池、审讯档案与模拟账本保存在 `workbench/`（Git 跟踪）。
- 本工作台不包含券商登录、实盘交易密钥或自动下单功能，也不再运行已归档的 `m3-paper` 进程。
