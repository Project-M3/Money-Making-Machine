use super::{
    FontRoles, MungerWorkbench, NavView, components::*, format::*, theme::*,
    universe::UniverseSearchMode,
};
use crate::{
    events::{EventCatchupOptions, catchup_events_with_progress},
    lake::{LakeDb, UniverseFilter},
    lake_l1::{SignalEvidence, SignalInboxRow, SignalKind, SignalStatus},
    lake_l2::BudgetStatusReport,
    model::Bucket,
    store::{self, WorkspacePaths},
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

const AUTO_CATCHUP_INTERVAL: Duration = Duration::from_secs(20 * 60);

/// Filter tabs shown in the Event-Driven Inbox bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InboxFilter {
    #[default]
    Unread,
    All,
    Discovery,
    Ripple,
    StrikeAndRedFlag,
    Archived,
}

impl InboxFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unread => "未读",
            Self::All => "全部",
            Self::Discovery => "新浮现",
            Self::Ripple => "隐空间传导",
            Self::StrikeAndRedFlag => "击球与红灯",
            Self::Archived => "已归档",
        }
    }

    pub fn all() -> [Self; 6] {
        [
            Self::Unread,
            Self::All,
            Self::Discovery,
            Self::Ripple,
            Self::StrikeAndRedFlag,
            Self::Archived,
        ]
    }
}

/// UI navigation state for the Inbox view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InboxViewState {
    pub filter: InboxFilter,
    pub selected_signal_id: Option<String>,
    pub cursor_index: usize,
}

/// Filters `signals` according to the active Inbox tab.
pub fn filter_inbox_signals(
    signals: &[SignalInboxRow],
    filter: InboxFilter,
) -> Vec<&SignalInboxRow> {
    signals
        .iter()
        .filter(|s| match filter {
            InboxFilter::Unread => s.status == SignalStatus::Unread,
            InboxFilter::All => true,
            InboxFilter::Discovery => {
                s.kind == SignalKind::Discovery && s.status != SignalStatus::Archived
            }
            InboxFilter::Ripple => {
                s.kind == SignalKind::Ripple && s.status != SignalStatus::Archived
            }
            InboxFilter::StrikeAndRedFlag => {
                matches!(s.kind, SignalKind::Strike | SignalKind::RedFlag)
                    && s.status != SignalStatus::Archived
            }
            InboxFilter::Archived => s.status == SignalStatus::Archived,
        })
        .collect()
}

/// Counts visible signals for a specific filter tab.
pub fn count_inbox_filter(signals: &[SignalInboxRow], filter: InboxFilter) -> usize {
    filter_inbox_signals(signals, filter).len()
}

/// Resolves the active signal in `visible`, preferring `selected_id` when present,
/// otherwise clamping `cursor` to the available slice.
pub fn resolve_active_inbox_signal<'a>(
    visible: &[&'a SignalInboxRow],
    selected_id: Option<&str>,
    cursor: usize,
) -> Option<(usize, &'a SignalInboxRow)> {
    if visible.is_empty() {
        return None;
    }
    if let Some(id) = selected_id
        && let Some((idx, sig)) = visible.iter().enumerate().find(|(_, s)| s.signal_id == id)
    {
        return Some((idx, *sig));
    }
    let idx = cursor.min(visible.len() - 1);
    Some((idx, visible[idx]))
}

/// Computes the next `(cursor_index, signal_id)` after stepping by `delta`.
pub fn step_inbox_selection(
    visible: &[&SignalInboxRow],
    selected_id: Option<&str>,
    cursor: usize,
    delta: isize,
) -> Option<(usize, String)> {
    if visible.is_empty() {
        return None;
    }
    let cur_idx = resolve_active_inbox_signal(visible, selected_id, cursor)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let next_idx = (cur_idx as isize + delta).clamp(0, (visible.len() - 1) as isize) as usize;
    Some((next_idx, visible[next_idx].signal_id.clone()))
}

