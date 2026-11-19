use crate::{
    calculator,
    model::{
        AnnualMetricRow, Bucket, DecisionJournal, Dossier, JournalAction, MungerReport, OhlcBar,
        PriceHistorySnapshot, PriceSnapshot, SandboxResult, StrikeZoneStatus, SyncStatusFile,
        ValuationAssumptions, ValuationSnapshot, WarningFlag, WarningSeverity, primary_circle_tag,
        secondary_circle_detail,
    },
    store::{self, WorkspacePaths},
};
use gpui::{
    ClipboardItem, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, SharedString,
    Window, WindowControlArea, bounds, canvas, div, fill, point, prelude::*, px, rgb, rgba, size,
};
use rust_decimal::Decimal;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
    sync::mpsc,
};

// M3 Obsidian Precision Instrument Theme (unified with #0B0B0E / #FFFFFF pixel-art AppIcon)
const BG_SHELL: u32 = 0x0B0B0E;
const BG_VIEWPORT: u32 = 0x101015;
const BG_TOOLBAR: u32 = 0x121218;
const BG_SURFACE: u32 = 0x14141C;
const BG_WELL: u32 = 0x181822;
const BG_ROW_ALT: u32 = 0x121219;
const BG_HOVER: u32 = 0x1D1D28;
const BG_SELECTED_ROW: u32 = 0x1B1B27;
const BG_ACTIVE: u32 = 0x252534;
const BG_STATUSLINE: u32 = 0x0D0D13;
const BG_PALETTE: u32 = 0x13131C;

const RAIL_HOVER: u32 = 0xFFFFFF0D;
const RAIL_ACTIVE: u32 = 0xFFFFFF18;
const RAIL_SELECTED: u32 = 0xFFFFFF14;
const SCRIM_BACKDROP: u32 = 0x050508AA;

const BORDER_VIEWPORT: u32 = 0x1D1D26;
const BORDER_HAIRLINE: u32 = 0x1C1C26;
const BORDER_SUBTLE: u32 = 0x242432;
const BORDER_STRONG: u32 = 0x323244;
const BORDER_PILL: u32 = 0x2D2D3D;
const BORDER_PALETTE: u32 = 0x2E2E40;

const INK_HERO: u32 = 0xFFFFFF;
const INK_PRIMARY: u32 = 0xECEEF2;
const INK_SECONDARY: u32 = 0xA0A6B4;
const INK_MUTED: u32 = 0x6B7282;
const INK_RAIL_PRICE: u32 = 0x858C9C;

const CTA_BG: u32 = 0xF4F4F6;
const CTA_BG_HOVER: u32 = 0xFFFFFF;
const CTA_BG_ACTIVE: u32 = 0xD8D8DF;
const CTA_INK: u32 = 0x09090C;

const EMERALD_FG: u32 = 0x34D399;
const EMERALD_BG: u32 = 0x10B9811E;
const EMERALD_BORDER: u32 = 0x34D39945;
const EMERALD_BAND: u32 = 0x10B98114;
const EMERALD_LINE: u32 = 0x34D399CC;
const EMERALD_VOL: u32 = 0x34D39938;

const GOLD_FG: u32 = 0xFBBF24;
const GOLD_BG: u32 = 0xF59E0B1E;
const GOLD_BORDER: u32 = 0xFBBF2445;

const CORAL_FG: u32 = 0xF87171;
const CORAL_BG: u32 = 0xEF44441E;
const CORAL_BORDER: u32 = 0xF8717145;
const CORAL_VOL: u32 = 0xF8717138;

const ORANGE_FG: u32 = 0xFB923C;
const ORANGE_BG: u32 = 0xF973161E;
const ORANGE_BORDER: u32 = 0xFB923C45;

const SKY_FG: u32 = 0x60A5FA;
const SKY_BG: u32 = 0x3B82F61E;
const SKY_BORDER: u32 = 0x60A5FA45;

const VIOLET_FG: u32 = 0xA78BFA;
const VIOLET_BG: u32 = 0x8B5CF61E;
const VIOLET_BORDER: u32 = 0xA78BFA45;

const SLATE_FG: u32 = 0x94A3B8;
const SLATE_BG: u32 = 0x94A3B818;
const SLATE_BORDER: u32 = 0x94A3B836;

const PIXEL_M3: [&str; 10] = [
    "##....##..#####.",
    "###..###..######",
    "########......##",
    "##.##.##......##",
    "##.##.##....###.",
    "##....##....###.",
    "##....##......##",
    "##....##......##",
    "##....##..######",
    "##....##..#####.",
];

