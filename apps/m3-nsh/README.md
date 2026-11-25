# M3 NSH 芒格价值研究工作台

低焦虑、高密度的 macOS 原生桌面研究工作台，基于 Rust 1.98.1 + Longbridge NSH `0.7.0` 构建；确定性内核与 CLI 分别沉淀在 `crates/m3-core`（macOS 主力 + iOS-ready 路径抽象，零 UI 框架依赖）与 `crates/m3-cli`（`m3`）。采用与黑底白字像素风 `M3` 应用图标一脉相承的**深黑曜石精度仪表盘**设计：原生透明沉浸式标题栏（Immersive Titlebar）、左侧边栏、像素风 `M3` 徽标与 `TTM / FY` 分段控制器。由确定性本地 Rust 引擎从 SEC EDGAR 官方 XBRL 财报与 Yahoo Finance 日级公开行情计算 5–8 年 ROIC、严苛股东盈余（`OCF - CapEx - SBC`）、股本缩减/稀释率、反向隐含增速与击球价，配合 Alan 的跨学科审讯档案与费曼答辩记录使用。

```sh
# 本机正常安装到 /Applications/M3.app + ~/.local/bin/m3（从 GitHub Release 下载并绑定工作区）
../../scripts/install.sh --channel beta

# 或本地开发构建（若应用已打开请先退出）
./build.sh
open artifacts/M3.app
# 也可将本地构建包直接安装到 /Applications/M3.app
../../scripts/install.sh --local --channel beta
```

### 版本、工作区绑定与双通道自更新

- **工作区定位优先级**：`M3_REPO_ROOT` 环境变量 > 当前工作目录向上查找 `workbench/watchlist.json` > `~/Library/Application Support/M3/config.json`（`m3 workspace set <仓库路径>`）> 默认空白工作区回退。
- **发布通道（`stable` / `beta`）**：
  - `beta`：每次合并到 `main`（改动应用代码/构建脚本）后由 GitHub Actions 自动构建发布 `beta` 滚动预发布（版本号 `<next>-beta.<YYYYMMDD>.<run>`）；
  - `stable`：推送 `vX.Y.Z` tag（可用 `./scripts/release.sh stable X.Y.Z --push`）触发正式版发布；
  - 若配置了 Apple Developer ID 签名与公证 Secrets（`MACOS_CERT_P12_BASE64`、`MACOS_CERT_P12_PASSWORD`、`APPLE_TEAM_ID`、`ASC_API_KEY_P8`、`ASC_API_KEY_ID`、`ASC_API_ISSUER_ID`），CI 会自动切换为严格签名 + 公证模式；未配置时自动降级为 ad-hoc 签名并如实在 `latest.json` 中标注 `signed: false, notarized: false`。
- **桌面端交互**：启动时低频静默检查更新（`beta` TTL 1h / `stable` TTL 6h）；左侧底部更新胶囊与「设置」页可展示分阶段实时进度条（Resolving / Downloading / Verifying / Extracting / Installing），完成校验后原子替换重启，同时支持命令面板 `⌘K` 检查更新、切换通道与访问 macOS 风格存储管理（Storage Manager）。
- **CLI 命令**：支持 `m3 --help` / `m3 help [cmd]`、交互终端自动美化与 `--pretty` / `--compact` 输出控制，覆盖 `-V`、`--version`、`version`、`doctor`、`storage [status|clean]`、`industry [list|show]`、`workspace [show|set <PATH>]`、`update [--check] [--channel stable|beta] [--version VER] [--force]`（非 `--check` 交互模式下带 `stderr` 实时进度条）及 `update channel [stable|beta]`。

CLI 二进制 `m3`（由 `crates/m3-cli` 构建并内置在 `M3.app/Contents/MacOS/m3`；仓库内也可通过 `apps/m3-nsh/m3` 包装脚本或 `cargo run -p m3-cli --` 调用）提供面向 Alan 与自动化的结构化 JSON 接口（含 `m3 universe status|sync|screen|peers|similar`、`m3 events catchup|list`、`m3 inbox list|triage`、`m3 industry list|show`、`m3 storage status|clean`、`m3 research run|status`、`m3 budget status` 与 `m3 themes list|sync`），详见 [Agent 接口](../../docs/agent-interface.md)。

