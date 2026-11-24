use super::{
    FontRoles, MungerWorkbench, NavView,
    components::*,
    format::*,
    industry::industry_status_colors,
    theme::*,
    universe::{UniverseSearchMode, format_sic_line},
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use m3_core::{
    events::{EventCatchupOptions, catchup_events_with_progress, fetch_company_10k_profile},
    i18n::UiLang,
    industry::{self, IndustryStatus},
    lake::{LakeDb, UniverseDrawerDetails, UniverseFilter},
    lake_l1::{SignalEvidence, SignalInboxRow, SignalKind, SignalStatus},
    lake_l2::BudgetStatusReport,
    model::{Bucket, WarningSeverity},
    store::{self, WorkspacePaths},
};
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
    pub fn label_for(self, lang: m3_core::i18n::UiLang) -> &'static str {
        match self {
            Self::Unread => lang.t("未读", "Unread"),
            Self::All => lang.t("全部", "All"),
            Self::Discovery => lang.t("量化候选", "Quant Screen"),
            Self::Ripple => lang.t("隐空间传导", "Ripple"),
            Self::StrikeAndRedFlag => lang.t("击球与红灯", "Strike & Red"),
            Self::Archived => lang.t("已归档", "Archived"),
        }
    }

    pub fn label(self) -> &'static str {
        self.label_for(m3_core::i18n::UiLang::Zh)
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

/// Returns `(label, fg, bg, border)` styling for a signal kind in the given language.
pub fn signal_kind_badge_for(
    kind: SignalKind,
    lang: m3_core::i18n::UiLang,
) -> (&'static str, u32, u32, u32) {
    match kind {
        SignalKind::Discovery => (
            lang.t("量化候选", "Quant Screen"),
            SKY_FG,
            SKY_BG,
            SKY_BORDER,
        ),
        SignalKind::Ripple => (
            lang.t("隐空间传导", "Ripple"),
            VIOLET_FG,
            VIOLET_BG,
            VIOLET_BORDER,
        ),
        SignalKind::Strike => (
            lang.t("击球区", "Strike"),
            EMERALD_FG,
            EMERALD_BG,
            EMERALD_BORDER,
        ),
        SignalKind::RedFlag => (
            lang.t("红灯预警", "Red Flag"),
            CORAL_FG,
            CORAL_BG,
            CORAL_BORDER,
        ),
    }
}

/// Returns `(label, fg, bg, border)` styling for a signal kind.
#[allow(dead_code)]
pub fn signal_kind_badge(kind: SignalKind) -> (&'static str, u32, u32, u32) {
    signal_kind_badge_for(kind, m3_core::i18n::UiLang::Zh)
}

/// Returns `(label, fg, bg, border)` styling for a signal triage status in the given language.
pub fn signal_status_badge_for(
    status: SignalStatus,
    lang: m3_core::i18n::UiLang,
) -> (&'static str, u32, u32, u32) {
    match status {
        SignalStatus::Unread => (lang.t("未读", "Unread"), SKY_FG, SKY_BG, SKY_BORDER),
        SignalStatus::Researching => (
            lang.t("Alan 研判", "Alan"),
            VIOLET_FG,
            VIOLET_BG,
            VIOLET_BORDER,
        ),
        SignalStatus::Watch => (
            lang.t("观察池", "Watch"),
            EMERALD_FG,
            EMERALD_BG,
            EMERALD_BORDER,
        ),
        SignalStatus::TooHard => ("TOO HARD", SLATE_FG, SLATE_BG, SLATE_BORDER),
        SignalStatus::Archived => (
            lang.t("已归档", "Archived"),
            INK_MUTED,
            BG_WELL,
            BORDER_SUBTLE,
        ),
    }
}

/// Returns `(label, fg, bg, border)` styling for a signal triage status.
#[allow(dead_code)]
pub fn signal_status_badge(status: SignalStatus) -> (&'static str, u32, u32, u32) {
    signal_status_badge_for(status, m3_core::i18n::UiLang::Zh)
}