#[derive(Clone)]
pub struct FontRoles {
    pub ui: SharedString,
    pub mono_num: SharedString,
    pub mono_term: SharedString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavView {
    Baskets,
    Dossier,
    Agent,
    Journal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DossierTab {
    Overview,
    Inspector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasketsViewMode {
    List,
    Board,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartTimeframe {
    FiveYearWeekly,
    ThreeYearWeekly,
    OneYearDaily,
}

impl ChartTimeframe {
    fn label(&self) -> &'static str {
        match self {
            ChartTimeframe::FiveYearWeekly => "5Y 周K",
            ChartTimeframe::ThreeYearWeekly => "3Y 周K",
            ChartTimeframe::OneYearDaily => "1Y 日K",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusQuickFilter {
    All,
    StrikeOnly,
    WarnedOnly,
}

impl StatusQuickFilter {
    fn label(&self) -> &'static str {
        match self {
            StatusQuickFilter::All => "全部",
            StatusQuickFilter::StrikeOnly => "击球区",
            StatusQuickFilter::WarnedOnly => "红灯",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortCol {
    Default,
    Ticker,
    Price,
    Margin,
    Roic,
    FcfYield,
    ShareDelta,
    NetDebt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodayPulse {
    pub strike_tickers: Vec<String>,
    pub red_core_tickers: Vec<String>,
    pub watch_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    OpenDossier(String),
    OpenAgent(String),
    SwitchView(NavView),
    FocusBasket(Option<Bucket>),
    ToggleBasketsView,
    MoveCompanyBucket(String, Bucket),
    FilterCircle(Option<String>),
    ToggleBasis,
    ToggleInspector,
    CopyCli(String, String),
    SyncNow,
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub group: String,
    pub title: String,
    pub subtitle: String,
    pub shortcut: Option<&'static str>,
    pub dot_color: u32,
    pub action: PaletteAction,
}

pub struct MungerWorkbench {
    paths: WorkspacePaths,
    fonts: FontRoles,
    focus_handle: FocusHandle,
    reports: Vec<MungerReport>,
    journal: DecisionJournal,
    benchmark: Option<PriceSnapshot>,
    sync_status: Option<SyncStatusFile>,
    benchmark_symbol: String,
    active_view: NavView,
    dossier_tab: DossierTab,
    baskets_view_mode: BasketsViewMode,
    chart_timeframe: ChartTimeframe,
    basket_filter: Option<Bucket>,
    circle_filter: Option<String>,
    status_filter: StatusQuickFilter,
    collapsed_baskets: HashSet<Bucket>,
    sort_col: SortCol,
    sort_asc: bool,
    cursor_index: usize,
    selected_ticker: String,
    selected_dossier: Option<Dossier>,
    selected_history_wk: Option<PriceHistorySnapshot>,
    selected_history_1d: Option<PriceHistorySnapshot>,
    use_ttm_basis: bool,
    sandbox_multiple: Decimal,
    sandbox_target_return: Decimal,
    sandbox_expected_growth: Decimal,
    palette_open: bool,
    palette_query: String,
    palette_cursor: usize,
    syncing: bool,
    sync_rx: Option<mpsc::Receiver<Result<SyncStatusFile, String>>>,
    banner: Option<(String, bool)>,
}

impl MungerWorkbench {
    pub fn new(paths: WorkspacePaths, fonts: FontRoles, cx: &mut Context<Self>) -> Self {
        let mut wb = Self {
            focus_handle: cx.focus_handle(),
            fonts,
            reports: Vec::new(),
            journal: DecisionJournal {
                schema_version: 1,
                account_note: "PAPER RESEARCH FIXTURE".into(),
                currency: "USD".into(),
                starting_cash: Decimal::from(100_000),
                cash_balance: Decimal::from(100_000),
                benchmark_symbol: "^GSPC".into(),
                benchmark_start_price: None,
                positions: Vec::new(),
                entries: Vec::new(),
            },
            benchmark: None,
            sync_status: None,
            benchmark_symbol: "^GSPC".into(),
            active_view: NavView::Baskets,
            dossier_tab: DossierTab::Overview,
            baskets_view_mode: BasketsViewMode::List,
            chart_timeframe: ChartTimeframe::FiveYearWeekly,
            basket_filter: None,
            circle_filter: None,
            status_filter: StatusQuickFilter::All,
            collapsed_baskets: HashSet::from([Bucket::Out, Bucket::TooHard]),
            sort_col: SortCol::Default,
            sort_asc: false,
            cursor_index: 0,
            selected_ticker: "AAPL".into(),
            selected_dossier: None,
            selected_history_wk: None,
            selected_history_1d: None,
            use_ttm_basis: true,
            sandbox_multiple: Decimal::from(18),
            sandbox_target_return: Decimal::from(10),
            sandbox_expected_growth: Decimal::from(8),
            palette_open: false,
            palette_query: String::new(),
            palette_cursor: 0,
            syncing: false,
            sync_rx: None,
            banner: None,
            paths,
        };
        wb.reload_from_disk();
        wb
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub fn reload_from_disk(&mut self) {
        if let Ok(wl) = store::load_watchlist(&self.paths) {
            self.benchmark_symbol = wl.benchmark_symbol;
        }
        match store::load_or_compute_reports(&self.paths) {
            Ok(reports) => {
                self.reports = reports;
                if !self
                    .reports
                    .iter()
                    .any(|r| r.ticker.eq_ignore_ascii_case(&self.selected_ticker))
                    && let Some(first) = self.reports.first()
                {
                    self.selected_ticker = first.ticker.clone();
                }
            }
            Err(err) => {
                self.banner = Some((format!("加载计算报告失败: {err:#}"), true));
            }
        }
        if let Ok(j) = store::load_journal(&self.paths) {
            self.journal = j;
        }
        self.benchmark = store::load_cached_price(&self.paths, &self.benchmark_symbol);
        self.sync_status = store::read_json(&self.paths.sync_status_path()).ok();
        let current = self.selected_ticker.clone();
        self.load_selection(&current);
    }

    fn load_selection(&mut self, ticker: &str) {
        self.selected_ticker = ticker.to_owned();
        self.selected_dossier = store::load_dossier(&self.paths, ticker).ok().flatten();
        self.selected_history_wk = store::load_cached_price_history(&self.paths, ticker, "1wk");
        self.selected_history_1d = store::load_cached_price_history(&self.paths, ticker, "1d");
        if let Some(rep) = self.reports.iter().find(|r| r.ticker == ticker) {
            self.sandbox_multiple = rep.assumptions.multiple_low;
            self.sandbox_target_return = rep.assumptions.target_return_pct;
            self.sandbox_expected_growth = rep.assumptions.expected_growth_pct;
        }
    }

    pub fn select_company(&mut self, ticker: &str, open_view: Option<NavView>) {
        self.load_selection(ticker);
        if let Some(view) = open_view {
            if view == NavView::Dossier && self.active_view != NavView::Dossier {
                self.dossier_tab = DossierTab::Overview;
            }
            self.active_view = view;
        }
    }

    fn select_company_contextual(&mut self, ticker: &str) {
        let target = if self.active_view == NavView::Agent {
            NavView::Agent
        } else {
            NavView::Dossier
        };
        let reset_overview = self.active_view != NavView::Dossier && target == NavView::Dossier;
        self.load_selection(ticker);
        if reset_overview {
            self.dossier_tab = DossierTab::Overview;
        }
        self.active_view = target;
    }

    fn step_selected_company(&mut self, delta: isize) {
        let len = self.reports.len();
        if len == 0 {
            return;
        }
        if let Some(pos) = self
            .reports
            .iter()
            .position(|r| r.ticker == self.selected_ticker)
        {
            let next_idx = (pos as isize + delta).rem_euclid(len as isize) as usize;
            let next_ticker = self.reports[next_idx].ticker.clone();
            self.load_selection(&next_ticker);
        }
    }

    pub fn switch_view(&mut self, view: NavView) {
        if view == NavView::Dossier {
            self.dossier_tab = DossierTab::Overview;
        }
        self.active_view = view;
    }

    pub fn focus_basket(&mut self, bucket: Option<Bucket>) {
        self.basket_filter = bucket;
        if let Some(b) = bucket {
            self.collapsed_baskets.remove(&b);
        }
        self.cursor_index = 0;
        self.active_view = NavView::Baskets;
    }

    pub fn jump_to_tag(&mut self, tag: String) {
        self.circle_filter = Some(tag);
        self.basket_filter = None;
        self.cursor_index = 0;
        self.active_view = NavView::Baskets;
    }

    fn toggle_circle_filter(&mut self, tag: &str) {
        if self.circle_filter.as_deref() == Some(tag) {
            self.circle_filter = None;
        } else {
            self.circle_filter = Some(tag.to_owned());
        }
        self.cursor_index = 0;
    }

    fn toggle_status_filter(&mut self, target: StatusQuickFilter) {
        self.status_filter = if self.status_filter == target {
            StatusQuickFilter::All
        } else {
            target
        };
        self.cursor_index = 0;
    }

    pub fn move_company_bucket(&mut self, ticker: &str, target: Bucket, cx: &mut Context<Self>) {
        match store::update_company_bucket(&self.paths, ticker, target) {
            Ok(_) => {
                self.reload_from_disk();
                self.banner = Some((
                    format!(
                        "已将 {ticker} 分拣至 [{} · {}]（已写入 workbench/watchlist.json）",
                        target.code(),
                        target.short_label()
                    ),
                    false,
                ));
            }
            Err(err) => {
                self.banner = Some((format!("切换篮子失败: {err:#}"), true));
            }
        }
        cx.notify();
    }

    pub fn toggle_palette(&mut self, window: &mut Window) {
        self.palette_open = !self.palette_open;
        self.palette_query.clear();
        self.palette_cursor = 0;
        window.focus(&self.focus_handle);
    }

    pub fn start_sync(&mut self, cx: &mut Context<Self>) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        self.banner = Some(("正在同步 SEC EDGAR 财报、公开股价与 K 线历史…".into(), false));
        let root = self.paths.repo_root.clone();
        let (tx, rx) = mpsc::channel();
        self.sync_rx = Some(rx);
        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            let res = store::sync_all(&paths, None, false).map_err(|e| format!("{e:#}"));
            let _ = tx.send(res);
        });
        cx.notify();
    }

    pub fn poll_background_sync(&mut self) -> bool {
        let Some(rx) = self.sync_rx.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(status)) => {
                self.syncing = false;
                self.sync_rx = None;
                let failed: Vec<&str> = status
                    .tickers
                    .iter()
                    .filter(|t| t.error.is_some())
                    .map(|t| t.ticker.as_str())
                    .collect();
                self.reload_from_disk();
                if failed.is_empty() {
                    self.banner = Some((
                        format!("同步完成 · 已更新 {} 家标的", status.tickers.len()),
                        false,
                    ));
                } else {
                    self.banner = Some((
                        format!("部分标的同步异常（已保留旧缓存）：{}", failed.join(", ")),
                        true,
                    ));
                }
                true
            }
            Ok(Err(err)) => {
                self.syncing = false;
                self.sync_rx = None;
                self.banner = Some((format!("同步失败：{err}"), true));
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.syncing = false;
                self.sync_rx = None;
                true
            }
        }
    }

    fn save_sandbox_assumptions(&mut self) {
        let Some(rep) = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .cloned()
        else {
            return;
        };
        let next = ValuationAssumptions {
            multiple_low: self.sandbox_multiple,
            multiple_high: self.sandbox_multiple + Decimal::from(4),
            target_return_pct: self.sandbox_target_return,
            expected_growth_pct: self.sandbox_expected_growth,
            strike_price_override: None,
        };
        match store::update_company_assumptions(&self.paths, &rep.ticker, next) {
            Ok(_) => {
                self.reload_from_disk();
                self.banner = Some((
                    format!(
                        "已保存 {} 假设 · {}x · 目标 {}% · 增速 {}%",
                        rep.ticker,
                        self.sandbox_multiple,
                        self.sandbox_target_return,
                        self.sandbox_expected_growth
                    ),
                    false,
                ));
            }
            Err(err) => {
                self.banner = Some((format!("保存假设失败: {err:#}"), true));
            }
        }
    }

    fn reset_sandbox_to_saved(&mut self) {
        if let Some(rep) = self.reports.iter().find(|r| r.ticker == self.selected_ticker) {
            self.sandbox_multiple = rep.assumptions.multiple_low;
            self.sandbox_target_return = rep.assumptions.target_return_pct;
            self.sandbox_expected_growth = rep.assumptions.expected_growth_pct;
        }
    }

    fn copy_cli_command(&mut self, cmd: String, label: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(cmd.clone()));
        self.banner = Some((format!("已复制 ({label}): {cmd}"), false));
    }

    fn toggle_sort(&mut self, col: SortCol) {
        if self.sort_col == col {
            if self.sort_asc {
                self.sort_asc = false;
            } else {
                self.sort_col = SortCol::Default;
            }
        } else {
            self.sort_col = col;
            self.sort_asc = matches!(col, SortCol::Ticker | SortCol::Margin | SortCol::NetDebt);
        }
        self.cursor_index = 0;
    }

    fn visible_baskets_order(&self) -> Vec<Bucket> {
        match self.basket_filter {
            Some(b) => vec![b],
            None => Bucket::display_order().to_vec(),
        }
    }

    fn visible_linear_tickers(&self) -> Vec<String> {
        let mut out = Vec::new();
        let order: Vec<Bucket> = match self.baskets_view_mode {
            BasketsViewMode::List => self.visible_baskets_order(),
            BasketsViewMode::Board => vec![Bucket::Watch, Bucket::Core, Bucket::Out, Bucket::TooHard],
        };
        for b in order {
            if self.baskets_view_mode == BasketsViewMode::List
                && self.basket_filter.is_none()
                && self.collapsed_baskets.contains(&b)
            {
                continue;
            }
            let rows = filter_reports_for_basket(
                &self.reports,
                b,
                self.circle_filter.as_deref(),
                self.status_filter,
                self.sort_col,
                self.sort_asc,
            );
            for r in rows {
                out.push(r.ticker.clone());
            }
        }
        out
    }

    fn build_palette_items(&self) -> Vec<PaletteItem> {
        let mut items = Vec::with_capacity(self.reports.len() * 2 + 24);
        for rep in &self.reports {
            let (bc_fg, _, _) = bucket_colors(rep.bucket);
            items.push(PaletteItem {
                group: "DOSSIER".into(),
                title: format!("{}  {} [{}]", rep.ticker, rep.name, rep.bucket.code()),
                subtitle: format!(
                    "#{} · {}",
                    primary_circle_tag(&rep.circle),
                    rep.status.label()
                ),
                shortcut: Some("↵"),
                dot_color: bc_fg,
                action: PaletteAction::OpenDossier(rep.ticker.clone()),
            });
        }
        for rep in &self.reports {
            items.push(PaletteItem {
                group: "ALAN".into(),
                title: format!("Alan 审讯 · {} {}", rep.ticker, rep.name),
                subtitle: "三句话过滤、跨学科格栅与 5 年事前验尸".into(),
                shortcut: None,
                dot_color: VIOLET_FG,
                action: PaletteAction::OpenAgent(rep.ticker.clone()),
            });
        }
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "总览 (IN / WATCH / OUT / TOO HARD)".into(),
            subtitle: "今日信号与芒格三篮子筛选".into(),
            shortcut: Some("⌘1"),
            dot_color: INK_PRIMARY,
            action: PaletteAction::FocusBasket(None),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "切换 总览 列表 / 看板".into(),
            subtitle: match self.baskets_view_mode {
                BasketsViewMode::List => "切换为 4 列工作流看板",
                BasketsViewMode::Board => "切换为分组指标列表",
            }
            .into(),
            shortcut: Some("v"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleBasketsView,
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: format!("财报与 K 线 · {}", self.selected_ticker),
            subtitle: "值锚定 K 线、查理沙盒、ROIC 与人类判词".into(),
            shortcut: Some("⌘2"),
            dot_color: EMERALD_FG,
            action: PaletteAction::SwitchView(NavView::Dossier),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: format!("Alan 审讯台 · {}", self.selected_ticker),
            subtitle: "独立 AI 幕僚审讯与 Prompt/CLI 协同".into(),
            shortcut: Some("⌘3"),
            dot_color: VIOLET_FG,
            action: PaletteAction::SwitchView(NavView::Agent),
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "XBRL 底表 · 多年原账与公式审计".into(),
            subtitle: format!("查看 {} 算术推导与 SHA-256 溯源", self.selected_ticker),
            shortcut: Some("i"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleInspector,
        });
        items.push(PaletteItem {
            group: "VIEW".into(),
            title: "决策账本 · 机会成本对照".into(),
            subtitle: "PAPER RESEARCH FIXTURE · 100% 现金与 S&P 500 基准".into(),
            shortcut: Some("⌘4"),
            dot_color: GOLD_FG,
            action: PaletteAction::SwitchView(NavView::Journal),
        });
        for b in Bucket::display_order() {
            let (fg, _, _) = bucket_colors(b);
            items.push(PaletteItem {
                group: "BASKET".into(),
                title: format!("聚焦篮子 · {} ({})", b.code(), b.short_label()),
                subtitle: b.description().into(),
                shortcut: None,
                dot_color: fg,
                action: PaletteAction::FocusBasket(Some(b)),
            });
        }
        for b in Bucket::display_order() {
            let (fg, _, _) = bucket_colors(b);
            items.push(PaletteItem {
                group: "MOVE".into(),
                title: format!(
                    "将 {} 归入 [{} · {}]",
                    self.selected_ticker,
                    b.code(),
                    b.short_label()
                ),
                subtitle: "写入 workbench/watchlist.json 并重算状态".into(),
                shortcut: None,
                dot_color: fg,
                action: PaletteAction::MoveCompanyBucket(self.selected_ticker.clone(), b),
            });
        }
        for (tag, cnt) in collect_circle_tags(&self.reports) {
            items.push(PaletteItem {
                group: "TAG".into(),
                title: format!("按标签筛选 #{tag}"),
                subtitle: format!("跨篮子关联 {cnt} 家标的"),
                shortcut: None,
                dot_color: SKY_FG,
                action: PaletteAction::FilterCircle(Some(tag)),
            });
        }
        items.push(PaletteItem {
            group: "DOSSIER".into(),
            title: "切换 TTM / FY 口径".into(),
            subtitle: if self.use_ttm_basis {
                "当前: TTM 滚动 12 个月"
            } else {
                "当前: FY 最近完整财年"
            }
            .into(),
            shortcut: Some("t"),
            dot_color: SKY_FG,
            action: PaletteAction::ToggleBasis,
        });
        let selected_name = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .map(|r| r.name.as_str())
            .unwrap_or(&self.selected_ticker);
        let selected_bucket = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .map(|r| r.bucket)
            .unwrap_or(Bucket::Watch);
        items.push(PaletteItem {
            group: "PROMPT".into(),
            title: format!("复制 Alan 费曼审讯 Prompt ({})", self.selected_ticker),
            subtitle: "按 docs/munger-workflow.md 生成只读审讯指令".into(),
            shortcut: Some("p"),
            dot_color: VIOLET_FG,
            action: PaletteAction::CopyCli(
                build_alan_interrogation_prompt(
                    &self.selected_ticker,
                    selected_name,
                    selected_bucket,
                ),
                format!("Alan Prompt {}", self.selected_ticker),
            ),
        });
        items.push(PaletteItem {
            group: "CLI".into(),
            title: format!("复制 CLI: ./apps/m3-desktop/m3 report {}", self.selected_ticker),
            subtitle: "写入系统剪贴板".into(),
            shortcut: Some("c"),
            dot_color: INK_SECONDARY,
            action: PaletteAction::CopyCli(
                format!("./apps/m3-desktop/m3 report {}", self.selected_ticker),
                format!("report {}", self.selected_ticker),
            ),
        });
        items.push(PaletteItem {
            group: "DATA".into(),
            title: "同步 SEC EDGAR 财报、股价与 K 线历史".into(),
            subtitle: "更新一手 XBRL 与 5Y/1Y 行情".into(),
            shortcut: Some("⌘R"),
            dot_color: EMERALD_FG,
            action: PaletteAction::SyncNow,
        });
        items
    }

    fn execute_palette_action(&mut self, action: PaletteAction, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_cursor = 0;
        match action {
            PaletteAction::OpenDossier(ticker) => {
                self.select_company(&ticker, Some(NavView::Dossier));
            }
            PaletteAction::OpenAgent(ticker) => {
                self.select_company(&ticker, Some(NavView::Agent));
            }
            PaletteAction::SwitchView(view) => {
                self.switch_view(view);
            }
            PaletteAction::FocusBasket(bucket) => {
                self.focus_basket(bucket);
            }
            PaletteAction::ToggleBasketsView => {
                self.baskets_view_mode = match self.baskets_view_mode {
                    BasketsViewMode::List => BasketsViewMode::Board,
                    BasketsViewMode::Board => BasketsViewMode::List,
                };
                self.active_view = NavView::Baskets;
            }
            PaletteAction::MoveCompanyBucket(ticker, target) => {
                self.move_company_bucket(&ticker, target, cx);
                return;
            }
            PaletteAction::FilterCircle(tag) => {
                self.circle_filter = tag;
                self.basket_filter = None;
                self.cursor_index = 0;
                self.active_view = NavView::Baskets;
            }
            PaletteAction::ToggleBasis => {
                self.use_ttm_basis = !self.use_ttm_basis;
                self.active_view = NavView::Dossier;
            }
            PaletteAction::ToggleInspector => {
                self.dossier_tab = if self.active_view == NavView::Dossier
                    && self.dossier_tab == DossierTab::Inspector
                {
                    DossierTab::Overview
                } else {
                    DossierTab::Inspector
                };
                self.active_view = NavView::Dossier;
            }
            PaletteAction::CopyCli(cmd, label) => {
                self.copy_cli_command(cmd, &label, cx);
            }
            PaletteAction::SyncNow => {
                self.start_sync(cx);
                return;
            }
        }
        cx.notify();
    }

    fn handle_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let mods = &ev.keystroke.modifiers;

        if mods.platform {
            return;
        }

        if self.palette_open {
            let all_items = self.build_palette_items();
            let filtered = filter_palette_items(&all_items, &self.palette_query);
            match key {
                "escape" => {
                    self.palette_open = false;
                    self.palette_query.clear();
                    self.palette_cursor = 0;
                    cx.stop_propagation();
                    cx.notify();
                }
                "up" | "p" if key == "up" || mods.control => {
                    if !filtered.is_empty() {
                        self.palette_cursor = if self.palette_cursor == 0 {
                            filtered.len() - 1
                        } else {
                            self.palette_cursor - 1
                        };
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
                "down" | "n" if key == "down" || mods.control => {
                    if !filtered.is_empty() {
                        self.palette_cursor = (self.palette_cursor + 1) % filtered.len();
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
                "backspace" => {
                    self.palette_query.pop();
                    self.palette_cursor = 0;
                    cx.stop_propagation();
                    cx.notify();
                }
                "enter" => {
                    let idx = self.palette_cursor.min(filtered.len().saturating_sub(1));
                    if let Some(item) = filtered.get(idx) {
                        let action = item.action.clone();
                        self.execute_palette_action(action, cx);
                        cx.stop_propagation();
                    }
                }
                _ if !mods.control && !mods.alt => {
                    if let Some(ch) = ev.keystroke.key_char.as_deref()
                        && !ch.is_empty()
                        && !ch.chars().any(char::is_control)
                    {
                        self.palette_query.push_str(ch);
                        self.palette_cursor = 0;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
                _ => {}
            }
            return;
        }

        if mods.control || mods.alt {
            return;
        }

        if key == "/" || ev.keystroke.key_char.as_deref() == Some("/") {
            self.toggle_palette(window);
            cx.stop_propagation();
            cx.notify();
            return;
        }

        match self.active_view {
            NavView::Baskets => {
                let mut handled = true;
                match key {
                    "0" => self.focus_basket(None),
                    "1" => self.focus_basket(Some(Bucket::Core)),
                    "2" => self.focus_basket(Some(Bucket::Watch)),
                    "3" => self.focus_basket(Some(Bucket::Out)),
                    "4" => self.focus_basket(Some(Bucket::TooHard)),
                    "v" => {
                        self.baskets_view_mode = match self.baskets_view_mode {
                            BasketsViewMode::List => BasketsViewMode::Board,
                            BasketsViewMode::Board => BasketsViewMode::List,
                        };
                        self.cursor_index = 0;
                    }
                    "escape" => {
                        if self.basket_filter.is_some()
                            || self.circle_filter.is_some()
                            || self.status_filter != StatusQuickFilter::All
                        {
                            self.basket_filter = None;
                            self.circle_filter = None;
                            self.status_filter = StatusQuickFilter::All;
                            self.cursor_index = 0;
                        } else {
                            handled = false;
                        }
                    }
                    _ => handled = false,
                }
                if handled {
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }

                let tickers = self.visible_linear_tickers();
                if tickers.is_empty() {
                    return;
                }
                let max_idx = tickers.len() - 1;
                self.cursor_index = self.cursor_index.min(max_idx);
                match key {
                    "j" | "down" => {
                        self.cursor_index = (self.cursor_index + 1).min(max_idx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "k" | "up" => {
                        self.cursor_index = self.cursor_index.saturating_sub(1);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "enter" | "l" => {
                        let ticker = tickers[self.cursor_index].clone();
                        self.select_company(&ticker, Some(NavView::Dossier));
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "a" => {
                        let ticker = tickers[self.cursor_index].clone();
                        self.select_company(&ticker, Some(NavView::Agent));
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "c" => {
                        let ticker = tickers[self.cursor_index].clone();
                        self.copy_cli_command(
                            format!("./apps/m3-desktop/m3 report {ticker}"),
                            &format!("report {ticker}"),
                            cx,
                        );
                        cx.stop_propagation();
                        cx.notify();
                    }
                    _ => {}
                }
            }
            NavView::Dossier | NavView::Agent => match key {
                "escape" | "h" => {
                    self.switch_view(NavView::Baskets);
                    cx.stop_propagation();
                    cx.notify();
                }
                "1" | "d" => {
                    self.dossier_tab = DossierTab::Overview;
                    self.active_view = NavView::Dossier;
                    cx.stop_propagation();
                    cx.notify();
                }
                "2" | "a" => {
                    self.active_view = NavView::Agent;
                    cx.stop_propagation();
                    cx.notify();
                }
                "3" => {
                    self.dossier_tab = DossierTab::Inspector;
                    self.active_view = NavView::Dossier;
                    cx.stop_propagation();
                    cx.notify();
                }
                "i" => {
                    if self.active_view == NavView::Dossier {
                        self.dossier_tab = match self.dossier_tab {
                            DossierTab::Overview => DossierTab::Inspector,
                            DossierTab::Inspector => DossierTab::Overview,
                        };
                    } else {
                        self.active_view = NavView::Dossier;
                        self.dossier_tab = DossierTab::Inspector;
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
                "t" => {
                    self.use_ttm_basis = !self.use_ttm_basis;
                    cx.stop_propagation();
                    cx.notify();
                }
                "p" => {
                    let ticker = self.selected_ticker.clone();
                    let (name, bucket) = self
                        .reports
                        .iter()
                        .find(|r| r.ticker == ticker)
                        .map(|r| (r.name.clone(), r.bucket))
                        .unwrap_or_else(|| (ticker.clone(), Bucket::Watch));
                    let prompt = build_alan_interrogation_prompt(&ticker, &name, bucket);
                    self.copy_cli_command(prompt, &format!("Alan Prompt {ticker}"), cx);
                    cx.stop_propagation();
                    cx.notify();
                }
                "c" => {
                    let ticker = self.selected_ticker.clone();
                    let (cmd, label) = if self.active_view == NavView::Agent {
                        (
                            format!("./apps/m3-desktop/m3 dossier {ticker}"),
                            format!("dossier {ticker}"),
                        )
                    } else {
                        (
                            format!("./apps/m3-desktop/m3 report {ticker}"),
                            format!("report {ticker}"),
                        )
                    };
                    self.copy_cli_command(cmd, &label, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
                "j" | "]" => {
                    self.step_selected_company(1);
                    cx.stop_propagation();
                    cx.notify();
                }
                "k" | "[" => {
                    self.step_selected_company(-1);
                    cx.stop_propagation();
                    cx.notify();
                }
                _ => {}
            },
            NavView::Journal => match key {
                "escape" => {
                    self.switch_view(NavView::Baskets);
                    cx.stop_propagation();
                    cx.notify();
                }
                "c" => {
                    self.copy_cli_command(
                        "./apps/m3-desktop/m3 journal list".into(),
                        "journal list",
                        cx,
                    );
                    cx.stop_propagation();
                    cx.notify();
                }
                _ => {}
            },
        }
    }
}

pub fn compute_today_pulse(reports: &[MungerReport]) -> TodayPulse {
    let mut strike_tickers = Vec::new();
    let mut red_core_tickers = Vec::new();
    let mut watch_count = 0usize;

    for r in reports {
        match r.bucket {
            Bucket::Core => {
                if r.status == StrikeZoneStatus::InStrikeZone {
                    strike_tickers.push(r.ticker.clone());
                }
                if r.status == StrikeZoneStatus::WarningBlocked
                    || r.warnings.iter().any(|w| w.severity == WarningSeverity::Red)
                {
                    red_core_tickers.push(r.ticker.clone());
                }
            }
            Bucket::Watch => watch_count += 1,
            Bucket::Out | Bucket::TooHard => {}
        }
    }

    TodayPulse {
        strike_tickers,
        red_core_tickers,
        watch_count,
    }
}

pub fn collect_circle_tags(reports: &[MungerReport]) -> Vec<(String, usize)> {
    let mut map: BTreeMap<String, usize> = BTreeMap::new();
    for rep in reports {
        let tag = primary_circle_tag(&rep.circle);
        if !tag.is_empty() {
            *map.entry(tag).or_insert(0) += 1;
        }
    }
    let mut pairs: Vec<(String, usize)> = map.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs
}

pub fn filter_reports_for_basket<'a>(
    reports: &'a [MungerReport],
    bucket: Bucket,
    circle_filter: Option<&str>,
    status_filter: StatusQuickFilter,
    sort_col: SortCol,
    sort_asc: bool,
) -> Vec<&'a MungerReport> {
    let mut rows: Vec<&'a MungerReport> = reports
        .iter()
        .filter(|r| r.bucket == bucket)
        .filter(|r| circle_filter.is_none_or(|t| primary_circle_tag(&r.circle) == t))
        .filter(|r| match status_filter {
            StatusQuickFilter::All => true,
            StatusQuickFilter::StrikeOnly => r.status == StrikeZoneStatus::InStrikeZone,
            StatusQuickFilter::WarnedOnly => {
                r.status == StrikeZoneStatus::WarningBlocked
                    || r.warnings.iter().any(|w| w.severity == WarningSeverity::Red)
            }
        })
        .collect();
    sort_reports(&mut rows, sort_col, sort_asc);
    rows
}

pub fn build_alan_interrogation_prompt(ticker: &str, name: &str, bucket: Bucket) -> String {
    format!(
        "请作为我的芒格跨学科研究幕僚 Alan，执行 [{ticker} · {name}]（当前分桶: {}）的审讯工作流：\n\
        1) 先运行 `./apps/m3-desktop/m3 report {ticker}` 与 `./apps/m3-desktop/m3 dossier {ticker}` 获取确定性财务与现有草稿（严禁口算或改动数字）。\n\
        2) 基于 10-K/电话会更新三句话业务过滤、跨学科思维格栅与 5 年逆向事前验尸。\n\
        3) 针对我提出的质疑进行一轮费曼答辩追问；保留 workbench/dossiers/{ticker}.json 中的 human_judgment 待我本人拍板，严禁输出买卖指令。",
        bucket.code()
    )
}

fn dec_to_f64(d: Decimal) -> Option<f64> {
    d.to_string().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Computes `(y_min, y_max, max_volume)` for OHLC candlestick rendering.
/// Includes Munger Strike Price and Fair Value band smoothly when within reasonable proximity
/// of historical prices so candlesticks never get flattened.
pub fn compute_kline_domain(
    bars: &[OhlcBar],
    strike: Option<Decimal>,
    fv_high: Option<Decimal>,
) -> Option<(f64, f64, u64)> {
    if bars.is_empty() {
        return None;
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut max_vol: u64 = 1;
    for bar in bars {
        if let Some(l) = dec_to_f64(bar.low) {
            lo = lo.min(l);
        }
        if let Some(h) = dec_to_f64(bar.high) {
            hi = hi.max(h);
        }
        max_vol = max_vol.max(bar.volume);
    }
    if !lo.is_finite() || !hi.is_finite() || lo <= 0.0 || hi <= 0.0 {
        return None;
    }
    let orig_lo = lo;
    let orig_hi = hi;
    if let Some(s) = strike.and_then(dec_to_f64)
        && s > 0.0
        && s >= orig_lo * 0.42
        && s <= orig_hi * 2.2
    {
        lo = lo.min(s);
        hi = hi.max(s);
    }
    if let Some(fh) = fv_high.and_then(dec_to_f64)
        && fh > 0.0
        && fh >= orig_lo * 0.42
        && fh <= orig_hi * 2.2
    {
        lo = lo.min(fh);
        hi = hi.max(fh);
    }
    let span = (hi - lo).max(hi * 0.05);
    let pad = span * 0.08;
    Some(((lo - pad).max(0.01), hi + pad, max_vol))
}

fn currency_prefix(currency: &str) -> &'static str {
    match currency {
        "USD" => "$",
        "EUR" => "€",
        "HKD" => "HK$",
        "JPY" => "¥",
        "CNY" => "¥",
        _ => "",
    }
}

fn fmt_price(val: Option<Decimal>, currency: &str) -> String {
    match val {
        Some(v) => format!("{}{:.2}", currency_prefix(currency), v),
        None => "—".into(),
    }
}

fn fmt_money_compact(val: Decimal, currency: &str) -> String {
    let sign = if val < Decimal::ZERO { "-" } else { "" };
    let abs = val.abs();
    let pfx = currency_prefix(currency);
    let billion = Decimal::from(1_000_000_000i64);
    let million = Decimal::from(1_000_000i64);
    if abs >= billion {
        format!("{sign}{pfx}{:.2}B", abs / billion)
    } else if abs >= million {
        format!("{sign}{pfx}{:.1}M", abs / million)
    } else {
        format!("{sign}{pfx}{:.0}", abs)
    }
}

fn fmt_shares_compact(val: Decimal) -> String {
    let billion = Decimal::from(1_000_000_000i64);
    let million = Decimal::from(1_000_000i64);
    if val >= billion {
        format!("{:.2}B", val / billion)
    } else if val >= million {
        format!("{:.1}M", val / million)
    } else {
        format!("{:.0}", val)
    }
}

/// Returns `(formatted, polarity)` where `polarity` is `-1`, `0`, or `1`.
/// Magnitudes `< 0.005` clamp to neutral `"0.00%"` (`polarity = 0`).
pub fn fmt_pct_clean(val: Option<Decimal>, signed: bool) -> (String, i8) {
    let Some(v) = val else {
        return ("—".into(), 0);
    };
    let threshold = Decimal::new(5, 3); // 0.005
    if v.abs() < threshold {
        return ("0.00%".into(), 0);
    }
    let pol = if v > Decimal::ZERO { 1 } else { -1 };
    let text = if signed {
        format!("{:+.2}%", v)
    } else {
        format!("{:.2}%", v)
    };
    (text, pol)
}

fn share_delta_color(val: Option<Decimal>, pol: i8) -> u32 {
    match val {
        Some(v) if v <= Decimal::new(-5, 1) => EMERALD_FG,
        Some(v) if v > Decimal::from(2) => GOLD_FG,
        Some(_) if pol == 0 => INK_MUTED,
        Some(_) => INK_SECONDARY,
        None => INK_MUTED,
    }
}

fn civil_ym_from_ms(ms: u64) -> String {
    let days = (ms / 1000) as i64 / 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let yr = y + i64::from(m <= 2);
    format!("{yr:04}-{m:02}")
}

fn fmt_time_cn(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return "未同步".into();
    };
    let secs = (ms / 1000 + 8 * 3600) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let yr = y + i64::from(m <= 2);
    format!(
        "{yr:04}-{m:02}-{d:02} {:02}:{:02}",
        tod / 3600,
        (tod % 3600) / 60
    )
}

fn bucket_colors(bucket: Bucket) -> (u32, u32, u32) {
    match bucket {
        Bucket::Core => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        Bucket::Watch => (SKY_FG, SKY_BG, SKY_BORDER),
        Bucket::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        Bucket::TooHard => (SLATE_FG, SLATE_BG, SLATE_BORDER),
    }
}

fn status_colors(status: StrikeZoneStatus) -> (u32, u32, u32) {
    match status {
        StrikeZoneStatus::InStrikeZone => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        StrikeZoneStatus::NearStrikeZone => (GOLD_FG, GOLD_BG, GOLD_BORDER),
        StrikeZoneStatus::WarningBlocked => (CORAL_FG, CORAL_BG, CORAL_BORDER),
        StrikeZoneStatus::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        StrikeZoneStatus::Wait | StrikeZoneStatus::TooHard | StrikeZoneStatus::NoFinancials => {
            (SLATE_FG, SLATE_BG, SLATE_BORDER)
        }
    }
}

fn status_dot_color(status: StrikeZoneStatus) -> u32 {
    match status {
        StrikeZoneStatus::InStrikeZone => EMERALD_FG,
        StrikeZoneStatus::NearStrikeZone => GOLD_FG,
        StrikeZoneStatus::WarningBlocked => CORAL_FG,
        StrikeZoneStatus::Out => ORANGE_FG,
        StrikeZoneStatus::Wait | StrikeZoneStatus::TooHard | StrikeZoneStatus::NoFinancials => {
            INK_MUTED
        }
    }
}

fn journal_action_colors(action: JournalAction) -> (u32, u32, u32) {
    match action {
        JournalAction::Wait => (SKY_FG, SKY_BG, SKY_BORDER),
        JournalAction::Out => (ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
        JournalAction::TooHard => (SLATE_FG, SLATE_BG, SLATE_BORDER),
        JournalAction::PaperBuy => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        JournalAction::PaperSell => (CORAL_FG, CORAL_BG, CORAL_BORDER),
    }
}

fn cmp_opt_dec(a: Option<Decimal>, b: Option<Decimal>, asc: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => {
            if asc {
                x.cmp(&y)
            } else {
                y.cmp(&x)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub fn sort_reports(rows: &mut [&MungerReport], col: SortCol, asc: bool) {
    rows.sort_by(|a, b| {
        let va = primary_val(a, true);
        let vb = primary_val(b, true);
        let ord = match col {
            SortCol::Default => {
                let rank = |bkt: Bucket| match bkt {
                    Bucket::Core => 0,
                    Bucket::Watch => 1,
                    Bucket::Out => 2,
                    Bucket::TooHard => 3,
                };
                rank(a.bucket).cmp(&rank(b.bucket))
            }
            SortCol::Ticker => {
                if asc {
                    a.ticker.cmp(&b.ticker)
                } else {
                    b.ticker.cmp(&a.ticker)
                }
            }
            SortCol::Price => cmp_opt_dec(
                a.price.as_ref().map(|p| p.price),
                b.price.as_ref().map(|p| p.price),
                asc,
            ),
            SortCol::Margin => cmp_opt_dec(
                va.and_then(|v| v.distance_to_strike_pct),
                vb.and_then(|v| v.distance_to_strike_pct),
                asc,
            ),
            SortCol::Roic => cmp_opt_dec(a.roic_5y_avg_pct, b.roic_5y_avg_pct, asc),
            SortCol::FcfYield => cmp_opt_dec(
                va.and_then(|v| v.strict_fcf_yield_mcap_pct),
                vb.and_then(|v| v.strict_fcf_yield_mcap_pct),
                asc,
            ),
            SortCol::ShareDelta => cmp_opt_dec(a.share_change_3y_pct, b.share_change_3y_pct, asc),
            SortCol::NetDebt => cmp_opt_dec(
                a.net_debt_to_owner_earnings,
                b.net_debt_to_owner_earnings,
                asc,
            ),
        };
        ord.then_with(|| a.ticker.cmp(&b.ticker))
    });
}

pub fn filter_palette_items<'a>(items: &'a [PaletteItem], query: &str) -> Vec<&'a PaletteItem> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return items.iter().collect();
    }
    let terms: Vec<&str> = q.split_whitespace().collect();
    items
        .iter()
        .filter(|it| {
            let hay = format!("{} {} {}", it.group, it.title, it.subtitle).to_ascii_lowercase();
            terms.iter().all(|t| hay.contains(t))
        })
        .collect()
}

fn pixel_m3_mark(cell_px: f32) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .children(PIXEL_M3.iter().map(|row| {
            let mut runs: Vec<(bool, usize)> = Vec::new();
            for ch in row.chars() {
                let on = ch == '#';
                if let Some((last_on, count)) = runs.last_mut()
                    && *last_on == on
                {
                    *count += 1;
                } else {
                    runs.push((on, 1));
                }
            }
            div()
                .flex()
                .flex_row()
                .h(px(cell_px))
                .children(runs.into_iter().map(|(on, count)| {
                    let seg = div().w(px(count as f32 * cell_px)).h(px(cell_px));
                    if on { seg.bg(rgb(INK_HERO)) } else { seg }
                }))
        }))
}

fn card_shell() -> gpui::Div {
    div()
        .rounded_lg()
        .bg(rgb(BG_SURFACE))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .overflow_hidden()
}

fn seg_group() -> gpui::Div {
    div()
        .p_0p5()
        .rounded_md()
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .flex()
        .items_center()
        .gap_0p5()
}

fn seg_tab(
    id: impl Into<SharedString>,
    label: &str,
    active: bool,
    accent_bg: Option<(u32, u32)>,
    mono: bool,
    fonts: &FontRoles,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let family = if mono {
        fonts.mono_term.clone()
    } else {
        fonts.ui.clone()
    };
    let (bg_el, fg_col) = match (active, accent_bg) {
        (true, Some((fg, _))) => (Some(accent_bg.map(|(_, bg)| rgba(bg))), fg),
        (true, None) => (None, INK_HERO),
        (false, _) => (None, INK_MUTED),
    };
    let mut el = div()
        .id(id.into())
        .px_2p5()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .font_family(family)
        .text_xs()
        .on_click(on_click);
    if active {
        el = el.font_weight(gpui::FontWeight::SEMIBOLD);
        if let Some(Some(bg_rgba)) = bg_el {
            el = el.bg(bg_rgba);
        } else {
            el = el.bg(rgb(BG_HOVER));
        }
    } else {
        el = el.hover(|h| h.text_color(rgb(INK_PRIMARY)));
    }
    el.text_color(rgb(fg_col)).child(label.to_owned())
}

fn kbd(label: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .px_1p5()
        .py_0p5()
        .rounded_xs()
        .bg(rgb(0x161620))
        .border_1()
        .border_color(rgb(BORDER_PILL))
        .font_family(fonts.mono_term.clone())
        .text_size(px(10.))
        .text_color(rgb(INK_MUTED))
        .child(label.to_owned())
}

fn badge(text: String, fg: u32, bg: u32, border: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(rgba(border))
        .bg(rgba(bg))
        .font_family(fonts.ui.clone())
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(fg))
        .whitespace_nowrap()
        .child(text)
}

fn prop_section(title: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .pt_3()
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .font_family(fonts.mono_term.clone())
                .text_size(px(10.))
                .text_color(rgb(INK_MUTED))
                .child(title.to_owned()),
        )
}

fn circle_tag_button(
    id: impl Into<SharedString>,
    tag: String,
    active: bool,
    quiet_when_inactive: bool,
    count: Option<usize>,
    fonts: &FontRoles,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let text = match count {
        Some(c) => format!("#{tag} {c}"),
        None => format!("#{tag}"),
    };
    let mut el = div()
        .id(id.into())
        .px_2()
        .py_0p5()
        .rounded_xs()
        .cursor_pointer()
        .font_family(fonts.mono_term.clone())
        .text_size(px(11.))
        .whitespace_nowrap()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, win, cx| {
            cx.stop_propagation();
            on_click(ev, win, cx);
        });
    if active {
        el = el
            .bg(rgba(SKY_BG))
            .border_1()
            .border_color(rgba(SKY_BORDER))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .text_color(rgb(SKY_FG));
    } else if quiet_when_inactive {
        el = el
            .text_color(rgb(INK_MUTED))
            .hover(|s| s.text_color(rgb(INK_PRIMARY)).bg(rgb(BG_HOVER)));
    } else {
        el = el
            .bg(rgb(0x161622))
            .border_1()
            .border_color(rgb(BORDER_SUBTLE))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .text_color(rgb(INK_SECONDARY));
    }
    el.child(text)
}

fn v_divider() -> gpui::Div {
    div()
        .w(px(1.))
        .h(px(14.))
        .flex_shrink_0()
        .bg(rgb(BORDER_HAIRLINE))
}

fn section_header(title: &str, subtitle: String, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .pb_2p5()
        .mb_3()
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO))
                .child(title.to_owned()),
        )
        .child(
            div()
                .font_family(fonts.mono_term.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(subtitle),
        )
}

fn primary_val(rep: &MungerReport, prefer_ttm: bool) -> Option<&ValuationSnapshot> {
    if prefer_ttm {
        rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref())
    } else {
        rep.fy_valuation.as_ref().or(rep.ttm_valuation.as_ref())
    }
}

fn primary_row(rep: &MungerReport, prefer_ttm: bool) -> Option<&AnnualMetricRow> {
    if prefer_ttm {
        rep.ttm_row.as_ref().or_else(|| rep.annual_rows.last())
    } else {
        rep.annual_rows.last()
    }
}

fn ratio_bar(
    label: String,
    right_label: String,
    amount: Decimal,
    base: Decimal,
    color: u32,
    fonts: &FontRoles,
) -> gpui::Div {
    let pct = if base > Decimal::ZERO {
        let raw = (amount.max(Decimal::ZERO) / base) * Decimal::from(100);
        raw.min(Decimal::from(100))
            .to_string()
            .parse::<f32>()
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let fill_px = (pct * 1.1).clamp(2.0, 110.0);
    div()
        .mt_2()
        .flex()
        .items_center()
        .justify_between()
        .gap_2p5()
        .child(
            div()
                .w(px(76.))
                .flex_shrink_0()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_SECONDARY))
                .child(label),
        )
        .child(
            div()
                .w(px(110.))
                .flex_shrink_0()
                .h(px(6.))
                .rounded_full()
                .bg(rgb(BG_WELL))
                .child(div().w(px(fill_px)).h_full().rounded_full().bg(rgb(color))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_right()
                .font_family(fonts.mono_num.clone())
                .text_xs()
                .text_color(rgb(INK_PRIMARY))
                .child(right_label),
        )
}

fn roic_sparkline(rows: &[AnnualMetricRow], fonts: &FontRoles) -> gpui::Div {
    let slice: Vec<&AnnualMetricRow> = rows
        .iter()
        .rev()
        .take(7)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let max_roic = slice
        .iter()
        .filter_map(|r| r.roic_pct)
        .fold(Decimal::from(25), |acc, v| acc.max(v))
        .max(Decimal::from(20));

    div()
        .mt_3()
        .pt_2p5()
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb_2()
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child("历年 ROIC 走势（15% 基准）"),
                )
                .child(
                    div()
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_SECONDARY))
                        .child(format!("峰值 {:.0}%", max_roic)),
                ),
        )
        .child(
            div()
                .h(px(66.))
                .flex()
                .items_end()
                .gap_2()
                .children(slice.into_iter().map(|r| {
                    let (bar_h, bar_color, val_txt, is_num) = match r.roic_pct {
                        Some(v) if v > Decimal::ZERO => {
                            let h = ((v / max_roic) * Decimal::from(40))
                                .to_string()
                                .parse::<f32>()
                                .unwrap_or(4.0)
                                .clamp(4.0, 40.0);
                            let col = if v >= Decimal::from(15) {
                                EMERALD_FG
                            } else {
                                GOLD_FG
                            };
                            (h, col, format!("{v:.0}%"), true)
                        }
                        Some(v) => (4.0, CORAL_FG, format!("{v:.0}%"), true),
                        None => (4.0, INK_MUTED, "净现".into(), false),
                    };
                    let fy_short = r.label.replace("FY20", "'");
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_end()
                        .gap_0p5()
                        .child(
                            div()
                                .font_family(if is_num {
                                    fonts.mono_num.clone()
                                } else {
                                    fonts.ui.clone()
                                })
                                .text_size(px(11.))
                                .text_color(rgb(if is_num { INK_SECONDARY } else { INK_MUTED }))
                                .child(val_txt),
                        )
                        .child(
                            div()
                                .w(px(14.))
                                .h(px(bar_h))
                                .rounded_xs()
                                .bg(rgb(bar_color)),
                        )
                        .child(
                            div()
                                .font_family(fonts.mono_num.clone())
                                .text_size(px(10.))
                                .text_color(rgb(INK_MUTED))
                                .child(fy_short),
                        )
                })),
        )
}

fn stepper_inline<F1, F2>(
    label: &str,
    value: String,
    id_prefix: &str,
    fonts: &FontRoles,
    on_dec: F1,
    on_inc: F2,
) -> gpui::Div
where
    F1: Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    F2: Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(label.to_owned()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .rounded_sm()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgb(BORDER_SUBTLE))
                .overflow_hidden()
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("{id_prefix}-dec")))
                        .w(px(20.))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_SECONDARY))
                        .cursor_pointer()
                        .on_click(on_dec)
                        .child("−"),
                )
                .child(
                    div()
                        .w(px(40.))
                        .text_center()
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(INK_HERO))
                        .child(value),
                )
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("{id_prefix}-inc")))
                        .w(px(20.))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_SECONDARY))
                        .cursor_pointer()
                        .on_click(on_inc)
                        .child("+"),
                ),
        )
}