## 核心工作区视图

1. **简报收件箱（`⌘1`，默认启动视图）**：L1 事件驱动研报分拣台（`0 Flagship Token`），展示全美股覆盖、语义索引片段数、未读信号、**本月 AI 预算水位（`$X.XX / $10.00`）** 与「追平事件流」后台任务；支持按「未读 / 全部 / 新浮现 / 隐空间传导 / 击球与红灯 / 已归档」过滤，右侧呈现**一手原文出处证据卡片**与快捷键分拣。
2. **全市场雷达（`⌘2`）**：基于嵌入式 DuckDB + ZSTD Parquet（`data/munger/lake/`）的 L0/L1 全美股筛选台，支持 SEC `companyfacts.zip` 批量解析、SIC 6000–6799 金融业过滤、价格方案 1（Top 400 质量候选池 + 观察池日级报价）、**ROIC × 严苛 FCF 收益率** Canvas 二维散点图、**混合语义相似搜索（Lookalike）** 与详情抽屉「找相似公司」、5 年下钻与同行分位数对照，以及一键入观察池或「入池并派 Alan 深研」。
3. **行业研究（`⌘3`）**：Industry Research Studio，支持创建或聚焦行业专题（`workbench/industries/<SLUG>.md`）、批量补全 10-K/L1 语义切片、4 视角行业深研与行业 Alan 对话台。
4. **能力圈总览（`⌘4`）**：核心（`CORE`）、持续跟踪（`WATCH`）、能力圈外（`OUT`）与 **Too Hard 博物馆**（`TOO HARD`）四篮子列表与看板视图，展示现价、击球价、距击球价安全边际 %、5 年 ROIC 均值、严苛 FCF 收益率、3 年股本变化与自动预警状态。点击任意公司直接进入公司详情与 Research Desk。
5. **决策账本与主题（`⌘5`）**：上半区展示基于 `workbench/themes.json` 的商业原型主题图谱（支持同步主题图谱并保留人类 `user_notes`）；下半区延续事前冻结模拟决策账本（`TOO_HARD` / `WAIT` / `PAPER_BUY` / `PAPER_SELL`，长期对照 `100% 现金` 与 `^GSPC` 机会成本）。
6. **公司详情 & Research Desk（`⌘6` 财报与活研报 / `⌘7` Research Desk · Alan）**：Canvas 交互式 K 线图、查理沙盒、ROIC/现金流支柱、XBRL 溯源、人类主权 `human_judgment`、**Living Research Memo（`workbench/memos/<TICKER>.md`）**、跨公司同行分位与 Lookalike 对照条、受预算硬门控的 **「Deep Research · 4 视角并行」** 实时 Turing Tape 流式执行，以及交互式 Alan 费曼审讯对话台。

## 数据与验证

- `cargo test --workspace --locked -j 4` 运行 `m3-core`、`m3-cli` 与 `m3-nsh` 全部单元测试。
- `alan-*` 依赖在 [crates/m3-core/Cargo.toml](../../crates/m3-core/Cargo.toml) 中固定指向公开仓库 `https://github.com/realmorrisliu/alan.git` 的 `v0.1.4` tag；如需与本地 Alan 源码联调，可通过 `.cargo/config.toml` 或 `[patch."https://github.com/realmorrisliu/alan.git"]` 临时覆写，避免将本机绝对路径提交进仓库。
- 一手原始财报、行情缓存与 L0 全美股 DuckDB/Parquet 湖仓写入仓库 `data/munger/`（Git 忽略）；观察池、审讯档案与模拟账本保存在 `workbench/`（Git 跟踪）。
- 本工作台不包含券商登录、实盘交易密钥或自动下单功能，也不再运行已归档的 `m3-paper` 进程。