/// Formats the monthly AI budget watermark for the Inbox KPI ribbon & sidebar dock.
pub fn budget_watermark_summary_for(
    report: Option<&BudgetStatusReport>,
    lang: m3_core::i18n::UiLang,
) -> (String, u32) {
    let prefix = lang.t("本月 AI 预算", "AI Budget");
    let Some(r) = report else {
        return (format!("{prefix} $0.00 / $10.00"), EMERALD_FG);
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
    (format!("{prefix} ${used:.2} / ${limit:.2}"), color)
}

/// Formats the monthly AI budget watermark for the Inbox KPI ribbon (`本月 AI 预算 $X.XX / $10.00`).
#[allow(dead_code)]
pub fn budget_watermark_summary(report: Option<&BudgetStatusReport>) -> (String, u32) {
    budget_watermark_summary_for(report, m3_core::i18n::UiLang::Zh)
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
                    self.lang
                        .t(
                            "其他后台同步任务进行中，请等待完成后再追平事件流",
                            "Another background sync is active; wait until it finishes",
                        )
                        .into(),
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
        let l = self.lang;
        self.inbox_catching_up = true;
        self.inbox_last_auto_catchup_at = Some(Instant::now());
        self.inbox_catchup_progress = Some(if silent_startup {
            l.t("启动后台追平 L1 事件流…", "Starting L1 catch-up…")
                .into()
        } else {
            l.t(
                "正在追平 SEC 公告、新闻与语义索引…",
                "Catching up SEC filings, news & semantic index…",
            )
            .into()
        });
        if !silent_startup {
            self.banner = Some((
                l.t(
                    "正在后台追平 L1 事件流（SEC 公告 + 候选新闻 + 本地语义索引，0 Flagship Token）…",
                    "Catching up L1 stream in background (SEC filings + news + local index, 0 Flagship Tokens)…",
                )
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
                        match self.lang {
                            m3_core::i18n::UiLang::Zh => format!(
                                "L1 事件流追平完成 · 新增公告 {} / 新闻 {} / 索引 {} 片段 / 信号 {}（未读 {} · {} Flagship Token）",
                                report.sec_filings_ingested,
                                report.news_events_ingested,
                                report.chunks_indexed,
                                report.signals_created_or_updated,
                                report.summary.unread_signals,
                                report.flagship_tokens_used,
                            ),
                            m3_core::i18n::UiLang::En => format!(
                                "L1 catch-up complete · +{} filings / +{} news / {} chunks / {} signals ({} unread · {} Flagship Tokens)",
                                report.sec_filings_ingested,
                                report.news_events_ingested,
                                report.chunks_indexed,
                                report.signals_created_or_updated,
                                report.summary.unread_signals,
                                report.flagship_tokens_used,
                            ),
                        },
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
                    self.banner = Some((
                        format!(
                            "{}: {err}",
                            self.lang.t("L1 事件流追平失败", "L1 catch-up failed")
                        ),
                        true,
                    ));
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

    /// Starts a zero-LLM background SEC 10-K Item 1/1A/7 fetch & local index for one company.
    pub(super) fn start_fetch_company_10k_profile(
        &mut self,
        ticker: &str,
        cik: &str,
        cx: &mut Context<Self>,
    ) {
        let l = self.lang;
        if self.universe_syncing || self.inbox_catching_up || self.syncing {
            self.banner = Some((
                l.t(
                    "后台同步或事件追平进行中，请稍候再拉取 10-K 原文",
                    "Background sync active — try fetching 10-K in a moment",
                )
                .into(),
                false,
            ));
            cx.notify();
            return;
        }
        if self.inbox_profile_fetching_ticker.is_some() {
            return;
        }
        let upper = ticker.trim().to_ascii_uppercase();
        let cik_owned = cik.trim().to_owned();
        if upper.is_empty() || cik_owned.is_empty() {
            return;
        }
        self.inbox_profile_fetching_ticker = Some(upper.clone());
        let (tx, rx) = mpsc::channel();
        self.inbox_profile_fetch_rx = Some(rx);
        let root = self.paths.repo_root.clone();
        let ticker_thread = upper.clone();
        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            let res = fetch_company_10k_profile(&paths, &ticker_thread, &cik_owned)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(res);
        });
        self.banner = Some((
            match l {
                UiLang::Zh => format!("正在从 SEC EDGAR 拉取 {upper} 最新 10-K 业务简介…"),
                UiLang::En => format!("Fetching {upper} latest 10-K business profile from SEC…"),
            },
            false,
        ));
        cx.notify();
    }

    /// Non-blocking poll for single-company 10-K business profile fetch completion.
    pub(super) fn poll_inbox_profile_fetch(&mut self) -> bool {
        let Some(rx) = self.inbox_profile_fetch_rx.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(rep)) => {
                self.inbox_profile_fetching_ticker = None;
                self.inbox_profile_fetch_rx = None;
                self.reload_inbox_from_lake();
                self.reload_inbox_selection_details();
                self.banner = Some((
                    match self.lang {
                        UiLang::Zh => format!(
                            "已拉取 {} 10-K 业务简介（{} 个文本片段，0 LLM Token）",
                            rep.ticker, rep.chunks_indexed
                        ),
                        UiLang::En => format!(
                            "Fetched {} 10-K business profile ({} chunks, 0 LLM tokens)",
                            rep.ticker, rep.chunks_indexed
                        ),
                    },
                    false,
                ));
                true
            }
            Ok(Err(err)) => {
                self.inbox_profile_fetching_ticker = None;
                self.inbox_profile_fetch_rx = None;
                self.banner = Some((
                    match self.lang {
                        UiLang::Zh => format!("拉取 10-K 业务简介失败：{err}"),
                        UiLang::En => format!("Failed to fetch 10-K profile: {err}"),
                    },
                    true,
                ));
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.inbox_profile_fetching_ticker = None;
                self.inbox_profile_fetch_rx = None;
                true
            }
        }
    }

    /// Reloads L0/L1 financial snapshot and SEC business profile excerpts for the selected signal.
    pub(super) fn reload_inbox_selection_details(&mut self) {
        let Some(ticker) = self
            .active_inbox_signal()
            .map(|s| s.primary_ticker.trim().to_ascii_uppercase())
            .filter(|t| !t.is_empty())
        else {
            self.inbox_selected_details = None;
            self.inbox_selected_business_chunks.clear();
            return;
        };
        if self.universe_syncing || self.inbox_catching_up {
            if !self
                .inbox_selected_details
                .as_ref()
                .is_some_and(|d| d.row.ticker.eq_ignore_ascii_case(&ticker))
            {
                self.inbox_selected_details = None;
                self.inbox_selected_business_chunks.clear();
            }
            return;
        }
        match LakeDb::open_read_only_if_exists(&self.paths) {
            Ok(Some(db)) => match db.load_drawer_details_by_ticker(&ticker, 5) {
                Ok(Some(details)) => {
                    let chunks = db
                        .load_company_business_chunks(&details.row.cik, 6)
                        .unwrap_or_default();
                    self.inbox_selected_details = Some(details);
                    self.inbox_selected_business_chunks = chunks;
                }
                Ok(None) | Err(_) => {
                    self.inbox_selected_details = None;
                    self.inbox_selected_business_chunks.clear();
                }
            },
            Ok(None) | Err(_) => {
                self.inbox_selected_details = None;
                self.inbox_selected_business_chunks.clear();
            }
        }
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
                        self.inbox_error = Some(format!(
                            "{}: {err:#}",
                            self.lang
                                .t("读取 L1 简报收件箱失败", "Failed to read L1 Briefing Inbox")
                        ));
                    }
                }
            }
            Err(err) => {
                self.inbox_error = Some(format!(
                    "{}: {err:#}",
                    self.lang
                        .t("打开 m3.duckdb 失败", "Failed to open m3.duckdb")
                ));
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
        self.reload_inbox_selection_details();
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
            self.reload_inbox_selection_details();
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
            self.banner = Some((
                self.lang
                    .t(
                        "后台事件追平或湖仓同步进行中，请稍候再分拣",
                        "Background sync or catch-up running — try triage in a moment",
                    )
                    .into(),
                false,
            ));
            cx.notify();
            return;
        }

        let Some(sig) = self
            .inbox_signals
            .iter()
            .find(|s| s.signal_id == signal_id)
            .cloned()
        else {
            self.banner = Some((
                format!(
                    "{} {signal_id}",
                    self.lang.t("未找到信号", "Signal not found:")
                ),
                true,
            ));
            cx.notify();
            return;
        };

        let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
        let in_watchlist_already = self
            .reports
            .iter()
            .any(|r| r.ticker.eq_ignore_ascii_case(&ticker));

        let Ok(db) = LakeDb::open(&self.paths) else {
            self.banner = Some((
                self.lang
                    .t(
                        "打开 m3.duckdb 失败，无法更新信号状态",
                        "Failed to open m3.duckdb to update signal status",
                    )
                    .into(),
                true,
            ));
            cx.notify();
            return;
        };

        match db.triage_signal(signal_id, target_status, store::now_ms()) {
            Ok(true) => {}
            Ok(false) => {
                self.banner = Some((
                    match self.lang {
                        m3_core::i18n::UiLang::Zh => format!("信号 {signal_id} 不存在或未变更"),
                        m3_core::i18n::UiLang::En => {
                            format!("Signal {signal_id} missing or unchanged")
                        }
                    },
                    true,
                ));
                cx.notify();
                return;
            }
            Err(err) => {
                self.banner = Some((
                    format!(
                        "{}: {err:#}",
                        self.lang.t("分拣信号失败", "Failed to triage signal")
                    ),
                    true,
                ));
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
                    match self.lang {
                        m3_core::i18n::UiLang::Zh => {
                            format!("已更新信号状态，但加入观察池 ({ticker}) 失败：{err:#}")
                        }
                        m3_core::i18n::UiLang::En => {
                            format!("Updated signal, but failed to add {ticker} to Watch: {err:#}")
                        }
                    },
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

        let status_label = signal_status_badge_for(target_status, self.lang).0;
        self.banner = Some((
            if promote_watch {
                match self.lang {
                    m3_core::i18n::UiLang::Zh => {
                        format!("已将 {ticker} 加入观察池 [WATCH] 并标记信号为「{status_label}」")
                    }
                    m3_core::i18n::UiLang::En => {
                        format!("Added {ticker} to [WATCH] and marked signal as {status_label}")
                    }
                }
            } else {
                match self.lang {
                    m3_core::i18n::UiLang::Zh => {
                        format!("已将 {ticker} 信号标记为「{status_label}」")
                    }
                    m3_core::i18n::UiLang::En => {
                        format!("Marked {ticker} signal as {status_label}")
                    }
                }
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
        let last_catchup = fmt_time_cn_for(l1_sum.last_catchup_ms, self.lang);

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
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(journal_stat(
                        self.lang.t("全美股池", "US Universe"),
                        match self.lang {
                            m3_core::i18n::UiLang::Zh => format!("{} 家", u_sum.total_companies),
                            m3_core::i18n::UiLang::En => format!("{}", u_sum.total_companies),
                        },
                        INK_HERO,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        self.lang.t("上次追平", "Last Catch-Up"),
                        last_catchup,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        self.lang.t("语义索引", "Semantic Index"),
                        match self.lang {
                            m3_core::i18n::UiLang::Zh => format!(
                                "{} 片段 / {} 公司",
                                l1_sum.doc_chunks_count, l1_sum.indexed_companies_count
                            ),
                            m3_core::i18n::UiLang::En => format!(
                                "{} chunks / {} cos",
                                l1_sum.doc_chunks_count, l1_sum.indexed_companies_count
                            ),
                        },
                        SKY_FG,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        self.lang.t("未读信号", "Unread"),
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
                div().flex_shrink_0().flex().items_center().gap_2().child(
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
                        .whitespace_nowrap()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_l1_catchup(false, cx);
                        }))
                        .child(if self.inbox_catching_up {
                            self.lang.t("追平中…", "Catching Up…")
                        } else {
                            self.lang.t("追平事件流", "Catch Up")
                        }),
                ),
            )
    }

    fn render_inbox_filter_bar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
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
                    let display_lbl = f.label_for(l);
                    let label = if count > 0 || matches!(f, InboxFilter::Unread | InboxFilter::All)
                    {
                        format!("{display_lbl} {count}")
                    } else {
                        display_lbl.to_owned()
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
                    .child(kbd(l.t("w 观察", "w Watch"), &self.fonts))
                    .child(kbd("x TooHard", &self.fonts))
                    .child(kbd(l.t("e 归档", "e Archive"), &self.fonts))
                    .child(kbd("d Alan", &self.fonts))
                    .child(kbd(l.t("r 追平", "r Catch-Up"), &self.fonts)),
            )
    }

    fn render_inbox_row(
        &self,
        sig: &SignalInboxRow,
        idx: usize,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (kind_label, kind_fg, kind_bg, kind_br) = signal_kind_badge_for(sig.kind, self.lang);
        let (st_label, st_fg, st_bg, st_br) = signal_status_badge_for(sig.status, self.lang);
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
                this.reload_inbox_selection_details();
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
                            .child(format!(
                                "{} {}",
                                sig.evidence.len(),
                                self.lang.t("条证据", "evidence")
                            )),
                    ),
            )
    }

    fn render_inbox_detail(&self, sig: &SignalInboxRow, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let (kind_label, kind_fg, kind_bg, kind_br) = signal_kind_badge_for(sig.kind, l);
        let (st_label, st_fg, st_bg, st_br) = signal_status_badge_for(sig.status, l);
        let ticker_open = sig.primary_ticker.clone();
        let id_watch = sig.signal_id.clone();
        let id_alan = sig.signal_id.clone();
        let id_toohard = sig.signal_id.clone();
        let id_archive = sig.signal_id.clone();
        let id_unread = sig.signal_id.clone();

        let report_opt = self
            .reports
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(&sig.primary_ticker));
        let in_watchlist = report_opt.is_some();
        let details_opt = self
            .inbox_selected_details
            .as_ref()
            .filter(|d| d.row.ticker.eq_ignore_ascii_case(&sig.primary_ticker));

        let matched_ind = industry::find_industry_for_company(
            &self.industries,
            &sig.primary_ticker,
            details_opt.and_then(|d| d.row.sic),
        );
        let (circle_badge_label, cb_fg, cb_bg, cb_br) = if let Some(rep) = report_opt {
            let (fg, bg, br) = bucket_colors(rep.bucket);
            (
                match l {
                    UiLang::Zh => format!("能力圈 · {}", rep.bucket.code()),
                    UiLang::En => format!("Circle · {}", rep.bucket.code()),
                },
                fg,
                bg,
                br,
            )
        } else if let Some(ind) = matched_ind {
            let (fg, bg, br) = industry_status_colors(ind.status);
            let lbl = match (ind.status, l) {
                (IndustryStatus::InsideCircle, UiLang::Zh) => format!("能力圈行业 · {}", ind.title),
                (IndustryStatus::InsideCircle, UiLang::En) => {
                    format!("Circle Ind. · {}", ind.title)
                }
                (IndustryStatus::Studying, UiLang::Zh) => format!("行业学习中 · {}", ind.title),
                (IndustryStatus::Studying, UiLang::En) => format!("Studying · {}", ind.title),
                (IndustryStatus::TooHard, UiLang::Zh) => format!("行业 Too Hard · {}", ind.title),
                (IndustryStatus::TooHard, UiLang::En) => format!("Ind. Too Hard · {}", ind.title),
            };
            (lbl, fg, bg, br)
        } else if let Some(det) = details_opt {
            let lbl = match (l, det.row.sic) {
                (UiLang::Zh, Some(sic)) => format!("未研究行业 · SIC {sic}"),
                (UiLang::En, Some(sic)) => format!("Unstudied · SIC {sic}"),
                (UiLang::Zh, None) => "未研究行业".into(),
                (UiLang::En, None) => "Unstudied Industry".into(),
            };
            (lbl, SLATE_FG, SLATE_BG, SLATE_BORDER)
        } else {
            (
                l.t("未入能力圈", "Outside Circle").into(),
                SLATE_FG,
                SLATE_BG,
                SLATE_BORDER,
            )
        };
        let ind_ticker = sig.primary_ticker.clone();
        let ind_name = details_opt
            .map(|d| d.row.name.clone())
            .or_else(|| report_opt.map(|r| r.name.clone()))
            .unwrap_or_else(|| sig.primary_ticker.clone());
        let ind_sic = details_opt.and_then(|d| d.row.sic);
        let ind_sic_desc = details_opt.and_then(|d| d.row.sic_description.clone());

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
                                    .child(badge(
                                        circle_badge_label,
                                        cb_fg,
                                        cb_bg,
                                        cb_br,
                                        &self.fonts,
                                    ))
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
                                        fmt_time_cn_for(Some(sig.updated_at_ms), self.lang)
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
                                    l.t("打开研报 Dossier", "Open Dossier")
                                } else {
                                    l.t("在全市场雷达查看", "Inspect in Radar")
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
                                "inbox-act-industry",
                                l.t("研究所在行业", "Research Industry"),
                                "⌘3",
                                false,
                                (SKY_FG, SKY_BG, SKY_BORDER),
                                &self.fonts,
                                cx.listener(move |this, _, _, cx| {
                                    this.open_or_create_industry_for_company(
                                        &ind_ticker,
                                        &ind_name,
                                        ind_sic,
                                        ind_sic_desc.as_deref(),
                                        cx,
                                    );
                                }),
                            ))
                            .child(Self::inbox_action_button(
                                "inbox-act-watch",
                                l.t("入观察池 WATCH", "Add to Watch"),
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
                                l.t("派 Alan 深研", "Dispatch Alan"),
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
                                l.t("归档", "Archive"),
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
                                    l.t("标为未读", "Mark Unread"),
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
            .child(self.render_inbox_business_profile_card(sig, details_opt, cx))
            .when_some(details_opt, |d, det| {
                d.child(self.render_inbox_financial_snapshot_card(det, in_watchlist, cx))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2p5()
                    .child(section_header(
                        l.t("一手原文出处证据", "PRIMARY SOURCE EVIDENCE"),
                        format!(
                            "{} {} · 0 Flagship Token",
                            sig.evidence.len(),
                            l.t("条检索片段", "retrieved chunks")
                        ),
                        &self.fonts,
                    ))
                    .when(sig.evidence.is_empty(), |d| {
                        d.child(
                            card_shell().p_4().child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(l.t(
                                        "该信号暂无附带原文引用片段。",
                                        "No source excerpts attached to this signal.",
                                    )),
                            ),
                        )
                    })
                    .children(sig.evidence.iter().enumerate().map(|(e_idx, ev)| {
                        self.render_evidence_card(ev, &sig.signal_id, e_idx, cx)
                    })),
            )
    }

    fn render_inbox_business_profile_card(
        &self,
        sig: &SignalInboxRow,
        details_opt: Option<&UniverseDrawerDetails>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let ticker_upper = sig.primary_ticker.trim().to_ascii_uppercase();
        let narrative_chunks: Vec<&SignalEvidence> = self
            .inbox_selected_business_chunks
            .iter()
            .filter(|c| {
                c.ticker.eq_ignore_ascii_case(&ticker_upper)
                    && !c.section.eq_ignore_ascii_case("company_profile")
            })
            .collect();

        let item1_chunks: Vec<&&SignalEvidence> = narrative_chunks
            .iter()
            .filter(|c| c.section == "item_1")
            .take(2)
            .collect();
        let risk_chunk = narrative_chunks
            .iter()
            .find(|c| c.section == "item_1a" || c.section == "item_7")
            .copied();

        let cik_opt = details_opt.map(|d| d.row.cik.clone());
        let sic_line = details_opt.map(|d| format_sic_line(&d.row, l));
        let is_fetching = self
            .inbox_profile_fetching_ticker
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(&ticker_upper));
        let id_alan = sig.signal_id.clone();

        let filing_tag = narrative_chunks
            .first()
            .and_then(|c| c.filing_date.as_deref())
            .map(|d| format!("SEC 10-K · {d}"))
            .unwrap_or_else(|| l.t("公司业务与行业档案", "BUSINESS & SECTOR").into());

        let mut card = card_shell()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_header(
                l.t(
                    "公司业务简介 · Business Profile",
                    "Business Profile · 10-K Item 1",
                ),
                filing_tag,
                &self.fonts,
            ));

        if let Some(sic) = sic_line {
            card = card.child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(11.))
                    .text_color(rgb(INK_SECONDARY))
                    .child(sic),
            );
        }

        if !item1_chunks.is_empty() || risk_chunk.is_some() {
            for ch in item1_chunks {
                card = card.child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(BG_VIEWPORT))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(10.))
                                .text_color(rgb(SKY_FG))
                                .child(ch.title.clone()),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .line_height(px(20.))
                                .text_color(rgb(INK_PRIMARY))
                                .child(ch.quote.clone()),
                        ),
                );
            }
            if let Some(rc) = risk_chunk {
                card = card.child(
                    div()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(BG_SHELL))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(10.))
                                .text_color(rgb(GOLD_FG))
                                .child(rc.title.clone()),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .line_height(px(19.))
                                .text_color(rgb(INK_SECONDARY))
                                .child(rc.quote.clone()),
                        ),
                );
            }
            return card;
        }

        card.child(
            div()
                .font_family(self.fonts.ui.clone())
                .text_xs()
                .line_height(px(19.))
                .text_color(rgb(INK_MUTED))
                .child(l.t(
                    "本地仅缓存了代码与行业元数据，尚未下载最新 SEC 10-K 业务原文。可一键零 LLM 拉取 10-K Item 1 业务说明与核心风险，或直接派 Alan 快速梳理。",
                    "Only registration metadata is indexed locally; latest 10-K Item 1 business text has not been fetched yet. Pull 10-K sections below (0 LLM tokens) or ask Alan.",
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_2()
                .when_some(cik_opt, |d, cik_val| {
                    let tk_fetch = ticker_upper.clone();
                    d.child(
                        div()
                            .id("inbox-fetch-10k-btn")
                            .px_3()
                            .py_1p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgba(SKY_BG))
                            .border_1()
                            .border_color(rgba(SKY_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(SKY_FG))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_fetch_company_10k_profile(&tk_fetch, &cik_val, cx);
                            }))
                            .child(if is_fetching {
                                l.t("正在拉取 10-K 业务原文…", "Fetching 10-K Profile…")
                            } else {
                                l.t(
                                    "拉取 10-K 业务简介 (0 LLM)",
                                    "Fetch 10-K Profile (0 LLM)",
                                )
                            }),
                    )
                })
                .child(
                    div()
                        .id("inbox-profile-ask-alan")
                        .px_3()
                        .py_1p5()
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgba(VIOLET_BG))
                        .border_1()
                        .border_color(rgba(VIOLET_BORDER))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(VIOLET_FG))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.triage_inbox_signal(
                                &id_alan,
                                SignalStatus::Researching,
                                false,
                                true,
                                cx,
                            );
                        }))
                        .child(l.t("派 Alan 快速解读", "Ask Alan")),
                ),
        )
    }

    fn render_inbox_financial_snapshot_card(
        &self,
        details: &UniverseDrawerDetails,
        in_watchlist: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let row = &details.row;
        let roic_txt = row
            .roic_5y_avg_pct
            .map(|v| format!("{:.1}%", v.round_dp(1)))
            .unwrap_or_else(|| "—".into());
        let gm_txt = row
            .gross_margin_5y_avg_pct
            .map(|v| format!("{:.1}%", v.round_dp(1)))
            .unwrap_or_else(|| "—".into());
        let (fcf_txt, fcf_pol) = fmt_pct_clean(row.strict_fcf_yield_pct, false);
        let (sbc_txt, _) = fmt_pct_clean(row.sbc_to_ocf_pct, false);
        let (share_txt, share_pol) = fmt_pct_clean(row.share_change_3y_pct, true);
        let nd_txt = row
            .net_debt_to_oe
            .map(|v| format!("{v:.2}x"))
            .unwrap_or_else(|| "—".into());

        let kpi_tile = |label: &str, val: String, val_col: u32| {
            div()
                .flex_1()
                .min_w_0()
                .p_2p5()
                .rounded_md()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .child(label.to_owned()),
                )
                .child(
                    div()
                        .font_family(self.fonts.mono_num.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(val_col))
                        .truncate()
                        .child(val),
                )
        };

        let mut card = card_shell()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(section_header(
                l.t(
                    "确定性 5Y 财务快照 & 同业对照",
                    "Deterministic 5Y Financials & Peers",
                ),
                "DUCKDB L0 · 0 LLM TOKEN".into(),
                &self.fonts,
            ));

        if !in_watchlist {
            card = card.child(
                div()
                    .px_3()
                    .py_1p5()
                    .rounded_sm()
                    .bg(rgb(BG_SHELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(11.))
                    .text_color(rgb(INK_MUTED))
                    .child(l.t(
                        "未入能力圈 · 仅展示历史确定性财务与同业分位；认可行业与商业模式后再入观察池并设定个人击球倍数。",
                        "Outside Circle · showing historical deterministic metrics and peers only; add to Watch and set personal multiples after understanding the business.",
                    )),
            );
        }

        card = card
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(kpi_tile(
                        l.t("最新股价", "Price"),
                        fmt_price(row.latest_price, "USD"),
                        INK_HERO,
                    ))
                    .child(kpi_tile("5Y ROIC", roic_txt, EMERALD_FG))
                    .child(kpi_tile(
                        l.t("严苛 FCF%", "Strict FCF%"),
                        fcf_txt,
                        if fcf_pol < 0 { CORAL_FG } else { INK_PRIMARY },
                    ))
                    .child(kpi_tile(
                        l.t("5Y 毛利率", "5Y Gross Margin"),
                        gm_txt,
                        INK_PRIMARY,
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(kpi_tile("SBC / OCF", sbc_txt, INK_SECONDARY))
                    .child(kpi_tile(
                        l.t("3Y 股本变化", "3Y Shrs Δ"),
                        share_txt,
                        share_delta_color(row.share_change_3y_pct, share_pol),
                    ))
                    .child(kpi_tile(
                        l.t("净负债 / OE", "Net Debt / OE"),
                        nd_txt,
                        INK_SECONDARY,
                    ))
                    .child(kpi_tile(
                        l.t("质量综合分", "Quality Score"),
                        format!("{:.1}", row.quality_score),
                        SKY_FG,
                    )),
            );

        if in_watchlist && (row.strike_price.is_some() || row.distance_to_strike_pct.is_some()) {
            let (dist_txt, _) = fmt_pct_clean(row.distance_to_strike_pct, true);
            let dist_col = if row
                .distance_to_strike_pct
                .is_some_and(|d| d <= Decimal::ZERO)
            {
                EMERALD_FG
            } else {
                GOLD_FG
            };
            card = card.child(
                div()
                    .flex()
                    .gap_2()
                    .child(kpi_tile(
                        l.t("个人击球价", "Strike Price"),
                        fmt_price(row.strike_price, "USD"),
                        INK_HERO,
                    ))
                    .child(kpi_tile(
                        l.t("距击球价", "Dist. to Strike"),
                        dist_txt,
                        dist_col,
                    )),
            );
        }

        if !details.annuals.is_empty() {
            let recent: Vec<_> = details.annuals.iter().rev().take(5).collect();
            let recent_len = recent.len();
            let mut tbl = div()
                .rounded_md()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .px_3()
                .py_1p5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .py_1()
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .child(div().w(px(82.)).flex_shrink_0().child(l.t("财年", "FY")))
                        .child(div().flex_1().text_right().child(l.t("营收", "Revenue")))
                        .child(div().w(px(64.)).text_right().child(l.t("毛利%", "GM%")))
                        .child(div().w(px(64.)).text_right().child("ROIC"))
                        .child(
                            div()
                                .flex_1()
                                .text_right()
                                .child(l.t("严苛OE", "Strict OE")),
                        ),
                );
            for (idx, a) in recent.into_iter().enumerate() {
                let gm = a
                    .gross_margin_pct
                    .map(|v| format!("{:.1}%", v.round_dp(1)))
                    .unwrap_or_else(|| "—".into());
                let roic = a
                    .roic_pct
                    .map(|v| format!("{:.1}%", v.round_dp(1)))
                    .unwrap_or_else(|| "—".into());
                let oe_col = match a.strict_owner_earnings {
                    Some(v) if v > Decimal::ZERO => EMERALD_FG,
                    Some(_) => CORAL_FG,
                    None => INK_MUTED,
                };
                tbl = tbl.child(
                    div()
                        .flex()
                        .items_center()
                        .py_1()
                        .when(idx + 1 < recent_len, |d| {
                            d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                        })
                        .font_family(self.fonts.mono_num.clone())
                        .text_size(px(11.))
                        .child(
                            div()
                                .w(px(82.))
                                .flex_shrink_0()
                                .text_color(rgb(INK_SECONDARY))
                                .child(a.period_end.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_right()
                                .text_color(rgb(INK_PRIMARY))
                                .child(fmt_money_compact(a.revenue, "USD")),
                        )
                        .child(
                            div()
                                .w(px(64.))
                                .text_right()
                                .text_color(rgb(INK_PRIMARY))
                                .child(gm),
                        )
                        .child(
                            div()
                                .w(px(64.))
                                .text_right()
                                .text_color(rgb(EMERALD_FG))
                                .child(roic),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_right()
                                .text_color(rgb(oe_col))
                                .child(fmt_money_compact_opt(a.strict_owner_earnings, "USD")),
                        ),
                );
            }
            card = card.child(tbl);
        }

        if let Some(peers) = details.peers.as_ref().filter(|p| p.peer_count > 1) {
            let roic_p = peers
                .roic_percentile
                .map(|p| format!("P{p:.0}"))
                .unwrap_or_else(|| "—".into());
            let gm_p = peers
                .gross_margin_percentile
                .map(|p| format!("P{p:.0}"))
                .unwrap_or_else(|| "—".into());
            let fcf_p = peers
                .strict_fcf_yield_percentile
                .map(|p| format!("P{p:.0}"))
                .unwrap_or_else(|| "—".into());

            card = card.child(
                div()
                    .pt_1()
                    .flex()
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(11.))
                            .text_color(rgb(SKY_FG))
                            .child(match l {
                                UiLang::Zh => format!(
                                    "{} · {} 家同业分位: ROIC {roic_p} · 毛利 {gm_p} · FCF {fcf_p}",
                                    peers.sic_group, peers.peer_count
                                ),
                                UiLang::En => format!(
                                    "{} · {} peers: ROIC {roic_p} · GM {gm_p} · FCF {fcf_p}",
                                    peers.sic_group, peers.peer_count
                                ),
                            }),
                    )
                    .when(!peers.top_peers.is_empty(), |d| {
                        d.child(div().flex().items_center().flex_wrap().gap_1p5().children(
                            peers.top_peers.iter().take(4).enumerate().map(|(i, p)| {
                                let jump = p.ticker.clone();
                                div()
                                    .id(("inbox-peer-chip", i))
                                    .px_2()
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
                                        this.open_inbox_signal_target(&jump, cx);
                                    }))
                                    .child(p.ticker.clone())
                            }),
                        ))
                    }),
            );
        }

        let warnings = row.parsed_warnings();
        if !warnings.is_empty() {
            card = card.child(div().flex().flex_wrap().gap_1p5().children(
                warnings.into_iter().map(|w| {
                    let (fg, bg, br) = match w.severity {
                        WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                        WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                        WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
                    };
                    badge(w.title, fg, bg, br, &self.fonts)
                }),
            ));
        }

        card
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
                                    .child(format!(
                                        "{} {:.0}%",
                                        self.lang.t("相似度", "Sim"),
                                        ev.similarity * 100.0
                                    )),
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
                                        .child(self.lang.t("复制 URL", "Copy URL")),
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
        let l = self.lang;
        let cli = "./apps/m3-desktop/m3 events catchup --bootstrap";
        card_shell()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(section_header(
                l.t(
                    "L1 事件流与简报收件箱尚未初始化",
                    "L1 Event Stream & Briefing Inbox Not Initialized",
                ),
                "MILESTONE B · ZERO-FLAGSHIP-LLM".into(),
                &self.fonts,
            ))
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(13.))
                    .line_height(px(22.))
                    .text_color(rgb(INK_SECONDARY))
                    .child(l.t(
                        "先同步全美股 L0 湖仓，或点击「追平事件流」拉取 SEC EDGAR submissions / 8-K / 10-K 与公开新闻片段，自动生成「新浮现」「隐空间传导」「击球与红灯」待审讯简报。",
                        "Sync the US L0 lakehouse or click Catch Up Now to ingest SEC EDGAR submissions / 8-K / 10-K and public news snippets into Discovery, Ripple, Strike & Red-Flag briefings.",
                    )),
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
                            .child(l.t("立即追平事件流", "Catch Up Now")),
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
        let l = self.lang;
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
                    .child(match l {
                        m3_core::i18n::UiLang::Zh => {
                            format!("「{}」下暂无信号", self.inbox_state.filter.label_for(l))
                        }
                        m3_core::i18n::UiLang::En => {
                            format!("No signals under \"{}\"", self.inbox_state.filter.label_for(l))
                        }
                    }),
            )
            .child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(18.))
                    .text_color(rgb(INK_MUTED))
                    .child(l.t(
                        "可点击右上角「追平事件流」拉取最新 SEC 公告与候选新闻，或切换至「全部」查看历史已分拣简报。",
                        "Click Catch Up above to fetch new SEC filings and candidate news, or switch to All to inspect triaged briefings.",
                    )),
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
                            .child(l.t("查看全部信号", "View All Signals")),
                    ),
            )
    }

    fn render_inbox_no_selection(&self) -> gpui::Div {
        card_shell().p_6().child(
            div()
                .font_family(self.fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(self.lang.t(
                    "在左侧列表选择一条信号，或按 r 追平 L1 事件流。",
                    "Select a signal on the left, or press r to catch up L1 events.",
                )),
        )
    }
}
