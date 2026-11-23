mod agent;
mod baskets;
mod chart;
mod company;
mod components;
mod format;
mod journal;
mod palette;
mod shell;
mod theme;
#[cfg(test)]
mod tests;

use format::{
    build_alan_interrogation_prompt, filter_palette_items, filter_reports_for_basket,
};

use crate::{
    alan_interrogator::{AcmeEvidenceToken, M3InterrogatorHandle, TuringTapeEntry},
    model::{
        Bucket, DecisionJournal, Dossier, MungerReport, PriceHistorySnapshot, PriceSnapshot,
        SyncStatusFile, ValuationAssumptions,
    },
    store::{self, WorkspacePaths},
};
use gpui::{
    ClipboardItem, Context, FocusHandle, KeyDownEvent, SharedString, Window, div, prelude::*, px,
    rgb, rgba,
};
use rust_decimal::Decimal;
use std::{
    collections::HashSet,
    sync::mpsc,
};
use theme::*;

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
enum AgentMessageRole {
    User,
    Assistant,
    SystemNote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentUiMessage {
    id: String,
    role: AgentMessageRole,
    raw_text: String,
    text: String,
    tokens: Vec<AcmeEvidenceToken>,
    streaming: bool,
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
    alan_handle: Option<M3InterrogatorHandle>,
    agent_session_ticker: Option<String>,
    agent_input_focused: bool,
    agent_input_draft: String,
    agent_streaming: bool,
    agent_status_hint: Option<String>,
    agent_messages: Vec<AgentUiMessage>,
    agent_tape: Vec<TuringTapeEntry>,
    agent_expanded_evidence: Option<String>,
    agent_expanded_tape: Option<usize>,
    agent_seq: usize,
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
            alan_handle: None,
            agent_session_ticker: None,
            agent_input_focused: false,
            agent_input_draft: String::new(),
            agent_streaming: false,
            agent_status_hint: None,
            agent_messages: Vec::new(),
            agent_tape: Vec::new(),
            agent_expanded_evidence: None,
            agent_expanded_tape: None,
            agent_seq: 1,
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
        self.agent_input_focused = false;
        self.agent_expanded_evidence = None;
        if let Some(handle) = &self.alan_handle {
            handle.refresh_fs();
        }
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
        if view != NavView::Agent {
            self.agent_input_focused = false;
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

        if self.active_view == NavView::Agent && self.agent_input_focused {
            match key {
                "escape" => {
                    self.agent_input_focused = false;
                    cx.stop_propagation();
                    cx.notify();
                }
                "backspace" => {
                    self.agent_input_draft.pop();
                    cx.stop_propagation();
                    cx.notify();
                }
                "enter" => {
                    let draft = self.agent_input_draft.clone();
                    let ticker = self.selected_ticker.clone();
                    let (name, bucket) = self
                        .reports
                        .iter()
                        .find(|r| r.ticker == ticker)
                        .map(|r| (r.name.clone(), r.bucket))
                        .unwrap_or_else(|| (ticker.clone(), Bucket::Watch));
                    self.submit_alan_question(&ticker, &name, bucket, draft);
                    cx.stop_propagation();
                    cx.notify();
                }
                _ => {
                    if let Some(ch) = ev.keystroke.key_char.as_deref()
                        && !ch.is_empty()
                        && !ch.chars().any(char::is_control)
                    {
                        self.agent_input_draft.push_str(ch);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
            }
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

impl Drop for MungerWorkbench {
    fn drop(&mut self) {
        if let Some(handle) = self.alan_handle.take() {
            handle.shutdown();
        }
    }
}