impl MungerWorkbench {
    fn render_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let nav_items = [
            (
                NavView::Baskets,
                "总览",
                "⌘1",
                self.active_view == NavView::Baskets && self.basket_filter.is_none(),
                INK_HERO,
            ),
            (
                NavView::Journal,
                "决策账本",
                "⌘4",
                self.active_view == NavView::Journal,
                GOLD_FG,
            ),
        ];

        div()
            .w(px(224.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(BG_SHELL))
            // Immersive Traffic-Light Header Zone + Pixel-Art M3 Monogram
            .child(
                div()
                    .h(px(40.))
                    .flex_shrink_0()
                    .pl(px(88.))
                    .pr_3()
                    .flex()
                    .items_center()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, ev: &MouseDownEvent, window, _| {
                            if ev.click_count == 2 {
                                window.titlebar_double_click();
                            }
                        }),
                    )
                    .child(pixel_m3_mark(1.4)),
            )
            // Single Command Palette Trigger Bar
            .child(
                div()
                    .px_2p5()
                    .pb_2()
                    .child(
                        div()
                            .id("sidebar-cmd-trigger")
                            .px_2p5()
                            .py_1p5()
                            .rounded_md()
                            .bg(rgb(0x13131B))
                            .border_1()
                            .border_color(rgb(0x222230))
                            .hover(|s| s.bg(rgb(0x1A1A26)))
                            .active(|s| s.bg(rgb(BG_ACTIVE)))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_between()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_palette(window);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child("> 跳转 / 命令…"),
                            )
                            .child(kbd("⌘K", &self.fonts)),
                    ),
            )
            // Portfolio-Level Workspaces (Overview + Decision Journal)
            .child(
                div()
                    .px_2p5()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        div()
                            .px_2()
                            .mb_1()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED))
                            .child("工作台"),
                    )
                    .children(
                        nav_items
                            .into_iter()
                            .map(|(view, title, key, selected, rail_col)| {
                                div()
                                    .id(gpui::SharedString::from(format!("nav-{key}")))
                                    .px_2()
                                    .py_1p5()
                                    .mb_0p5()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(selected, |d| d.bg(rgba(RAIL_SELECTED)))
                                    .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                    .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if view == NavView::Baskets {
                                            this.focus_basket(None);
                                        } else {
                                            this.switch_view(view);
                                        }
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .w(px(2.))
                                                    .h(px(13.))
                                                    .rounded_full()
                                                    .when(selected, |d| d.bg(rgb(rail_col))),
                                            )
                                            .child(
                                                div()
                                                    .font_family(self.fonts.ui.clone())
                                                    .text_xs()
                                                    .font_weight(if selected {
                                                        gpui::FontWeight::SEMIBOLD
                                                    } else {
                                                        gpui::FontWeight::NORMAL
                                                    })
                                                    .text_color(rgb(if selected {
                                                        INK_HERO
                                                    } else {
                                                        INK_SECONDARY
                                                    }))
                                                    .child(title),
                                            ),
                                    )
                                    .child(kbd(key, &self.fonts))
                            }),
                    ),
            )
            // Unified Scrollable Basket Tree (Click header to filter basket; click ticker to open workspace)
            .child(
                div()
                    .id("rail-companies")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2p5()
                    .py_2p5()
                    .child(
                        div()
                            .px_2()
                            .mb_1p5()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED))
                            .child("芒格篮子"),
                    )
                    .children(Bucket::display_order().into_iter().map(|b| {
                        let members: Vec<&MungerReport> =
                            self.reports.iter().filter(|r| r.bucket == b).collect();
                        let cnt = members.len();
                        let (fg, _, _) = bucket_colors(b);
                        let bkt_active = self.active_view == NavView::Baskets
                            && self.basket_filter == Some(b);

                        div()
                            .mb_2()
                            .child(
                                div()
                                    .id(gpui::SharedString::from(format!("rail-bkt-{}", b.code())))
                                    .px_2()
                                    .py_1()
                                    .mb_0p5()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(bkt_active, |d| d.bg(rgba(RAIL_SELECTED)))
                                    .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                    .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if bkt_active {
                                            this.focus_basket(None);
                                        } else {
                                            this.focus_basket(Some(b));
                                        }
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
                                            .child(
                                                div()
                                                    .w(px(6.))
                                                    .h(px(6.))
                                                    .rounded_full()
                                                    .bg(rgb(fg)),
                                            )
                                            .child(
                                                div()
                                                    .font_family(self.fonts.mono_term.clone())
                                                    .text_size(px(10.))
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_color(rgb(if bkt_active {
                                                        fg
                                                    } else {
                                                        INK_PRIMARY
                                                    }))
                                                    .child(b.code()),
                                            )
                                            .child(
                                                div()
                                                    .font_family(self.fonts.ui.clone())
                                                    .text_size(px(11.))
                                                    .text_color(rgb(if bkt_active {
                                                        INK_HERO
                                                    } else {
                                                        INK_MUTED
                                                    }))
                                                    .child(b.short_label()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(10.))
                                            .text_color(rgb(if bkt_active {
                                                INK_HERO
                                            } else {
                                                INK_MUTED
                                            }))
                                            .child(format!("{cnt}")),
                                    ),
                            )
                            .children(members.into_iter().map(|rep| {
                                let is_selected = self.selected_ticker == rep.ticker
                                    && matches!(
                                        self.active_view,
                                        NavView::Dossier | NavView::Agent
                                    );
                                let dot_col = status_dot_color(rep.status);
                                let ticker = rep.ticker.clone();
                                let price_str = fmt_price(
                                    rep.price.as_ref().map(|p| p.price),
                                    &rep.currency,
                                );
                                div()
                                    .id(gpui::SharedString::from(format!("pick-{}", rep.ticker)))
                                    .pl_3p5()
                                    .pr_2()
                                    .py_1()
                                    .mb_0p5()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(is_selected, |d| d.bg(rgba(RAIL_SELECTED)))
                                    .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                    .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_company_contextual(&ticker);
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .w(px(5.))
                                                    .h(px(5.))
                                                    .rounded_full()
                                                    .bg(rgb(dot_col)),
                                            )
                                            .child(
                                                div()
                                                    .font_family(self.fonts.mono_num.clone())
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(if is_selected {
                                                        INK_HERO
                                                    } else {
                                                        INK_PRIMARY
                                                    }))
                                                    .child(rep.ticker.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(11.))
                                            .text_color(rgb(if is_selected {
                                                INK_SECONDARY
                                            } else {
                                                INK_RAIL_PRICE
                                            }))
                                            .child(price_str),
                                    )
                            }))
                    })),
            )
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let bench_val = match &self.benchmark {
            Some(b) => format!("{:.2}", b.price),
            None => "—".into(),
        };
        let sync_label = match &self.sync_status {
            Some(s) => fmt_time_cn(Some(s.synced_at_ms)),
            None => "未同步".into(),
        };
        let selected_name = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .map(|r| r.name.clone())
            .unwrap_or_default();

        let breadcrumb = match self.active_view {
            NavView::Baskets => match (self.basket_filter, &self.circle_filter) {
                (Some(b), Some(tag)) => {
                    format!("总览 / {} ({}) / #{tag}", b.code(), b.short_label())
                }
                (Some(b), None) => format!("总览 / {} ({})", b.code(), b.short_label()),
                (None, Some(tag)) => format!("总览 / #{tag}"),
                (None, None) => "总览".to_owned(),
            },
            NavView::Dossier | NavView::Agent => {
                format!("{} · {}", self.selected_ticker, selected_name)
            }
            NavView::Journal => "决策账本".to_owned(),
        };

        div()
            .h(px(40.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(BG_TOOLBAR))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .when(
                        matches!(self.active_view, NavView::Dossier | NavView::Agent),
                        |d| {
                            d.child(
                                div()
                                    .id("tb-back")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .active(|s| s.bg(rgb(BG_ACTIVE)))
                                    .cursor_pointer()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.switch_view(NavView::Baskets);
                                        cx.notify();
                                    }))
                                    .child("←"),
                            )
                        },
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_PRIMARY))
                            .child(breadcrumb),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, ev: &MouseDownEvent, window, _| {
                            if ev.click_count == 2 {
                                window.titlebar_double_click();
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_SURFACE))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child("S&P 500"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(bench_val),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(
                                if self.sync_status.is_some() {
                                    EMERALD_FG
                                } else {
                                    INK_MUTED
                                },
                            )))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(sync_label),
                            ),
                    )
                    .child(
                        div()
                            .id("sync-btn")
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .border_1()
                            .border_color(rgb(if self.syncing {
                                BORDER_SUBTLE
                            } else {
                                BORDER_PILL
                            }))
                            .bg(rgb(BG_WELL))
                            .hover(|s| s.bg(rgb(0x21212F)))
                            .active(|s| s.bg(rgb(BG_ACTIVE)))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_sync(cx);
                            }))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(if self.syncing {
                                        INK_MUTED
                                    } else {
                                        INK_PRIMARY
                                    }))
                                    .child(if self.syncing { "同步中…" } else { "同步" }),
                            )
                            .child(kbd("⌘R", &self.fonts)),
                    ),
            )
    }

    fn render_baskets_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let tags = collect_circle_tags(&self.reports);
        let pulse = compute_today_pulse(&self.reports);
        let has_any_filter = self.basket_filter.is_some()
            || self.circle_filter.is_some()
            || self.status_filter != StatusQuickFilter::All;

        let mut root = div().flex().flex_col();

        // Row 1: 38px Today's Pulse Strip (Self-Speaking Routine Beat 1) + [List | Board]
        let pulse_quiet = pulse.strike_tickers.is_empty()
            && pulse.red_core_tickers.is_empty()
            && pulse.watch_count == 0;

        let mut pulse_left = div()
            .flex()
            .items_center()
            .gap_2p5()
            .min_w_0()
            .child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .text_color(rgb(INK_MUTED))
                    .child("今日信号"),
            )
            .child(v_divider());

        if pulse_quiet {
            pulse_left = pulse_left.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(EMERALD_FG)))
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child("今日无球可挥 · 耐心等待"),
                    ),
            );
        } else {
            if let Some(first_strike) = pulse.strike_tickers.first().cloned() {
                let list_txt = pulse.strike_tickers.join(", ");
                pulse_left = pulse_left.child(
                    div()
                        .id("pulse-strike")
                        .px_2()
                        .py_0p5()
                        .rounded_xs()
                        .bg(rgba(EMERALD_BG))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_company(&first_strike, Some(NavView::Dossier));
                            cx.notify();
                        }))
                        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(EMERALD_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(EMERALD_FG))
                                .child(format!("击球区: {list_txt}")),
                        ),
                );
            } else {
                pulse_left = pulse_left.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child("0 击球"),
                );
            }

            pulse_left = pulse_left.child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child("·"),
            );

            if let Some(first_red) = pulse.red_core_tickers.first().cloned() {
                let list_txt = pulse.red_core_tickers.join(", ");
                pulse_left = pulse_left.child(
                    div()
                        .id("pulse-red")
                        .px_2()
                        .py_0p5()
                        .rounded_xs()
                        .bg(rgba(CORAL_BG))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_company(&first_red, Some(NavView::Dossier));
                            cx.notify();
                        }))
                        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(CORAL_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(CORAL_FG))
                                .child(format!("红灯复核: {list_txt}")),
                        ),
                );
            } else {
                pulse_left = pulse_left.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child("核心池无红灯"),
                );
            }

            pulse_left = pulse_left.child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child("·"),
            );

            if pulse.watch_count > 0 {
                let watch_cnt = pulse.watch_count;
                pulse_left = pulse_left.child(
                    div()
                        .id("pulse-watch")
                        .px_2()
                        .py_0p5()
                        .rounded_xs()
                        .bg(rgba(SKY_BG))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(SKY_FG))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.focus_basket(Some(Bucket::Watch));
                            cx.notify();
                        }))
                        .child(format!("○ 待分拣 {watch_cnt} 家")),
                );
            } else {
                pulse_left = pulse_left.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child("待审讯已清空"),
                );
            }
        }

        root = root.child(
            div()
                .h(px(38.))
                .px_5()
                .bg(rgb(BG_SURFACE))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(pulse_left)
                .child(
                    seg_group()
                        .child(seg_tab(
                            "bkt-mode-list",
                            "列表",
                            self.baskets_view_mode == BasketsViewMode::List,
                            None,
                            false,
                            &self.fonts,
                            cx.listener(|this, _, _, cx| {
                                this.baskets_view_mode = BasketsViewMode::List;
                                this.cursor_index = 0;
                                cx.notify();
                            }),
                        ))
                        .child(seg_tab(
                            "bkt-mode-board",
                            "看板",
                            self.baskets_view_mode == BasketsViewMode::Board,
                            None,
                            false,
                            &self.fonts,
                            cx.listener(|this, _, _, cx| {
                                this.baskets_view_mode = BasketsViewMode::Board;
                                this.cursor_index = 0;
                                cx.notify();
                            }),
                        ))
                        .child(div().px_1().child(kbd("v", &self.fonts))),
                ),
        );

        // Row 2: 36px Basket Focus Tabs + Quiet Status Filters
        let total_cnt = self.reports.len();
        root = root.child(
            div()
                .h(px(36.))
                .px_5()
                .bg(rgb(BG_ROW_ALT))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child({
                            let active = self.basket_filter.is_none();
                            div()
                                .id("tab-bkt-all")
                                .px_2()
                                .py_0p5()
                                .rounded_xs()
                                .cursor_pointer()
                                .when(active, |d| {
                                    d.bg(rgb(BG_HOVER))
                                        .border_1()
                                        .border_color(rgb(BORDER_PILL))
                                })
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .flex()
                                .items_center()
                                .gap_1()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.focus_basket(None);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .font_weight(if active {
                                            gpui::FontWeight::SEMIBOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(rgb(if active { INK_HERO } else { INK_MUTED }))
                                        .child("全部"),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(if active {
                                            INK_PRIMARY
                                        } else {
                                            INK_MUTED
                                        }))
                                        .child(format!("{total_cnt}")),
                                )
                        })
                        .children(Bucket::display_order().into_iter().map(|b| {
                            let (fg, bg, br) = bucket_colors(b);
                            let active = self.basket_filter == Some(b);
                            let cnt = self.reports.iter().filter(|r| r.bucket == b).count();
                            div()
                                .id(gpui::SharedString::from(format!("tab-bkt-{}", b.code())))
                                .px_2()
                                .py_0p5()
                                .rounded_xs()
                                .cursor_pointer()
                                .when(active, |d| {
                                    d.bg(rgba(bg)).border_1().border_color(rgba(br))
                                })
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .flex()
                                .items_center()
                                .gap_1()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if active {
                                        this.focus_basket(None);
                                    } else {
                                        this.focus_basket(Some(b));
                                    }
                                    cx.notify();
                                }))
                                .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(fg)))
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_size(px(11.))
                                        .font_weight(if active {
                                            gpui::FontWeight::BOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(rgb(if active { fg } else { INK_SECONDARY }))
                                        .child(b.code()),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(if active { INK_HERO } else { INK_MUTED }))
                                        .child(format!("{cnt}")),
                                )
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .children(
                            [StatusQuickFilter::StrikeOnly, StatusQuickFilter::WarnedOnly]
                                .into_iter()
                                .map(|sf| {
                                    let active = self.status_filter == sf;
                                    div()
                                        .id(gpui::SharedString::from(format!("sf-{}", sf.label())))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_xs()
                                        .cursor_pointer()
                                        .when(active, |d| {
                                            d.bg(rgb(BG_HOVER))
                                                .border_1()
                                                .border_color(rgb(BORDER_PILL))
                                        })
                                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .text_color(rgb(if active { INK_HERO } else { INK_MUTED }))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_status_filter(sf);
                                            cx.notify();
                                        }))
                                        .child(sf.label())
                                }),
                        )
                        .when(has_any_filter, |d| {
                            d.child(
                                div()
                                    .id("clear-all-filters")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_xs()
                                    .bg(rgba(GOLD_BG))
                                    .border_1()
                                    .border_color(rgba(GOLD_BORDER))
                                    .cursor_pointer()
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(GOLD_FG))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.basket_filter = None;
                                        this.circle_filter = None;
                                        this.status_filter = StatusQuickFilter::All;
                                        this.cursor_index = 0;
                                        cx.notify();
                                    }))
                                    .child("× 重置"),
                            )
                        }),
                ),
        );

        // Bare Quiet #能力圈 Tag Line (no background strip)
        if !tags.is_empty() {
            root = root.child(
                div()
                    .px_5()
                    .pt_3()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_1p5()
                    .children(tags.into_iter().map(|(tag, cnt)| {
                        let active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_click = tag.clone();
                        circle_tag_button(
                            format!("tag-bar-{tag}"),
                            tag,
                            active,
                            true,
                            Some(cnt),
                            &self.fonts,
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_circle_filter(&tag_click);
                                cx.notify();
                            }),
                        )
                    })),
            );
        }

        if self.baskets_view_mode == BasketsViewMode::Board {
            return root.child(self.render_baskets_board(cx));
        }

        // Collapsible Grouped Basket Cards
        let mut linear_idx: usize = 0;
        let headers: &[(&str, f32, bool, Option<SortCol>)] = &[
            ("标的 / 能力圈", 0., false, Some(SortCol::Ticker)),
            ("现价", 90., true, Some(SortCol::Price)),
            ("击球价", 90., true, None),
            ("距击球价", 90., true, Some(SortCol::Margin)),
            ("5Y ROIC", 84., true, Some(SortCol::Roic)),
            ("严苛 FCF%", 88., true, Some(SortCol::FcfYield)),
            ("3Y 股本Δ", 84., true, Some(SortCol::ShareDelta)),
            ("净债/OE", 82., true, Some(SortCol::NetDebt)),
            ("状态", 112., false, None),
        ];

        let mut cards = div().px_5().pt_3().pb_5().flex().flex_col().gap_3();
        for bucket in self.visible_baskets_order() {
            let (fg, _, _) = bucket_colors(bucket);
            let is_collapsed =
                self.basket_filter.is_none() && self.collapsed_baskets.contains(&bucket);
            let rows = filter_reports_for_basket(
                &self.reports,
                bucket,
                self.circle_filter.as_deref(),
                self.status_filter,
                self.sort_col,
                self.sort_asc,
            );
            let row_cnt = rows.len();
            let collapsed_preview = if is_collapsed && !rows.is_empty() {
                Some(
                    rows.iter()
                        .map(|r| r.ticker.as_str())
                        .collect::<Vec<_>>()
                        .join(" · "),
                )
            } else {
                None
            };

            let mut block = card_shell().child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "bkt-hdr-{}",
                        bucket.code()
                    )))
                    .px_4()
                    .py_2p5()
                    .bg(rgb(BG_WELL))
                    .when(!is_collapsed, |d| {
                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.basket_filter.is_none() {
                            if this.collapsed_baskets.contains(&bucket) {
                                this.collapsed_baskets.remove(&bucket);
                            } else {
                                this.collapsed_baskets.insert(bucket);
                            }
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .when(self.basket_filter.is_none(), |d| {
                                d.child(
                                    div()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .child(if is_collapsed { "▸" } else { "▾" }),
                                )
                            })
                            .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(fg)))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(fg))
                                    .child(bucket.code()),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(bucket.short_label()),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("{row_cnt}")),
                            ),
                    )
                    .when_some(collapsed_preview, |d, prev| {
                        d.child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .text_color(rgb(INK_MUTED))
                                .child(prev),
                        )
                    }),
            );

            if !is_collapsed {
                if rows.is_empty() {
                    let empty_msg = if has_any_filter {
                        format!(
                            "当前筛选下 [{} · {}] 暂无匹配标的。",
                            bucket.code(),
                            bucket.short_label()
                        )
                    } else {
                        format!("当前 [{} · {}] 暂无标的。", bucket.code(), bucket.short_label())
                    };
                    block = block.child(
                        div()
                            .px_4()
                            .py_3p5()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(empty_msg),
                    );
                } else if matches!(bucket, Bucket::Core | Bucket::Watch) {
                    block = block.child(
                        div()
                            .flex()
                            .items_center()
                            .px_4()
                            .py_2()
                            .bg(rgb(BG_ROW_ALT))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .children(headers.iter().map(|(label, w, right, sort_opt)| {
                                let is_sorted = sort_opt.is_some_and(|c| c == self.sort_col);
                                let arrow = if is_sorted {
                                    if self.sort_asc { " ↑" } else { " ↓" }
                                } else {
                                    ""
                                };
                                let is_nd = *label == "净债/OE";
                                let is_status = *label == "状态";
                                div()
                                    .id(gpui::SharedString::from(format!(
                                        "hdr-{}-{label}",
                                        bucket.code()
                                    )))
                                    .px_1p5()
                                    .when(is_nd, |d| d.pr_4())
                                    .when(is_status, |d| d.pl_3())
                                    .flex()
                                    .items_center()
                                    .when(*right, |d| d.justify_end())
                                    .gap_1()
                                    .when(*w > 0., |d| d.w(px(*w)).flex_shrink_0())
                                    .when(*w == 0., |d| d.flex_1().min_w_0())
                                    .when_some(*sort_opt, |d, col| {
                                        d.cursor_pointer()
                                            .hover(|s| s.text_color(rgb(INK_PRIMARY)))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle_sort(col);
                                                cx.notify();
                                            }))
                                    })
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_size(px(11.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(if is_sorted {
                                                INK_PRIMARY
                                            } else {
                                                INK_MUTED
                                            }))
                                            .child(*label),
                                    )
                                    .when(!arrow.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_size(px(10.))
                                                .text_color(rgb(INK_HERO))
                                                .child(arrow),
                                        )
                                    })
                            })),
                    );

                    for (local_idx, rep) in rows.into_iter().enumerate() {
                        let global_idx = linear_idx;
                        linear_idx += 1;

                        let val = primary_val(rep, true);
                        let (s_fg, s_bg, s_br) = status_colors(rep.status);
                        let dist_opt = val.and_then(|v| v.distance_to_strike_pct);
                        let (dist_str, _) = fmt_pct_clean(dist_opt, true);
                        let dist_col = match dist_opt {
                            Some(d) if d <= Decimal::ZERO => EMERALD_FG,
                            Some(_) => INK_SECONDARY,
                            None => INK_MUTED,
                        };
                        let (roic_str, _) = fmt_pct_clean(rep.roic_5y_avg_pct, false);
                        let roic_col = match rep.roic_5y_avg_pct {
                            Some(v) if v >= Decimal::from(15) => INK_PRIMARY,
                            Some(_) => GOLD_FG,
                            None => INK_MUTED,
                        };
                        let yield_opt = val.and_then(|v| v.strict_fcf_yield_mcap_pct);
                        let (yield_str, yield_pol) = fmt_pct_clean(yield_opt, false);
                        let yield_col = match yield_opt {
                            Some(_) if yield_pol < 0 => CORAL_FG,
                            Some(_) => INK_PRIMARY,
                            None => INK_MUTED,
                        };
                        let (share_str, share_pol) = fmt_pct_clean(rep.share_change_3y_pct, true);
                        let share_col = share_delta_color(rep.share_change_3y_pct, share_pol);
                        let nd_oe = rep
                            .net_debt_to_owner_earnings
                            .map(|v| format!("{v:.2}x"))
                            .unwrap_or_else(|| "—".into());
                        let nd_col = if rep.net_debt_to_owner_earnings.is_some() {
                            INK_SECONDARY
                        } else {
                            INK_MUTED
                        };

                        let is_cursor = global_idx == self.cursor_index;
                        let row_bg = if is_cursor {
                            BG_SELECTED_ROW
                        } else if local_idx % 2 == 0 {
                            BG_VIEWPORT
                        } else {
                            BG_ROW_ALT
                        };
                        let ticker = rep.ticker.clone();
                        let tag = primary_circle_tag(&rep.circle);
                        let tag_active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_for_click = tag.clone();
                        let sub_detail = secondary_circle_detail(&rep.circle);

                        block = block.child(
                            div()
                                .id(gpui::SharedString::from(format!("wl-row-{}", rep.ticker)))
                                .relative()
                                .flex()
                                .items_center()
                                .px_4()
                                .py_2p5()
                                .bg(rgb(row_bg))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .active(|s| s.bg(rgb(BG_ACTIVE)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cursor_index = global_idx;
                                    this.select_company(&ticker, Some(NavView::Dossier));
                                    cx.notify();
                                }))
                                .when(is_cursor, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .bottom_0()
                                            .w(px(2.))
                                            .bg(rgb(INK_HERO)),
                                    )
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .px_1p5()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.mono_num.clone())
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_color(rgb(INK_HERO))
                                                        .child(rep.ticker.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.ui.clone())
                                                        .text_xs()
                                                        .text_color(rgb(INK_PRIMARY))
                                                        .truncate()
                                                        .child(rep.name.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .mt_1()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(circle_tag_button(
                                                    format!("row-tag-{}", rep.ticker),
                                                    tag,
                                                    tag_active,
                                                    false,
                                                    None,
                                                    &self.fonts,
                                                    cx.listener(move |this, _, _, cx| {
                                                        this.toggle_circle_filter(&tag_for_click);
                                                        cx.notify();
                                                    }),
                                                ))
                                                .when_some(sub_detail, |d, det| {
                                                    d.child(
                                                        div()
                                                            .font_family(self.fonts.ui.clone())
                                                            .text_size(px(11.))
                                                            .text_color(rgb(INK_MUTED))
                                                            .truncate()
                                                            .child(det),
                                                    )
                                                }),
                                        ),
                                )
                                .child(mono_right(
                                    fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency),
                                    90.,
                                    INK_HERO,
                                    &self.fonts,
                                ))
                                .child(mono_right(
                                    fmt_price(val.and_then(|v| v.strike_price), &rep.currency),
                                    90.,
                                    if val.and_then(|v| v.strike_price).is_some() {
                                        INK_SECONDARY
                                    } else {
                                        INK_MUTED
                                    },
                                    &self.fonts,
                                ))
                                .child(mono_right(dist_str, 90., dist_col, &self.fonts))
                                .child(mono_right(roic_str, 84., roic_col, &self.fonts))
                                .child(mono_right(yield_str, 88., yield_col, &self.fonts))
                                .child(mono_right(share_str, 84., share_col, &self.fonts))
                                .child(
                                    div()
                                        .w(px(82.))
                                        .flex_shrink_0()
                                        .pl_1p5()
                                        .pr_4()
                                        .text_right()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .text_color(rgb(nd_col))
                                        .truncate()
                                        .child(nd_oe),
                                )
                                .child(
                                    div()
                                        .w(px(112.))
                                        .flex_shrink_0()
                                        .pl_3()
                                        .pr_1p5()
                                        .flex()
                                        .items_center()
                                        .child(badge(
                                            rep.status.label().to_owned(),
                                            s_fg,
                                            s_bg,
                                            s_br,
                                            &self.fonts,
                                        )),
                                ),
                        );
                    }
                } else {
                    // OUT & TOO HARD: Compact Decision Specimen Cards
                    for (local_idx, rep) in rows.into_iter().enumerate() {
                        let global_idx = linear_idx;
                        linear_idx += 1;

                        let is_cursor = global_idx == self.cursor_index;
                        let row_bg = if is_cursor {
                            BG_SELECTED_ROW
                        } else if local_idx % 2 == 0 {
                            BG_VIEWPORT
                        } else {
                            BG_ROW_ALT
                        };
                        let ticker = rep.ticker.clone();
                        let tag = primary_circle_tag(&rep.circle);
                        let tag_active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_for_click = tag.clone();
                        let latest_row = rep.ttm_row.as_ref().or_else(|| rep.annual_rows.last());
                        let price_txt =
                            fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency);
                        let oe_txt = latest_row
                            .map(|r| fmt_money_compact(r.strict_owner_earnings, &rep.currency))
                            .unwrap_or_else(|| "—".into());

                        block = block.child(
                            div()
                                .id(gpui::SharedString::from(format!("sp-row-{}", rep.ticker)))
                                .relative()
                                .px_4()
                                .py_3()
                                .bg(rgb(row_bg))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cursor_index = global_idx;
                                    this.select_company(&ticker, Some(NavView::Dossier));
                                    cx.notify();
                                }))
                                .when(is_cursor, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top_0()
                                            .bottom_0()
                                            .w(px(2.))
                                            .bg(rgb(INK_HERO)),
                                    )
                                })
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.mono_num.clone())
                                                        .text_sm()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_color(rgb(INK_HERO))
                                                        .child(rep.ticker.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.ui.clone())
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .text_color(rgb(INK_PRIMARY))
                                                        .child(rep.name.clone()),
                                                )
                                                .child(circle_tag_button(
                                                    format!("sp-tag-{}", rep.ticker),
                                                    tag,
                                                    tag_active,
                                                    false,
                                                    None,
                                                    &self.fonts,
                                                    cx.listener(move |this, _, _, cx| {
                                                        this.toggle_circle_filter(&tag_for_click);
                                                        cx.notify();
                                                    }),
                                                )),
                                        )
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_num.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_SECONDARY))
                                                .child(format!(
                                                    "现价 {price_txt} · 严苛 OE {oe_txt}"
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .mt_1p5()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(13.))
                                        .line_height(px(20.))
                                        .text_color(rgb(INK_SECONDARY))
                                        .child(
                                            rep.too_hard_reason
                                                .clone()
                                                .unwrap_or_else(|| "已拒绝或能力圈外。".into()),
                                        ),
                                ),
                        );
                    }
                }
            }

            cards = cards.child(block);
        }

        root.child(cards)
    }

    fn render_baskets_board(&self, cx: &mut Context<Self>) -> gpui::Div {
        let board_order = [Bucket::Watch, Bucket::Core, Bucket::Out, Bucket::TooHard];
        let mut linear_idx: usize = 0;

        div()
            .px_5()
            .pt_3()
            .pb_5()
            .flex()
            .gap_3()
            .children(board_order.into_iter().map(|bucket| {
                let (fg, bg, br) = bucket_colors(bucket);
                let is_focused = self.basket_filter == Some(bucket);
                let rows = filter_reports_for_basket(
                    &self.reports,
                    bucket,
                    self.circle_filter.as_deref(),
                    self.status_filter,
                    self.sort_col,
                    self.sort_asc,
                );
                let cnt = rows.len();

                let col = div()
                    .flex_1()
                    .min_w_0()
                    .rounded_lg()
                    .bg(rgb(BG_ROW_ALT))
                    .border_1()
                    .border_color(rgba(if is_focused {
                        br
                    } else {
                        BORDER_SUBTLE << 8 | 0xFF
                    }))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_3()
                            .py_2p5()
                            .bg(rgba(if is_focused { bg } else { 0x161620FF }))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(fg)))
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(fg))
                                            .child(bucket.code()),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child(bucket.short_label()),
                                    ),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(format!("{cnt}")),
                            ),
                    );

                let mut body = div().p_2p5().flex().flex_col().gap_2();
                if rows.is_empty() {
                    body = body.child(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(BG_VIEWPORT))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED))
                            .child("暂无标的"),
                    );
                } else {
                    for rep in rows {
                        let global_idx = linear_idx;
                        linear_idx += 1;

                        let is_cursor = global_idx == self.cursor_index;
                        let val = primary_val(rep, true);
                        let (s_fg, s_bg, s_br) = status_colors(rep.status);
                        let price_txt =
                            fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency);
                        let dist_txt =
                            fmt_pct_clean(val.and_then(|v| v.distance_to_strike_pct), true).0;
                        let roic_txt = fmt_pct_clean(rep.roic_5y_avg_pct, false).0;
                        let tag = primary_circle_tag(&rep.circle);
                        let tag_active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_click = tag.clone();
                        let ticker = rep.ticker.clone();

                        body = body.child(
                            div()
                                .id(gpui::SharedString::from(format!("brd-card-{}", rep.ticker)))
                                .p_2p5()
                                .rounded_md()
                                .bg(rgb(if is_cursor {
                                    BG_SELECTED_ROW
                                } else {
                                    BG_SURFACE
                                }))
                                .border_1()
                                .border_color(rgb(if is_cursor {
                                    INK_HERO
                                } else {
                                    BORDER_SUBTLE
                                }))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cursor_index = global_idx;
                                    this.select_company(&ticker, Some(NavView::Dossier));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(
                                                    div()
                                                        .w(px(6.))
                                                        .h(px(6.))
                                                        .rounded_full()
                                                        .bg(rgb(status_dot_color(rep.status))),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.mono_num.clone())
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_color(rgb(INK_HERO))
                                                        .child(rep.ticker.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_num.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_PRIMARY))
                                                .child(price_txt),
                                        ),
                                )
                                .child(
                                    div()
                                        .mt_0p5()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .truncate()
                                        .child(rep.name.clone()),
                                )
                                .child(match bucket {
                                    Bucket::Core | Bucket::Watch => div()
                                        .mt_2()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_num.clone())
                                                .text_size(px(10.))
                                                .text_color(rgb(INK_MUTED))
                                                .truncate()
                                                .child(format!("边际 {dist_txt} · ROIC {roic_txt}")),
                                        )
                                        .child(badge(
                                            rep.status.label().to_owned(),
                                            s_fg,
                                            s_bg,
                                            s_br,
                                            &self.fonts,
                                        )),
                                    Bucket::Out | Bucket::TooHard => div()
                                        .mt_2()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .line_height(px(16.))
                                        .text_color(rgb(INK_MUTED))
                                        .child(
                                            rep.too_hard_reason
                                                .clone()
                                                .unwrap_or_else(|| "已拒绝或能力圈外".into()),
                                        ),
                                })
                                .child(
                                    div()
                                        .mt_2()
                                        .pt_1p5()
                                        .border_t_1()
                                        .border_color(rgb(BORDER_HAIRLINE))
                                        .flex()
                                        .items_center()
                                        .child(circle_tag_button(
                                            format!("brd-tag-{}", rep.ticker),
                                            tag,
                                            tag_active,
                                            false,
                                            None,
                                            &self.fonts,
                                            cx.listener(move |this, _, _, cx| {
                                                this.toggle_circle_filter(&tag_click);
                                                cx.notify();
                                            }),
                                        )),
                                ),
                        );
                    }
                }

                col.child(body)
            }))
    }

    fn render_kline_card(
        &self,
        rep: &MungerReport,
        row_opt: Option<&AnnualMetricRow>,
        strike_price: Option<Decimal>,
        sandbox: &SandboxResult,
        sandbox_dirty: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let bars: Vec<OhlcBar> = match self.chart_timeframe {
            ChartTimeframe::FiveYearWeekly => self
                .selected_history_wk
                .as_ref()
                .map(|h| h.bars.clone())
                .unwrap_or_default(),
            ChartTimeframe::ThreeYearWeekly => self
                .selected_history_wk
                .as_ref()
                .map(|h| {
                    let keep = 156usize.min(h.bars.len());
                    h.bars[h.bars.len() - keep..].to_vec()
                })
                .unwrap_or_default(),
            ChartTimeframe::OneYearDaily => self
                .selected_history_1d
                .as_ref()
                .map(|h| h.bars.clone())
                .unwrap_or_default(),
        };

        let fv_high = strike_price.and_then(|s| {
            if self.sandbox_multiple > Decimal::ZERO {
                Some(s * (self.sandbox_multiple + Decimal::from(4)) / self.sandbox_multiple)
            } else {
                None
            }
        });
        let domain = compute_kline_domain(&bars, strike_price, fv_high);

        let last_bar = bars.last();
        let ohlc_readout = match last_bar {
            Some(b) => {
                let pfx = currency_prefix(&rep.currency);
                format!(
                    "O {pfx}{:.2}   H {pfx}{:.2}   L {pfx}{:.2}   C {pfx}{:.2}",
                    b.open, b.high, b.low, b.close
                )
            }
            None => "暂无本地 K 线缓存".into(),
        };

        let peak_high = bars.iter().map(|b| b.high).fold(Decimal::ZERO, Decimal::max);
        let drawdown_pct = match (last_bar, peak_high) {
            (Some(b), pk) if pk > Decimal::ZERO => Some(((b.close - pk) / pk) * Decimal::from(100)),
            _ => None,
        };
        let (dd_str, _) = fmt_pct_clean(drawdown_pct, true);

        let cur_price = rep.price.as_ref().map(|p| p.price);
        let dist_to_strike = match (cur_price, strike_price) {
            (Some(p), Some(s)) if s > Decimal::ZERO => Some(((p - s) / s) * Decimal::from(100)),
            _ => None,
        };
        let (strike_dist_str, _) = fmt_pct_clean(dist_to_strike, true);

        let strike_f64 = strike_price.and_then(dec_to_f64);
        let fv_high_f64 = fv_high.and_then(dec_to_f64);
        let strike_below_visible = match (domain, strike_f64) {
            (Some((y_min, _, _)), Some(s)) => s < y_min,
            _ => false,
        };

        let date_ticks: Vec<String> = if bars.is_empty() {
            Vec::new()
        } else {
            let n = bars.len();
            [0, n / 4, n / 2, (n * 3) / 4, n - 1]
                .into_iter()
                .map(|idx| civil_ym_from_ms(bars[idx].ts_ms))
                .collect()
        };

        let mut card = card_shell().p_4().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            seg_group().children(
                                [
                                    ChartTimeframe::FiveYearWeekly,
                                    ChartTimeframe::ThreeYearWeekly,
                                    ChartTimeframe::OneYearDaily,
                                ]
                                .into_iter()
                                .map(|tf| {
                                    seg_tab(
                                        format!("tf-{}", tf.label()),
                                        tf.label(),
                                        self.chart_timeframe == tf,
                                        None,
                                        true,
                                        &self.fonts,
                                        cx.listener(move |this, _, _, cx| {
                                            this.chart_timeframe = tf;
                                            cx.notify();
                                        }),
                                    )
                                }),
                            ),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .text_color(rgb(INK_SECONDARY))
                                .child(ohlc_readout),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap_1()
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .text_color(rgb(INK_MUTED))
                                        .child("高点回撤"),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .child(dd_str),
                                ),
                        )
                        .child(v_divider())
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap_1()
                                .child(div().w(px(8.)).h(px(2.)).bg(rgb(EMERALD_FG)))
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .text_color(rgb(INK_MUTED))
                                        .child("击球线"),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(if strike_price.is_some() {
                                            EMERALD_FG
                                        } else {
                                            INK_MUTED
                                        }))
                                        .child(fmt_price(strike_price, &rep.currency)),
                                )
                                .when(dist_to_strike.is_some(), |d| {
                                    d.child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(11.))
                                            .text_color(rgb(INK_MUTED))
                                            .child(format!("({strike_dist_str})")),
                                    )
                                }),
                        ),
                ),
        );

        if let Some((y_min, y_max, max_vol)) = domain {
            let pfx = currency_prefix(&rep.currency);
            let y_ticks = [
                format!("{pfx}{:.2}", y_max),
                format!("{pfx}{:.2}", y_min + (y_max - y_min) * 0.667),
                format!("{pfx}{:.2}", y_min + (y_max - y_min) * 0.333),
                format!("{pfx}{:.2}", y_min),
            ];
            let bars_paint = bars.clone();

            card = card
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .h(px(228.))
                                .rounded_md()
                                .bg(rgb(BG_VIEWPORT))
                                .border_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .overflow_hidden()
                                .child(
                                    canvas(
                                        |_, _, _| (),
                                        move |b, _, window, _| {
                                            let ox = f32::from(b.origin.x);
                                            let oy = f32::from(b.origin.y);
                                            let w = f32::from(b.size.width);
                                            let h = f32::from(b.size.height);
                                            if w <= 20.0 || h <= 40.0 || bars_paint.is_empty() {
                                                return;
                                            }
                                            let pad_x = 10.0f32;
                                            let plot_w = (w - pad_x * 2.0).max(10.0);
                                            let price_top = oy + 8.0;
                                            let price_h = h * 0.72;
                                            let vol_top = oy + h * 0.78;
                                            let vol_h = h * 0.19;
                                            let y_span = (y_max - y_min).max(1e-6);
                                            let price_to_y = |p: f64| -> f32 {
                                                let ratio = ((p - y_min) / y_span).clamp(0.0, 1.0);
                                                price_top + price_h - (ratio as f32) * price_h
                                            };

                                            for frac in [0.0f32, 0.333, 0.667, 1.0] {
                                                let gy = price_top + price_h * frac;
                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(px(ox), px(gy)),
                                                        size(px(w), px(1.)),
                                                    ),
                                                    rgb(BORDER_HAIRLINE),
                                                ));
                                            }

                                            if let Some(s) = strike_f64 {
                                                let band_top = fv_high_f64.unwrap_or(s);
                                                if band_top >= y_min && s <= y_max {
                                                    let y_hi = price_to_y(band_top.min(y_max));
                                                    let y_lo = price_to_y(s.max(y_min));
                                                    let band_h = (y_lo - y_hi).max(2.0);
                                                    window.paint_quad(fill(
                                                        bounds(
                                                            point(px(ox), px(y_hi)),
                                                            size(px(w), px(band_h)),
                                                        ),
                                                        rgba(EMERALD_BAND),
                                                    ));
                                                }
                                            }

                                            let n = bars_paint.len() as f32;
                                            let step = plot_w / n;
                                            let body_w = (step * 0.62).clamp(1.2, 8.0);
                                            let max_v_f = (max_vol as f64).max(1.0);

                                            for (idx, bar) in bars_paint.iter().enumerate() {
                                                let cx_p = ox + pad_x + (idx as f32 + 0.5) * step;
                                                let (Some(o), Some(hi), Some(lo), Some(c)) = (
                                                    dec_to_f64(bar.open),
                                                    dec_to_f64(bar.high),
                                                    dec_to_f64(bar.low),
                                                    dec_to_f64(bar.close),
                                                ) else {
                                                    continue;
                                                };
                                                let is_up = c >= o;
                                                let col = if is_up { EMERALD_FG } else { CORAL_FG };
                                                let y_hi = price_to_y(hi);
                                                let y_lo = price_to_y(lo);
                                                let y_o = price_to_y(o);
                                                let y_c = price_to_y(c);

                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(px(cx_p - 0.5), px(y_hi)),
                                                        size(px(1.0), px((y_lo - y_hi).max(1.0))),
                                                    ),
                                                    rgb(col),
                                                ));
                                                let body_top = y_o.min(y_c);
                                                let body_h = (y_o - y_c).abs().max(1.2);
                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(px(cx_p - body_w * 0.5), px(body_top)),
                                                        size(px(body_w), px(body_h)),
                                                    ),
                                                    rgb(col),
                                                ));

                                                let vh = ((bar.volume as f64 / max_v_f)
                                                    * vol_h as f64)
                                                    as f32;
                                                let vh = vh.max(1.0).min(vol_h);
                                                let vy = vol_top + (vol_h - vh);
                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(px(cx_p - body_w * 0.5), px(vy)),
                                                        size(px(body_w), px(vh)),
                                                    ),
                                                    rgba(if is_up { EMERALD_VOL } else { CORAL_VOL }),
                                                ));
                                            }

                                            if let Some(s) = strike_f64
                                                && s >= y_min
                                                && s <= y_max
                                            {
                                                let sy = price_to_y(s);
                                                let mut dx = ox;
                                                while dx < ox + w {
                                                    let seg_w = (ox + w - dx).min(6.0);
                                                    window.paint_quad(fill(
                                                        bounds(
                                                            point(px(dx), px(sy - 0.75)),
                                                            size(px(seg_w), px(1.5)),
                                                        ),
                                                        rgba(EMERALD_LINE),
                                                    ));
                                                    dx += 10.0;
                                                }
                                            }
                                        },
                                    )
                                    .size_full(),
                                )
                                .when(strike_below_visible, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .bottom_2()
                                            .right_2()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_xs()
                                            .bg(rgba(EMERALD_BG))
                                            .border_1()
                                            .border_color(rgba(EMERALD_BORDER))
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(10.))
                                            .text_color(rgb(EMERALD_FG))
                                            .child(format!(
                                                "↓ 击球线 {} (低于区间底价)",
                                                fmt_price(strike_price, &rep.currency)
                                            )),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .w(px(72.))
                                .h(px(228.))
                                .flex_shrink_0()
                                .py_1()
                                .flex()
                                .flex_col()
                                .justify_between()
                                .items_end()
                                .children(y_ticks.into_iter().map(|lbl| {
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(INK_MUTED))
                                        .child(lbl)
                                })),
                        ),
                )
                .child(
                    div()
                        .pr(px(80.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .children(date_ticks.into_iter().map(|dt| {
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_size(px(10.))
                                .text_color(rgb(INK_MUTED))
                                .child(dt)
                        })),
                );
        } else {
            card = card.child(
                div()
                    .h(px(120.))
                    .rounded_md()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2p5()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(format!("尚未同步 {} 的历史 K 线缓存", rep.ticker)),
                    )
                    .child(
                        div()
                            .id("kline-sync-now")
                            .px_3()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_PILL))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_PRIMARY))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_sync(cx);
                            }))
                            .child("同步行情 K 线"),
                    ),
            );
        }

        // Docked 1-Row Inline Charlie Sandbox Footer
        if row_opt.is_some() {
            let (sb_imp_str, _) = fmt_pct_clean(sandbox.implied_growth_for_target_pct, true);
            let (sb_exp_str, _) = fmt_pct_clean(sandbox.expected_total_return_pct, false);

            card = card.child(
                div()
                    .pt_2p5()
                    .border_t_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_3()
                            .child(stepper_inline(
                                "倍数",
                                format!("{}x", self.sandbox_multiple),
                                "sb-mult",
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_multiple > Decimal::from(5) {
                                        this.sandbox_multiple -= Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_multiple < Decimal::from(45) {
                                        this.sandbox_multiple += Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(stepper_inline(
                                "目标",
                                format!("{}%", self.sandbox_target_return),
                                "sb-ret",
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_target_return > Decimal::from(5) {
                                        this.sandbox_target_return -= Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_target_return < Decimal::from(25) {
                                        this.sandbox_target_return += Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(stepper_inline(
                                "增速",
                                format!("{}%", self.sandbox_expected_growth),
                                "sb-gro",
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_expected_growth > Decimal::from(-10) {
                                        this.sandbox_expected_growth -= Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_expected_growth < Decimal::from(25) {
                                        this.sandbox_expected_growth += Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .child("隐含增速"),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(GOLD_FG))
                                            .child(sb_imp_str),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .child("预期年化"),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child(sb_exp_str),
                                    ),
                            )
                            .when(sandbox_dirty, |d| {
                                d.child(
                                    div()
                                        .id("reset-sandbox")
                                        .px_2()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(rgb(BG_VIEWPORT))
                                        .border_1()
                                        .border_color(rgb(BORDER_STRONG))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reset_sandbox_to_saved();
                                            cx.notify();
                                        }))
                                        .child("重置"),
                                )
                            })
                            .child(
                                div()
                                    .id("save-sandbox")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .when(sandbox_dirty, |d| {
                                        d.bg(rgb(CTA_BG))
                                            .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                            .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                                            .text_color(rgb(CTA_INK))
                                    })
                                    .when(!sandbox_dirty, |d| {
                                        d.bg(rgb(BG_WELL))
                                            .border_1()
                                            .border_color(rgb(BORDER_PILL))
                                            .hover(|s| s.bg(rgb(BG_HOVER)))
                                            .text_color(rgb(INK_MUTED))
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_sandbox_assumptions();
                                        cx.notify();
                                    }))
                                    .child(if sandbox_dirty {
                                        "保存假设"
                                    } else {
                                        "已保存"
                                    }),
                            ),
                    ),
            );
        }

        card
    }

    fn render_company_shell(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(rep) = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
        else {
            return div()
                .p_6()
                .font_family(self.fonts.ui.clone())
                .text_color(rgb(INK_SECONDARY))
                .child("未选择公司");
        };

        let row_opt = primary_row(rep, self.use_ttm_basis);
        let val_opt = primary_val(rep, self.use_ttm_basis);
        let cur_price = rep.price.as_ref().map(|p| p.price);
        let oe_per_share = val_opt.and_then(|v| v.owner_earnings_per_share);
        let sandbox = calculator::evaluate_sandbox(
            oe_per_share,
            cur_price,
            self.sandbox_multiple,
            self.sandbox_target_return,
            self.sandbox_expected_growth,
        );
        let sandbox_dirty = self.sandbox_multiple != rep.assumptions.multiple_low
            || self.sandbox_target_return != rep.assumptions.target_return_pct
            || self.sandbox_expected_growth != rep.assumptions.expected_growth_pct;

        let active_strike = sandbox
            .fair_value
            .or_else(|| val_opt.and_then(|v| v.strike_price));
        let dist_live = match (cur_price, active_strike) {
            (Some(p), Some(s)) if s > Decimal::ZERO => Some(((p - s) / s) * Decimal::from(100)),
            _ => val_opt.and_then(|v| v.distance_to_strike_pct),
        };
        let (dist_live_str, _) = fmt_pct_clean(dist_live, true);

        let is_tab_overview =
            self.active_view == NavView::Dossier && self.dossier_tab == DossierTab::Overview;
        let is_tab_agent = self.active_view == NavView::Agent;
        let is_tab_inspector =
            self.active_view == NavView::Dossier && self.dossier_tab == DossierTab::Inspector;

        let prompt_ticker = rep.ticker.clone();
        let prompt_str = build_alan_interrogation_prompt(&rep.ticker, &rep.name, rep.bucket);

        // Unified 44px Sub-header: Routine Tabs + Single Contextual Control
        let subheader = div()
            .h(px(44.))
            .flex_shrink_0()
            .px_5()
            .bg(rgb(BG_SURFACE))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .font_family(self.fonts.mono_num.clone())
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(INK_HERO))
                            .child(rep.ticker.clone()),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_sm()
                            .text_color(rgb(INK_SECONDARY))
                            .child(rep.name.clone()),
                    )
                    .when(rep.status == StrikeZoneStatus::InStrikeZone, |d| {
                        d.child(badge(
                            "击球区".into(),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                            &self.fonts,
                        ))
                    })
                    .child(v_divider())
                    .child(
                        seg_group()
                            .child(seg_tab(
                                "tab-overview",
                                "1 财报与 K 线",
                                is_tab_overview,
                                None,
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.dossier_tab = DossierTab::Overview;
                                    this.active_view = NavView::Dossier;
                                    cx.notify();
                                }),
                            ))
                            .child(seg_tab(
                                "tab-agent",
                                "2 Alan 审讯台",
                                is_tab_agent,
                                Some((VIOLET_FG, VIOLET_BG)),
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.active_view = NavView::Agent;
                                    cx.notify();
                                }),
                            ))
                            .child(seg_tab(
                                "tab-inspector",
                                "3 XBRL 底表",
                                is_tab_inspector,
                                None,
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.dossier_tab = DossierTab::Inspector;
                                    this.active_view = NavView::Dossier;
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(is_tab_agent, |d| {
                        d.child(
                            div()
                                .id("ag-copy-prompt")
                                .px_2p5()
                                .py_1()
                                .rounded_sm()
                                .bg(rgba(VIOLET_BG))
                                .border_1()
                                .border_color(rgba(VIOLET_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_cli_command(
                                        prompt_str.clone(),
                                        &format!("Alan Prompt {prompt_ticker}"),
                                        cx,
                                    );
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(VIOLET_FG))
                                        .child("复制审讯 Prompt"),
                                )
                                .child(kbd("p", &self.fonts)),
                        )
                    })
                    .when(!is_tab_agent && rep.ttm_valuation.is_some(), |d| {
                        d.child(
                            seg_group()
                                .child(seg_tab(
                                    "seg-ttm",
                                    "TTM",
                                    self.use_ttm_basis,
                                    None,
                                    true,
                                    &self.fonts,
                                    cx.listener(|this, _, _, cx| {
                                        this.use_ttm_basis = true;
                                        cx.notify();
                                    }),
                                ))
                                .child(seg_tab(
                                    "seg-fy",
                                    "FY",
                                    !self.use_ttm_basis,
                                    None,
                                    true,
                                    &self.fonts,
                                    cx.listener(|this, _, _, cx| {
                                        this.use_ttm_basis = false;
                                        cx.notify();
                                    }),
                                )),
                        )
                    }),
            );

        // Left Scrollable Canvas
        let mut main_canvas = div()
            .id("company-main-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_4();

        if is_tab_overview {
            main_canvas = self.render_company_overview_body(
                main_canvas,
                rep,
                row_opt,
                val_opt,
                active_strike,
                &sandbox,
                sandbox_dirty,
                cx,
            );
        } else if is_tab_agent {
            main_canvas = self.render_company_agent_body(main_canvas, rep, cx);
        } else {
            main_canvas = main_canvas
                .child(self.render_multi_year_table(rep))
                .child(self.render_inspector_panel(rep, row_opt, val_opt, cx));
        }

        let props_rail = self.render_company_properties_rail(
            rep,
            val_opt,
            cur_price,
            active_strike,
            dist_live,
            dist_live_str,
            cx,
        );

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(subheader)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(main_canvas)
                    .child(props_rail),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_company_overview_body(
        &self,
        mut canvas_el: gpui::Stateful<gpui::Div>,
        rep: &MungerReport,
        row_opt: Option<&AnnualMetricRow>,
        val_opt: Option<&ValuationSnapshot>,
        active_strike: Option<Decimal>,
        sandbox: &SandboxResult,
        sandbox_dirty: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        if !rep.warnings.is_empty() {
            canvas_el = canvas_el.child(render_warning_panel(&rep.warnings, &self.fonts));
        }

        if let Some(reason) = &rep.too_hard_reason {
            canvas_el = canvas_el.child(
                div()
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .bg(rgba(SLATE_BG))
                    .border_1()
                    .border_color(rgba(SLATE_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(rgb(INK_PRIMARY))
                    .child(format!("边界理由：{reason}")),
            );
        }

        // Block 1 (HERO): Value-Anchored K-Line + Docked Inline Charlie Sandbox
        canvas_el = canvas_el.child(self.render_kline_card(
            rep,
            row_opt,
            active_strike,
            sandbox,
            sandbox_dirty,
            cx,
        ));

        // Block 2: Core Financial Check (2 side-by-side cards)
        if let Some(row) = row_opt {
            let roic_display = match (rep.roic_5y_avg_pct, rep.roic_5y_stddev_pct) {
                (Some(avg), Some(std)) => format!("{avg:.1}% ± {std:.1}%"),
                _ => row
                    .roic_total_capital_pct
                    .map(|v| format!("总资本 {v:.1}% (净现金 > 账面)"))
                    .unwrap_or_else(|| "—".into()),
            };
            let gm_display = match (rep.gross_margin_5y_avg_pct, rep.gross_margin_5y_stddev_pct) {
                (Some(avg), Some(std)) => format!("{avg:.1}% ± {std:.1}%"),
                _ => fmt_pct_clean(row.gross_margin_pct, false).0,
            };
            let (s3_str, s3_pol) = fmt_pct_clean(rep.share_change_3y_pct, true);
            let s3_col = share_delta_color(rep.share_change_3y_pct, s3_pol);
            let (s5_str, s5_pol) = fmt_pct_clean(rep.share_change_5y_pct, true);
            let s5_col = if s5_pol == 0 { INK_MUTED } else { INK_SECONDARY };

            let (ym_str, ym_pol) =
                fmt_pct_clean(val_opt.and_then(|v| v.strict_fcf_yield_mcap_pct), false);
            let (ye_str, _) =
                fmt_pct_clean(val_opt.and_then(|v| v.strict_fcf_yield_ev_pct), false);
            let ytd_callout = rep.latest_ytd.as_ref().map(|y| {
                let q_part = match (
                    &y.latest_quarter_label,
                    y.latest_quarter_ocf,
                    y.latest_quarter_capex,
                ) {
                    (Some(ql), Some(qo), Some(qc)) => format!(
                        " | {ql}: OCF {} vs CapEx {}",
                        fmt_money_compact(qo, &rep.currency),
                        fmt_money_compact(qc, &rep.currency)
                    ),
                    _ => String::new(),
                };
                format!(
                    "{}: OCF {} vs CapEx {}{}",
                    y.period_label,
                    fmt_money_compact(y.ocf, &rep.currency),
                    fmt_money_compact(y.capex, &rep.currency),
                    q_part
                )
            });

            canvas_el = canvas_el.child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        card_shell()
                            .flex_1()
                            .min_w_0()
                            .p_4()
                            .child(card_title("护城河 & 股本质量", "5Y 稳定性", &self.fonts))
                            .child(kv_line("5Y ROIC", roic_display, INK_HERO, &self.fonts))
                            .child(kv_line("5Y 毛利率", gm_display, INK_PRIMARY, &self.fonts))
                            .child(kv_line(
                                "当期净负债",
                                fmt_money_compact(row.net_debt, &rep.currency),
                                if row.net_debt <= Decimal::ZERO {
                                    EMERALD_FG
                                } else {
                                    INK_PRIMARY
                                },
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "净负债 / 严苛 OE",
                                rep.net_debt_to_owner_earnings
                                    .map(|v| format!("{v:.2}x"))
                                    .unwrap_or_else(|| "—".into()),
                                if rep.net_debt_to_owner_earnings.is_some() {
                                    INK_SECONDARY
                                } else {
                                    INK_MUTED
                                },
                                &self.fonts,
                            ))
                            .child(kv_line("3Y 稀释股本Δ", s3_str, s3_col, &self.fonts))
                            .child(kv_line("5Y 稀释股本Δ", s5_str, s5_col, &self.fonts))
                            .when(!rep.annual_rows.is_empty(), |d| {
                                d.child(roic_sparkline(&rep.annual_rows, &self.fonts))
                            }),
                    )
                    .child(
                        card_shell()
                            .flex_1()
                            .min_w_0()
                            .p_4()
                            .child(card_title("现金流真相", &row.label, &self.fonts))
                            .child(kv_line(
                                "经营现金流 (OCF)",
                                fmt_money_compact(row.ocf, &rep.currency),
                                INK_HERO,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "严苛 OE (OCF−CapEx−SBC)",
                                fmt_money_compact(row.strict_owner_earnings, &rep.currency),
                                if row.strict_owner_earnings > Decimal::ZERO {
                                    EMERALD_FG
                                } else {
                                    CORAL_FG
                                },
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "严苛 FCF% (市值 / EV)",
                                format!("{ym_str} / {ye_str}"),
                                if ym_pol < 0 { CORAL_FG } else { INK_PRIMARY },
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                "CapEx 开支".into(),
                                fmt_money_compact(row.capex, &rep.currency),
                                row.capex,
                                row.ocf,
                                GOLD_FG,
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                "SBC 激励".into(),
                                fmt_money_compact(row.sbc, &rep.currency),
                                row.sbc,
                                row.ocf,
                                CORAL_FG,
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                "剩余严苛 OE".into(),
                                fmt_money_compact(row.strict_owner_earnings, &rep.currency),
                                row.strict_owner_earnings.max(Decimal::ZERO),
                                row.ocf,
                                EMERALD_FG,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "回购 / SBC",
                                format!(
                                    "{} / {}",
                                    fmt_money_compact(row.buybacks, &rep.currency),
                                    fmt_money_compact(row.sbc, &rep.currency)
                                ),
                                INK_SECONDARY,
                                &self.fonts,
                            ))
                            .when_some(ytd_callout, |d, note| {
                                d.child(
                                    div()
                                        .mt_3()
                                        .px_2p5()
                                        .py_1p5()
                                        .rounded_sm()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgb(BORDER_HAIRLINE))
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .truncate()
                                        .child(note),
                                )
                            }),
                    ),
            );
        } else {
            canvas_el = canvas_el.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgba(GOLD_BG))
                    .border_1()
                    .border_color(rgba(GOLD_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(13.))
                    .line_height(px(21.))
                    .text_color(rgb(GOLD_FG))
                    .child(format!(
                        "{} 暂无结构化财报数据；如需接入计算器，可录入 workbench/manual_financials/{}.json。",
                        rep.ticker, rep.ticker
                    )),
            );
        }

        // Block 3: Human Sovereignty & Feynman Gate
        if let Some(dossier) = &self.selected_dossier {
            let human = &dossier.human_judgment;
            canvas_el = canvas_el.child(
                card_shell()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pb_2p5()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_2p5()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child("人类判词 · 费曼防线"),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .child("仅本人确认 · AI 草稿见「2 Alan 审讯台」"),
                                    ),
                            )
                            .child(if human.pending_human_review {
                                badge("待答辩".into(), GOLD_FG, GOLD_BG, GOLD_BORDER, &self.fonts)
                            } else {
                                badge(
                                    "已确认".into(),
                                    EMERALD_FG,
                                    EMERALD_BG,
                                    EMERALD_BORDER,
                                    &self.fonts,
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(div().flex_1().min_w_0().child(prose_block(
                                "我的定性判词 (my_stance)",
                                human.my_stance.as_deref().unwrap_or("（待填写）"),
                                &self.fonts,
                            )))
                            .child(div().flex_1().min_w_0().child(prose_block(
                                "分歧权重判断 (weight_rationale)",
                                human.weight_rationale.as_deref().unwrap_or("（待填写）"),
                                &self.fonts,
                            ))),
                    )
                    .child(
                        div()
                            .pt_2p5()
                            .border_t_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_SECONDARY))
                                            .child("事前证伪指标 (falsification_metrics)"),
                                    )
                                    .when(human.falsification_metrics.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .mt_1()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_MUTED))
                                                .child("（尚未设置）"),
                                        )
                                    })
                                    .children(human.falsification_metrics.iter().map(|m| {
                                        div()
                                            .mt_1()
                                            .font_family(self.fonts.ui.clone())
                                            .text_size(px(13.))
                                            .line_height(px(20.))
                                            .text_color(rgb(INK_PRIMARY))
                                            .child(format!("• {m}"))
                                    })),
                            )
                            .when_some(human.feynman_checked_at.clone(), |d, checked| {
                                d.child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .text_color(rgb(EMERALD_FG))
                                        .child(format!("核验时间 {checked}")),
                                )
                            }),
                    ),
            );
        }

        canvas_el
    }

    fn render_company_agent_body(
        &self,
        mut canvas_el: gpui::Stateful<gpui::Div>,
        rep: &MungerReport,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let draft_source = self
            .selected_dossier
            .as_ref()
            .map(|d| d.draft_source.clone())
            .unwrap_or_else(|| "agent_draft".into());
        let updated_at = self
            .selected_dossier
            .as_ref()
            .map(|d| d.updated_at.clone())
            .unwrap_or_else(|| "—".into());
        let dossier_file = format!("workbench/dossiers/{}.json", rep.ticker);
        let dossier_cmd = format!("./apps/m3-desktop/m3 dossier {}", rep.ticker);
        let ticker_cli = rep.ticker.clone();

        canvas_el = canvas_el.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(VIOLET_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .text_color(rgb(VIOLET_FG))
                                .child(format!("AI 幕僚草稿 · {draft_source}")),
                        ),
                )
                .child(
                    div()
                        .font_family(self.fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child(format!("更新于 {updated_at}")),
                ),
        );

        if let Some(dossier) = &self.selected_dossier {
            let pending = dossier.human_judgment.pending_human_review;

            // 1. 3-Sentence Business Filter
            canvas_el = canvas_el.child(
                card_shell()
                    .p_4()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_SECONDARY))
                            .child("能力圈三句话过滤"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(14.))
                            .line_height(px(23.))
                            .text_color(rgb(INK_PRIMARY))
                            .child(dossier.three_sentence_business.clone()),
                    ),
            );

            // 2. Interdisciplinary Latticework (Single flat 2x2 grid separated by hairlines)
            let quad = |title: &str, body: &str| {
                div()
                    .flex_1()
                    .min_w_0()
                    .p_4()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(title.to_owned()),
                    )
                    .child(
                        div()
                            .mt_1p5()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(13.))
                            .line_height(px(21.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(body.to_owned()),
                    )
            };

            canvas_el = canvas_el.child(
                card_shell()
                    .child(
                        div()
                            .px_4()
                            .py_2p5()
                            .bg(rgb(BG_WELL))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child("跨学科思维格栅"),
                    )
                    .child(
                        div()
                            .flex()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .child(
                                quad("微观经济与护城河", &dossier.latticework.microeconomics)
                                    .border_r_1()
                                    .border_color(rgb(BORDER_HAIRLINE)),
                            )
                            .child(quad("生态位与技术演化", &dossier.latticework.ecology)),
                    )
                    .child(
                        div()
                            .flex()
                            .child(
                                quad("心理学与管理层激励", &dossier.latticework.psychology_incentives)
                                    .border_r_1()
                                    .border_color(rgb(BORDER_HAIRLINE)),
                            )
                            .child(quad("资本配置历史", &dossier.latticework.capital_allocation)),
                    ),
            );

            // 3. 5Y Pre-mortem + Feynman Interrogation Questions
            let pm_len = dossier.premortem.len();
            let q_len = dossier.open_questions.len();

            canvas_el = canvas_el.child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        card_shell()
                            .flex_1()
                            .min_w_0()
                            .p_4()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .pb_2p5()
                                    .border_b_1()
                                    .border_color(rgb(BORDER_HAIRLINE))
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(CORAL_FG))
                                    .child("5 年逆向事前验尸（亏损 >50% 路径）"),
                            )
                            .when(dossier.premortem.is_empty(), |d| {
                                d.child(
                                    div()
                                        .pt_3()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .child("（尚无事前验尸草稿）"),
                                )
                            })
                            .children(dossier.premortem.iter().enumerate().map(|(i, item)| {
                                div()
                                    .py_2p5()
                                    .when(i + 1 < pm_len, |d| {
                                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                                    })
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(13.))
                                    .line_height(px(21.))
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(format!("{}. {item}", i + 1))
                            })),
                    )
                    .child(
                        card_shell()
                            .w(px(340.))
                            .flex_shrink_0()
                            .p_4()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .pb_2p5()
                                    .border_b_1()
                                    .border_color(rgb(BORDER_HAIRLINE))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child("费曼待答辩关键质疑"),
                                    )
                                    .child(if pending {
                                        badge(
                                            "待作答".into(),
                                            GOLD_FG,
                                            GOLD_BG,
                                            GOLD_BORDER,
                                            &self.fonts,
                                        )
                                    } else {
                                        badge(
                                            "已通过".into(),
                                            EMERALD_FG,
                                            EMERALD_BG,
                                            EMERALD_BORDER,
                                            &self.fonts,
                                        )
                                    }),
                            )
                            .children(dossier.open_questions.iter().enumerate().map(|(i, q)| {
                                div()
                                    .py_2p5()
                                    .when(i + 1 < q_len, |d| {
                                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                                    })
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(13.))
                                    .line_height(px(20.))
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(format!("? {q}"))
                            })),
                    ),
            );
        } else {
            canvas_el = canvas_el.child(
                card_shell()
                    .p_4()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child(format!(
                        "尚无 {} 审讯档案（{dossier_file}），可点击右上角「复制审讯 Prompt (p)」启动。",
                        rep.ticker
                    )),
            );
        }

        canvas_el.child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(11.))
                        .text_color(rgb(INK_MUTED))
                        .child(dossier_file),
                )
                .child(
                    div()
                        .id("ag-copy-dossier")
                        .px_2p5()
                        .py_1()
                        .rounded_sm()
                        .bg(rgb(BG_SURFACE))
                        .border_1()
                        .border_color(rgb(BORDER_PILL))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_cli_command(
                                dossier_cmd.clone(),
                                &format!("dossier {ticker_cli}"),
                                cx,
                            );
                            cx.notify();
                        }))
                        .child(
                            div()
                                .font_family(self.fonts.mono_term.clone())
                                .text_xs()
                                .text_color(rgb(INK_SECONDARY))
                                .child("Copy dossier CLI"),
                        )
                        .child(kbd("c", &self.fonts)),
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_company_properties_rail(
        &self,
        rep: &MungerReport,
        val_opt: Option<&ValuationSnapshot>,
        cur_price: Option<Decimal>,
        active_strike: Option<Decimal>,
        dist_live: Option<Decimal>,
        dist_live_str: String,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let basis_tag = val_opt
            .map(|v| format!("{} · {}", v.basis_label, v.period_end))
            .unwrap_or_else(|| "待录入".into());
        let sha_short = rep
            .raw_sha256
            .as_deref()
            .map(|s| &s[..s.len().min(12)])
            .unwrap_or("—");
        let tag = primary_circle_tag(&rep.circle);
        let tag_for_jump = tag.clone();
        let sub_circle = secondary_circle_detail(&rep.circle);

        let red_cnt = rep
            .warnings
            .iter()
            .filter(|w| w.severity == WarningSeverity::Red)
            .count();
        let amber_cnt = rep
            .warnings
            .iter()
            .filter(|w| w.severity == WarningSeverity::Amber)
            .count();

        let (fin_dot, fin_lbl, fin_col) = if rep.annual_rows.is_empty() {
            ("○", "待录财报".into(), INK_MUTED)
        } else if red_cnt > 0 {
            ("●", format!("红灯 {red_cnt}"), CORAL_FG)
        } else if amber_cnt > 0 {
            ("○", format!("黄灯 {amber_cnt}"), GOLD_FG)
        } else {
            ("●", "无红灯".into(), EMERALD_FG)
        };

        let pending_feynman = self
            .selected_dossier
            .as_ref()
            .is_none_or(|d| d.human_judgment.pending_human_review);

        let (strike_dot, strike_lbl, strike_col) = match dist_live {
            Some(d) if d <= Decimal::ZERO => ("●", "击球区".into(), EMERALD_FG),
            Some(_) => ("○", dist_live_str.clone(), INK_SECONDARY),
            None => ("○", "—".into(), INK_MUTED),
        };

        div()
            .id("company-props-scroll")
            .w(px(256.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(0x111118))
            .border_l_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .p_4()
            .flex()
            .flex_col()
            .gap_3p5()
            .child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(INK_MUTED))
                    .child("PROPERTIES"),
            )
            // 1. Valuation Anchor (flat directly on rail)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child("现价"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(fmt_price(cur_price, &rep.currency)),
                            ),
                    )
                    .child(kv_line(
                        "沙盒击球价",
                        fmt_price(active_strike, &rep.currency),
                        if active_strike.is_some() {
                            EMERALD_FG
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "距击球价",
                        dist_live_str,
                        match dist_live {
                            Some(d) if d <= Decimal::ZERO => EMERALD_FG,
                            Some(_) => GOLD_FG,
                            None => INK_MUTED,
                        },
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "市值",
                        val_opt
                            .and_then(|v| v.market_cap)
                            .map(|m| fmt_money_compact(m, &rep.currency))
                            .unwrap_or_else(|| "—".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "企业价值 (EV)",
                        val_opt
                            .and_then(|v| v.enterprise_value)
                            .map(|m| fmt_money_compact(m, &rep.currency))
                            .unwrap_or_else(|| "—".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    )),
            )
            // 2. Silent 3-Line Munger Decision Gate
            .child(
                prop_section("DECISION GATE · 决策检查", &self.fonts)
                    .child(
                        div()
                            .id("gate-fin")
                            .flex()
                            .items_center()
                            .justify_between()
                            .py_0p5()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(RAIL_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dossier_tab = DossierTab::Overview;
                                this.active_view = NavView::Dossier;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child("财务体检"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(fin_col))
                                    .child(format!("{fin_dot} {fin_lbl}")),
                            ),
                    )
                    .child(
                        div()
                            .id("gate-feynman")
                            .flex()
                            .items_center()
                            .justify_between()
                            .py_0p5()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(RAIL_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_view = NavView::Agent;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child("费曼答辩"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(if pending_feynman {
                                        GOLD_FG
                                    } else {
                                        EMERALD_FG
                                    }))
                                    .child(if pending_feynman {
                                        "○ 待答辩 →"
                                    } else {
                                        "● 已确认"
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .py_0p5()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child("击球价格"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(strike_col))
                                    .child(format!("{strike_dot} {strike_lbl}")),
                            ),
                    ),
            )
            // 3. Basket 2x2 Switcher
            .child(
                prop_section("BASKET · 归入篮子", &self.fonts)
                    .child(
                        div()
                            .flex()
                            .gap_1p5()
                            .children([Bucket::Core, Bucket::Watch].into_iter().map(|b| {
                                self.render_prop_bucket_btn(&rep.ticker, rep.bucket, b, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1p5()
                            .children([Bucket::Out, Bucket::TooHard].into_iter().map(|b| {
                                self.render_prop_bucket_btn(&rep.ticker, rep.bucket, b, cx)
                            })),
                    ),
            )
            // 4. Circle Tag Linkage
            .child(
                prop_section("LABELS · 能力圈", &self.fonts).child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_1p5()
                        .child(circle_tag_button(
                            "prop-tag-jump",
                            tag,
                            false,
                            false,
                            None,
                            &self.fonts,
                            cx.listener(move |this, _, _, cx| {
                                this.jump_to_tag(tag_for_jump.clone());
                                cx.notify();
                            }),
                        ))
                        .when_some(sub_circle, |d, det| {
                            d.child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(det),
                            )
                        }),
                ),
            )
            // 5. Provenance Metadata
            .child(
                prop_section("PROVENANCE · 溯源", &self.fonts)
                    .child(kv_line(
                        "CIK",
                        rep.cik.clone().unwrap_or_else(|| "MANUAL".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line("口径", basis_tag, INK_SECONDARY, &self.fonts))
                    .child(kv_line(
                        "最近财报",
                        rep.latest_filing_end
                            .clone()
                            .unwrap_or_else(|| "待录入".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "SHA-256",
                        sha_short.to_owned(),
                        INK_MUTED,
                        &self.fonts,
                    )),
            )
    }

    fn render_prop_bucket_btn(
        &self,
        ticker: &str,
        current: Bucket,
        target: Bucket,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let active = current == target;
        let (fg, bg, br) = bucket_colors(target);
        let t = ticker.to_owned();
        div()
            .id(gpui::SharedString::from(format!("prop-bkt-{}", target.code())))
            .flex_1()
            .min_w_0()
            .px_2()
            .py_1p5()
            .rounded_sm()
            .cursor_pointer()
            .bg(rgba(if active { bg } else { 0x161620FF }))
            .border_1()
            .border_color(rgba(if active {
                br
            } else {
                BORDER_SUBTLE << 8 | 0xFF
            }))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .flex()
            .items_center()
            .gap_1p5()
            .on_click(cx.listener(move |this, _, _, cx| {
                if !active {
                    this.move_company_bucket(&t, target, cx);
                }
            }))
            .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(fg)))
            .child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .font_weight(if active {
                        gpui::FontWeight::BOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(rgb(if active { fg } else { INK_SECONDARY }))
                    .truncate()
                    .child(target.code()),
            )
    }

    fn render_multi_year_table(&self, rep: &MungerReport) -> gpui::Div {
        let mut all_rows: Vec<&AnnualMetricRow> = rep.annual_rows.iter().collect();
        if let Some(ttm) = &rep.ttm_row {
            all_rows.push(ttm);
        }
        if all_rows.is_empty() {
            return card_shell()
                .p_4()
                .font_family(self.fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child("暂无多年结构化财务底表。");
        }
        let fin_cols: &[(&str, f32, bool)] = &[
            ("期间", 122., false),
            ("营收", 84., true),
            ("毛利率", 68., true),
            ("ROIC", 68., true),
            ("OCF", 84., true),
            ("CapEx", 80., true),
            ("SBC", 74., true),
            ("严苛 OE", 88., true),
            ("回购", 80., true),
            ("稀释股本", 0., true),
        ];
        let sha_short = rep
            .raw_sha256
            .as_deref()
            .map(|s| &s[..s.len().min(16)])
            .unwrap_or("—");
        let tag_summary = rep
            .provenance_summary
            .iter()
            .take(5)
            .map(|p| format!("{}:{}", p.field, p.source_tag))
            .collect::<Vec<_>>()
            .join(" · ");

        card_shell()
            .child(
                div()
                    .px_4()
                    .py_3()
                    .bg(rgb(BG_WELL))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child("多年确定性财务底表（Rust 解析自 SEC EDGAR XBRL）"),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(format!(
                                "CIK {} · SHA256 {}",
                                rep.cik.as_deref().unwrap_or("MANUAL"),
                                sha_short
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .px_3()
                    .py_2()
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .children(fin_cols.iter().map(|(lbl, w, right)| {
                        div()
                            .px_1()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED))
                            .when(*right, |d| d.text_right())
                            .when(*w > 0., |d| d.w(px(*w)).flex_shrink_0())
                            .when(*w == 0., |d| d.flex_1().min_w_0())
                            .child(*lbl)
                    })),
            )
            .children(all_rows.into_iter().rev().enumerate().map(|(i, r)| {
                let is_ttm = r.label.starts_with("TTM");
                let row_el = div()
                    .flex()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE));
                let row_el = if is_ttm {
                    row_el.bg(rgba(EMERALD_BG))
                } else {
                    row_el.bg(rgb(if i % 2 == 0 {
                        BG_SURFACE
                    } else {
                        BG_ROW_ALT
                    }))
                };
                let (gm_str, _) = fmt_pct_clean(r.gross_margin_pct, false);
                let (roic_str, _) = fmt_pct_clean(r.roic_pct, false);
                row_el
                    .child(
                        div()
                            .w(px(122.))
                            .flex_shrink_0()
                            .px_1()
                            .font_family(self.fonts.mono_num.clone())
                            .text_xs()
                            .font_weight(if is_ttm {
                                gpui::FontWeight::BOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(rgb(if is_ttm { EMERALD_FG } else { INK_PRIMARY }))
                            .truncate()
                            .child(r.label.clone()),
                    )
                    .child(mono_right(
                        fmt_money_compact(r.revenue, &rep.currency),
                        84.,
                        INK_PRIMARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        gm_str,
                        68.,
                        if r.gross_margin_pct.is_some() {
                            INK_SECONDARY
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        roic_str,
                        68.,
                        match r.roic_pct {
                            Some(v) if v >= Decimal::from(15) => INK_PRIMARY,
                            Some(_) => GOLD_FG,
                            None => INK_MUTED,
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.ocf, &rep.currency),
                        84.,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.capex, &rep.currency),
                        80.,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.sbc, &rep.currency),
                        74.,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.strict_owner_earnings, &rep.currency),
                        88.,
                        if r.strict_owner_earnings > Decimal::ZERO {
                            EMERALD_FG
                        } else {
                            CORAL_FG
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.buybacks, &rep.currency),
                        80.,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_1()
                            .text_right()
                            .font_family(self.fonts.mono_num.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child(fmt_shares_compact(r.diluted_shares)),
                    )
            }))
            .when(!tag_summary.is_empty(), |d| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(rgb(BG_WELL))
                        .font_family(self.fonts.mono_term.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .truncate()
                        .child(format!("XBRL 溯源: {tag_summary}")),
                )
            })
    }

    fn render_inspector_panel(
        &self,
        rep: &MungerReport,
        row_opt: Option<&AnnualMetricRow>,
        val_opt: Option<&ValuationSnapshot>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let sha_full = rep
            .raw_sha256
            .as_deref()
            .unwrap_or("— (无一手 XBRL 源文件)");
        let raw_file_path = match &rep.cik {
            Some(cik) => format!("data/munger/raw/sec/CIK{cik}.json"),
            None => format!("workbench/manual_financials/{}.json", rep.ticker),
        };
        let report_cmd = format!("./apps/m3-desktop/m3 report {}", rep.ticker);
        let t_report = rep.ticker.clone();

        let formula_lines: Vec<(&str, String)> = match (row_opt, val_opt) {
            (Some(r), Some(v)) => {
                let oe_ps = v
                    .owner_earnings_per_share
                    .map(|x| format!("{}{:.4}/sh", currency_prefix(&rep.currency), x))
                    .unwrap_or_else(|| "—".into());
                let roic_str = r
                    .roic_pct
                    .map(|x| format!("{x:.2}%"))
                    .unwrap_or_else(|| "Net Cash > Book Capital".into());
                let mcap = v
                    .market_cap
                    .map(|m| fmt_money_compact(m, &rep.currency))
                    .unwrap_or_else(|| "—".into());
                let yield_s = fmt_pct_clean(v.strict_fcf_yield_mcap_pct, false).0;
                let imp_s = fmt_pct_clean(v.implied_growth_for_target_pct, true).0;
                vec![
                    (
                        "严苛股东盈余",
                        format!(
                            "Strict_OE = OCF({}) - CapEx({}) - SBC({}) = {}",
                            fmt_money_compact(r.ocf, &rep.currency),
                            fmt_money_compact(r.capex, &rep.currency),
                            fmt_money_compact(r.sbc, &rep.currency),
                            fmt_money_compact(r.strict_owner_earnings, &rep.currency)
                        ),
                    ),
                    (
                        "投入资本回报率",
                        format!(
                            "ROIC = NOPAT({}) / IC(Debt {} + Eq {} - Cash {} = {}) = {}",
                            fmt_money_compact(r.nopat, &rep.currency),
                            fmt_money_compact(r.total_debt, &rep.currency),
                            fmt_money_compact(r.equity, &rep.currency),
                            fmt_money_compact(r.cash_and_st, &rep.currency),
                            fmt_money_compact(r.invested_capital, &rep.currency),
                            roic_str
                        ),
                    ),
                    (
                        "保守击球价格",
                        format!(
                            "Strike = OE_per_share({oe_ps}) * Multiple_low({}x) = {}",
                            rep.assumptions.multiple_low,
                            fmt_price(v.strike_price, &rep.currency)
                        ),
                    ),
                    (
                        "反向隐含增速",
                        format!(
                            "Implied_g = Target({}%) - Strict_OE/MCap({} / {} = {yield_s}) = {imp_s}",
                            rep.assumptions.target_return_pct,
                            fmt_money_compact(v.strict_owner_earnings, &rep.currency),
                            mcap,
                        ),
                    ),
                ]
            }
            (Some(r), None) => vec![(
                "严苛股东盈余",
                format!(
                    "Strict_OE = OCF({}) - CapEx({}) - SBC({}) = {}",
                    fmt_money_compact(r.ocf, &rep.currency),
                    fmt_money_compact(r.capex, &rep.currency),
                    fmt_money_compact(r.sbc, &rep.currency),
                    fmt_money_compact(r.strict_owner_earnings, &rep.currency)
                ),
            )],
            _ => vec![(
                "STATUS",
                "无一手结构化财务事实（绝不由 LLM 补全）。".into(),
            )],
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                card_shell()
                    .p_5()
                    .child(section_header(
                        "Deterministic Rust Arithmetic Trace (rust_decimal)",
                        "ZERO LLM ESTIMATION".into(),
                        &self.fonts,
                    ))
                    .children(formula_lines.into_iter().map(|(lbl, expr)| {
                        div()
                            .py_2()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .w(px(120.))
                                    .flex_shrink_0()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(lbl),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(expr),
                            )
                    })),
            )
            .child(
                card_shell()
                    .p_5()
                    .child(section_header(
                        "Source Provenance & CLI Verification",
                        format!("CIK {}", rep.cik.as_deref().unwrap_or("MANUAL")),
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(inspector_kv("SOURCE_FILE", raw_file_path, &self.fonts))
                            .child(inspector_kv(
                                "RAW_SHA256",
                                sha_full.to_owned(),
                                &self.fonts,
                            ))
                            .child(inspector_kv(
                                "FILING_END",
                                rep.latest_filing_end
                                    .clone()
                                    .unwrap_or_else(|| "—".into()),
                                &self.fonts,
                            ))
                            .child(inspector_kv(
                                "PRICE_URL",
                                rep.price
                                    .as_ref()
                                    .map(|p| p.source_url.clone())
                                    .unwrap_or_else(|| "—".into()),
                                &self.fonts,
                            )),
                    )
                    .child(
                        div()
                            .mt_4()
                            .pt_3()
                            .border_t_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .id("insp-copy-report")
                                    .px_3()
                                    .py_1p5()
                                    .rounded_sm()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_PILL))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy_cli_command(
                                            report_cmd.clone(),
                                            &format!("report {t_report}"),
                                            cx,
                                        );
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_PRIMARY))
                                            .child("Copy CLI"),
                                    )
                                    .child(kbd("c", &self.fonts)),
                            ),
                    ),
            )
            .child(
                card_shell()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .bg(rgb(BG_WELL))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child("SEC XBRL Concept Mapping (Latest Annual Provenance)"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("{} fields", rep.provenance_summary.len())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .px_4()
                            .py_2()
                            .bg(rgb(BG_ROW_ALT))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED))
                            .child(div().w(px(130.)).child("FIELD"))
                            .child(div().flex_1().min_w_0().child("XBRL_TAG / FORMULA"))
                            .child(div().w(px(110.)).child("PERIOD_END"))
                            .child(div().w(px(110.)).child("FILED"))
                            .child(div().w(px(180.)).child("NOTE")),
                    )
                    .when(rep.provenance_summary.is_empty(), |d| {
                        d.child(
                            div()
                                .p_4()
                                .font_family(self.fonts.mono_term.clone())
                                .text_xs()
                                .text_color(rgb(INK_MUTED))
                                .child("(no XBRL concept provenance records)"),
                        )
                    })
                    .children(rep.provenance_summary.iter().enumerate().map(|(i, p)| {
                        div()
                            .flex()
                            .px_4()
                            .py_2()
                            .bg(rgb(if i % 2 == 0 {
                                BG_SURFACE
                            } else {
                                BG_ROW_ALT
                            }))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.mono_term.clone())
                            .text_xs()
                            .child(
                                div()
                                    .w(px(130.))
                                    .flex_shrink_0()
                                    .text_color(rgb(INK_HERO))
                                    .child(p.field.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(rgb(SKY_FG))
                                    .truncate()
                                    .child(p.source_tag.clone()),
                            )
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_shrink_0()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(p.period_end.clone()),
                            )
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_shrink_0()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_color(rgb(INK_MUTED))
                                    .child(p.filed.clone().unwrap_or_else(|| "—".into())),
                            )
                            .child(
                                div()
                                    .w(px(180.))
                                    .flex_shrink_0()
                                    .text_color(rgb(INK_MUTED))
                                    .truncate()
                                    .child(p.note.clone().unwrap_or_else(|| "direct".into())),
                            )
                    })),
            )
    }

    fn render_journal_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let bench_now = self
            .benchmark
            .as_ref()
            .map(|b| format!("{:.2}", b.price))
            .unwrap_or_else(|| "—".into());
        let bench_start = self
            .journal
            .benchmark_start_price
            .map(|p| format!("{p:.2}"))
            .unwrap_or_else(|| "待锁定".into());

        let cols: &[(&str, f32, bool)] = &[
            ("日期", 98., false),
            ("标的", 84., false),
            ("决策动作", 112., false),
            ("冻结价格", 92., true),
            ("严苛 FCF%", 88., true),
            ("S&P 500", 88., true),
            ("判断依据与事前证伪条件", 0., false),
        ];

        let journal_cmd =
            "./apps/m3-desktop/m3 journal record --ticker SYM --action wait --note \"...\"";

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
                    .bg(rgb(BG_SURFACE))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .child(badge(
                                self.journal.account_note.clone(),
                                EMERALD_FG,
                                EMERALD_BG,
                                EMERALD_BORDER,
                                &self.fonts,
                            ))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child("事前冻结决策账本"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(journal_stat(
                                "模拟现金",
                                format!("${:.2}", self.journal.cash_balance),
                                EMERALD_FG,
                                &self.fonts,
                            ))
                            .child(v_divider())
                            .child(journal_stat(
                                "初始现金",
                                format!("${:.2}", self.journal.starting_cash),
                                INK_HERO,
                                &self.fonts,
                            ))
                            .child(v_divider())
                            .child(journal_stat(
                                "S&P 500",
                                format!("{bench_now} (起点 {bench_start})"),
                                INK_PRIMARY,
                                &self.fonts,
                            ))
                            .child(v_divider())
                            .child(journal_stat(
                                "记录数",
                                format!("{}", self.journal.entries.len()),
                                SLATE_FG,
                                &self.fonts,
                            )),
                    ),
            )
            .child(
                div().p_5().child(
                    card_shell()
                        .child(
                            div()
                                .px_4()
                                .py_3()
                                .bg(rgb(BG_WELL))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO))
                                        .child("决策记录（workbench/journal.json）"),
                                )
                                .child(
                                    div()
                                        .id("copy-journal-cli")
                                        .px_2p5()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(rgb(BG_SURFACE))
                                        .border_1()
                                        .border_color(rgb(BORDER_PILL))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .cursor_pointer()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy_cli_command(
                                                journal_cmd.into(),
                                                "journal record",
                                                cx,
                                            );
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_SECONDARY))
                                                .child("Copy CLI"),
                                        )
                                        .child(kbd("c", &self.fonts)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .px_3()
                                .py_2()
                                .bg(rgb(BG_ROW_ALT))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .children(cols.iter().map(|(lbl, w, right)| {
                                    div()
                                        .px_1()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_MUTED))
                                        .when(*right, |d| d.text_right())
                                        .when(*w > 0., |d| d.w(px(*w)).flex_shrink_0())
                                        .when(*w == 0., |d| d.flex_1().min_w_0())
                                        .child(*lbl)
                                })),
                        )
                        .children(self.journal.entries.iter().rev().enumerate().map(|(i, e)| {
                            let (fy_str, _) = fmt_pct_clean(e.strict_fcf_yield_pct, false);
                            let (a_fg, a_bg, a_br) = journal_action_colors(e.action);
                            div()
                                .flex()
                                .px_3()
                                .py_3()
                                .bg(rgb(if i % 2 == 0 {
                                    BG_SURFACE
                                } else {
                                    BG_ROW_ALT
                                }))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .child(mono_left(
                                    e.recorded_at.clone(),
                                    98.,
                                    INK_SECONDARY,
                                    &self.fonts,
                                ))
                                .child(mono_left(e.ticker.clone(), 84., INK_HERO, &self.fonts))
                                .child(
                                    div()
                                        .w(px(112.))
                                        .flex_shrink_0()
                                        .px_1()
                                        .flex()
                                        .items_start()
                                        .child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded_xs()
                                                .bg(rgba(a_bg))
                                                .border_1()
                                                .border_color(rgba(a_br))
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_size(px(10.))
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(a_fg))
                                                .child(e.action.label().to_owned()),
                                        ),
                                )
                                .child(mono_right(
                                    e.price
                                        .map(|p| format!("{p:.2}"))
                                        .unwrap_or_else(|| "—".into()),
                                    92.,
                                    if e.price.is_some() {
                                        INK_SECONDARY
                                    } else {
                                        INK_MUTED
                                    },
                                    &self.fonts,
                                ))
                                .child(mono_right(
                                    fy_str,
                                    88.,
                                    if e.strict_fcf_yield_pct.is_some() {
                                        INK_SECONDARY
                                    } else {
                                        INK_MUTED
                                    },
                                    &self.fonts,
                                ))
                                .child(mono_right(
                                    e.benchmark_price
                                        .map(|p| format!("{p:.2}"))
                                        .unwrap_or_else(|| "—".into()),
                                    88.,
                                    INK_MUTED,
                                    &self.fonts,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .px_2()
                                        .child(
                                            div()
                                                .font_family(self.fonts.ui.clone())
                                                .text_size(px(13.))
                                                .line_height(px(20.))
                                                .text_color(rgb(INK_PRIMARY))
                                                .child(e.rationale.clone()),
                                        )
                                        .child(
                                            div()
                                                .mt_1()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_MUTED))
                                                .child(format!(
                                                    "证伪条件: {}",
                                                    e.falsification_trigger
                                                )),
                                        ),
                                )
                        })),
                ),
            )
    }

    fn render_command_palette(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let all_items = self.build_palette_items();
        let filtered = filter_palette_items(&all_items, &self.palette_query);
        let active_idx = self.palette_cursor.min(filtered.len().saturating_sub(1));

        div()
            .id("palette-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(SCRIM_BACKDROP))
            .flex()
            .flex_col()
            .items_center()
            .pt(px(88.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.palette_open = false;
                    this.palette_query.clear();
                    this.palette_cursor = 0;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("palette-modal")
                    .w(px(600.))
                    .max_h(px(470.))
                    .rounded_xl()
                    .bg(rgb(BG_PALETTE))
                    .border_1()
                    .border_color(rgb(BORDER_PALETTE))
                    .shadow_2xl()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(EMERALD_FG))
                                            .child(">"),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_sm()
                                            .text_color(rgb(if self.palette_query.is_empty() {
                                                INK_MUTED
                                            } else {
                                                INK_HERO
                                            }))
                                            .truncate()
                                            .child(if self.palette_query.is_empty() {
                                                "输入标的、篮子 (IN/OUT/TOO HARD)、#标签 或命令…"
                                                    .to_owned()
                                            } else {
                                                format!("{}_", self.palette_query)
                                            }),
                                    ),
                            )
                            .child(kbd("esc", &self.fonts)),
                    )
                    .child(
                        div()
                            .id("palette-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_2()
                            .when(filtered.is_empty(), |d| {
                                d.child(
                                    div()
                                        .p_4()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .child("无匹配命令、标的或标签"),
                                )
                            })
                            .children(filtered.into_iter().enumerate().map(|(i, item)| {
                                let selected = i == active_idx;
                                let action = item.action.clone();
                                div()
                                    .id(gpui::SharedString::from(format!("pal-item-{i}")))
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(selected, |d| d.bg(rgb(BG_HOVER)))
                                    .hover(|s| s.bg(rgb(BG_SELECTED_ROW)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.execute_palette_action(action.clone(), cx);
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2p5()
                                            .min_w_0()
                                            .flex_1()
                                            .child(
                                                div()
                                                    .w(px(6.))
                                                    .h(px(6.))
                                                    .rounded_full()
                                                    .flex_shrink_0()
                                                    .bg(rgb(item.dot_color)),
                                            )
                                            .child(
                                                div()
                                                    .w(px(76.))
                                                    .flex_shrink_0()
                                                    .font_family(self.fonts.mono_term.clone())
                                                    .text_size(px(10.))
                                                    .text_color(rgb(INK_MUTED))
                                                    .child(item.group.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .min_w_0()
                                                    .child(
                                                        div()
                                                            .font_family(
                                                                self.fonts.mono_term.clone(),
                                                            )
                                                            .text_xs()
                                                            .font_weight(
                                                                gpui::FontWeight::SEMIBOLD,
                                                            )
                                                            .text_color(rgb(if selected {
                                                                INK_HERO
                                                            } else {
                                                                INK_PRIMARY
                                                            }))
                                                            .truncate()
                                                            .child(item.title.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_family(self.fonts.ui.clone())
                                                            .text_size(px(11.))
                                                            .text_color(rgb(INK_MUTED))
                                                            .truncate()
                                                            .child(item.subtitle.clone()),
                                                    ),
                                            ),
                                    )
                                    .when_some(item.shortcut, |d, sc| {
                                        d.child(kbd(sc, &self.fonts))
                                    })
                            })),
                    ),
            )
    }

    fn render_statusline(&self) -> gpui::Div {
        let (dot_col, legend) = if self.palette_open {
            (
                SKY_FG,
                "↑/↓ 选择 · ↵ 执行 · esc 关闭命令面板",
            )
        } else {
            match self.active_view {
                NavView::Baskets => (
                    EMERALD_FG,
                    "v 列表/看板 · 1-4 切篮子 · 0 全部 · j/k 移动 · ↵ 打开 · ⌘K 命令",
                ),
                NavView::Dossier | NavView::Agent => (
                    if self.active_view == NavView::Agent {
                        VIOLET_FG
                    } else {
                        EMERALD_FG
                    },
                    "1 财报K线 · 2 Alan · 3 XBRL · t TTM/FY · j/k 换股 · p Prompt · c CLI · esc 返回",
                ),
                NavView::Journal => (
                    GOLD_FG,
                    "/ 或 ⌘K 命令面板 · c 复制 journal CLI · esc 返回总览",
                ),
            }
        };

        div()
            .h(px(24.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(BG_STATUSLINE))
            .border_t_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(dot_col)))
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED))
                            .truncate()
                            .child(legend),
                    ),
            )
            .child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .text_color(rgb(INK_MUTED))
                    .child("ZERO-LLM · PAPER ONLY"),
            )
    }
}