/// Returns `(label, fg, bg, border)` styling for a signal kind.
pub fn signal_kind_badge(kind: SignalKind) -> (&'static str, u32, u32, u32) {
    match kind {
        SignalKind::Discovery => ("新浮现", SKY_FG, SKY_BG, SKY_BORDER),
        SignalKind::Ripple => ("隐空间传导", VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
        SignalKind::Strike => ("击球区", EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        SignalKind::RedFlag => ("红灯预警", CORAL_FG, CORAL_BG, CORAL_BORDER),
    }
}

/// Returns `(label, fg, bg, border)` styling for a signal triage status.
pub fn signal_status_badge(status: SignalStatus) -> (&'static str, u32, u32, u32) {
    match status {
        SignalStatus::Unread => ("未读", SKY_FG, SKY_BG, SKY_BORDER),
        SignalStatus::Researching => ("Alan 研判", VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
        SignalStatus::Watch => ("观察池", EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        SignalStatus::TooHard => ("TOO HARD", SLATE_FG, SLATE_BG, SLATE_BORDER),
        SignalStatus::Archived => ("已归档", INK_MUTED, BG_WELL, BORDER_SUBTLE),
    }
}

/// Formats the monthly AI budget watermark for the Inbox KPI ribbon (`本月 AI 预算 $X.XX / $10.00`).
pub fn budget_watermark_summary(report: Option<&BudgetStatusReport>) -> (String, u32) {
    let Some(r) = report else {
        return ("本月 AI 预算 $0.00 / $10.00".to_owned(), EMERALD_FG);
    };
    let used = r.usage.total_cost_usd.round_dp(2).normalize();
    let limit = r.config.monthly_budget_usd.round_dp(2).normalize();
    let exhausted = r.remaining_monthly_usd <= Decimal::ZERO || r.remaining_monthly_tokens == 0;
    let color = if exhausted {
        CORAL_FG
    } else if r.config.monthly_budget_usd > Decimal::ZERO
        && used * Decimal::from(10) >= r.config.monthly_budget_usd * Decimal::from(8)
    {
        GOLD_FG
    } else {
        EMERALD_FG
    };
    (format!("本月 AI 预算 ${used:.2} / ${limit:.2}"), color)
}

/// Evaluates whether startup/background auto-catchup should be skipped.
pub(super) fn should_skip_auto_catchup_with(
    is_test: bool,
    no_auto_env: Option<&str>,
    lake_present: bool,
) -> bool {
    if is_test || !lake_present {
        return true;
    }
    no_auto_env.is_some_and(|raw| {
        let trimmed = raw.trim();
        trimmed == "1" || trimmed.eq_ignore_ascii_case("true")
    })
}

/// Returns `true` when startup / periodic auto-catchup must not run.
pub fn should_skip_auto_catchup(lake_present: bool) -> bool {
    let env_val = std::env::var("M3_NO_AUTO_CATCHUP").ok();
    should_skip_auto_catchup_with(cfg!(test), env_val.as_deref(), lake_present)
}

impl MungerWorkbench {
    /// Triggers a non-blocking startup L1 catch-up if enabled and lake exists.
    pub(super) fn maybe_start_startup_catchup(&mut self) {
        if should_skip_auto_catchup(self.universe_lake_present) {
            return;
        }
        let bootstrap = self
            .inbox_summary
            .as_ref()
            .is_none_or(|s| s.last_sec_bootstrap_ms.is_none());
        self.spawn_l1_catchup_worker(bootstrap, true);
    }

    /// Manually triggers background L1 event-stream catch-up from UI/keyboard.
    pub(super) fn start_l1_catchup(&mut self, bootstrap: bool, cx: &mut Context<Self>) {
        if self.inbox_catching_up || self.universe_syncing || self.syncing {
            if self.universe_syncing || self.syncing {
                self.banner = Some((
                    "其他后台同步任务进行中，请等待完成后再追平事件流".into(),
                    false,
                ));
                cx.notify();
            }
            return;
        }
        self.spawn_l1_catchup_worker(bootstrap, false);
        cx.notify();
    }

    fn spawn_l1_catchup_worker(&mut self, bootstrap: bool, silent_startup: bool) {
        if self.inbox_catching_up || self.universe_syncing || self.syncing {
            return;
        }
        self.inbox_catching_up = true;
        self.inbox_last_auto_catchup_at = Some(Instant::now());
        self.inbox_catchup_progress = Some(if silent_startup {
            "启动后台追平 L1 事件流…".into()
        } else {
            "正在追平 SEC 公告、新闻与语义索引…".into()
        });
        if !silent_startup {
            self.banner = Some((
                "正在后台追平 L1 事件流（SEC 公告 + 候选新闻 + 本地语义索引，0 Flagship Token）…"
                    .into(),
                false,
            ));
        }

        let root = self.paths.repo_root.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let (prog_tx, prog_rx) = mpsc::channel();
        self.inbox_catchup_rx = Some(done_rx);
        self.inbox_progress_rx = Some(prog_rx);

        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            let opts = EventCatchupOptions {
                bootstrap,
                ..EventCatchupOptions::default()
            };
            let res = catchup_events_with_progress(&paths, &opts, |msg| {
                let _ = prog_tx.send(msg.to_owned());
            })
            .map_err(|e| format!("{e:#}"));
            let _ = done_tx.send(res);
        });
    }

    /// Non-blocking poll for L1 catch-up progress, completion, and 20-minute cadence.
    pub(super) fn poll_l1_catchup(&mut self) -> bool {
        let mut dirty = false;
        if let Some(prog_rx) = self.inbox_progress_rx.as_ref() {
            while let Ok(msg) = prog_rx.try_recv() {
                self.inbox_catchup_progress = Some(msg);
                dirty = true;
            }
        }
        if let Some(rx) = self.inbox_catchup_rx.as_ref() {
            match rx.try_recv() {
                Ok(Ok(report)) => {
                    self.inbox_catching_up = false;
                    self.inbox_catchup_rx = None;
                    self.inbox_progress_rx = None;
                    self.inbox_catchup_progress = None;
                    self.inbox_last_auto_catchup_at = Some(Instant::now());
                    self.reload_inbox_from_lake();
                    self.banner = Some((
                        format!(
                            "L1 事件流追平完成 · 新增公告 {} / 新闻 {} / 索引 {} 片段 / 信号 {}（未读 {} · {} Flagship Token）",
                            report.sec_filings_ingested,
                            report.news_events_ingested,
                            report.chunks_indexed,
                            report.signals_created_or_updated,
                            report.summary.unread_signals,
                            report.flagship_tokens_used,
                        ),
                        false,
                    ));
                    return true;
                }
                Ok(Err(err)) => {
                    self.inbox_catching_up = false;
                    self.inbox_catchup_rx = None;
                    self.inbox_progress_rx = None;
                    self.inbox_catchup_progress = None;
                    self.inbox_last_auto_catchup_at = Some(Instant::now());
                    self.banner = Some((format!("L1 事件流追平失败：{err}"), true));
                    return true;
                }
                Err(mpsc::TryRecvError::Empty) => return dirty,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.inbox_catching_up = false;
                    self.inbox_catchup_rx = None;
                    self.inbox_progress_rx = None;
                    self.inbox_catchup_progress = None;
                    return true;
                }
            }
        }

        if !should_skip_auto_catchup(self.universe_lake_present)
            && !self.inbox_catching_up
            && !self.universe_syncing
            && !self.syncing
            && self
                .inbox_last_auto_catchup_at
                .is_some_and(|t| t.elapsed() >= AUTO_CATCHUP_INTERVAL)
        {
            self.spawn_l1_catchup_worker(false, true);
            return true;
        }

        dirty
    }

    /// Reloads L1 summary & inbox signals from `m3.duckdb` in read-only mode.
    pub(super) fn reload_inbox_from_lake(&mut self) {
        if self.universe_syncing {
            return;
        }
        match LakeDb::open_read_only_if_exists(&self.paths) {
            Ok(None) => {
                self.inbox_summary = None;
                self.inbox_signals.clear();
                self.inbox_error = None;
                self.inbox_state.selected_signal_id = None;
                self.inbox_state.cursor_index = 0;
            }
            Ok(Some(db)) => {
                let summary_res = db.l1_summary();
                let list_res = db.list_inbox_signals(None, None, 500);
                if let Ok(budget) = db.budget_status_report(&self.paths) {
                    self.budget_report = Some(budget);
                }
                match (summary_res, list_res) {
                    (Ok(summary), Ok(signals)) => {
                        self.inbox_summary = Some(summary);
                        self.inbox_signals = signals;
                        self.inbox_error = None;
                        self.sync_inbox_selection();
                    }
                    (Err(err), _) | (_, Err(err)) => {
                        self.inbox_error = Some(format!("读取 L1 简报收件箱失败：{err:#}"));
                    }
                }
            }
            Err(err) => {
                self.inbox_error = Some(format!("打开 m3.duckdb 失败：{err:#}"));
            }
        }
    }

    pub(super) fn sync_inbox_selection(&mut self) {
        let visible = filter_inbox_signals(&self.inbox_signals, self.inbox_state.filter);
        if let Some((idx, sig)) = resolve_active_inbox_signal(
            &visible,
            self.inbox_state.selected_signal_id.as_deref(),
            self.inbox_state.cursor_index,
        ) {
            self.inbox_state.cursor_index = idx;
            self.inbox_state.selected_signal_id = Some(sig.signal_id.clone());
        } else {
            self.inbox_state.cursor_index = 0;
            self.inbox_state.selected_signal_id = None;
        }
    }

    pub(super) fn set_inbox_filter(&mut self, filter: InboxFilter) {
        self.inbox_state.filter = filter;
        self.inbox_state.cursor_index = 0;
        self.inbox_state.selected_signal_id = None;
        self.sync_inbox_selection();
    }

    pub(super) fn step_inbox_cursor(&mut self, delta: isize) {
        let visible = filter_inbox_signals(&self.inbox_signals, self.inbox_state.filter);
        if let Some((idx, id)) = step_inbox_selection(
            &visible,
            self.inbox_state.selected_signal_id.as_deref(),
            self.inbox_state.cursor_index,
            delta,
        ) {
            self.inbox_state.cursor_index = idx;
            self.inbox_state.selected_signal_id = Some(id);
        }
    }

    pub(super) fn active_inbox_signal(&self) -> Option<&SignalInboxRow> {
        let visible = filter_inbox_signals(&self.inbox_signals, self.inbox_state.filter);
        resolve_active_inbox_signal(
            &visible,
            self.inbox_state.selected_signal_id.as_deref(),
            self.inbox_state.cursor_index,
        )
        .map(|(_, sig)| sig)
    }

    /// Updates a signal's triage status, optionally promoting its primary ticker to Watchlist
    /// and/or opening Alan interrogation desk.
    pub(super) fn triage_inbox_signal(
        &mut self,
        signal_id: &str,
        target_status: SignalStatus,
        promote_watch: bool,
        open_agent: bool,
        cx: &mut Context<Self>,
    ) {
        if self.universe_syncing || self.inbox_catching_up {
            self.banner = Some(("后台事件追平或湖仓同步进行中，请稍候再分拣".into(), false));
            cx.notify();
            return;
        }

        let Some(sig) = self
            .inbox_signals
            .iter()
            .find(|s| s.signal_id == signal_id)
            .cloned()
        else {
            self.banner = Some((format!("未找到信号 {signal_id}"), true));
            cx.notify();
            return;
        };

        let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
        let in_watchlist_already = self
            .reports
            .iter()
            .any(|r| r.ticker.eq_ignore_ascii_case(&ticker));

        let Ok(db) = LakeDb::open(&self.paths) else {
            self.banner = Some(("打开 m3.duckdb 失败，无法更新信号状态".into(), true));
            cx.notify();
            return;
        };

        match db.triage_signal(signal_id, target_status, store::now_ms()) {
            Ok(true) => {}
            Ok(false) => {
                self.banner = Some((format!("信号 {signal_id} 不存在或未变更"), true));
                cx.notify();
                return;
            }
            Err(err) => {
                self.banner = Some((format!("分拣信号失败：{err:#}"), true));
                cx.notify();
                return;
            }
        }

        let need_watch_add = promote_watch || (open_agent && !in_watchlist_already);
        if need_watch_add && !ticker.is_empty() {
            let universe_row = db
                .screen_universe(&UniverseFilter {
                    exclude_financials: false,
                    query: Some(ticker.clone()),
                    limit: 20,
                    ..Default::default()
                })
                .ok()
                .and_then(|rows| {
                    rows.into_iter()
                        .find(|r| r.ticker.eq_ignore_ascii_case(&ticker))
                });

            let (name_opt, cik_opt, circle_opt) = if let Some(row) = &universe_row {
                let circle = match (row.sic, row.sic_description.as_deref()) {
                    (Some(sic), Some(desc)) if !desc.trim().is_empty() => {
                        Some(format!("{} / SIC {sic}", desc.trim()))
                    }
                    (None, Some(desc)) if !desc.trim().is_empty() => Some(desc.trim().to_owned()),
                    (Some(sic), _) => Some(format!("SIC {sic}")),
                    _ => None,
                };
                let cik = (!row.cik.trim().is_empty()).then_some(row.cik.as_str());
                (Some(row.name.as_str()), cik, circle)
            } else {
                (None, None, None)
            };

            if let Err(err) = store::add_or_update_company_with_cik(
                &self.paths,
                &ticker,
                Bucket::Watch,
                name_opt,
                cik_opt,
                circle_opt.as_deref(),
                None,
            ) {
                self.reload_inbox_from_lake();
                self.banner = Some((
                    format!("已更新信号状态，但加入观察池 ({ticker}) 失败：{err:#}"),
                    true,
                ));
                cx.notify();
                return;
            }
            let _ = db.refresh_watchlist_flags(&self.paths);
            drop(db);
            self.reload_from_disk();
            self.reload_universe_from_lake();
        } else {
            drop(db);
        }

        self.reload_inbox_from_lake();

        let status_label = signal_status_badge(target_status).0;
        self.banner = Some((
            if promote_watch {
                format!("已将 {ticker} 加入观察池 [WATCH] 并标记信号为「{status_label}」")
            } else {
                format!("已将 {ticker} 信号标记为「{status_label}」")
            },
            false,
        ));

        if open_agent && !ticker.is_empty() {
            let canonical = self
                .reports
                .iter()
                .find(|r| r.ticker.eq_ignore_ascii_case(&ticker))
                .map(|r| r.ticker.clone())
                .unwrap_or(ticker);
            self.select_company(&canonical, Some(NavView::Agent));
            self.start_deep_research(&canonical, None, "inbox", cx);
            return;
        }

        cx.notify();
    }

    /// Opens Dossier when ticker is in the user's watchlist, otherwise focuses it in Universe Radar.
    pub(super) fn open_inbox_signal_target(&mut self, ticker: &str, cx: &mut Context<Self>) {
        let trimmed = ticker.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(canonical) = self
            .reports
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(trimmed))
            .map(|r| r.ticker.clone())
        {
            self.select_company(&canonical, Some(NavView::Dossier));
            cx.notify();
            return;
        }

        self.universe_state.search_mode = UniverseSearchMode::Keyword;
        self.universe_state.query = trimmed.to_owned();
        self.universe_state.exclude_financials = false;
        self.universe_state.require_5y_history = false;
        self.switch_view(NavView::Universe);
        if let Some(exact) = self
            .universe_rows
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(trimmed))
            .cloned()
        {
            self.select_universe_row(&exact);
        }
        cx.notify();
    }

    pub(super) fn render_inbox_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let lake_empty = !self.universe_lake_present
            || self
                .universe_summary
                .as_ref()
                .is_some_and(|s| s.total_companies == 0);
        let visible = filter_inbox_signals(&self.inbox_signals, self.inbox_state.filter);
        let active = resolve_active_inbox_signal(
            &visible,
            self.inbox_state.selected_signal_id.as_deref(),
            self.inbox_state.cursor_index,
        );

        let mut root = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(self.render_inbox_kpi_ribbon(cx))
            .child(self.render_inbox_filter_bar(cx));

        if let Some(err) = &self.inbox_error {
            root = root.child(
                div()
                    .px_5()
                    .py_2()
                    .bg(rgba(CORAL_BG))
                    .border_b_1()
                    .border_color(rgba(CORAL_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(CORAL_FG))
                    .child(err.clone()),
            );
        }

        if lake_empty && self.inbox_signals.is_empty() {
            return root.child(
                div()
                    .id("inbox-empty-lake-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_6()
                    .child(self.render_inbox_empty_lake_state(cx)),
            );
        }

        let left_list = div()
            .id("inbox-signal-list")
            .w(px(430.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .bg(rgb(BG_VIEWPORT))
            .when(visible.is_empty(), |d| {
                d.child(self.render_inbox_empty_filter_state(cx))
            })
            .when(!visible.is_empty(), |d| {
                d.children(visible.iter().enumerate().map(|(idx, sig)| {
                    let is_selected = active.is_some_and(|(_, a)| a.signal_id == sig.signal_id);
                    self.render_inbox_row(sig, idx, is_selected, cx)
                }))
            });

        let right_detail = div()
            .id("inbox-detail-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p_5()
            .child(match active {
                Some((_, sig)) => self.render_inbox_detail(sig, cx),
                None => self.render_inbox_no_selection(),
            });

        root.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_row()
                .overflow_hidden()
                .child(left_list)
                .child(right_detail),
        )
    }

    fn render_inbox_kpi_ribbon(&self, cx: &mut Context<Self>) -> gpui::Div {
        let u_sum = self.universe_summary.clone().unwrap_or_default();
        let l1_sum = self.inbox_summary.clone().unwrap_or_default();
        let last_catchup = fmt_time_cn(l1_sum.last_catchup_ms);
        let (budget_label, budget_col) = budget_watermark_summary(self.budget_report.as_ref());

        div()
            .h(px(44.))
            .flex_shrink_0()
            .px_5()
            .bg(rgb(BG_SURFACE))
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
                    .gap_3()
                    .min_w_0()
                    .child(journal_stat(
                        "全美股池",
                        format!("{} 家", u_sum.total_companies),
                        INK_HERO,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        "上次追平",
                        last_catchup,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        "语义索引",
                        format!(
                            "{} 片段 / {} 公司",
                            l1_sum.doc_chunks_count, l1_sum.indexed_companies_count
                        ),
                        SKY_FG,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        "未读信号",
                        format!("{}", l1_sum.unread_signals),
                        if l1_sum.unread_signals > 0 {
                            EMERALD_FG
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(badge(
                        budget_label,
                        budget_col,
                        if budget_col == CORAL_FG {
                            CORAL_BG
                        } else if budget_col == GOLD_FG {
                            GOLD_BG
                        } else {
                            EMERALD_BG
                        },
                        if budget_col == CORAL_FG {
                            CORAL_BORDER
                        } else if budget_col == GOLD_FG {
                            GOLD_BORDER
                        } else {
                            EMERALD_BORDER
                        },
                        &self.fonts,
                    ))
                    .child(badge(
                        "L0/L1 · 0 Flagship Token".into(),
                        EMERALD_FG,
                        EMERALD_BG,
                        EMERALD_BORDER,
                        &self.fonts,
                    ))
                    .when_some(self.inbox_catchup_progress.clone(), |d, msg| {
                        d.child(
                            div()
                                .max_w(px(220.))
                                .px_2()
                                .py_0p5()
                                .rounded_xs()
                                .bg(rgba(SKY_BG))
                                .border_1()
                                .border_color(rgba(SKY_BORDER))
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(10.))
                                .text_color(rgb(SKY_FG))
                                .truncate()
                                .child(msg),
                        )
                    })
                    .child(
                        div()
                            .id("inbox-catchup-btn")
                            .px_3()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .border_1()
                            .when(self.inbox_catching_up, |d| {
                                d.bg(rgb(BG_WELL))
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .text_color(rgb(INK_MUTED))
                            })
                            .when(!self.inbox_catching_up, |d| {
                                d.bg(rgb(CTA_BG))
                                    .border_color(rgb(CTA_BG))
                                    .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                    .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                                    .text_color(rgb(CTA_INK))
                            })
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_l1_catchup(false, cx);
                            }))
                            .child(if self.inbox_catching_up {
                                "追平中…"
                            } else {
                                "追平事件流"
                            }),
                    ),
            )
    }

    fn render_inbox_filter_bar(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex_shrink_0()
            .px_5()
            .py_2()
            .bg(rgb(BG_ROW_ALT))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(div().flex().items_center().flex_wrap().gap_1p5().children(
                InboxFilter::all().into_iter().map(|f| {
                    let active = self.inbox_state.filter == f;
                    let count = count_inbox_filter(&self.inbox_signals, f);
                    let label = if count > 0 || matches!(f, InboxFilter::Unread | InboxFilter::All)
                    {
                        format!("{} {count}", f.label())
                    } else {
                        f.label().to_owned()
                    };
                    div()
                        .id(gpui::SharedString::from(format!(
                            "inbox-filter-{}",
                            f.label()
                        )))
                        .px_2p5()
                        .py_1()
                        .rounded_xs()
                        .cursor_pointer()
                        .when(active, |d| {
                            d.bg(rgba(EMERALD_BG))
                                .border_1()
                                .border_color(rgba(EMERALD_BORDER))
                        })
                        .when(!active, |d| {
                            d.bg(rgb(BG_SURFACE))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                        })
                        .font_family(self.fonts.ui.clone())
                        .text_size(px(11.))
                        .font_weight(if active {
                            gpui::FontWeight::SEMIBOLD
                        } else {
                            gpui::FontWeight::NORMAL
                        })
                        .text_color(rgb(if active { EMERALD_FG } else { INK_SECONDARY }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_inbox_filter(f);
                            cx.notify();
                        }))
                        .child(label)
                }),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(kbd("j/k", &self.fonts))
                    .child(kbd("w 观察", &self.fonts))
                    .child(kbd("x TooHard", &self.fonts))
                    .child(kbd("e 归档", &self.fonts))
                    .child(kbd("d Alan", &self.fonts))
                    .child(kbd("r 追平", &self.fonts)),
            )
    }

    fn render_inbox_row(
        &self,
        sig: &SignalInboxRow,
        idx: usize,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (kind_label, kind_fg, kind_bg, kind_br) = signal_kind_badge(sig.kind);
        let (st_label, st_fg, st_bg, st_br) = signal_status_badge(sig.status);
        let sig_id = sig.signal_id.clone();
        let secondary_tickers: Vec<&str> = sig
            .tickers
            .iter()
            .filter(|t| !t.eq_ignore_ascii_case(&sig.primary_ticker))
            .take(3)
            .map(String::as_str)
            .collect();

        div()
            .id(gpui::SharedString::from(format!(
                "inbox-row-{}",
                sig.signal_id
            )))
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .cursor_pointer()
            .when(is_selected, |d| d.bg(rgb(BG_SELECTED_ROW)))
            .when(!is_selected, |d| {
                d.bg(rgb(if idx % 2 == 1 {
                    BG_ROW_ALT
                } else {
                    BG_VIEWPORT
                }))
                .hover(|s| s.bg(rgb(BG_HOVER)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.inbox_state.cursor_index = idx;
                this.inbox_state.selected_signal_id = Some(sig_id.clone());
                cx.notify();
            }))
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .when(sig.status == SignalStatus::Unread, |d| {
                                d.child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(kind_fg)))
                            })
                            .child(badge(
                                kind_label.into(),
                                kind_fg,
                                kind_bg,
                                kind_br,
                                &self.fonts,
                            ))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(sig.primary_ticker.clone()),
                            )
                            .when(!secondary_tickers.is_empty(), |d| {
                                d.child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(INK_MUTED))
                                        .truncate()
                                        .child(format!("← {}", secondary_tickers.join(", "))),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .when(sig.status != SignalStatus::Unread, |d| {
                                d.child(badge(st_label.into(), st_fg, st_bg, st_br, &self.fonts))
                            })
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("{:.2}", sig.score)),
                            ),
                    ),
            )
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .font_weight(if sig.status == SignalStatus::Unread {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(rgb(if is_selected { INK_HERO } else { INK_PRIMARY }))
                    .truncate()
                    .child(sig.title.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED))
                            .truncate()
                            .child(sig.summary.clone()),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(10.))
                            .text_color(rgb(INK_MUTED))
                            .child(format!("{} 条证据", sig.evidence.len())),
                    ),
            )
    }

    fn render_inbox_detail(&self, sig: &SignalInboxRow, cx: &mut Context<Self>) -> gpui::Div {
        let (kind_label, kind_fg, kind_bg, kind_br) = signal_kind_badge(sig.kind);
        let (st_label, st_fg, st_bg, st_br) = signal_status_badge(sig.status);
        let ticker_open = sig.primary_ticker.clone();
        let id_watch = sig.signal_id.clone();
        let id_alan = sig.signal_id.clone();
        let id_toohard = sig.signal_id.clone();
        let id_archive = sig.signal_id.clone();
        let id_unread = sig.signal_id.clone();

        let in_watchlist = self
            .reports
            .iter()
            .any(|r| r.ticker.eq_ignore_ascii_case(&sig.primary_ticker));

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                card_shell()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(badge(
                                        kind_label.into(),
                                        kind_fg,
                                        kind_bg,
                                        kind_br,
                                        &self.fonts,
                                    ))
                                    .child(badge(st_label.into(), st_fg, st_bg, st_br, &self.fonts))
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_base()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child(sig.primary_ticker.clone()),
                                    )
                                    .children(sig.tickers.iter().map(|t| {
                                        let tk = t.clone();
                                        div()
                                            .id(gpui::SharedString::from(format!(
                                                "inbox-chip-{}-{tk}",
                                                sig.signal_id
                                            )))
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded_xs()
                                            .bg(rgb(BG_WELL))
                                            .border_1()
                                            .border_color(rgb(BORDER_SUBTLE))
                                            .hover(|s| s.bg(rgb(BG_HOVER)))
                                            .cursor_pointer()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(10.))
                                            .text_color(rgb(INK_SECONDARY))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_inbox_signal_target(&tk, cx);
                                            }))
                                            .child(t.clone())
                                    })),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!(
                                        "Score {:.2} · {}",
                                        sig.score,
                                        fmt_time_cn(Some(sig.updated_at_ms))
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_base()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(sig.title.clone()),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(13.))
                            .line_height(px(21.))
                            .text_color(rgb(INK_PRIMARY))
                            .child(sig.summary.clone()),
                    )
                    .child(
                        div()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(Self::inbox_action_button(
                                "inbox-act-open",
                                if in_watchlist {
                                    "打开研报 Dossier"
                                } else {
                                    "在全市场雷达查看"
                                },
                                "↵",
                                true,
                                (CTA_INK, CTA_BG, CTA_BG),
                                &self.fonts,
                                cx.listener(move |this, _, _, cx| {
                                    this.open_inbox_signal_target(&ticker_open, cx);
                                }),
                            ))
                            .child(Self::inbox_action_button(
                                "inbox-act-watch",
                                "入观察池 WATCH",
                                "w",
                                false,
                                (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
                                &self.fonts,
                                cx.listener(move |this, _, _, cx| {
                                    this.triage_inbox_signal(
                                        &id_watch,
                                        SignalStatus::Watch,
                                        true,
                                        false,
                                        cx,
                                    );
                                }),
                            ))
                            .child(Self::inbox_action_button(
                                "inbox-act-alan",
                                "派 Alan 深研",
                                "d",
                                false,
                                (VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
                                &self.fonts,
                                cx.listener(move |this, _, _, cx| {
                                    this.triage_inbox_signal(
                                        &id_alan,
                                        SignalStatus::Researching,
                                        false,
                                        true,
                                        cx,
                                    );
                                }),
                            ))
                            .child(Self::inbox_action_button(
                                "inbox-act-toohard",
                                "Too Hard",
                                "x",
                                false,
                                (SLATE_FG, SLATE_BG, SLATE_BORDER),
                                &self.fonts,
                                cx.listener(move |this, _, _, cx| {
                                    this.triage_inbox_signal(
                                        &id_toohard,
                                        SignalStatus::TooHard,
                                        false,
                                        false,
                                        cx,
                                    );
                                }),
                            ))
                            .child(Self::inbox_action_button(
                                "inbox-act-archive",
                                "归档",
                                "e",
                                false,
                                (INK_SECONDARY, BG_WELL, BORDER_SUBTLE),
                                &self.fonts,
                                cx.listener(move |this, _, _, cx| {
                                    this.triage_inbox_signal(
                                        &id_archive,
                                        SignalStatus::Archived,
                                        false,
                                        false,
                                        cx,
                                    );
                                }),
                            ))
                            .when(sig.status != SignalStatus::Unread, |d| {
                                d.child(Self::inbox_action_button(
                                    "inbox-act-unread",
                                    "标为未读",
                                    "u",
                                    false,
                                    (SKY_FG, SKY_BG, SKY_BORDER),
                                    &self.fonts,
                                    cx.listener(move |this, _, _, cx| {
                                        this.triage_inbox_signal(
                                            &id_unread,
                                            SignalStatus::Unread,
                                            false,
                                            false,
                                            cx,
                                        );
                                    }),
                                ))
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .child(section_header(
                        "一手原文出处证据",
                        format!("{} 条检索片段 · 0 Flagship Token", sig.evidence.len()),
                        &self.fonts,
                    ))
                    .when(sig.evidence.is_empty(), |d| {
                        d.child(
                            card_shell().p_4().child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child("该信号暂无附带原文引用片段。"),
                            ),
                        )
                    })
                    .children(sig.evidence.iter().enumerate().map(|(e_idx, ev)| {
                        self.render_evidence_card(ev, &sig.signal_id, e_idx, cx)
                    })),
            )
    }

    fn inbox_action_button(
        id: &'static str,
        label: &'static str,
        key_hint: &'static str,
        primary: bool,
        colors: (u32, u32, u32),
        fonts: &FontRoles,
        on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let (fg, bg, border) = colors;
        let mut el = div()
            .id(id)
            .px_2p5()
            .py_1p5()
            .rounded_sm()
            .cursor_pointer()
            .border_1()
            .flex()
            .items_center()
            .gap_1p5()
            .on_click(on_click);
        if primary {
            el = el
                .bg(rgb(bg))
                .border_color(rgb(border))
                .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                .active(|s| s.bg(rgb(CTA_BG_ACTIVE)));
        } else {
            el = el
                .bg(rgba(bg))
                .border_color(rgba(border))
                .hover(|s| s.bg(rgb(BG_HOVER)))
                .active(|s| s.bg(rgb(BG_ACTIVE)));
        }
        el.child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(fg))
                .child(label),
        )
        .child(kbd(key_hint, fonts))
    }

    pub(super) fn render_evidence_card(
        &self,
        ev: &SignalEvidence,
        prefix: &str,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let copy_url = ev.source_url.clone();
        let copy_label = format!("{} {}", ev.ticker, ev.doc_type);
        let date_str = ev.filing_date.as_deref().unwrap_or("—");

        card_shell()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_1p5()
                            .child(badge(
                                ev.doc_type.clone(),
                                SKY_FG,
                                SKY_BG,
                                SKY_BORDER,
                                &self.fonts,
                            ))
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_xs()
                                    .bg(rgb(BG_WELL))
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(ev.section.clone()),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("{} · {}", ev.ticker, date_str)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(EMERALD_FG))
                                    .child(format!("相似度 {:.0}%", ev.similarity * 100.0)),
                            )
                            .when(!ev.source_url.trim().is_empty(), |d| {
                                d.child(
                                    div()
                                        .id(gpui::SharedString::from(format!(
                                            "ev-copy-{prefix}-{idx}"
                                        )))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_xs()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgb(BORDER_PILL))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .cursor_pointer()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(INK_SECONDARY))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy_cli_command(
                                                copy_url.clone(),
                                                &copy_label,
                                                cx,
                                            );
                                            cx.notify();
                                        }))
                                        .child("复制 URL"),
                                )
                            }),
                    ),
            )
            .when(!ev.title.trim().is_empty(), |d| {
                d.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO))
                        .child(ev.title.clone()),
                )
            })
            .child(
                div()
                    .px_3p5()
                    .py_2p5()
                    .rounded_md()
                    .bg(rgb(BG_SHELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(19.))
                    .text_color(rgb(INK_PRIMARY))
                    .child(ev.quote.clone()),
            )
            .when(!ev.source_url.trim().is_empty(), |d| {
                d.child(
                    div()
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .truncate()
                        .child(ev.source_url.clone()),
                )
            })
    }

    fn render_inbox_empty_lake_state(&self, cx: &mut Context<Self>) -> gpui::Div {
        let cli = "./apps/m3-desktop/m3 events catchup --bootstrap";
        card_shell()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(section_header(
                "L1 事件流与简报收件箱尚未初始化",
                "MILESTONE B · ZERO-FLAGSHIP-LLM".into(),
                &self.fonts,
            ))
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(13.))
                    .line_height(px(22.))
                    .text_color(rgb(INK_SECONDARY))
                    .child("先同步全美股 L0 湖仓，或点击「追平事件流」拉取 SEC EDGAR submissions / 8-K / 10-K 与公开新闻片段，自动生成「新浮现」「隐空间传导」「击球与红灯」待审讯简报。"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("inbox-empty-catchup")
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(CTA_BG))
                            .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(CTA_INK))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_l1_catchup(true, cx);
                            }))
                            .child("立即追平事件流"),
                    )
                    .child(
                        div()
                            .id("inbox-empty-copy-cli")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_PILL))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.mono_term.clone())
                            .text_xs()
                            .text_color(rgb(INK_PRIMARY))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_cli_command(cli.into(), "events catchup", cx);
                                cx.notify();
                            }))
                            .child(cli),
                    ),
            )
    }

    fn render_inbox_empty_filter_state(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO))
                    .child(format!("「{}」下暂无信号", self.inbox_state.filter.label())),
            )
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(18.))
                    .text_color(rgb(INK_MUTED))
                    .child("可点击右上角「追平事件流」拉取最新 SEC 公告与候选新闻，或切换至「全部」查看历史已分拣简报。"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("inbox-empty-view-all")
                            .px_3()
                            .py_1p5()
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_SUBTLE))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_PRIMARY))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_inbox_filter(InboxFilter::All);
                                cx.notify();
                            }))
                            .child("查看全部信号"),
                    ),
            )
    }

    fn render_inbox_no_selection(&self) -> gpui::Div {
        card_shell().p_6().child(
            div()
                .font_family(self.fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child("在左侧列表选择一条信号，或按 r 追平 L1 事件流。"),
        )
    }
}