impl Render for MungerWorkbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let in_company = matches!(self.active_view, NavView::Dossier | NavView::Agent);

        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .flex()
            .flex_row()
            .bg(rgb(BG_SHELL))
            .text_color(rgb(INK_PRIMARY))
            .font_family(self.fonts.ui.clone())
            .text_size(px(12.))
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .pt_1p5()
                    .pr_1p5()
                    .pb_1p5()
                    .bg(rgb(BG_SHELL))
                    .child(
                        div()
                            .size_full()
                            .rounded_lg()
                            .bg(rgb(BG_VIEWPORT))
                            .border_1()
                            .border_color(rgb(BORDER_VIEWPORT))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .child(self.render_toolbar(cx))
                            .when_some(self.banner.clone(), |d, (msg, is_err)| {
                                d.child(
                                    div()
                                        .px_4()
                                        .py_1p5()
                                        .bg(rgba(if is_err { CORAL_BG } else { EMERALD_BG }))
                                        .border_b_1()
                                        .border_color(rgba(if is_err {
                                            CORAL_BORDER
                                        } else {
                                            EMERALD_BORDER
                                        }))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .text_color(rgb(if is_err {
                                                    CORAL_FG
                                                } else {
                                                    EMERALD_FG
                                                }))
                                                .child(msg),
                                        )
                                        .child(
                                            div()
                                                .id("dismiss-banner")
                                                .px_2()
                                                .py_0p5()
                                                .rounded_xs()
                                                .cursor_pointer()
                                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_SECONDARY))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.banner = None;
                                                    cx.notify();
                                                }))
                                                .child("×"),
                                        ),
                                )
                            })
                            .when(in_company, |d| d.child(self.render_company_shell(cx)))
                            .when(!in_company, |d| {
                                let content = match self.active_view {
                                    NavView::Baskets => self.render_baskets_view(cx),
                                    NavView::Journal => self.render_journal_view(cx),
                                    NavView::Dossier | NavView::Agent => div(),
                                };
                                d.child(
                                    div()
                                        .id("main-scroll")
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .child(content),
                                )
                            })
                            .child(self.render_statusline()),
                    ),
            )
            .when(self.palette_open, |d| d.child(self.render_command_palette(cx)))
    }
}

fn journal_stat(label: &str, value: String, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_baseline()
        .gap_1p5()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(label.to_owned()),
        )
        .child(
            div()
                .font_family(fonts.mono_num.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(color))
                .child(value),
        )
}

fn mono_left(text: String, width: f32, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .px_1()
        .font_family(fonts.mono_num.clone())
        .text_xs()
        .text_color(rgb(color))
        .truncate()
        .child(text)
}

fn mono_right(text: String, width: f32, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .px_1p5()
        .text_right()
        .font_family(fonts.mono_num.clone())
        .text_xs()
        .text_color(rgb(color))
        .truncate()
        .child(text)
}

fn card_title(title: &str, tag: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .pb_2()
        .mb_2()
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO))
                .child(title.to_owned()),
        )
        .child(
            div()
                .font_family(fonts.mono_term.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(tag.to_owned()),
        )
}

fn kv_line(key: &str, value: String, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .mt_1p5()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_SECONDARY))
                .child(key.to_owned()),
        )
        .child(
            div()
                .font_family(fonts.mono_num.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(color))
                .child(value),
        )
}

fn inspector_kv(key: &str, value: String, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .w(px(120.))
                .flex_shrink_0()
                .font_family(fonts.mono_term.clone())
                .text_size(px(11.))
                .text_color(rgb(INK_MUTED))
                .child(key.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(fonts.mono_term.clone())
                .text_xs()
                .text_color(rgb(INK_PRIMARY))
                .truncate()
                .child(value),
        )
}

fn prose_block(title: &str, body: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(INK_SECONDARY))
                .child(title.to_owned()),
        )
        .child(
            div()
                .mt_1()
                .font_family(fonts.ui.clone())
                .text_size(px(13.))
                .line_height(px(21.))
                .text_color(rgb(INK_PRIMARY))
                .child(body.to_owned()),
        )
}

fn render_warning_panel(warnings: &[WarningFlag], fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(warnings.iter().map(|w| {
            let (fg, bg, br) = match w.severity {
                WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
            };
            let sev_label = match w.severity {
                WarningSeverity::Red => "红灯预警",
                WarningSeverity::Amber => "黄灯关注",
                WarningSeverity::Info => "提示",
            };
            div()
                .rounded_md()
                .bg(rgba(bg))
                .border_1()
                .border_color(rgba(br))
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_2p5()
                .child(badge(sev_label.to_owned(), fg, bg, br, fonts))
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(fg))
                        .child(w.title.clone()),
                )
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_PRIMARY))
                        .child(format!("— {}", w.detail)),
                )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, ValuationAssumptions};

    fn make_rep(
        ticker: &str,
        bucket: Bucket,
        circle: &str,
        roic: Option<Decimal>,
        margin: Option<Decimal>,
    ) -> MungerReport {
        MungerReport {
            schema_version: 1,
            computed_at_ms: 0,
            ticker: ticker.into(),
            name: ticker.into(),
            bucket,
            source: DataSource::Manual,
            cik: None,
            currency: "USD".into(),
            circle: circle.into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(18),
                multiple_high: Decimal::from(22),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
            price: None,
            raw_sha256: None,
            latest_filing_end: None,
            annual_rows: Vec::new(),
            ttm_row: None,
            latest_ytd: None,
            roic_5y_avg_pct: roic,
            roic_5y_stddev_pct: None,
            gross_margin_5y_avg_pct: None,
            gross_margin_5y_stddev_pct: None,
            share_change_3y_pct: None,
            share_change_5y_pct: None,
            net_debt: None,
            net_debt_to_owner_earnings: None,
            fy_valuation: None,
            ttm_valuation: margin.map(|d| ValuationSnapshot {
                basis_label: "TTM".into(),
                period_end: "2026-09-30".into(),
                strict_owner_earnings: Decimal::ZERO,
                shares_basis: Decimal::ONE,
                owner_earnings_per_share: None,
                market_cap: None,
                enterprise_value: None,
                strict_fcf_yield_mcap_pct: None,
                strict_fcf_yield_ev_pct: None,
                fair_value_low: None,
                fair_value_high: None,
                strike_price: None,
                distance_to_strike_pct: Some(d),
                implied_growth_for_target_pct: None,
                expected_total_return_pct: None,
            }),
            warnings: Vec::new(),
            status: if d_is_strike(margin) {
                StrikeZoneStatus::InStrikeZone
            } else {
                StrikeZoneStatus::Wait
            },
            provenance_summary: Vec::new(),
        }
    }

    fn d_is_strike(margin: Option<Decimal>) -> bool {
        margin.is_some_and(|d| d <= Decimal::ZERO)
    }

    #[test]
    fn fmt_pct_clean_clamps_near_zero_and_handles_none() {
        assert_eq!(
            fmt_pct_clean(Some(Decimal::new(-2, 3)), true),
            ("0.00%".into(), 0)
        );
        assert_eq!(
            fmt_pct_clean(Some(Decimal::new(4, 3)), false),
            ("0.00%".into(), 0)
        );
        assert_eq!(
            fmt_pct_clean(Some(Decimal::new(-690, 2)), true),
            ("-6.90%".into(), -1)
        );
        assert_eq!(
            fmt_pct_clean(Some(Decimal::new(215, 2)), true),
            ("+2.15%".into(), 1)
        );
        assert_eq!(fmt_pct_clean(None, false), ("—".into(), 0));
    }

    #[test]
    fn sort_reports_puts_none_last_in_both_directions() {
        let a = make_rep(
            "AAPL",
            Bucket::Core,
            "消费生态 / 硬件",
            Some(Decimal::from(90)),
            Some(Decimal::from(135)),
        );
        let b = make_rep("DSY.PA", Bucket::Watch, "工业软件 / CATIA", None, None);
        let c = make_rep(
            "PTC",
            Bucket::Core,
            "工业软件 / PLM",
            Some(Decimal::from(12)),
            Some(Decimal::from(80)),
        );

        let mut rows_desc = vec![&a, &b, &c];
        sort_reports(&mut rows_desc, SortCol::Roic, false);
        assert_eq!(
            rows_desc.iter().map(|r| r.ticker.as_str()).collect::<Vec<_>>(),
            vec!["AAPL", "PTC", "DSY.PA"]
        );

        let mut rows_asc = vec![&b, &a, &c];
        sort_reports(&mut rows_asc, SortCol::Roic, true);
        assert_eq!(
            rows_asc.iter().map(|r| r.ticker.as_str()).collect::<Vec<_>>(),
            vec!["PTC", "AAPL", "DSY.PA"]
        );

        let mut rows_margin = vec![&b, &a, &c];
        sort_reports(&mut rows_margin, SortCol::Margin, true);
        assert_eq!(
            rows_margin.iter().map(|r| r.ticker.as_str()).collect::<Vec<_>>(),
            vec!["PTC", "AAPL", "DSY.PA"]
        );
    }

    #[test]
    fn primary_circle_tag_extracts_shared_domain_and_filters_baskets() {
        let adsk = make_rep(
            "ADSK",
            Bucket::Core,
            "工业软件 / CAD",
            Some(Decimal::from(40)),
            Some(Decimal::from(30)),
        );
        let ptc = make_rep(
            "PTC",
            Bucket::Core,
            "工业软件 / PLM+CAD",
            Some(Decimal::from(12)),
            Some(Decimal::from(-5)),
        );
        let dsy = make_rep("DSY.PA", Bucket::Watch, "工业软件 / 3DX", None, None);
        let cost = make_rep(
            "COST",
            Bucket::Watch,
            "会员制零售 / 标杆",
            Some(Decimal::from(35)),
            Some(Decimal::from(90)),
        );
        let reports = vec![adsk, ptc, dsy, cost];

        let tags = collect_circle_tags(&reports);
        assert_eq!(tags[0], ("工业软件".to_owned(), 3));

        let core_ind = filter_reports_for_basket(
            &reports,
            Bucket::Core,
            Some("工业软件"),
            StatusQuickFilter::StrikeOnly,
            SortCol::Default,
            false,
        );
        assert_eq!(core_ind.len(), 1);
        assert_eq!(core_ind[0].ticker, "PTC");

        let watch_ind = filter_reports_for_basket(
            &reports,
            Bucket::Watch,
            Some("工业软件"),
            StatusQuickFilter::All,
            SortCol::Default,
            false,
        );
        assert_eq!(watch_ind.len(), 1);
        assert_eq!(watch_ind[0].ticker, "DSY.PA");
    }

    #[test]
    fn today_pulse_highlights_core_strike_red_and_watch_counts() {
        let strike_core = make_rep(
            "ADSK",
            Bucket::Core,
            "工业软件",
            Some(Decimal::from(40)),
            Some(Decimal::from(-8)),
        );
        let mut red_core = make_rep(
            "GOOGL",
            Bucket::Core,
            "搜索",
            Some(Decimal::from(35)),
            Some(Decimal::from(40)),
        );
        red_core.status = StrikeZoneStatus::WarningBlocked;
        let watch1 = make_rep("COST", Bucket::Watch, "零售", None, None);
        let mut out_strike = make_rep("INTC", Bucket::Out, "芯片", None, Some(Decimal::from(-10)));
        out_strike.status = StrikeZoneStatus::InStrikeZone;

        let pulse = compute_today_pulse(&[strike_core, red_core, watch1, out_strike]);
        assert_eq!(pulse.strike_tickers, vec!["ADSK".to_owned()]);
        assert_eq!(pulse.red_core_tickers, vec!["GOOGL".to_owned()]);
        assert_eq!(pulse.watch_count, 1);
    }

    #[test]
    fn compute_kline_domain_includes_strike_and_handles_empty() {
        assert!(compute_kline_domain(&[], Some(Decimal::from(100)), None).is_none());

        let bars = vec![
            OhlcBar {
                ts_ms: 1,
                open: Decimal::from(150),
                high: Decimal::from(210),
                low: Decimal::from(140),
                close: Decimal::from(195),
                volume: 5000,
            },
            OhlcBar {
                ts_ms: 2,
                open: Decimal::from(195),
                high: Decimal::from(240),
                low: Decimal::from(180),
                close: Decimal::from(220),
                volume: 8200,
            },
        ];

        let (ymin, ymax, maxv) =
            compute_kline_domain(&bars, Some(Decimal::from(120)), Some(Decimal::from(145)))
                .unwrap();
        assert!(ymin < 120.0);
        assert!(ymax > 240.0);
        assert_eq!(maxv, 8200);

        let (ymin2, ymax2, _) =
            compute_kline_domain(&bars, Some(Decimal::from(30)), None).unwrap();
        assert!(ymin2 > 110.0);
        assert!(ymax2 > 240.0);
    }

    #[test]
    fn filter_palette_matches_ticker_circle_and_commands() {
        let items = vec![
            PaletteItem {
                group: "DOSSIER".into(),
                title: "AAPL  Apple Inc. [IN]".into(),
                subtitle: "#消费生态 · 耐心等待".into(),
                shortcut: Some("↵"),
                dot_color: EMERALD_FG,
                action: PaletteAction::OpenDossier("AAPL".into()),
            },
            PaletteItem {
                group: "VIEW".into(),
                title: "XBRL 底表 · 多年原账与公式审计".into(),
                subtitle: "查看 AAPL 算术推导与 SHA-256 溯源".into(),
                shortcut: Some("i"),
                dot_color: SKY_FG,
                action: PaletteAction::ToggleInspector,
            },
            PaletteItem {
                group: "ALAN".into(),
                title: "Alan 审讯 · AAPL".into(),
                subtitle: "独立 AI 幕僚".into(),
                shortcut: None,
                dot_color: VIOLET_FG,
                action: PaletteAction::OpenAgent("AAPL".into()),
            },
        ];

        assert_eq!(filter_palette_items(&items, "").len(), 3);
        let aapl = filter_palette_items(&items, "aapl");
        assert_eq!(aapl.len(), 3);
        let xbrl = filter_palette_items(&items, "xbrl 审计");
        assert_eq!(xbrl.len(), 1);
        assert_eq!(xbrl[0].action, PaletteAction::ToggleInspector);
        assert!(filter_palette_items(&items, "nonexistent-ticker").is_empty());
    }
}
