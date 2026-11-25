//! Complete reactive Signal store for Project M3 (`apps/m3-nsh`).
//!
//! `AppStore` is a `Copy` handle containing NSH `Signal<T>` and `Stored<T>` handles,
//! providing complete MungerWorkbench state and action parity
//! while executing background DuckDB, SEC EDGAR, Updater, Deep Research, and EmbeddedAlan
//! workflows over event-driven Tokio -> NSH channels with zero idle polling.

use crate::format::{
    McpClientTarget, apply_deep_research_event_compat, collect_circle_tags, filter_inbox_signals,
    filter_reports_for_basket, format_universe_sync_progress, parse_research_memo,
    resolve_active_inbox_signal, should_skip_auto_catchup, signal_status_badge_for,
    step_inbox_selection,
};
use crate::stores::async_bridge::{
    can_open_duckdb_read_only, can_start_duckdb_writer, run_blocking_or_err, run_blocking_result,
    run_streamed_blocking, spawn_notify_listener,
};
use crate::stores::types::{
    AgentMessageRole, AgentUiMessage, AlanReplyTarget, BasketsViewMode, ChartTimeframe,
    CompanyViewTab, DeepResearchPerspectiveUi, InboxFilter, InboxViewState, PaletteItem,
    ParsedResearchMemo, SettingsTab, SortCol, StatusQuickFilter, StoredAppMessage, UniversePreset,
    UniverseSearchMode, UniverseViewState, WorkspaceTab, apply_ui_row_guards,
    default_deep_research_perspectives,
};
use m3_core::alan_connection::{
    self, AlanConnectionBrowserLoginResult, AlanConnectionProfileSummary, AlanConnectionProvider,
    AlanConnectionStatus, AlanConnectionTestResult, BrowserLoginAlanConnectionRequest,
    SaveAlanConnectionRequest,
};
use m3_core::alan_interrogator::{
    InterrogatorEvent, M3InterrogatorHandle, TuringTapeEntry, TuringTapeKind,
    compose_feynman_quiz_turn, compose_industry_tutor_turn, parse_acme_evidence_tokens,
};
use m3_core::app_config;
use m3_core::budget::{self, BudgetConfig, ThemeCluster, ThemesFile};
use m3_core::events::{
    self, CompanyProfileFetchReport, EventCatchupOptions, EventCatchupReport,
    catchup_events_with_progress, fetch_company_10k_profile,
};
use m3_core::i18n::UiLang;
use m3_core::industry::{self, IndustryProfile, IndustryStatus};
use m3_core::industry_research::{self, IndustryResearchEvent};
use m3_core::lake::{
    LakeAnnualRow, LakeDb, PeerComparison, UniverseDrawerDetails, UniverseFilter, UniverseRow,
    UniverseSortBy, UniverseSummary,
};
use m3_core::lake_l1::{
    L1Summary, SignalEvidence, SignalInboxRow, SignalStatus, SimilarCompanyHit, SimilarSearchFilter,
};
use m3_core::lake_l2::{self, BudgetStatusReport, ResearchRunRow};
use m3_core::model::{
    Bucket, DecisionJournal, Dossier, MungerReport, PriceHistorySnapshot, PriceSnapshot,
    SyncStatusFile, ValuationAssumptions,
};
use m3_core::research::{self, DeepResearchEvent, DeepResearchRequest, DeepResearchResult};
use m3_core::storage::{self, StorageCategory, StorageReport};
use m3_core::store::{self, WorkspacePaths};
use m3_core::universe::{self, UniverseSyncOptions, UniverseSyncProgress, UniverseSyncReport};
use m3_core::update::{self, PassiveUpdateNotice, ReleaseChannel, UpdateOptions, UpdateReport};
use nsh_core::prelude::*;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

const AUTO_CATCHUP_INTERVAL: Duration = Duration::from_secs(20 * 60);

enum CatchupMsg {
    Progress(String),
    Finished(Box<Result<EventCatchupReport, String>>),
}

enum UniverseSyncMsg {
    Progress(UniverseSyncProgress),
    Finished(Box<Result<UniverseSyncReport, String>>),
}

/// Complete reactive store for the M3 NSH application.
#[derive(Clone, Copy)]
pub struct AppStore {
    // ── Global / Shell / Navigation ──────────────────────────────────────────
    pub lang: Signal<UiLang>,
    pub paths: Signal<WorkspacePaths>,
    pub active_tab: Signal<WorkspaceTab>,
    pub company_open: Signal<bool>,
    pub company_view_tab: Signal<CompanyViewTab>,
    pub palette_open: Signal<bool>,
    pub palette_query: Signal<String>,
    pub palette_cursor: Signal<usize>,
    pub palette_expanded_groups: Signal<HashSet<String>>,
    pub cmd_held: Signal<bool>,
    pub sidebar_collapsed: Signal<bool>,
    pub message_box_open: Signal<bool>,
    pub settings_open: Signal<bool>,
    pub messages: Signal<Vec<StoredAppMessage>>,
    pub notice: Signal<Option<(String, bool)>>,
    pub sync_running: Signal<bool>,
    pub sync_stage: Signal<Option<String>>,
    pub sync_progress: Signal<Option<f32>>,
    pub sync_status: Signal<Option<SyncStatusFile>>,
    pub benchmark: Signal<Option<PriceSnapshot>>,
    pub benchmark_symbol: Signal<String>,
    pub startup_hydrated: Signal<bool>,
    pub onboarding_dismissed: Signal<bool>,
    pub onboarding_applying: Signal<bool>,

    // ── Circle Overview / Baskets / Journal / Budget ─────────────────────────
    pub reports: Signal<Vec<MungerReport>>,
    pub journal: Signal<DecisionJournal>,
    pub budget_config: Signal<BudgetConfig>,
    pub budget_report: Signal<Option<BudgetStatusReport>>,
    pub baskets_view_mode: Signal<BasketsViewMode>,
    pub basket_filter: Signal<Option<Bucket>>,
    pub circle_filter: Signal<Option<String>>,
    pub status_filter: Signal<StatusQuickFilter>,
    pub collapsed_baskets: Signal<HashSet<Bucket>>,
    pub sort_col: Signal<SortCol>,
    pub sort_asc: Signal<bool>,
    pub baskets_cursor_index: Signal<usize>,

    // ── Company Dossier / Chart / Charlie Sandbox ────────────────────────────
    pub selected_ticker: Signal<String>,
    pub selected_dossier: Signal<Option<Dossier>>,
    pub selected_history_wk: Signal<Option<PriceHistorySnapshot>>,
    pub selected_history_1d: Signal<Option<PriceHistorySnapshot>>,
    pub chart_timeframe: Signal<ChartTimeframe>,
    pub use_ttm_basis: Signal<bool>,
    pub sandbox_multiple: Signal<Decimal>,
    pub sandbox_target_return: Signal<Decimal>,
    pub sandbox_expected_growth: Signal<Decimal>,
    pub selected_memo_raw: Signal<Option<String>>,
    pub selected_memo_parsed: Signal<Option<ParsedResearchMemo>>,
    pub selected_research_runs: Signal<Vec<ResearchRunRow>>,
    pub selected_peers: Signal<Option<PeerComparison>>,
    pub selected_similar_hits: Signal<Vec<SimilarCompanyHit>>,

    // ── Deep Research & EmbeddedAlan ─────────────────────────────────────────
    pub deep_research_running: Signal<bool>,
    pub deep_research_ticker: Signal<Option<String>>,
    pub deep_research_run_id: Signal<Option<String>>,
    pub deep_research_perspectives: Signal<Vec<DeepResearchPerspectiveUi>>,
    pub deep_research_status_note: Signal<Option<String>>,
    pub deep_research_budget_blocked: Signal<Option<String>>,
    pub deep_research_last_result: Signal<Option<DeepResearchResult>>,
    pub agent_session_ticker: Signal<Option<String>>,
    pub agent_input_draft: Signal<String>,
    pub agent_streaming: Signal<bool>,
    pub agent_status_hint: Signal<Option<String>>,
    pub agent_messages: Signal<Vec<AgentUiMessage>>,
    pub agent_tape: Signal<Vec<TuringTapeEntry>>,
    pub agent_expanded_evidence: Signal<Option<String>>,
    pub agent_expanded_tape: Signal<Option<usize>>,
    pub alan_reply_target: Signal<AlanReplyTarget>,

    // ── Briefing Inbox (L1 Event-Driven) ─────────────────────────────────────
    pub inbox_state: Signal<InboxViewState>,
    pub inbox_summary: Signal<Option<L1Summary>>,
    pub inbox_signals: Signal<Vec<SignalInboxRow>>,
    pub inbox_selected_details: Signal<Option<UniverseDrawerDetails>>,
    pub inbox_selected_business_chunks: Signal<Vec<SignalEvidence>>,
    pub inbox_profile_fetching_ticker: Signal<Option<String>>,
    pub inbox_error: Signal<Option<String>>,
    pub inbox_catching_up: Signal<bool>,
    pub inbox_catchup_progress: Signal<Option<String>>,

    // ── Universe Radar (L0/L1 DuckDB) ────────────────────────────────────────
    pub universe_state: Signal<UniverseViewState>,
    pub universe_lake_present: Signal<bool>,
    pub universe_summary: Signal<Option<UniverseSummary>>,
    pub universe_rows: Signal<Vec<UniverseRow>>,
    pub universe_similar_hits: Signal<Vec<SimilarCompanyHit>>,
    pub universe_similar_query_ms: Signal<Option<u64>>,
    pub universe_search_running: Signal<bool>,
    pub universe_strike_zone_ciks: Signal<HashSet<String>>,
    pub universe_selected_cik: Signal<Option<String>>,
    pub universe_selected_row: Signal<Option<UniverseRow>>,
    pub universe_selected_annuals: Signal<Vec<LakeAnnualRow>>,
    pub universe_selected_peers: Signal<Option<PeerComparison>>,
    pub universe_error: Signal<Option<String>>,
    pub universe_syncing: Signal<bool>,
    pub universe_sync_progress: Signal<Option<String>>,
    pub universe_sync_progress_detail: Signal<Option<UniverseSyncProgress>>,
    pub pending_progressive_l0_after_sync: Signal<bool>,

    // ── Industry Research Studio & Themes ────────────────────────────────────
    pub themes_file: Signal<ThemesFile>,
    pub themes_syncing: Signal<bool>,
    pub themes_error: Signal<Option<String>>,
    pub industries: Signal<Vec<IndustryProfile>>,
    pub selected_industry_slug: Signal<Option<String>>,
    pub industry_status_filter: Signal<Option<IndustryStatus>>,
    pub industry_peer_rows: Signal<Vec<UniverseRow>>,
    pub industry_primer_md: Signal<Option<String>>,
    pub industry_new_topic_input: Signal<String>,
    pub industry_add_ticker_input: Signal<String>,
    pub industry_alan_input: Signal<String>,
    pub industry_human_note_draft: Signal<String>,
    pub industry_anchor_low_draft: Signal<String>,
    pub industry_anchor_high_draft: Signal<String>,
    pub industry_research_running: Signal<bool>,
    pub industry_research_slug: Signal<Option<String>>,
    pub industry_research_progress: Signal<Option<String>>,
    pub industry_live_perspectives: Signal<BTreeMap<String, String>>,
    pub industry_batch_10k_running: Signal<bool>,
    pub industry_alan_messages: Signal<Vec<AgentUiMessage>>,
    pub industry_alan_streaming: Signal<bool>,
    pub industry_alan_status_hint: Signal<Option<String>>,

    // ── Settings & Updater ───────────────────────────────────────────────────
    pub settings_tab: Signal<SettingsTab>,
    pub update_channel: Signal<ReleaseChannel>,
    pub update_notice: Signal<Option<PassiveUpdateNotice>>,
    pub update_checking: Signal<bool>,
    pub update_check_seq: Signal<u64>,
    pub update_check_channel: Signal<Option<ReleaseChannel>>,
    pub update_applying: Signal<bool>,
    pub update_progress: Signal<Option<String>>,
    pub update_progress_detail: Signal<Option<update::UpdateProgress>>,
    pub update_applied_version: Signal<Option<String>>,
    pub settings_monthly_budget_draft: Signal<String>,
    pub settings_monthly_tokens_draft: Signal<String>,
    pub alan_conn_status: Signal<Option<AlanConnectionStatus>>,
    pub alan_conn_provider: Signal<AlanConnectionProvider>,
    pub alan_conn_profile_id_draft: Signal<String>,
    pub alan_conn_model_draft: Signal<String>,
    pub alan_conn_base_url_draft: Signal<String>,
    pub alan_conn_api_key_draft: Signal<String>,
    pub alan_conn_project_id_draft: Signal<String>,
    pub alan_conn_location_draft: Signal<String>,
    pub alan_conn_saving: Signal<bool>,
    pub alan_conn_logging_in: Signal<bool>,
    pub alan_conn_testing: Signal<bool>,
    pub alan_conn_feedback: Signal<Option<(String, bool)>>,
    pub settings_mcp_target: Signal<McpClientTarget>,
    pub storage_report: Signal<Option<StorageReport>>,
    pub storage_scanning: Signal<bool>,
    pub storage_cleaning: Signal<Option<StorageCategory>>,

    // ── Internal Non-Reactive State ──────────────────────────────────────────
    alan_handle: Stored<Option<M3InterrogatorHandle>>,
    agent_seq: Stored<usize>,
    universe_search_seq: Stored<u64>,
    inbox_last_auto_catchup_at: Stored<Option<Instant>>,
    message_seq: Stored<u64>,
}

struct StartupHydrationSnapshot {
    repo_root: std::path::PathBuf,
    benchmark_symbol: Option<String>,
    reports_res: Result<Vec<MungerReport>, String>,
    budget_config: BudgetConfig,
    journal: Option<DecisionJournal>,
    benchmark: Option<PriceSnapshot>,
    sync_status: Option<SyncStatusFile>,
    selected_ticker: String,
    selected_dossier: Option<Dossier>,
    selected_history_wk: Option<PriceHistorySnapshot>,
    selected_history_1d: Option<PriceHistorySnapshot>,
    selected_memo_raw: Option<String>,
    selected_memo_parsed: Option<ParsedResearchMemo>,
    themes_res: Result<ThemesFile, String>,
    industries: Vec<IndustryProfile>,
    selected_industry_slug: Option<String>,
    industry_primer_md: Option<String>,
    industry_peer_rows: Vec<UniverseRow>,
    lake_present: bool,
    inbox_summary: Option<L1Summary>,
    inbox_signals: Vec<SignalInboxRow>,
    inbox_error: Option<String>,
    inbox_selected_id: Option<String>,
    inbox_cursor_index: usize,
    inbox_selected_details: Option<UniverseDrawerDetails>,
    inbox_selected_business_chunks: Vec<SignalEvidence>,
    budget_report: Option<BudgetStatusReport>,
    universe_summary: Option<UniverseSummary>,
    universe_strike_zone_ciks: HashSet<String>,
    universe_rows: Vec<UniverseRow>,
    universe_selected_details: Option<UniverseDrawerDetails>,
    universe_error: Option<String>,
    selected_research_runs: Vec<ResearchRunRow>,
    selected_peers: Option<PeerComparison>,
    selected_similar_hits: Vec<SimilarCompanyHit>,
    alan_conn_status: Option<AlanConnectionStatus>,
}

fn load_startup_snapshot(
    paths: &WorkspacePaths,
    lang: UiLang,
    universe_filter: &UniverseFilter,
    require_5y_history: bool,
    inbox_filter: InboxFilter,
) -> StartupHydrationSnapshot {
    let benchmark_symbol = store::load_watchlist(paths)
        .ok()
        .map(|wl| wl.benchmark_symbol);
    let bench_sym = benchmark_symbol.as_deref().unwrap_or("^GSPC").to_owned();

    let reports_res = store::load_or_compute_reports(paths).map_err(|err| {
        format!(
            "{}: {err:#}",
            lang.t("加载计算报告失败", "Failed to load reports")
        )
    });
    let selected_ticker = reports_res
        .as_ref()
        .ok()
        .and_then(|list| list.first())
        .map(|r| r.ticker.clone())
        .unwrap_or_default();

    let budget_config = budget::load_budget_config(paths).unwrap_or_default();
    let journal = store::load_journal(paths).ok();
    let benchmark = store::load_cached_price(paths, &bench_sym);
    let sync_status = store::read_json(&paths.sync_status_path()).ok();

    let (selected_dossier, selected_history_wk, selected_history_1d, selected_memo_raw) =
        if selected_ticker.trim().is_empty() {
            (None, None, None, None)
        } else {
            let trimmed = selected_ticker.trim();
            let upper = trimmed.to_ascii_uppercase();
            (
                store::load_dossier(paths, trimmed).ok().flatten(),
                store::load_cached_price_history(paths, trimmed, "1wk"),
                store::load_cached_price_history(paths, trimmed, "1d"),
                research::load_research_memo(paths, &upper),
            )
        };
    let selected_memo_parsed = selected_memo_raw.as_deref().map(parse_research_memo);

    let themes_res = budget::load_themes(paths).map_err(|err| {
        format!(
            "{}: {err:#}",
            lang.t(
                "读取 workbench/themes.json 失败",
                "Failed to read workbench/themes.json"
            )
        )
    });
    let industries = industry::list_industries(paths).unwrap_or_default();
    let selected_industry = industries.first().cloned();
    let selected_industry_slug = selected_industry.as_ref().map(|p| p.slug.clone());
    let industry_primer_md = selected_industry_slug
        .as_deref()
        .and_then(|slug| industry::load_industry_primer_md(paths, slug));

    let mut lake_present = false;
    let mut inbox_summary = None;
    let mut inbox_signals = Vec::new();
    let mut inbox_error = None;
    let mut inbox_selected_id = None;
    let mut inbox_cursor_index = 0usize;
    let mut inbox_selected_details = None;
    let mut inbox_selected_business_chunks = Vec::new();
    let mut budget_report = None;
    let mut universe_summary = None;
    let mut universe_strike_zone_ciks = HashSet::new();
    let mut universe_rows = Vec::new();
    let mut universe_selected_details = None;
    let mut universe_error = None;
    let mut selected_research_runs = Vec::new();
    let mut selected_peers = None;
    let mut selected_similar_hits = Vec::new();
    let mut industry_peer_rows = Vec::new();

    match LakeDb::open_read_only_if_exists(paths) {
        Ok(None) => {}
        Ok(Some(db)) => {
            lake_present = true;
            if let Ok(budget) = db.budget_status_report(paths) {
                budget_report = Some(budget);
            }
            match (db.l1_summary(), db.list_inbox_signals(None, None, 500)) {
                (Ok(summary), Ok(signals)) => {
                    let visible = filter_inbox_signals(&signals, inbox_filter);
                    if let Some((idx, sig)) = resolve_active_inbox_signal(&visible, None, 0) {
                        inbox_cursor_index = idx;
                        inbox_selected_id = Some(sig.signal_id.clone());
                        let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
                        if !ticker.is_empty()
                            && let Ok(Some(details)) = db.load_drawer_details_by_ticker(&ticker, 5)
                        {
                            inbox_selected_business_chunks = db
                                .load_company_business_chunks(&details.row.cik, 6)
                                .unwrap_or_default();
                            inbox_selected_details = Some(details);
                        }
                    }
                    inbox_summary = Some(summary);
                    inbox_signals = signals;
                }
                (Err(err), _) | (_, Err(err)) => {
                    inbox_error = Some(format!(
                        "{}: {err:#}",
                        lang.t("读取 L1 简报收件箱失败", "Failed to read L1 Briefing Inbox")
                    ));
                }
            }

            match db.load_universe_snapshot(universe_filter, None, 5) {
                Ok(snapshot) => {
                    universe_summary = Some(snapshot.summary);
                    universe_strike_zone_ciks = snapshot.strike_zone_ciks;
                    universe_rows = apply_ui_row_guards(snapshot.rows, require_5y_history);
                    universe_selected_details = snapshot.selected_details;
                }
                Err(err) => {
                    universe_error = Some(match lang {
                        UiLang::Zh => format!("读取全市场湖仓失败: {err:#}"),
                        UiLang::En => format!("Failed to load Universe lakehouse: {err:#}"),
                    });
                }
            }

            if !selected_ticker.trim().is_empty() {
                let upper = selected_ticker.trim().to_ascii_uppercase();
                selected_research_runs = db.list_research_runs(Some(&upper), 8).unwrap_or_default();
                selected_peers = db.query_peers(&upper, 6).ok();
                selected_similar_hits = db
                    .search_similar(
                        &upper,
                        true,
                        &SimilarSearchFilter {
                            exclude_financials: true,
                            limit: 5,
                            ..Default::default()
                        },
                    )
                    .unwrap_or_default();
            }

            if let Some(ind) = selected_industry.as_ref() {
                industry_peer_rows = db
                    .load_universe_rows_by_tickers(&ind.bound_tickers)
                    .unwrap_or_default();
            }
        }
        Err(err) => {
            lake_present = paths.lake_db_path().exists();
            let msg = format!(
                "{}: {err:#}",
                lang.t("打开 m3.duckdb 失败", "Failed to open m3.duckdb")
            );
            inbox_error = Some(msg.clone());
            universe_error = Some(msg);
        }
    }

    if budget_report.is_none()
        && !lake_present
        && let Ok(report) = lake_l2::load_budget_status_report(paths)
    {
        budget_report = Some(report);
    }

    let alan_conn_status = alan_connection::list_alan_connection_status().ok();

    StartupHydrationSnapshot {
        repo_root: paths.repo_root.clone(),
        benchmark_symbol,
        reports_res,
        budget_config,
        journal,
        benchmark,
        sync_status,
        selected_ticker,
        selected_dossier,
        selected_history_wk,
        selected_history_1d,
        selected_memo_raw,
        selected_memo_parsed,
        themes_res,
        industries,
        selected_industry_slug,
        industry_primer_md,
        industry_peer_rows,
        lake_present,
        inbox_summary,
        inbox_signals,
        inbox_error,
        inbox_selected_id,
        inbox_cursor_index,
        inbox_selected_details,
        inbox_selected_business_chunks,
        budget_report,
        universe_summary,
        universe_strike_zone_ciks,
        universe_rows,
        universe_selected_details,
        universe_error,
        selected_research_runs,
        selected_peers,
        selected_similar_hits,
        alan_conn_status,
    }
}

impl AppStore {
    /// Resolves the workspace root and initializes lightweight config synchronously so
    /// frame 1 renders immediately, then hydrates disk and DuckDB snapshots off-thread.
    pub fn start() -> Self {
        let root = match store::resolve_repo_root() {
            Ok(p) => p,
            Err(err) => {
                log::warn!("resolve_repo_root failed, falling back to CWD: {err:#}");
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
            }
        };
        let paths = WorkspacePaths::from_root(root);
        let store = Self::new_empty(paths);
        store.hydrate_startup_async();
        store
    }

    /// Creates a store bound to `paths` and loads synchronous disk/lake snapshots (used by tests).
    pub fn new(paths: WorkspacePaths) -> Self {
        let store = Self::new_empty(paths);
        store.reload_all();
        store
    }

    fn new_empty(paths: WorkspacePaths) -> Self {
        let (effective_ch, _) = update::effective_channel();
        let initial_lang = app_config::read_ui_language();
        let initial_budget = budget::load_budget_config(&paths).unwrap_or_default();
        let default_journal = DecisionJournal {
            schema_version: 1,
            account_note: "PAPER RESEARCH FIXTURE".into(),
            currency: "USD".into(),
            starting_cash: Decimal::from(100_000),
            cash_balance: Decimal::from(100_000),
            benchmark_symbol: "^GSPC".into(),
            benchmark_start_price: None,
            positions: Vec::new(),
            entries: Vec::new(),
        };

        Self {
            lang: signal(initial_lang),
            paths: signal_with(paths, SignalEquals::Never),
            active_tab: signal(WorkspaceTab::Inbox),
            company_open: signal(false),
            company_view_tab: signal(CompanyViewTab::FinancialsMemo),
            palette_open: signal(false),
            palette_query: signal(String::new()),
            palette_cursor: signal(0),
            palette_expanded_groups: signal(HashSet::new()),
            cmd_held: signal(false),
            sidebar_collapsed: signal(false),
            message_box_open: signal(false),
            settings_open: signal(false),
            messages: signal_with(Vec::new(), SignalEquals::Never),
            notice: signal(None),
            sync_running: signal(false),
            sync_stage: signal(None),
            sync_progress: signal(None),
            sync_status: signal_with(None, SignalEquals::Never),
            benchmark: signal_with(None, SignalEquals::Never),
            benchmark_symbol: signal("^GSPC".to_owned()),
            startup_hydrated: signal(false),
            onboarding_dismissed: signal(false),
            onboarding_applying: signal(false),

            reports: signal_with(Vec::new(), SignalEquals::Never),
            journal: signal_with(default_journal, SignalEquals::Never),
            budget_config: signal_with(initial_budget.clone(), SignalEquals::Never),
            budget_report: signal_with(None, SignalEquals::Never),
            baskets_view_mode: signal(BasketsViewMode::List),
            basket_filter: signal(None),
            circle_filter: signal(None),
            status_filter: signal(StatusQuickFilter::All),
            collapsed_baskets: signal(HashSet::from([Bucket::Out, Bucket::TooHard])),
            sort_col: signal(SortCol::Default),
            sort_asc: signal(false),
            baskets_cursor_index: signal(0),

            selected_ticker: signal(String::new()),
            selected_dossier: signal_with(None, SignalEquals::Never),
            selected_history_wk: signal_with(None, SignalEquals::Never),
            selected_history_1d: signal_with(None, SignalEquals::Never),
            chart_timeframe: signal(ChartTimeframe::FiveYearWeekly),
            use_ttm_basis: signal(true),
            sandbox_multiple: signal(Decimal::from(18)),
            sandbox_target_return: signal(Decimal::from(10)),
            sandbox_expected_growth: signal(Decimal::from(8)),
            selected_memo_raw: signal(None),
            selected_memo_parsed: signal(None),
            selected_research_runs: signal_with(Vec::new(), SignalEquals::Never),
            selected_peers: signal_with(None, SignalEquals::Never),
            selected_similar_hits: signal_with(Vec::new(), SignalEquals::Never),

            deep_research_running: signal(false),
            deep_research_ticker: signal(None),
            deep_research_run_id: signal(None),
            deep_research_perspectives: signal(default_deep_research_perspectives()),
            deep_research_status_note: signal(None),
            deep_research_budget_blocked: signal(None),
            deep_research_last_result: signal_with(None, SignalEquals::Never),
            agent_session_ticker: signal(None),
            agent_input_draft: signal(String::new()),
            agent_streaming: signal(false),
            agent_status_hint: signal(None),
            agent_messages: signal(Vec::new()),
            agent_tape: signal_with(Vec::new(), SignalEquals::Never),
            agent_expanded_evidence: signal(None),
            agent_expanded_tape: signal(None),
            alan_reply_target: signal(AlanReplyTarget::Company),

            inbox_state: signal(InboxViewState::default()),
            inbox_summary: signal_with(None, SignalEquals::Never),
            inbox_signals: signal_with(Vec::new(), SignalEquals::Never),
            inbox_selected_details: signal_with(None, SignalEquals::Never),
            inbox_selected_business_chunks: signal_with(Vec::new(), SignalEquals::Never),
            inbox_profile_fetching_ticker: signal(None),
            inbox_error: signal(None),
            inbox_catching_up: signal(false),
            inbox_catchup_progress: signal(None),

            universe_state: signal(UniverseViewState::default()),
            universe_lake_present: signal(false),
            universe_summary: signal_with(None, SignalEquals::Never),
            universe_rows: signal_with(Vec::new(), SignalEquals::Never),
            universe_similar_hits: signal_with(Vec::new(), SignalEquals::Never),
            universe_similar_query_ms: signal(None),
            universe_search_running: signal(false),
            universe_strike_zone_ciks: signal(HashSet::new()),
            universe_selected_cik: signal(None),
            universe_selected_row: signal_with(None, SignalEquals::Never),
            universe_selected_annuals: signal_with(Vec::new(), SignalEquals::Never),
            universe_selected_peers: signal_with(None, SignalEquals::Never),
            universe_error: signal(None),
            universe_syncing: signal(false),
            universe_sync_progress: signal(None),
            universe_sync_progress_detail: signal_with(None, SignalEquals::Never),
            pending_progressive_l0_after_sync: signal(false),

            themes_file: signal_with(ThemesFile::default(), SignalEquals::Never),
            themes_syncing: signal(false),
            themes_error: signal(None),
            industries: signal(Vec::new()),
            selected_industry_slug: signal(None),
            industry_status_filter: signal(None),
            industry_peer_rows: signal_with(Vec::new(), SignalEquals::Never),
            industry_primer_md: signal(None),
            industry_new_topic_input: signal(String::new()),
            industry_add_ticker_input: signal(String::new()),
            industry_alan_input: signal(String::new()),
            industry_human_note_draft: signal(String::new()),
            industry_anchor_low_draft: signal(String::new()),
            industry_anchor_high_draft: signal(String::new()),
            industry_research_running: signal(false),
            industry_research_slug: signal(None),
            industry_research_progress: signal(None),
            industry_live_perspectives: signal(BTreeMap::new()),
            industry_batch_10k_running: signal(false),
            industry_alan_messages: signal(Vec::new()),
            industry_alan_streaming: signal(false),
            industry_alan_status_hint: signal(None),

            settings_tab: signal(SettingsTab::General),
            update_channel: signal(effective_ch),
            update_notice: signal_with(None, SignalEquals::Never),
            update_checking: signal(false),
            update_check_seq: signal(0),
            update_check_channel: signal(None),
            update_applying: signal(false),
            update_progress: signal(None),
            update_progress_detail: signal_with(None, SignalEquals::Never),
            update_applied_version: signal(None),
            settings_monthly_budget_draft: signal(initial_budget.monthly_budget_usd.to_string()),
            settings_monthly_tokens_draft: signal(initial_budget.monthly_token_limit.to_string()),
            alan_conn_status: signal_with(None, SignalEquals::Never),
            alan_conn_provider: signal(AlanConnectionProvider::OpenRouter),
            alan_conn_profile_id_draft: signal(
                AlanConnectionProvider::OpenRouter
                    .default_profile_id()
                    .to_owned(),
            ),
            alan_conn_model_draft: signal(
                AlanConnectionProvider::OpenRouter
                    .default_model()
                    .to_owned(),
            ),
            alan_conn_base_url_draft: signal(String::new()),
            alan_conn_api_key_draft: signal(String::new()),
            alan_conn_project_id_draft: signal(String::new()),
            alan_conn_location_draft: signal(
                AlanConnectionProvider::OpenRouter
                    .default_location()
                    .to_owned(),
            ),
            alan_conn_saving: signal(false),
            alan_conn_logging_in: signal(false),
            alan_conn_testing: signal(false),
            alan_conn_feedback: signal(None),
            settings_mcp_target: signal(McpClientTarget::ClaudeCode),
            storage_report: signal_with(None, SignalEquals::Never),
            storage_scanning: signal(false),
            storage_cleaning: signal(None),

            alan_handle: stored(None),
            agent_seq: stored(1),
            universe_search_seq: stored(0),
            inbox_last_auto_catchup_at: stored(None),
            message_seq: stored(0),
        }
    }

    fn hydrate_startup_async(self) {
        let paths = self.paths.get_untracked();
        let lang = self.lang.get_untracked();
        let uni_st = self.universe_state.get_untracked();
        let universe_filter = uni_st.to_lake_filter();
        let require_5y = uni_st.require_5y_history;
        let inbox_filter = self.inbox_state.get_untracked().filter;

        run_blocking_result(
            move || load_startup_snapshot(&paths, lang, &universe_filter, require_5y, inbox_filter),
            move |snap| {
                if self.paths.get_untracked().repo_root != snap.repo_root {
                    return;
                }
                if let Some(sym) = snap.benchmark_symbol {
                    self.benchmark_symbol.set(sym);
                }
                match snap.reports_res {
                    Ok(list) => {
                        let cur = self.selected_ticker.get_untracked();
                        let next_sel = if list.is_empty() {
                            String::new()
                        } else if list.iter().any(|r| r.ticker.eq_ignore_ascii_case(&cur)) {
                            cur
                        } else {
                            snap.selected_ticker.clone()
                        };
                        if let Some(rep) = list.iter().find(|r| r.ticker == next_sel) {
                            self.sandbox_multiple.set(rep.assumptions.multiple_low);
                            self.sandbox_target_return
                                .set(rep.assumptions.target_return_pct);
                            self.sandbox_expected_growth
                                .set(rep.assumptions.expected_growth_pct);
                        }
                        self.reports.set(list);
                        self.selected_ticker.set(next_sel);
                    }
                    Err(err) => {
                        self.set_error_notice(err);
                    }
                }
                self.settings_monthly_budget_draft
                    .set(snap.budget_config.monthly_budget_usd.to_string());
                self.settings_monthly_tokens_draft
                    .set(snap.budget_config.monthly_token_limit.to_string());
                self.budget_config.set(snap.budget_config);
                if let Some(j) = snap.journal {
                    self.journal.set(j);
                }
                self.benchmark.set(snap.benchmark);
                self.sync_status.set(snap.sync_status);

                if !snap.selected_ticker.is_empty()
                    && self.selected_ticker.get_untracked() == snap.selected_ticker
                {
                    self.selected_dossier.set(snap.selected_dossier);
                    self.selected_history_wk.set(snap.selected_history_wk);
                    self.selected_history_1d.set(snap.selected_history_1d);
                    self.selected_memo_raw.set(snap.selected_memo_raw);
                    self.selected_memo_parsed.set(snap.selected_memo_parsed);
                }

                match snap.themes_res {
                    Ok(themes) => {
                        self.themes_file.set(themes);
                        self.themes_error.set(None);
                    }
                    Err(err) => {
                        self.themes_error.set(Some(err));
                    }
                }
                if let Some(first_ind) = snap.industries.first() {
                    self.industry_human_note_draft
                        .set(first_ind.human_notes.clone());
                    self.industry_anchor_low_draft.set(
                        first_ind
                            .multiple_low
                            .map(|d| d.round_dp(1).normalize().to_string())
                            .unwrap_or_default(),
                    );
                    self.industry_anchor_high_draft.set(
                        first_ind
                            .multiple_high
                            .map(|d| d.round_dp(1).normalize().to_string())
                            .unwrap_or_default(),
                    );
                }
                self.industries.set(snap.industries);
                self.selected_industry_slug.set(snap.selected_industry_slug);
                self.industry_primer_md.set(snap.industry_primer_md);

                if can_open_duckdb_read_only(
                    self.universe_syncing.get_untracked(),
                    self.inbox_catching_up.get_untracked(),
                ) {
                    self.universe_lake_present.set(snap.lake_present);
                    self.budget_report.set(snap.budget_report);
                    self.inbox_summary.set(snap.inbox_summary);
                    self.inbox_signals.set(snap.inbox_signals);
                    self.inbox_error.set(snap.inbox_error);
                    let cur_filter = self.inbox_state.get_untracked().filter;
                    if cur_filter == InboxFilter::Unread {
                        self.inbox_state.update(|st| {
                            st.cursor_index = snap.inbox_cursor_index;
                            st.selected_signal_id = snap.inbox_selected_id;
                        });
                        self.inbox_selected_details.set(snap.inbox_selected_details);
                        self.inbox_selected_business_chunks
                            .set(snap.inbox_selected_business_chunks);
                    } else {
                        self.sync_inbox_selection();
                    }

                    self.universe_summary.set(snap.universe_summary);
                    self.universe_strike_zone_ciks
                        .set(snap.universe_strike_zone_ciks);
                    self.universe_error.set(snap.universe_error);
                    if let Some(first_row) = snap.universe_rows.first().cloned() {
                        let first_cik = first_row.cik.clone();
                        self.universe_rows.set(snap.universe_rows);
                        self.universe_state.update(|s| s.cursor_index = 0);
                        self.universe_selected_cik.set(Some(first_cik.clone()));
                        if let Some(details) = snap.universe_selected_details
                            && details.row.cik == first_cik
                        {
                            self.apply_universe_drawer_details(details);
                        } else {
                            self.universe_selected_row.set(Some(first_row));
                        }
                    } else {
                        self.universe_rows.set(Vec::new());
                    }

                    if !snap.selected_ticker.is_empty()
                        && self.selected_ticker.get_untracked() == snap.selected_ticker
                    {
                        self.selected_research_runs.set(snap.selected_research_runs);
                        self.selected_peers.set(snap.selected_peers);
                        self.selected_similar_hits.set(snap.selected_similar_hits);
                    }
                    self.industry_peer_rows.set(snap.industry_peer_rows);
                }

                self.startup_hydrated.set(true);
                if let Some(status) = snap.alan_conn_status {
                    self.apply_alan_connection_status(status);
                }
                self.bootstrap_background();
            },
        );
    }

    fn bootstrap_background(self) {
        if !update::passive_check::should_skip_passive_check() {
            let (cached_notice, needs_refresh) =
                update::passive_check::evaluate_local_passive_notice();
            if cached_notice.is_some() {
                self.update_notice.set(cached_notice);
            }
            if needs_refresh {
                let effective_ch = self.update_channel.get_untracked();
                let req_seq = self.update_check_seq.get_untracked();
                run_blocking_result(
                    move || {
                        update::refresh_passive_cache_once(effective_ch)
                            .ok()
                            .flatten()
                    },
                    move |notice_opt| {
                        if self.update_check_seq.get_untracked() == req_seq
                            && !self.update_checking.get_untracked()
                            && !self.update_applying.get_untracked()
                            && self.update_applied_version.get_untracked().is_none()
                            && self.update_channel.get_untracked() == effective_ch
                        {
                            self.update_notice.set(notice_opt);
                        }
                    },
                );
            }
        }

        self.maybe_start_startup_catchup();
    }

    // ── Reloaders (Disk & Read-Only DuckDB) ──────────────────────────────────

    /// Reloads all disk files and read-only DuckDB snapshots.
    pub fn reload_all(self) {
        self.reload_from_disk();
        self.reload_themes_from_disk();
        self.reload_industries_from_disk();
        self.reload_universe_from_lake();
        self.reload_inbox_from_lake();
        self.reload_budget_from_lake();
        self.startup_hydrated.set(true);
    }

    pub fn reload_from_disk(self) {
        let paths = self.paths.get_untracked();
        if let Ok(wl) = store::load_watchlist(&paths) {
            self.benchmark_symbol.set(wl.benchmark_symbol);
        }
        let l = self.lang.get_untracked();
        match store::load_or_compute_reports(&paths) {
            Ok(list) => {
                let cur = self.selected_ticker.get_untracked();
                let next_sel = if list.is_empty() {
                    String::new()
                } else if list.iter().any(|r| r.ticker.eq_ignore_ascii_case(&cur)) {
                    cur
                } else {
                    list.first().map(|r| r.ticker.clone()).unwrap_or_default()
                };
                self.reports.set(list);
                self.selected_ticker.set(next_sel);
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("加载计算报告失败", "Failed to load reports")
                ));
            }
        }
        let cfg = budget::load_budget_config(&paths).unwrap_or_default();
        self.settings_monthly_budget_draft
            .set(cfg.monthly_budget_usd.to_string());
        self.settings_monthly_tokens_draft
            .set(cfg.monthly_token_limit.to_string());
        self.budget_config.set(cfg);
        if let Ok(j) = store::load_journal(&paths) {
            self.journal.set(j);
        }
        let bench_sym = self.benchmark_symbol.get_untracked();
        self.benchmark
            .set(store::load_cached_price(&paths, &bench_sym));
        self.sync_status
            .set(store::read_json(&paths.sync_status_path()).ok());
        let ticker = self.selected_ticker.get_untracked();
        self.load_selection(&ticker);
    }

    pub fn load_selection(self, ticker: &str) {
        let trimmed = ticker.trim();
        self.agent_expanded_evidence.set(None);
        if trimmed.is_empty() {
            self.selected_ticker.set(String::new());
            self.selected_dossier.set(None);
            self.selected_history_wk.set(None);
            self.selected_history_1d.set(None);
            self.selected_memo_raw.set(None);
            self.selected_memo_parsed.set(None);
            self.selected_research_runs.set(Vec::new());
            self.selected_peers.set(None);
            self.selected_similar_hits.set(Vec::new());
            return;
        }
        let paths = self.paths.get_untracked();
        self.selected_ticker.set(trimmed.to_owned());
        self.alan_handle.with(|opt| {
            if let Some(handle) = opt {
                handle.refresh_fs();
            }
        });
        self.selected_dossier
            .set(store::load_dossier(&paths, trimmed).ok().flatten());
        self.selected_history_wk
            .set(store::load_cached_price_history(&paths, trimmed, "1wk"));
        self.selected_history_1d
            .set(store::load_cached_price_history(&paths, trimmed, "1d"));
        self.reports.with_untracked(|reps| {
            if let Some(rep) = reps.iter().find(|r| r.ticker == trimmed) {
                self.sandbox_multiple.set(rep.assumptions.multiple_low);
                self.sandbox_target_return
                    .set(rep.assumptions.target_return_pct);
                self.sandbox_expected_growth
                    .set(rep.assumptions.expected_growth_pct);
            }
        });
        self.reload_research_desk_context(trimmed);
    }

    pub fn reload_budget_from_lake(self) {
        let paths = self.paths.get_untracked();
        let cfg = budget::load_budget_config(&paths).unwrap_or_default();
        self.budget_config.set(cfg);
        if !can_open_duckdb_read_only(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
        ) {
            return;
        }
        if let Ok(report) = lake_l2::load_budget_status_report(&paths) {
            self.budget_report.set(Some(report));
        }
    }

    pub fn reload_research_desk_context(self, ticker: &str) {
        let upper = ticker.trim().to_ascii_uppercase();
        let paths = self.paths.get_untracked();
        let raw = research::load_research_memo(&paths, &upper);
        let parsed = raw.as_deref().map(parse_research_memo);
        self.selected_memo_raw.set(raw);
        self.selected_memo_parsed.set(parsed);

        if can_open_duckdb_read_only(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
        ) {
            if let Ok(Some(db)) = LakeDb::open_read_only_if_exists(&paths) {
                self.selected_research_runs
                    .set(db.list_research_runs(Some(&upper), 8).unwrap_or_default());
                self.selected_peers.set(db.query_peers(&upper, 6).ok());
                self.selected_similar_hits.set(
                    db.search_similar(
                        &upper,
                        true,
                        &SimilarSearchFilter {
                            exclude_financials: true,
                            limit: 5,
                            ..Default::default()
                        },
                    )
                    .unwrap_or_default(),
                );
            } else {
                self.selected_research_runs.set(Vec::new());
                self.selected_peers.set(None);
                self.selected_similar_hits.set(Vec::new());
            }
        }
        self.reload_budget_from_lake();
    }

    // ── Navigation & Shell Actions ───────────────────────────────────────────

    pub fn open_tab(self, tab: WorkspaceTab) {
        if tab == WorkspaceTab::Settings {
            self.open_settings();
            return;
        }
        self.settings_open.set(false);
        self.company_open.set(false);
        self.active_tab.set(tab);
        match tab {
            WorkspaceTab::Inbox => {
                self.reload_inbox_from_lake();
                self.reload_budget_from_lake();
            }
            WorkspaceTab::Universe => {
                self.reload_universe_from_lake();
            }
            WorkspaceTab::Industry => {
                self.reload_industries_from_disk();
                self.reload_themes_from_disk();
            }
            WorkspaceTab::Baskets | WorkspaceTab::Journal | WorkspaceTab::Settings => {}
        }
    }

    fn refresh_settings_tab_data(self, tab: SettingsTab, force_storage_rescan: bool) {
        match tab {
            SettingsTab::General => {}
            SettingsTab::Ai => {
                if self.alan_conn_status.with_untracked(|s| s.is_none()) {
                    self.refresh_alan_connection_status();
                }
                if self.budget_report.with_untracked(|r| r.is_none()) {
                    self.reload_budget_from_lake();
                }
            }
            SettingsTab::Data => {
                if !self.storage_scanning.get_untracked()
                    && (force_storage_rescan || self.storage_report.with_untracked(|r| r.is_none()))
                {
                    self.refresh_storage_report();
                }
            }
        }
    }

    pub fn set_settings_tab(self, tab: SettingsTab) {
        self.settings_tab.set(tab);
        self.refresh_settings_tab_data(tab, false);
    }

    pub fn open_settings(self) {
        self.palette_open.set(false);
        self.message_box_open.set(false);
        self.settings_open.set(true);
        let tab = self.settings_tab.get_untracked();
        self.refresh_settings_tab_data(tab, true);
    }

    pub fn close_settings(self) {
        self.settings_open.set(false);
    }

    pub fn toggle_settings(self) {
        if self.settings_open.get_untracked() {
            self.close_settings();
        } else {
            self.open_settings();
        }
    }

    pub fn open_company(self, ticker: &str, tab: CompanyViewTab) {
        self.load_selection(ticker);
        self.company_view_tab.set(tab);
        self.company_open.set(true);
    }

    pub fn open_selected_company(self, tab: CompanyViewTab) {
        let ticker = self.selected_ticker.get_untracked();
        if ticker.trim().is_empty() {
            let l = self.lang.get_untracked();
            self.set_info_notice(
                l.t(
                    "当前未选中公司，请先从能力圈或收件箱选择标的",
                    "No company selected — pick one from Circle or Inbox first",
                )
                .to_owned(),
            );
            return;
        }
        self.company_view_tab.set(tab);
        self.company_open.set(true);
        self.reload_research_desk_context(&ticker);
    }

    pub fn open_company_contextual(self, ticker: &str) {
        let target_tab = if self.company_open.get_untracked()
            && self.company_view_tab.get_untracked() == CompanyViewTab::DeepResearchAlan
        {
            CompanyViewTab::DeepResearchAlan
        } else {
            CompanyViewTab::FinancialsMemo
        };
        self.open_company(ticker, target_tab);
    }

    pub fn close_company_overlay(self) {
        self.company_open.set(false);
    }

    pub fn toggle_palette(self) {
        let open = !self.palette_open.get_untracked();
        if open {
            self.message_box_open.set(false);
            self.settings_open.set(false);
        }
        self.palette_open.set(open);
        self.palette_query.set(String::new());
        self.palette_cursor.set(0);
        self.palette_expanded_groups.set(HashSet::new());
    }

    pub fn open_palette(self) {
        self.message_box_open.set(false);
        self.settings_open.set(false);
        self.palette_open.set(true);
        self.palette_query.set(String::new());
        self.palette_cursor.set(0);
        self.palette_expanded_groups.set(HashSet::new());
    }

    pub fn close_palette(self) {
        self.palette_open.set(false);
        self.palette_query.set(String::new());
        self.palette_cursor.set(0);
        self.palette_expanded_groups.set(HashSet::new());
    }

    pub fn toggle_palette_group(self, group: &str) {
        self.palette_expanded_groups.update(|groups| {
            if !groups.remove(group) {
                groups.insert(group.to_owned());
            }
        });
    }

    pub fn toggle_sidebar(self) {
        self.sidebar_collapsed
            .set(!self.sidebar_collapsed.get_untracked());
    }

    pub fn toggle_message_box(self) {
        let open = !self.message_box_open.get_untracked();
        if open {
            self.settings_open.set(false);
            self.palette_open.set(false);
        }
        self.message_box_open.set(open);
    }

    pub fn close_message_box(self) {
        self.message_box_open.set(false);
    }

    pub fn set_language(self, lang: UiLang) {
        if let Err(err) = app_config::write_ui_language(lang) {
            let cur = self.lang.get_untracked();
            self.set_error_notice(format!(
                "{}: {err:#}",
                cur.t("保存语言设置失败", "Failed to save language preference")
            ));
            return;
        }
        self.lang.set(lang);
        self.set_info_notice(
            lang.t(
                "已切换界面语言为中文",
                "Interface language switched to English",
            )
            .to_owned(),
        );
    }

    pub fn show_about_notice(self) {
        let l = self.lang.get_untracked();
        let ch = self.update_channel.get_untracked();
        let root = self.paths.get_untracked().repo_root;
        self.set_info_notice(format!(
            "Project M3 (Money · Making · Machine) v{} ({}) · Apache-2.0 · {}: {}",
            update::current_binary_version(),
            ch.as_str(),
            l.t("工作区", "Workspace"),
            root.display()
        ));
    }

    pub fn show_m3_engraving_notice(self) {
        let l = self.lang.get_untracked();
        self.set_info_notice(
            l.t(
                "M³ = Money · Making · Machine — 机器读完 6,000 家财报，你只挥击甜区好球。",
                "M³ = Money · Making · Machine — The machine reads 6,000 filings; you swing at fat pitches.",
            )
            .to_owned(),
        );
    }

    pub fn show_budget_status_notice(self) {
        self.reload_budget_from_lake();
        let l = self.lang.get_untracked();
        if let Some(b) = self.budget_report.get_untracked() {
            let used = b.usage.total_cost_usd.round_dp(4).normalize();
            let limit = b.config.monthly_budget_usd.round_dp(2).normalize();
            let rem = b.remaining_monthly_usd.round_dp(4).normalize();
            let tok_used = b
                .usage
                .total_input_tokens
                .saturating_add(b.usage.total_output_tokens);
            self.set_info_notice(match l {
                UiLang::Zh => format!(
                    "本月 AI 预算 ({})：已用 ${used} / ${limit}（剩余 ${rem} · Token {tok_used} / {} · 完成 {} / 拦截 {}）",
                    b.usage.month_utc,
                    b.config.monthly_token_limit,
                    b.usage.completed_runs,
                    b.usage.blocked_runs
                ),
                UiLang::En => format!(
                    "AI Budget ({}): used ${used} / ${limit} (rem ${rem} · tokens {tok_used} / {} · ok {} / blocked {})",
                    b.usage.month_utc,
                    b.config.monthly_token_limit,
                    b.usage.completed_runs,
                    b.usage.blocked_runs
                ),
            });
        } else {
            self.set_error_notice(
                l.t("无法读取 AI 预算状态", "Unable to read AI budget status")
                    .to_owned(),
            );
        }
    }

    fn push_stored_message(self, is_error: bool, message: &str) {
        let already_head = self.messages.with_untracked(|list| {
            list.first()
                .is_some_and(|head| head.is_error == is_error && head.message == message)
        });
        if already_head {
            return;
        }
        let next_id = self.message_seq.with(|seq| {
            *seq = seq.wrapping_add(1);
            *seq
        });
        let entry = StoredAppMessage {
            id: next_id,
            is_error,
            message: message.to_owned(),
        };
        self.messages.update(|list| {
            list.insert(0, entry);
            if list.len() > 20 {
                list.truncate(20);
            }
        });
    }

    pub fn set_info_notice(self, message: String) {
        self.push_stored_message(false, &message);
        self.notice.set(Some((message, false)));
    }

    pub fn set_error_notice(self, message: String) {
        self.push_stored_message(true, &message);
        self.notice.set(Some((message, true)));
    }

    pub fn dismiss_message(self, id: u64) {
        let mut head_before = None;
        let mut head_after = None;
        self.messages.update(|list| {
            head_before = list.first().cloned();
            list.retain(|m| m.id != id);
            head_after = list.first().cloned();
        });
        if head_after.is_none() {
            self.notice.set(None);
        } else if let Some(prev) = head_before
            && prev.id == id
        {
            self.notice.update(|cur| {
                if let Some((msg, err)) = cur.as_ref()
                    && *err == prev.is_error
                    && *msg == prev.message
                {
                    *cur = None;
                }
            });
        }
    }

    pub fn clear_messages(self) {
        self.messages.set(Vec::new());
        self.notice.set(None);
    }

    pub fn clear_notice(self) {
        let first_id = self
            .messages
            .with_untracked(|list| list.first().map(|m| m.id));
        if let Some(id) = first_id {
            self.dismiss_message(id);
        } else {
            self.notice.set(None);
        }
    }

    /// Scans local workspace & updater storage usage on a background thread.
    pub fn refresh_storage_report(self) {
        if self.storage_scanning.get_untracked() {
            return;
        }
        let paths = self.paths.get_untracked();
        let expected_root = paths.repo_root.clone();
        let updater_dir = app_config::updater_dir().ok();
        self.storage_scanning.set(true);

        run_blocking_result(
            move || storage::scan_workspace_storage(&paths, updater_dir.as_deref()),
            move |report| {
                if self.paths.get_untracked().repo_root != expected_root {
                    return;
                }
                self.storage_scanning.set(false);
                self.storage_report.set(Some(report));
            },
        );
    }

    /// Safely cleans a rebuildable local storage bucket and updates Settings telemetry.
    pub fn clean_storage_category(self, category: StorageCategory) {
        let l = self.lang.get_untracked();
        if category == StorageCategory::PersonalWorkbench {
            return;
        }
        if self.sync_running.get_untracked()
            || self.universe_syncing.get_untracked()
            || self.inbox_catching_up.get_untracked()
            || self.themes_syncing.get_untracked()
        {
            self.set_info_notice(
                l.t(
                    "同步进行中，请等待同步完成后再清理",
                    "Sync in progress — wait before cleaning",
                )
                .to_owned(),
            );
            return;
        }
        if self.storage_cleaning.get_untracked().is_some() {
            return;
        }

        let paths = self.paths.get_untracked();
        let expected_root = paths.repo_root.clone();
        let updater_dir = app_config::updater_dir().ok();
        self.storage_cleaning.set(Some(category));

        run_blocking_result(
            move || {
                let freed =
                    storage::clean_storage_category(&paths, updater_dir.as_deref(), category)
                        .map_err(|e| format!("{e:#}"))?;
                let report = storage::scan_workspace_storage(&paths, updater_dir.as_deref());
                Ok::<(u64, StorageReport), String>((freed, report))
            },
            move |res| {
                self.storage_cleaning.set(None);
                let l = self.lang.get_untracked();
                match res {
                    Ok((freed, report)) => {
                        if self.paths.get_untracked().repo_root == expected_root {
                            self.storage_report.set(Some(report));
                            if matches!(
                                category,
                                StorageCategory::DuckDbLakehouse | StorageCategory::QuotesAndCache
                            ) {
                                self.reload_all();
                                self.reload_universe_from_lake();
                                self.reload_inbox_from_lake();
                            }
                        }
                        self.set_info_notice(format!(
                            "{} {}",
                            l.t("已释放", "Freed"),
                            storage::format_bytes(freed)
                        ));
                    }
                    Err(err) => {
                        self.set_error_notice(format!(
                            "{}: {err}",
                            l.t("清理失败", "Cleanup failed")
                        ));
                    }
                }
            },
        );
    }

    // ── Workspace Switch & Settings ──────────────────────────────────────────

    pub fn switch_workspace_to(self, dir: &Path) {
        let l = self.lang.get_untracked();
        match app_config::ensure_initialized_workspace(dir)
            .and_then(|canon| app_config::write_workspace_root(&canon))
        {
            Ok(canonical) => {
                self.alan_handle.with(|opt| {
                    if let Some(h) = opt {
                        h.shutdown();
                    }
                });
                self.alan_handle.replace(None);
                self.paths.set(WorkspacePaths::from_root(canonical.clone()));
                self.selected_ticker.set(String::new());
                self.company_open.set(false);
                self.onboarding_dismissed.set(false);
                self.reload_all();
                self.refresh_storage_report();
                self.set_info_notice(format!(
                    "{}: {}",
                    l.t("已切换工作区至", "Switched workspace to"),
                    canonical.display()
                ));
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("切换工作区失败", "Failed to switch workspace")
                ));
            }
        }
    }

    pub fn reset_workspace_to_default(self) {
        let l = self.lang.get_untracked();
        let res = app_config::clear_workspace_root()
            .and_then(|()| app_config::default_workspace_dir())
            .and_then(|dir| app_config::ensure_initialized_workspace(&dir));
        match res {
            Ok(canonical) => {
                self.alan_handle.with(|opt| {
                    if let Some(h) = opt {
                        h.shutdown();
                    }
                });
                self.alan_handle.replace(None);
                self.paths.set(WorkspacePaths::from_root(canonical.clone()));
                self.selected_ticker.set(String::new());
                self.company_open.set(false);
                self.onboarding_dismissed.set(false);
                self.reload_all();
                self.refresh_storage_report();
                self.set_info_notice(format!(
                    "{}: {}",
                    l.t(
                        "已恢复默认空白工作区",
                        "Reset to default standalone workspace"
                    ),
                    canonical.display()
                ));
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("恢复默认工作区失败", "Failed to reset workspace")
                ));
            }
        }
    }

    pub fn update_budget_config(self, f: impl FnOnce(&mut BudgetConfig)) {
        let l = self.lang.get_untracked();
        let paths = self.paths.get_untracked();
        let mut next = self.budget_config.get_untracked();
        f(&mut next);
        match budget::save_budget_config(&paths, &next) {
            Ok(()) => {
                self.budget_config.set(next.clone());
                self.settings_monthly_budget_draft
                    .set(next.monthly_budget_usd.to_string());
                self.settings_monthly_tokens_draft
                    .set(next.monthly_token_limit.to_string());
                self.reload_budget_from_lake();
                self.set_info_notice(
                    l.t(
                        "已保存 AI 预算配置到 workbench/budget.json",
                        "Saved AI budget configuration to workbench/budget.json",
                    )
                    .to_owned(),
                );
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("保存预算配置失败", "Failed to save budget config")
                ));
            }
        }
    }

    pub fn apply_budget_drafts(self) {
        let l = self.lang.get_untracked();
        let usd_raw = self.settings_monthly_budget_draft.get_untracked();
        let tok_raw = self.settings_monthly_tokens_draft.get_untracked();
        let usd = usd_raw.trim().parse::<Decimal>();
        let toks = tok_raw.trim().parse::<u64>();
        match (usd, toks) {
            (Ok(u), Ok(t)) if u >= Decimal::ZERO => {
                self.update_budget_config(|cfg| {
                    cfg.monthly_budget_usd = u;
                    cfg.monthly_token_limit = t;
                });
            }
            _ => {
                self.set_error_notice(
                    l.t(
                        "请输入有效的月度美元上限与 Token 上限",
                        "Enter valid monthly USD and token limits",
                    )
                    .to_owned(),
                );
            }
        }
    }

    // ── Alan Connection Setup ────────────────────────────────────────────────

    fn populate_alan_drafts_from_profile(
        self,
        profile: &AlanConnectionProfileSummary,
        provider: AlanConnectionProvider,
    ) {
        self.alan_conn_provider.set(provider);
        self.alan_conn_profile_id_draft.set(profile.id.clone());
        self.alan_conn_model_draft.set(
            profile
                .model
                .clone()
                .unwrap_or_else(|| provider.default_model().to_owned()),
        );
        let custom_base = if provider.uses_base_url() {
            profile
                .base_url
                .as_deref()
                .filter(|url| *url != provider.default_base_url())
                .unwrap_or("")
        } else {
            ""
        };
        self.alan_conn_base_url_draft.set(custom_base.to_owned());
        self.alan_conn_project_id_draft
            .set(profile.project_id.clone().unwrap_or_default());
        self.alan_conn_location_draft.set(
            profile
                .location
                .clone()
                .unwrap_or_else(|| provider.default_location().to_owned()),
        );
    }

    /// Applies an updated [`AlanConnectionStatus`] snapshot and syncs form drafts
    /// to the active or first supported profile when available.
    pub fn apply_alan_connection_status(self, status: AlanConnectionStatus) {
        let preferred = status
            .active_profile()
            .and_then(|p| {
                AlanConnectionProvider::parse(&p.provider)
                    .ok()
                    .map(|prov| (p, prov))
            })
            .or_else(|| {
                status.profiles.iter().find_map(|p| {
                    AlanConnectionProvider::parse(&p.provider)
                        .ok()
                        .map(|prov| (p, prov))
                })
            });

        if let Some((profile, provider)) = preferred {
            self.populate_alan_drafts_from_profile(profile, provider);
        }
        self.alan_conn_status.set(Some(status));
    }

    /// Switches the selected Alan provider in the Settings form and updates default drafts.
    pub fn select_alan_conn_provider(self, provider: AlanConnectionProvider) {
        self.alan_conn_provider.set(provider);
        self.alan_conn_feedback.set(None);
        self.alan_conn_api_key_draft.set(String::new());

        let existing = self.alan_conn_status.with_untracked(|opt| {
            opt.as_ref().and_then(|st| {
                st.profiles
                    .iter()
                    .find(|p| p.provider == provider.as_str() && p.is_default)
                    .or_else(|| st.profiles.iter().find(|p| p.provider == provider.as_str()))
                    .cloned()
            })
        });

        if let Some(profile) = existing {
            self.populate_alan_drafts_from_profile(&profile, provider);
            return;
        }

        let cur_id = self.alan_conn_profile_id_draft.get_untracked();
        let is_any_default_id = AlanConnectionProvider::ALL
            .iter()
            .any(|p| cur_id.trim() == p.default_profile_id());
        if cur_id.trim().is_empty() || is_any_default_id {
            self.alan_conn_profile_id_draft
                .set(provider.default_profile_id().to_owned());
        }
        let cur_model = self.alan_conn_model_draft.get_untracked();
        let is_any_default_model = AlanConnectionProvider::ALL
            .iter()
            .any(|p| cur_model.trim() == p.default_model());
        if cur_model.trim().is_empty() || is_any_default_model {
            self.alan_conn_model_draft
                .set(provider.default_model().to_owned());
        }
        let cur_base = self.alan_conn_base_url_draft.get_untracked();
        let is_any_default_base = AlanConnectionProvider::ALL
            .iter()
            .any(|p| cur_base.trim() == p.default_base_url());
        if !provider.uses_base_url() || is_any_default_base {
            self.alan_conn_base_url_draft.set(String::new());
        }
        if provider == AlanConnectionProvider::GoogleVertex {
            let cur_loc = self.alan_conn_location_draft.get_untracked();
            if cur_loc.trim().is_empty() {
                self.alan_conn_location_draft
                    .set(provider.default_location().to_owned());
            }
        }
    }

    /// Reloads Alan's canonical connection status off-thread.
    pub fn refresh_alan_connection_status(self) {
        run_blocking_result(alan_connection::list_alan_connection_status, move |res| {
            if let Ok(status) = res {
                self.apply_alan_connection_status(status);
            }
        });
    }

    /// Saves the current Alan connection form into Alan's canonical System & Host Stores
    /// and sets the profile as Alan's default.
    pub fn save_alan_connection(self) {
        if self.alan_conn_saving.get_untracked() || self.alan_conn_logging_in.get_untracked() {
            return;
        }

        let provider = self.alan_conn_provider.get_untracked();
        let profile_id = self.alan_conn_profile_id_draft.get_untracked();
        let model = self.alan_conn_model_draft.get_untracked();
        let base_url_raw = self.alan_conn_base_url_draft.get_untracked();
        let api_key_raw = self.alan_conn_api_key_draft.get_untracked();
        let project_id_raw = self.alan_conn_project_id_draft.get_untracked();
        let location_raw = self.alan_conn_location_draft.get_untracked();

        let req = SaveAlanConnectionRequest {
            provider,
            profile_id,
            model,
            base_url: (provider.uses_base_url() && !base_url_raw.trim().is_empty())
                .then_some(base_url_raw),
            api_key: (!api_key_raw.trim().is_empty()).then_some(api_key_raw),
            project_id: (!project_id_raw.trim().is_empty()).then_some(project_id_raw),
            location: (!location_raw.trim().is_empty()).then_some(location_raw),
            set_as_default: true,
        };

        self.alan_conn_saving.set(true);
        self.alan_conn_feedback.set(None);

        run_blocking_result(
            move || -> Result<(AlanConnectionProfileSummary, AlanConnectionStatus), String> {
                let saved =
                    alan_connection::save_alan_connection(&req).map_err(|e| format!("{e:#}"))?;
                let status =
                    alan_connection::list_alan_connection_status().map_err(|e| format!("{e:#}"))?;
                Ok((saved, status))
            },
            move |res| {
                self.alan_conn_saving.set(false);
                let l = self.lang.get_untracked();
                match res {
                    Ok((saved, status)) => {
                        self.alan_conn_api_key_draft.set(String::new());
                        self.apply_alan_connection_status(status);
                        let msg = match l {
                            UiLang::Zh => format!(
                                "已保存并设为默认连接：{} ({})",
                                saved.id,
                                saved.model.as_deref().unwrap_or(&saved.provider)
                            ),
                            UiLang::En => format!(
                                "Saved & set default: {} ({})",
                                saved.id,
                                saved.model.as_deref().unwrap_or(&saved.provider)
                            ),
                        };
                        self.alan_conn_feedback.set(Some((msg.clone(), false)));
                        self.set_info_notice(msg);
                    }
                    Err(err) => {
                        let msg = format!(
                            "{}: {err}",
                            l.t("保存 Alan 连接失败", "Failed to save Alan connection")
                        );
                        self.alan_conn_feedback.set(Some((msg.clone(), true)));
                        self.set_error_notice(msg);
                    }
                }
            },
        );
    }

    /// Launches interactive browser OAuth/ADC sign-in for ChatGPT, OpenRouter, or Vertex AI.
    pub fn start_alan_browser_login(self) {
        if self.alan_conn_logging_in.get_untracked() || self.alan_conn_saving.get_untracked() {
            return;
        }
        let provider = self.alan_conn_provider.get_untracked();
        if !provider.supports_browser_login() {
            return;
        }

        let profile_id = self.alan_conn_profile_id_draft.get_untracked();
        let model = self.alan_conn_model_draft.get_untracked();
        let base_url_raw = self.alan_conn_base_url_draft.get_untracked();
        let project_id_raw = self.alan_conn_project_id_draft.get_untracked();
        let location_raw = self.alan_conn_location_draft.get_untracked();

        let req = BrowserLoginAlanConnectionRequest {
            provider,
            profile_id,
            model,
            base_url: (provider.uses_base_url() && !base_url_raw.trim().is_empty())
                .then_some(base_url_raw),
            project_id: (!project_id_raw.trim().is_empty()).then_some(project_id_raw),
            location: (!location_raw.trim().is_empty()).then_some(location_raw),
            set_as_default: true,
        };

        let l = self.lang.get_untracked();
        self.alan_conn_logging_in.set(true);
        let hint = match l {
            UiLang::Zh => format!("正在打开浏览器完成 {} 授权…", provider.display_name()),
            UiLang::En => format!("Opening browser for {} sign-in…", provider.display_name()),
        };
        self.alan_conn_feedback.set(Some((hint, false)));

        run_blocking_result(
            move || -> Result<(AlanConnectionBrowserLoginResult, AlanConnectionStatus), String> {
                let res = alan_connection::browser_login_alan_connection(&req)
                    .map_err(|e| format!("{e:#}"))?;
                let status =
                    alan_connection::list_alan_connection_status().map_err(|e| format!("{e:#}"))?;
                Ok((res, status))
            },
            move |res| {
                self.alan_conn_logging_in.set(false);
                let l = self.lang.get_untracked();
                match res {
                    Ok((outcome, status)) => {
                        self.alan_conn_api_key_draft.set(String::new());
                        self.apply_alan_connection_status(status);
                        let msg = outcome.message;
                        self.alan_conn_feedback.set(Some((msg.clone(), false)));
                        self.set_info_notice(msg);
                    }
                    Err(err) => {
                        let msg = format!(
                            "{}: {err}",
                            l.t("浏览器登录未完成", "Browser sign-in failed")
                        );
                        self.alan_conn_feedback.set(Some((msg.clone(), true)));
                        self.set_error_notice(msg);
                    }
                }
            },
        );
    }

    /// Tests connectivity for the configured Alan connection profile off-thread.
    pub fn test_alan_connection(self) {
        if self.alan_conn_testing.get_untracked() {
            return;
        }

        let draft_id = self.alan_conn_profile_id_draft.get_untracked();
        let target_profile = self.alan_conn_status.with_untracked(|opt| {
            let trimmed = draft_id.trim();
            if let Some(st) = opt
                && !trimmed.is_empty()
                && st.profiles.iter().any(|p| p.id == trimmed)
            {
                return Some(trimmed.to_owned());
            }
            None
        });

        self.alan_conn_testing.set(true);
        self.alan_conn_feedback.set(None);

        run_blocking_result(
            move || -> Result<AlanConnectionTestResult, String> {
                alan_connection::test_alan_connection(target_profile.as_deref())
                    .map_err(|e| format!("{e:#}"))
            },
            move |res| {
                self.alan_conn_testing.set(false);
                let l = self.lang.get_untracked();
                match res {
                    Ok(report) => {
                        let msg = match l {
                            UiLang::Zh => format!(
                                "连接测试成功：{} · HTTP {} ({}ms)",
                                report.profile_id, report.status_code, report.latency_ms
                            ),
                            UiLang::En => format!(
                                "Connection OK: {} · HTTP {} ({}ms)",
                                report.profile_id, report.status_code, report.latency_ms
                            ),
                        };
                        self.alan_conn_feedback.set(Some((msg.clone(), false)));
                        self.set_info_notice(msg);
                    }
                    Err(err) => {
                        let msg = format!(
                            "{}: {err}",
                            l.t("Alan 连接测试失败", "Alan connection test failed")
                        );
                        self.alan_conn_feedback.set(Some((msg.clone(), true)));
                        self.set_error_notice(msg);
                    }
                }
            },
        );
    }

    // ── Circle Overview / Baskets / Dossier Actions ──────────────────────────

    pub fn focus_basket(self, bucket: Option<Bucket>) {
        self.basket_filter.set(bucket);
        if let Some(b) = bucket {
            self.collapsed_baskets.update(|set| {
                set.remove(&b);
            });
        }
        self.baskets_cursor_index.set(0);
        self.company_open.set(false);
        self.active_tab.set(WorkspaceTab::Baskets);
    }

    pub fn jump_to_tag(self, tag: String) {
        self.circle_filter.set(Some(tag));
        self.basket_filter.set(None);
        self.baskets_cursor_index.set(0);
        self.company_open.set(false);
        self.active_tab.set(WorkspaceTab::Baskets);
    }

    pub fn toggle_circle_filter(self, tag: &str) {
        self.circle_filter.update(|cur| {
            if cur.as_deref() == Some(tag) {
                *cur = None;
            } else {
                *cur = Some(tag.to_owned());
            }
        });
        self.baskets_cursor_index.set(0);
    }

    pub fn toggle_status_filter(self, target: StatusQuickFilter) {
        self.status_filter.update(|cur| {
            *cur = if *cur == target {
                StatusQuickFilter::All
            } else {
                target
            };
        });
        self.baskets_cursor_index.set(0);
    }

    pub fn toggle_baskets_view(self) {
        self.baskets_view_mode.update(|m| {
            *m = match *m {
                BasketsViewMode::List => BasketsViewMode::Board,
                BasketsViewMode::Board => BasketsViewMode::List,
            };
        });
        self.company_open.set(false);
        self.active_tab.set(WorkspaceTab::Baskets);
    }

    pub fn toggle_basket_collapsed(self, bucket: Bucket) {
        self.collapsed_baskets.update(|set| {
            if !set.remove(&bucket) {
                set.insert(bucket);
            }
        });
    }

    pub fn toggle_sort(self, col: SortCol) {
        let cur_col = self.sort_col.get_untracked();
        if cur_col == col {
            if self.sort_asc.get_untracked() {
                self.sort_asc.set(false);
            } else {
                self.sort_col.set(SortCol::Default);
            }
        } else {
            self.sort_col.set(col);
            self.sort_asc.set(matches!(
                col,
                SortCol::Ticker | SortCol::Margin | SortCol::NetDebt
            ));
        }
        self.baskets_cursor_index.set(0);
    }

    pub fn toggle_basis(self) {
        self.use_ttm_basis.update(|b| *b = !*b);
    }

    pub fn toggle_inspector(self) {
        let is_open = self.company_open.get_untracked();
        let cur = self.company_view_tab.get_untracked();
        self.company_view_tab
            .set(if is_open && cur == CompanyViewTab::XbrlInspector {
                CompanyViewTab::FinancialsMemo
            } else {
                CompanyViewTab::XbrlInspector
            });
        self.company_open.set(true);
    }

    pub fn move_company_bucket(self, ticker: &str, target: Bucket) {
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match store::update_company_bucket(&paths, ticker, target) {
            Ok(_) => {
                self.reload_from_disk();
                self.set_info_notice(match l {
                    UiLang::Zh => format!(
                        "已将 {ticker} 分拣至 [{} · {}]（已写入 workbench/watchlist.json）",
                        target.code(),
                        target.short_label_for(UiLang::Zh)
                    ),
                    UiLang::En => format!(
                        "Moved {ticker} to [{} · {}] (saved to workbench/watchlist.json)",
                        target.code(),
                        target.short_label_for(UiLang::En)
                    ),
                });
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("切换能力圈分组失败", "Failed to update Circle tier")
                ));
            }
        }
    }

    pub fn visible_linear_tickers(self) -> Vec<String> {
        let mode = self.baskets_view_mode.get_untracked();
        let basket_filter = self.basket_filter.get_untracked();
        let collapsed = self.collapsed_baskets.get_untracked();
        let circle_filter = self.circle_filter.get_untracked();
        let status_filter = self.status_filter.get_untracked();
        let sort_col = self.sort_col.get_untracked();
        let sort_asc = self.sort_asc.get_untracked();

        let order: Vec<Bucket> = match mode {
            BasketsViewMode::List => match basket_filter {
                Some(b) => vec![b],
                None => Bucket::display_order().to_vec(),
            },
            BasketsViewMode::Board => {
                vec![Bucket::Watch, Bucket::Core, Bucket::Out, Bucket::TooHard]
            }
        };

        let mut out = Vec::new();
        self.reports.with_untracked(|reports| {
            for b in order {
                if mode == BasketsViewMode::List
                    && basket_filter.is_none()
                    && collapsed.contains(&b)
                {
                    continue;
                }
                for r in filter_reports_for_basket(
                    reports,
                    b,
                    circle_filter.as_deref(),
                    status_filter,
                    sort_col,
                    sort_asc,
                ) {
                    out.push(r.ticker.clone());
                }
            }
        });
        out
    }

    pub fn step_selected_company(self, delta: isize) {
        let next_ticker = self.reports.with_untracked(|reps| {
            let len = reps.len();
            if len == 0 {
                return None;
            }
            let cur = self.selected_ticker.get_untracked();
            reps.iter()
                .position(|r| r.ticker == cur)
                .map(|pos| {
                    let next_idx = (pos as isize + delta).rem_euclid(len as isize) as usize;
                    reps[next_idx].ticker.clone()
                })
                .or_else(|| reps.first().map(|r| r.ticker.clone()))
        });
        if let Some(ticker) = next_ticker {
            self.load_selection(&ticker);
        }
    }

    pub fn save_sandbox_assumptions(self) {
        let ticker = self.selected_ticker.get_untracked();
        let has_rep = self
            .reports
            .with_untracked(|reps| reps.iter().any(|r| r.ticker == ticker));
        if !has_rep {
            return;
        }
        let mult = self.sandbox_multiple.get_untracked();
        let ret = self.sandbox_target_return.get_untracked();
        let growth = self.sandbox_expected_growth.get_untracked();
        let next = ValuationAssumptions {
            multiple_low: mult,
            multiple_high: mult + Decimal::from(4),
            target_return_pct: ret,
            expected_growth_pct: growth,
            strike_price_override: None,
        };
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match store::update_company_assumptions(&paths, &ticker, next) {
            Ok(_) => {
                self.reload_from_disk();
                self.set_info_notice(match l {
                    UiLang::Zh => {
                        format!("已保存 {ticker} 假设 · {mult}x · 目标 {ret}% · 增速 {growth}%")
                    }
                    UiLang::En => format!(
                        "Saved {ticker} assumptions · {mult}x · target {ret}% · growth {growth}%"
                    ),
                });
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("保存假设失败", "Failed to save assumptions")
                ));
            }
        }
    }

    pub fn reset_sandbox_to_saved(self) {
        let ticker = self.selected_ticker.get_untracked();
        self.reports.with_untracked(|reps| {
            if let Some(rep) = reps.iter().find(|r| r.ticker == ticker) {
                self.sandbox_multiple.set(rep.assumptions.multiple_low);
                self.sandbox_target_return
                    .set(rep.assumptions.target_return_pct);
                self.sandbox_expected_growth
                    .set(rep.assumptions.expected_growth_pct);
            }
        });
    }

    // ── Guided First-Run Onboarding ──────────────────────────────────────────

    /// Returns `true` when the active workspace is fresh and onboarding has not been dismissed.
    pub fn should_show_onboarding(self) -> bool {
        if !self.startup_hydrated.get() {
            return false;
        }
        let watchlist_empty = self.reports.with(|r| r.is_empty());
        let dismissed = self.onboarding_dismissed.get();
        self.universe_summary.with(|sum| {
            crate::views::onboarding::is_fresh_workspace_for_onboarding(
                watchlist_empty,
                sum.as_ref(),
                dismissed,
            )
        })
    }

    /// Dismisses the first-run onboarding card for the current session.
    pub fn dismiss_onboarding(self) {
        self.onboarding_dismissed.set(true);
    }

    /// Applies the curated starter pack, syncs the 8 classics (~5s), and queues
    /// non-blocking progressive L0 lakehouse synchronization right afterward if L0 is empty.
    pub fn start_starter_pack_and_progressive_l0(self) {
        self.pending_progressive_l0_after_sync.set(true);
        self.start_starter_pack_and_sync();
    }

    /// Applies the curated Berkshire × Munger starter pack, refreshes store state,
    /// switches to Circle Overview, and launches background SEC/price sync.
    pub fn start_starter_pack_and_sync(self) {
        let l = self.lang.get_untracked();
        if self.onboarding_applying.get_untracked()
            || self.sync_running.get_untracked()
            || self.universe_syncing.get_untracked()
        {
            if self.universe_syncing.get_untracked() {
                self.set_info_notice(
                    l.t(
                        "全美股 L0 正在后台同步中，请等待完成后再装载经典能力圈",
                        "US L0 lakehouse is syncing in background; wait until it finishes",
                    )
                    .to_owned(),
                );
            }
            return;
        }
        self.onboarding_applying.set(true);
        self.set_info_notice(
            l.t(
                "正在装载 8 家伯克希尔 × 芒格经典标的…",
                "Loading 8 Berkshire × Munger classic companies…",
            )
            .to_owned(),
        );

        let root = self.paths.get_untracked().repo_root;
        run_blocking_result(
            move || {
                let paths = WorkspacePaths::from_root(root);
                m3_core::starter::apply_starter_pack(&paths).map_err(|e| format!("{e:#}"))
            },
            move |res| {
                self.onboarding_applying.set(false);
                let l = self.lang.get_untracked();
                match res {
                    Ok(report) => {
                        self.reload_from_disk();
                        self.open_tab(WorkspaceTab::Baskets);
                        self.set_info_notice(match l {
                            UiLang::Zh => format!(
                                "已装载 {} 家经典标的（共 {} 家）· 正在后台同步财报、股价与 K 线…",
                                report.added_tickers.len(),
                                report.watchlist_total
                            ),
                            UiLang::En => format!(
                                "Loaded {} classic companies ({} total) · syncing filings, quotes & K-lines…",
                                report.added_tickers.len(),
                                report.watchlist_total
                            ),
                        });
                        self.start_sync();
                    }
                    Err(err) => {
                        self.set_error_notice(format!(
                            "{}: {err}",
                            l.t("装载经典能力圈失败", "Failed to load starter pack")
                        ));
                    }
                }
            },
        );
    }

    // ── Circle Sync (Event-Driven Blocking) ──────────────────────────────────

    pub fn start_sync(self) {
        let l = self.lang.get_untracked();
        if self.sync_running.get_untracked() || self.universe_syncing.get_untracked() {
            if self.universe_syncing.get_untracked() {
                self.set_info_notice(
                    l.t(
                        "全美股 L0 正在后台同步中，请等待完成后再同步自选池",
                        "US L0 lakehouse is syncing in background; wait until it finishes",
                    )
                    .to_owned(),
                );
            }
            return;
        }
        self.sync_running.set(true);
        self.sync_stage.set(Some(
            l.t("SEC EDGAR / 行情同步", "SEC EDGAR / Quote Sync")
                .to_owned(),
        ));
        self.sync_progress.set(Some(0.15));
        self.set_info_notice(
            l.t(
                "正在同步 SEC EDGAR 财报、公开股价与 K 线历史…",
                "Syncing SEC EDGAR filings, market quotes, and price history…",
            )
            .to_owned(),
        );

        let root = self.paths.get_untracked().repo_root;
        run_blocking_result(
            move || {
                let paths = WorkspacePaths::from_root(root);
                store::sync_all(&paths, None, false).map_err(|e| format!("{e:#}"))
            },
            move |res| {
                self.sync_running.set(false);
                self.sync_stage.set(None);
                self.sync_progress.set(None);
                let l = self.lang.get_untracked();
                match res {
                    Ok(status) => {
                        let failed: Vec<&str> = status
                            .tickers
                            .iter()
                            .filter(|t| t.error.is_some())
                            .map(|t| t.ticker.as_str())
                            .collect();
                        self.reload_from_disk();
                        if failed.is_empty() {
                            self.set_info_notice(match l {
                                UiLang::Zh => {
                                    format!("同步完成 · 已更新 {} 家标的", status.tickers.len())
                                }
                                UiLang::En => {
                                    format!(
                                        "Sync complete · updated {} holdings",
                                        status.tickers.len()
                                    )
                                }
                            });
                        } else {
                            self.set_error_notice(format!(
                                "{}: {}",
                                l.t(
                                    "部分标的同步异常（已保留旧缓存）",
                                    "Partial sync errors (cached data preserved)"
                                ),
                                failed.join(", ")
                            ));
                        }
                        if self.pending_progressive_l0_after_sync.get_untracked() {
                            self.pending_progressive_l0_after_sync.set(false);
                            let l0_empty = self.universe_summary.with_untracked(|s| {
                                s.as_ref().is_none_or(|u| u.total_companies == 0)
                            });
                            if l0_empty {
                                self.start_universe_sync(false);
                            }
                        }
                    }
                    Err(err) => {
                        self.pending_progressive_l0_after_sync.set(false);
                        self.set_error_notice(format!("{}: {err}", l.t("同步失败", "Sync failed")));
                    }
                }
            },
        );
    }

    // ── L1 Briefing Inbox ────────────────────────────────────────────────────

    pub fn maybe_start_startup_catchup(self) {
        if should_skip_auto_catchup(self.universe_lake_present.get_untracked()) {
            return;
        }
        let bootstrap = self
            .inbox_summary
            .with_untracked(|s| s.as_ref().is_none_or(|x| x.last_sec_bootstrap_ms.is_none()));
        self.spawn_l1_catchup_worker(bootstrap, true);
    }

    pub fn trigger_l1_catchup(self) {
        self.open_tab(WorkspaceTab::Inbox);
        self.start_l1_catchup(false);
    }

    pub fn start_l1_catchup(self, bootstrap: bool) {
        let l = self.lang.get_untracked();
        if !can_start_duckdb_writer(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
            self.sync_running.get_untracked(),
        ) {
            if self.universe_syncing.get_untracked() || self.sync_running.get_untracked() {
                self.set_info_notice(
                    l.t(
                        "其他后台同步任务进行中，请等待完成后再追平事件流",
                        "Another background sync is active; wait until it finishes",
                    )
                    .to_owned(),
                );
            }
            return;
        }
        self.spawn_l1_catchup_worker(bootstrap, false);
    }

    fn spawn_l1_catchup_worker(self, bootstrap: bool, silent_startup: bool) {
        if !can_start_duckdb_writer(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
            self.sync_running.get_untracked(),
        ) {
            return;
        }
        let l = self.lang.get_untracked();
        self.inbox_catching_up.set(true);
        self.inbox_last_auto_catchup_at
            .replace(Some(Instant::now()));
        self.inbox_catchup_progress.set(Some(if silent_startup {
            l.t("启动后台追平 L1 事件流…", "Starting L1 catch-up…")
                .into()
        } else {
            l.t(
                "正在追平 SEC 公告、新闻与语义索引…",
                "Catching up SEC filings, news & semantic index…",
            )
            .into()
        }));
        if !silent_startup {
            self.set_info_notice(
                l.t(
                    "正在后台追平 L1 事件流（SEC 公告 + 候选新闻 + 本地语义索引，0 Flagship Token）…",
                    "Catching up L1 stream in background (SEC filings + news + local index, 0 Flagship Tokens)…",
                )
                .to_owned(),
            );
        }

        let root = self.paths.get_untracked().repo_root;
        run_streamed_blocking(
            move |tx| {
                let paths = WorkspacePaths::from_root(root);
                let opts = EventCatchupOptions {
                    bootstrap,
                    ..EventCatchupOptions::default()
                };
                let res = catchup_events_with_progress(&paths, &opts, |msg| {
                    let _ = tx.send(CatchupMsg::Progress(msg.to_owned()));
                })
                .map_err(|e| format!("{e:#}"));
                let _ = tx.send(CatchupMsg::Finished(Box::new(res)));
            },
            move |msg| match msg {
                CatchupMsg::Progress(p) => {
                    self.inbox_catchup_progress.set(Some(p));
                }
                CatchupMsg::Finished(res) => {
                    let res = *res;
                    self.inbox_catching_up.set(false);
                    self.inbox_catchup_progress.set(None);
                    self.inbox_last_auto_catchup_at
                        .replace(Some(Instant::now()));
                    let l = self.lang.get_untracked();
                    match res {
                        Ok(report) => {
                            self.reload_inbox_from_lake();
                            self.set_info_notice(match l {
                                UiLang::Zh => format!(
                                    "L1 事件流追平完成 · 新增公告 {} / 新闻 {} / 索引 {} 片段 / 信号 {}（未读 {} · {} Flagship Token）",
                                    report.sec_filings_ingested,
                                    report.news_events_ingested,
                                    report.chunks_indexed,
                                    report.signals_created_or_updated,
                                    report.summary.unread_signals,
                                    report.flagship_tokens_used,
                                ),
                                UiLang::En => format!(
                                    "L1 catch-up complete · +{} filings / +{} news / {} chunks / {} signals ({} unread · {} Flagship Tokens)",
                                    report.sec_filings_ingested,
                                    report.news_events_ingested,
                                    report.chunks_indexed,
                                    report.signals_created_or_updated,
                                    report.summary.unread_signals,
                                    report.flagship_tokens_used,
                                ),
                            });
                        }
                        Err(err) => {
                            self.set_error_notice(format!(
                                "{}: {err}",
                                l.t("L1 事件流追平失败", "L1 catch-up failed")
                            ));
                        }
                    }
                }
            },
            move || {
                self.inbox_catching_up.set(false);
            },
        );
    }

    pub fn maybe_schedule_periodic_catchup(self) {
        if should_skip_auto_catchup(self.universe_lake_present.get_untracked()) {
            return;
        }
        let elapsed_ok = self
            .inbox_last_auto_catchup_at
            .with(|opt| opt.is_some_and(|t| t.elapsed() >= AUTO_CATCHUP_INTERVAL));
        if elapsed_ok
            && can_start_duckdb_writer(
                self.universe_syncing.get_untracked(),
                self.inbox_catching_up.get_untracked(),
                self.sync_running.get_untracked(),
            )
        {
            self.spawn_l1_catchup_worker(false, true);
        }
    }

    pub fn trigger_profile_fetch(self, ticker: &str, cik: &str) {
        let l = self.lang.get_untracked();
        if !can_start_duckdb_writer(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
            self.sync_running.get_untracked(),
        ) {
            self.set_info_notice(
                l.t(
                    "后台同步或事件追平进行中，请稍候再拉取 10-K 原文",
                    "Background sync active — try fetching 10-K in a moment",
                )
                .to_owned(),
            );
            return;
        }
        if self.inbox_profile_fetching_ticker.get_untracked().is_some() {
            return;
        }
        let upper = ticker.trim().to_ascii_uppercase();
        let cik_owned = cik.trim().to_owned();
        if upper.is_empty() || cik_owned.is_empty() {
            return;
        }
        self.inbox_profile_fetching_ticker.set(Some(upper.clone()));
        self.set_info_notice(match l {
            UiLang::Zh => format!("正在从 SEC EDGAR 拉取 {upper} 最新 10-K 业务简介…"),
            UiLang::En => format!("Fetching {upper} latest 10-K business profile from SEC…"),
        });

        let root = self.paths.get_untracked().repo_root;
        run_blocking_result(
            move || {
                let paths = WorkspacePaths::from_root(root);
                fetch_company_10k_profile(&paths, &upper, &cik_owned).map_err(|e| format!("{e:#}"))
            },
            move |res: Result<CompanyProfileFetchReport, String>| {
                self.inbox_profile_fetching_ticker.set(None);
                let l = self.lang.get_untracked();
                match res {
                    Ok(rep) => {
                        self.reload_inbox_from_lake();
                        self.reload_inbox_selection_details();
                        self.set_info_notice(match l {
                            UiLang::Zh => format!(
                                "已拉取 {} 10-K 业务简介（{} 个文本片段，0 LLM Token）",
                                rep.ticker, rep.chunks_indexed
                            ),
                            UiLang::En => format!(
                                "Fetched {} 10-K business profile ({} chunks, 0 LLM tokens)",
                                rep.ticker, rep.chunks_indexed
                            ),
                        });
                    }
                    Err(err) => {
                        self.set_error_notice(match l {
                            UiLang::Zh => format!("拉取 10-K 业务简介失败：{err}"),
                            UiLang::En => format!("Failed to fetch 10-K profile: {err}"),
                        });
                    }
                }
            },
        );
    }

    pub fn reload_inbox_from_lake(self) {
        if self.universe_syncing.get_untracked() {
            return;
        }
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match LakeDb::open_read_only_if_exists(&paths) {
            Ok(None) => {
                self.inbox_summary.set(None);
                self.inbox_signals.set(Vec::new());
                self.inbox_error.set(None);
                self.inbox_state.update(|st| {
                    st.selected_signal_id = None;
                    st.cursor_index = 0;
                });
            }
            Ok(Some(db)) => {
                let summary_res = db.l1_summary();
                let list_res = db.list_inbox_signals(None, None, 500);
                if let Ok(budget) = db.budget_status_report(&paths) {
                    self.budget_report.set(Some(budget));
                }
                match (summary_res, list_res) {
                    (Ok(summary), Ok(signals)) => {
                        self.inbox_summary.set(Some(summary));
                        self.inbox_signals.set(signals);
                        self.inbox_error.set(None);
                        self.sync_inbox_selection();
                    }
                    (Err(err), _) | (_, Err(err)) => {
                        self.inbox_error.set(Some(format!(
                            "{}: {err:#}",
                            l.t("读取 L1 简报收件箱失败", "Failed to read L1 Briefing Inbox")
                        )));
                    }
                }
            }
            Err(err) => {
                self.inbox_error.set(Some(format!(
                    "{}: {err:#}",
                    l.t("打开 m3.duckdb 失败", "Failed to open m3.duckdb")
                )));
            }
        }
    }

    pub fn reload_inbox_selection_details(self) {
        let active_ticker = self
            .active_inbox_signal()
            .map(|s| s.primary_ticker.trim().to_ascii_uppercase())
            .filter(|t| !t.is_empty());
        let Some(ticker) = active_ticker else {
            self.inbox_selected_details.set(None);
            self.inbox_selected_business_chunks.set(Vec::new());
            return;
        };
        if !can_open_duckdb_read_only(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
        ) {
            let keep = self.inbox_selected_details.with_untracked(|opt| {
                opt.as_ref()
                    .is_some_and(|d| d.row.ticker.eq_ignore_ascii_case(&ticker))
            });
            if !keep {
                self.inbox_selected_details.set(None);
                self.inbox_selected_business_chunks.set(Vec::new());
            }
            return;
        }
        let paths = self.paths.get_untracked();
        match LakeDb::open_read_only_if_exists(&paths) {
            Ok(Some(db)) => match db.load_drawer_details_by_ticker(&ticker, 5) {
                Ok(Some(details)) => {
                    let chunks = db
                        .load_company_business_chunks(&details.row.cik, 6)
                        .unwrap_or_default();
                    self.inbox_selected_details.set(Some(details));
                    self.inbox_selected_business_chunks.set(chunks);
                }
                Ok(None) | Err(_) => {
                    self.inbox_selected_details.set(None);
                    self.inbox_selected_business_chunks.set(Vec::new());
                }
            },
            Ok(None) | Err(_) => {
                self.inbox_selected_details.set(None);
                self.inbox_selected_business_chunks.set(Vec::new());
            }
        }
    }

    pub fn sync_inbox_selection(self) {
        let st = self.inbox_state.get_untracked();
        let next = self.inbox_signals.with_untracked(|sigs| {
            let visible = filter_inbox_signals(sigs, st.filter);
            resolve_active_inbox_signal(&visible, st.selected_signal_id.as_deref(), st.cursor_index)
                .map(|(idx, sig)| (idx, sig.signal_id.clone()))
        });
        self.inbox_state.update(|state| match next {
            Some((idx, id)) => {
                state.cursor_index = idx;
                state.selected_signal_id = Some(id);
            }
            None => {
                state.cursor_index = 0;
                state.selected_signal_id = None;
            }
        });
        self.reload_inbox_selection_details();
    }

    pub fn set_inbox_filter(self, filter: InboxFilter) {
        self.inbox_state.update(|st| {
            st.filter = filter;
            st.cursor_index = 0;
            st.selected_signal_id = None;
        });
        self.sync_inbox_selection();
    }

    pub fn select_inbox_signal(self, signal_id: &str, cursor_index: usize) {
        self.inbox_state.update(|st| {
            st.selected_signal_id = Some(signal_id.to_owned());
            st.cursor_index = cursor_index;
        });
        self.reload_inbox_selection_details();
    }

    pub fn step_inbox_cursor(self, delta: isize) {
        let st = self.inbox_state.get_untracked();
        let next = self.inbox_signals.with_untracked(|sigs| {
            let visible = filter_inbox_signals(sigs, st.filter);
            step_inbox_selection(
                &visible,
                st.selected_signal_id.as_deref(),
                st.cursor_index,
                delta,
            )
        });
        if let Some((idx, id)) = next {
            self.inbox_state.update(|state| {
                state.cursor_index = idx;
                state.selected_signal_id = Some(id);
            });
            self.reload_inbox_selection_details();
        }
    }

    pub fn active_inbox_signal(self) -> Option<SignalInboxRow> {
        let st = self.inbox_state.get_untracked();
        self.inbox_signals.with_untracked(|sigs| {
            let visible = filter_inbox_signals(sigs, st.filter);
            resolve_active_inbox_signal(&visible, st.selected_signal_id.as_deref(), st.cursor_index)
                .map(|(_, sig)| sig.clone())
        })
    }

    pub fn triage_inbox_signal(
        self,
        signal_id: &str,
        target_status: SignalStatus,
        promote_watch: bool,
        open_agent: bool,
    ) {
        let l = self.lang.get_untracked();
        if self.universe_syncing.get_untracked() || self.inbox_catching_up.get_untracked() {
            self.set_info_notice(
                l.t(
                    "后台事件追平或湖仓同步进行中，请稍候再分拣",
                    "Background sync or catch-up running — try triage in a moment",
                )
                .to_owned(),
            );
            return;
        }

        let Some(sig) = self
            .inbox_signals
            .with_untracked(|sigs| sigs.iter().find(|s| s.signal_id == signal_id).cloned())
        else {
            self.set_error_notice(format!(
                "{} {signal_id}",
                l.t("未找到信号", "Signal not found:")
            ));
            return;
        };

        let ticker = sig.primary_ticker.trim().to_ascii_uppercase();
        let in_watchlist = self
            .reports
            .with_untracked(|reps| reps.iter().any(|r| r.ticker.eq_ignore_ascii_case(&ticker)));
        let paths = self.paths.get_untracked();

        let Ok(db) = LakeDb::open(&paths) else {
            self.set_error_notice(
                l.t(
                    "打开 m3.duckdb 失败，无法更新信号状态",
                    "Failed to open m3.duckdb to update signal status",
                )
                .to_owned(),
            );
            return;
        };

        match db.triage_signal(signal_id, target_status, store::now_ms()) {
            Ok(true) => {}
            Ok(false) => {
                self.set_error_notice(match l {
                    UiLang::Zh => format!("信号 {signal_id} 不存在或未变更"),
                    UiLang::En => format!("Signal {signal_id} missing or unchanged"),
                });
                return;
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("分拣信号失败", "Failed to triage signal")
                ));
                return;
            }
        }

        let need_watch_add = promote_watch || (open_agent && !in_watchlist);
        if need_watch_add && !ticker.is_empty() {
            let universe_row = db
                .screen_universe(&m3_core::lake::UniverseFilter {
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
                &paths,
                &ticker,
                Bucket::Watch,
                name_opt,
                cik_opt,
                circle_opt.as_deref(),
                None,
            ) {
                drop(db);
                self.reload_inbox_from_lake();
                self.set_error_notice(match l {
                    UiLang::Zh => format!("已更新信号状态，但加入观察池 ({ticker}) 失败：{err:#}"),
                    UiLang::En => {
                        format!("Updated signal, but failed to add {ticker} to Watch: {err:#}")
                    }
                });
                return;
            }
            let _ = db.refresh_watchlist_flags(&paths);
            drop(db);
            self.reload_from_disk();
            self.reload_universe_from_lake();
        } else {
            drop(db);
        }

        self.reload_inbox_from_lake();

        let status_label = signal_status_badge_for(target_status, l).0;
        self.set_info_notice(if promote_watch {
            match l {
                UiLang::Zh => {
                    format!("已将 {ticker} 加入观察池 [WATCH] 并标记信号为「{status_label}」")
                }
                UiLang::En => {
                    format!("Added {ticker} to [WATCH] and marked signal as {status_label}")
                }
            }
        } else {
            match l {
                UiLang::Zh => format!("已将 {ticker} 信号标记为「{status_label}」"),
                UiLang::En => format!("Marked {ticker} signal as {status_label}"),
            }
        });

        if open_agent && !ticker.is_empty() {
            let canonical = self.reports.with_untracked(|reps| {
                reps.iter()
                    .find(|r| r.ticker.eq_ignore_ascii_case(&ticker))
                    .map(|r| r.ticker.clone())
                    .unwrap_or(ticker)
            });
            self.open_company(&canonical, CompanyViewTab::DeepResearchAlan);
            self.start_deep_research(&canonical, None, "inbox");
        }
    }

    pub fn open_inbox_signal_target(self, ticker: &str) {
        let trimmed = ticker.trim();
        if trimmed.is_empty() {
            return;
        }
        let found = self.reports.with_untracked(|reps| {
            reps.iter()
                .find(|r| r.ticker.eq_ignore_ascii_case(trimmed))
                .map(|r| r.ticker.clone())
        });
        if let Some(canonical) = found {
            self.open_company(&canonical, CompanyViewTab::FinancialsMemo);
            return;
        }
        self.universe_state.update(|st| {
            st.search_mode = UniverseSearchMode::Keyword;
            st.query = trimmed.to_owned();
            st.exclude_financials = false;
            st.require_5y_history = false;
        });
        self.open_tab(WorkspaceTab::Universe);
        let exact = self.universe_rows.with_untracked(|rows| {
            rows.iter()
                .find(|r| r.ticker.eq_ignore_ascii_case(trimmed))
                .cloned()
        });
        if let Some(row) = exact {
            self.select_universe_row(&row);
        }
    }

    // ── Universe Radar (L0/L1 DuckDB + Async Lookalike) ───────────────────────

    pub fn trigger_universe_sync(self) {
        self.open_tab(WorkspaceTab::Universe);
        self.start_universe_sync(false);
    }

    pub fn reload_universe_from_lake(self) {
        if self.universe_syncing.get_untracked() {
            return;
        }
        let st = self.universe_state.get_untracked();
        let trimmed_query = st.query.trim().to_owned();
        if st.search_mode == UniverseSearchMode::Similar && !trimmed_query.is_empty() {
            self.run_universe_lookalike_async();
            return;
        }

        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match LakeDb::open_read_only_if_exists(&paths) {
            Ok(None) => {
                self.universe_lake_present.set(false);
                self.universe_summary.set(None);
                self.universe_rows.set(Vec::new());
                self.universe_similar_hits.set(Vec::new());
                self.universe_similar_query_ms.set(None);
                self.universe_strike_zone_ciks.set(HashSet::new());
                self.universe_selected_cik.set(None);
                self.universe_selected_row.set(None);
                self.clear_universe_drawer_details();
                self.universe_error.set(None);
            }
            Ok(Some(db)) => {
                self.universe_lake_present.set(true);
                self.universe_similar_hits.set(Vec::new());
                self.universe_similar_query_ms.set(None);
                let sel_cik = self.universe_selected_cik.get_untracked();
                match db.load_universe_snapshot(&st.to_lake_filter(), sel_cik.as_deref(), 5) {
                    Ok(snapshot) => {
                        self.universe_summary.set(Some(snapshot.summary));
                        self.universe_strike_zone_ciks
                            .set(snapshot.strike_zone_ciks);
                        self.universe_rows
                            .set(apply_ui_row_guards(snapshot.rows, st.require_5y_history));
                        self.universe_error.set(None);
                        self.sync_universe_drawer_selection(&db, snapshot.selected_details);
                    }
                    Err(err) => {
                        self.clear_universe_drawer_details();
                        self.universe_error.set(Some(match l {
                            UiLang::Zh => format!("读取全市场湖仓失败: {err:#}"),
                            UiLang::En => format!("Failed to load Universe lakehouse: {err:#}"),
                        }));
                    }
                }
            }
            Err(err) => {
                self.universe_lake_present
                    .set(paths.lake_db_path().exists());
                self.clear_universe_drawer_details();
                self.universe_error.set(Some(match l {
                    UiLang::Zh => format!("打开 m3.duckdb 失败: {err:#}"),
                    UiLang::En => format!("Failed to open m3.duckdb: {err:#}"),
                }));
            }
        }
    }

    /// Executes Lookalike semantic search on Tokio's blocking pool so the UI never stalls.
    pub fn run_universe_lookalike_async(self) {
        let st = self.universe_state.get_untracked();
        let trimmed_query = st.query.trim().to_owned();
        if trimmed_query.is_empty() {
            self.reload_universe_from_lake();
            return;
        }
        if !can_open_duckdb_read_only(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
        ) {
            return;
        }

        let next_seq = self.universe_search_seq.with(|s| *s + 1);
        self.universe_search_seq.replace(next_seq);
        self.universe_search_running.set(true);

        let paths = self.paths.get_untracked();
        let sim_filter = st.to_similar_filter();
        let as_ticker = st.similar_as_ticker;
        let sel_cik = self.universe_selected_cik.get_untracked();

        run_blocking_result(
            move || {
                let db = match LakeDb::open_read_only_if_exists(&paths) {
                    Ok(Some(db)) => db,
                    Ok(None) => return Ok(None),
                    Err(err) => return Err(format!("open: {err:#}")),
                };
                let summary = db
                    .universe_summary()
                    .map_err(|e| format!("summary: {e:#}"))?;
                let started = Instant::now();
                let hits = db
                    .search_similar(&trimmed_query, as_ticker, &sim_filter)
                    .map_err(|e| format!("search: {e:#}"))?;
                let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let first_cik = sel_cik
                    .filter(|c| hits.iter().any(|h| h.company.cik == *c))
                    .or_else(|| hits.first().map(|h| h.company.cik.clone()));
                let details = first_cik
                    .as_deref()
                    .and_then(|c| db.load_drawer_details_by_cik(c, 5).ok().flatten());
                Ok(Some((summary, hits, elapsed_ms, details)))
            },
            move |res| {
                let cur_seq = self.universe_search_seq.get();
                if cur_seq != next_seq {
                    return;
                }
                self.universe_search_running.set(false);
                let l = self.lang.get_untracked();
                match res {
                    Ok(None) => {
                        self.universe_lake_present.set(false);
                        self.universe_summary.set(None);
                        self.universe_rows.set(Vec::new());
                        self.universe_similar_hits.set(Vec::new());
                        self.universe_similar_query_ms.set(None);
                        self.clear_universe_drawer_details();
                    }
                    Ok(Some((summary, hits, elapsed_ms, details))) => {
                        self.universe_lake_present.set(true);
                        self.universe_summary.set(Some(summary));
                        self.universe_similar_query_ms.set(Some(elapsed_ms));
                        let rows: Vec<UniverseRow> =
                            hits.iter().map(|h| h.company.clone()).collect();
                        self.universe_rows.set(rows.clone());
                        self.universe_similar_hits.set(hits);
                        self.universe_error.set(None);
                        if let Some(d) = details {
                            self.universe_selected_cik.set(Some(d.row.cik.clone()));
                            let idx = rows.iter().position(|r| r.cik == d.row.cik).unwrap_or(0);
                            self.universe_state.update(|s| s.cursor_index = idx);
                            self.apply_universe_drawer_details(d);
                        } else if let Some(first) = rows.first() {
                            self.universe_selected_cik.set(Some(first.cik.clone()));
                            self.universe_selected_row.set(Some(first.clone()));
                            self.universe_state.update(|s| s.cursor_index = 0);
                            self.clear_universe_drawer_details();
                        } else {
                            self.universe_selected_cik.set(None);
                            self.universe_selected_row.set(None);
                            self.clear_universe_drawer_details();
                        }
                    }
                    Err(err) => {
                        self.universe_similar_hits.set(Vec::new());
                        self.universe_similar_query_ms.set(None);
                        self.universe_rows.set(Vec::new());
                        self.clear_universe_drawer_details();
                        self.universe_error.set(Some(match l {
                            UiLang::Zh => format!("混合语义相似搜索失败: {err}"),
                            UiLang::En => format!("Hybrid similarity search failed: {err}"),
                        }));
                    }
                }
            },
        );
    }

    fn clear_universe_drawer_details(self) {
        self.universe_selected_annuals.set(Vec::new());
        self.universe_selected_peers.set(None);
    }

    fn apply_universe_drawer_details(self, details: UniverseDrawerDetails) {
        if self.universe_selected_cik.get_untracked().as_deref() != Some(details.row.cik.as_str()) {
            return;
        }
        self.universe_rows.update(|rows| {
            if let Some(existing) = rows.iter_mut().find(|r| r.cik == details.row.cik) {
                *existing = details.row.clone();
            }
        });
        self.universe_selected_row.set(Some(details.row));
        self.universe_selected_annuals.set(details.annuals);
        self.universe_selected_peers.set(details.peers);
    }

    fn sync_universe_drawer_selection(
        self,
        db: &LakeDb,
        snapshot_details: Option<UniverseDrawerDetails>,
    ) {
        let sel = self.universe_selected_cik.get_untracked();
        let target = self.universe_rows.with_untracked(|rows| {
            let idx = sel
                .as_ref()
                .and_then(|s| rows.iter().position(|r| r.cik == *s))
                .or_else(|| (!rows.is_empty()).then_some(0))?;
            Some((idx, rows[idx].clone()))
        });
        let Some((idx, target_row)) = target else {
            self.universe_state.update(|s| s.cursor_index = 0);
            self.universe_selected_cik.set(None);
            self.universe_selected_row.set(None);
            self.clear_universe_drawer_details();
            return;
        };
        let target_cik = target_row.cik.clone();
        self.universe_state.update(|s| s.cursor_index = idx);
        self.universe_selected_cik.set(Some(target_cik.clone()));
        self.universe_selected_row.set(Some(target_row));
        if let Some(details) = snapshot_details.filter(|d| d.row.cik == target_cik) {
            self.apply_universe_drawer_details(details);
        } else {
            self.clear_universe_drawer_details();
            match db.load_drawer_details_by_cik(&target_cik, 5) {
                Ok(Some(details)) => self.apply_universe_drawer_details(details),
                Ok(None) => self.clear_universe_drawer_details(),
                Err(err) => {
                    let l = self.lang.get_untracked();
                    self.clear_universe_drawer_details();
                    self.universe_error.set(Some(match l {
                        UiLang::Zh => format!("读取全市场湖仓详情失败: {err:#}"),
                        UiLang::En => format!("Failed to read Universe drawer details: {err:#}"),
                    }));
                }
            }
        }
    }

    pub fn select_universe_row(self, row: &UniverseRow) {
        self.universe_selected_cik.set(Some(row.cik.clone()));
        self.universe_selected_row.set(Some(row.clone()));
        self.clear_universe_drawer_details();
        let idx_opt = self
            .universe_rows
            .with_untracked(|rows| rows.iter().position(|r| r.cik == row.cik));
        if let Some(idx) = idx_opt {
            self.universe_state.update(|s| s.cursor_index = idx);
        }
        if self.universe_syncing.get_untracked() {
            return;
        }
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match LakeDb::open_read_only_if_exists(&paths) {
            Ok(Some(db)) => match db.load_drawer_details_by_cik(&row.cik, 5) {
                Ok(Some(details)) => self.apply_universe_drawer_details(details),
                Ok(None) => self.clear_universe_drawer_details(),
                Err(err) => {
                    self.clear_universe_drawer_details();
                    self.universe_error.set(Some(match l {
                        UiLang::Zh => format!("读取全市场湖仓详情失败: {err:#}"),
                        UiLang::En => format!("Failed to read Universe drawer details: {err:#}"),
                    }));
                }
            },
            Ok(None) => {}
            Err(err) => {
                self.clear_universe_drawer_details();
                self.universe_error.set(Some(match l {
                    UiLang::Zh => format!("打开 m3.duckdb 失败: {err:#}"),
                    UiLang::En => format!("Failed to open m3.duckdb: {err:#}"),
                }));
            }
        }
    }

    pub fn step_universe_cursor(self, delta: isize) {
        let next_row = self.universe_rows.with_untracked(|rows| {
            let len = rows.len();
            if len == 0 {
                return None;
            }
            let cur = self
                .universe_state
                .get_untracked()
                .cursor_index
                .min(len - 1);
            let next = (cur as isize + delta).clamp(0, (len - 1) as isize) as usize;
            rows.get(next).cloned()
        });
        if let Some(row) = next_row {
            self.select_universe_row(&row);
        }
    }

    pub fn set_universe_query(self, query: String) {
        self.universe_state.update(|st| {
            st.query = query;
            st.similar_as_ticker = false;
            st.cursor_index = 0;
        });
        self.reload_universe_from_lake();
    }

    pub fn apply_universe_preset(self, preset: UniversePreset) {
        self.universe_state.update(|st| st.apply_preset(preset));
        self.reload_universe_from_lake();
    }

    pub fn toggle_universe_sort(self, col: UniverseSortBy) {
        self.universe_state.update(|st| st.toggle_sort(col));
        self.reload_universe_from_lake();
    }

    pub fn set_universe_search_mode(self, mode: UniverseSearchMode) {
        self.universe_state.update(|st| st.set_search_mode(mode));
        self.reload_universe_from_lake();
    }

    pub fn find_similar_for_company(self, ticker: &str) {
        self.universe_state
            .update(|st| st.start_similar_for_ticker(ticker));
        self.universe_selected_cik.set(None);
        self.universe_selected_row.set(None);
        self.reload_universe_from_lake();
    }

    pub fn start_universe_sync(self, full_sec: bool) {
        let l = self.lang.get_untracked();
        if !can_start_duckdb_writer(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
            self.sync_running.get_untracked(),
        ) {
            if self.sync_running.get_untracked() || self.inbox_catching_up.get_untracked() {
                self.set_info_notice(
                    l.t(
                        "自选池或事件流正在后台同步中，请等待完成后再同步全美股 L0",
                        "Another sync is running; start US Universe L0 sync after it completes.",
                    )
                    .to_owned(),
                );
            }
            return;
        }
        self.universe_syncing.set(true);
        self.universe_sync_progress_detail.set(None);
        self.universe_sync_progress.set(Some(
            l.t(
                "正在启动全美股 L0 同步（SEC EDGAR Bulk + 质量分层）…",
                "Starting US Universe L0 sync (SEC EDGAR Bulk + quality tiering)…",
            )
            .to_owned(),
        ));
        self.set_info_notice(
            l.t(
                "正在后台同步全美股 L0 确定性湖仓（SEC companyfacts.zip + Top 400 报价）…",
                "Syncing US Universe L0 deterministic lakehouse (SEC companyfacts.zip + Top 400 quotes)…",
            )
            .to_owned(),
        );

        let root = self.paths.get_untracked().repo_root;
        run_streamed_blocking(
            move |tx| {
                let paths = WorkspacePaths::from_root(root);
                let opts = UniverseSyncOptions {
                    full_sec,
                    ..UniverseSyncOptions::default()
                };
                let res = universe::sync_universe_with_structured_progress(&paths, &opts, |prog| {
                    let _ = tx.send(UniverseSyncMsg::Progress(prog));
                })
                .map_err(|e| format!("{e:#}"));
                let _ = tx.send(UniverseSyncMsg::Finished(Box::new(res)));
            },
            move |msg| match msg {
                UniverseSyncMsg::Progress(prog) => {
                    let l = self.lang.get_untracked();
                    let was_lake_ready = self
                        .universe_sync_progress_detail
                        .with_untracked(|d| d.as_ref().is_some_and(|p| p.lake_ready));
                    let formatted = format_universe_sync_progress(&prog, l);
                    let became_ready = !was_lake_ready && prog.lake_ready;
                    self.universe_sync_progress.set(Some(formatted));
                    self.universe_sync_progress_detail.set(Some(prog));
                    if became_ready {
                        self.reload_universe_from_lake();
                    }
                }
                UniverseSyncMsg::Finished(res) => {
                    let res = *res;
                    self.universe_syncing.set(false);
                    self.universe_sync_progress.set(None);
                    self.universe_sync_progress_detail.set(None);
                    let l = self.lang.get_untracked();
                    match res {
                        Ok(report) => {
                            self.reload_universe_from_lake();
                            self.notice
                                .set(Some(Self::format_universe_sync_banner(&report, l)));
                        }
                        Err(err) => {
                            self.set_error_notice(match l {
                                UiLang::Zh => format!("全美股 L0 同步失败：{err}"),
                                UiLang::En => format!("US Universe L0 sync failed: {err}"),
                            });
                        }
                    }
                }
            },
            move || {
                self.universe_syncing.set(false);
                self.universe_sync_progress.set(None);
                self.universe_sync_progress_detail.set(None);
            },
        );
    }

    fn format_universe_sync_banner(report: &UniverseSyncReport, lang: UiLang) -> (String, bool) {
        let base = match lang {
            UiLang::Zh => format!(
                "全美股 L0 同步完成 · 解析 {} 家 / 质量池 {} 家 / 报价 {}/{} 家",
                report.parsed_companies,
                report.quality_pool_count,
                report.prices_succeeded,
                report.prices_attempted
            ),
            UiLang::En => format!(
                "US Universe L0 sync complete · parsed {} / quality {} / quotes {}/{}",
                report.parsed_companies,
                report.quality_pool_count,
                report.prices_succeeded,
                report.prices_attempted
            ),
        };
        match report.sic_warning.as_deref() {
            Some(warn) => (
                match lang {
                    UiLang::Zh => format!("{base}（SIC 提示: {warn}）"),
                    UiLang::En => format!("{base} (SIC note: {warn})"),
                },
                true,
            ),
            None => (base, false),
        }
    }

    pub fn add_universe_company_to_watch(self, row: &UniverseRow) {
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        let circle = match (row.sic, row.sic_description.as_deref()) {
            (Some(sic), Some(desc)) if !desc.trim().is_empty() => {
                Some(format!("{} / SIC {sic}", desc.trim()))
            }
            (None, Some(desc)) if !desc.trim().is_empty() => Some(desc.trim().to_owned()),
            (Some(sic), _) => Some(format!("SIC {sic}")),
            _ => None,
        };
        let verified_cik = (!row.cik.trim().is_empty()).then_some(row.cik.as_str());
        match store::add_or_update_company_with_cik(
            &paths,
            &row.ticker,
            Bucket::Watch,
            Some(&row.name),
            verified_cik,
            circle.as_deref(),
            None,
        ) {
            Ok(_) => {
                let lake_err =
                    if !self.universe_syncing.get_untracked() && paths.lake_db_path().exists() {
                        LakeDb::open(&paths)
                            .and_then(|db| db.refresh_watchlist_flags(&paths))
                            .err()
                            .map(|e| format!("{e:#}"))
                    } else {
                        None
                    };
                self.reload_from_disk();
                if !self.universe_syncing.get_untracked() {
                    self.reload_universe_from_lake();
                }
                if let Some(err) = lake_err {
                    self.set_error_notice(format!("{} ({}): {err}", row.ticker, row.name));
                } else {
                    self.set_info_notice(match l {
                        UiLang::Zh => format!("已将 {}（{}）加入 [WATCH]", row.ticker, row.name),
                        UiLang::En => format!("Added {} ({}) to [WATCH]", row.ticker, row.name),
                    });
                }
            }
            Err(err) => {
                self.set_error_notice(match l {
                    UiLang::Zh => format!("加入观察池失败 ({})：{err:#}", row.ticker),
                    UiLang::En => format!("Failed to add {} to Watch: {err:#}", row.ticker),
                });
            }
        }
    }

    pub fn dispatch_universe_company_research(self, row: &UniverseRow) {
        let in_wl = self.reports.with_untracked(|reps| {
            reps.iter()
                .any(|r| r.ticker.eq_ignore_ascii_case(&row.ticker))
        });
        if !in_wl {
            self.add_universe_company_to_watch(row);
        }
        let canonical = self.reports.with_untracked(|reps| {
            reps.iter()
                .find(|r| r.ticker.eq_ignore_ascii_case(&row.ticker))
                .map(|r| r.ticker.clone())
                .unwrap_or_else(|| row.ticker.clone())
        });
        self.open_company(&canonical, CompanyViewTab::DeepResearchAlan);
        self.start_deep_research(&canonical, None, "universe");
    }

    // ── Themes & Industry Research Studio ────────────────────────────────────

    pub fn reload_themes_from_disk(self) {
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match budget::load_themes(&paths) {
            Ok(themes) => {
                self.themes_file.set(themes);
                self.themes_error.set(None);
            }
            Err(err) => {
                self.themes_error.set(Some(format!(
                    "{}: {err:#}",
                    l.t(
                        "读取 workbench/themes.json 失败",
                        "Failed to read workbench/themes.json"
                    )
                )));
            }
        }
    }

    pub fn trigger_themes_sync(self) {
        self.open_tab(WorkspaceTab::Industry);
        self.start_themes_sync();
    }

    pub fn start_themes_sync(self) {
        let l = self.lang.get_untracked();
        if self.themes_syncing.get_untracked() || self.universe_syncing.get_untracked() {
            if self.universe_syncing.get_untracked() {
                self.set_info_notice(
                    l.t(
                        "全美股 L0 湖仓正在同步，请完成后再同步主题图谱",
                        "US L0 lakehouse is syncing; wait until it finishes to sync themes",
                    )
                    .to_owned(),
                );
            }
            return;
        }
        self.themes_syncing.set(true);
        self.themes_error.set(None);
        self.set_info_notice(
            l.t(
                "正在从 DuckDB 湖仓同步跨公司商业原型主题图谱…",
                "Syncing cross-company business archetype clusters from DuckDB…",
            )
            .to_owned(),
        );

        let root = self.paths.get_untracked().repo_root;
        run_blocking_result(
            move || {
                let paths = WorkspacePaths::from_root(root);
                match LakeDb::open_read_only_if_exists(&paths) {
                    Ok(Some(db)) => db
                        .sync_and_load_themes(&paths)
                        .map_err(|e| format!("{e:#}")),
                    Ok(None) => Err(l
                        .t(
                            "全美股湖仓尚未同步，请先同步全美股 L0",
                            "US lakehouse not synced yet — sync L0 first",
                        )
                        .to_owned()),
                    Err(e) => Err(format!(
                        "{}: {e:#}",
                        l.t("打开 m3.duckdb 失败", "Failed to open m3.duckdb")
                    )),
                }
            },
            move |res| {
                self.themes_syncing.set(false);
                let l = self.lang.get_untracked();
                match res {
                    Ok(themes) => {
                        let clusters = themes.clusters.len();
                        let indexed = themes.total_indexed_companies;
                        self.themes_file.set(themes);
                        self.themes_error.set(None);
                        self.set_info_notice(match l {
                            UiLang::Zh => format!(
                                "主题图谱已同步 · {clusters} 个商业原型簇 / 覆盖 {indexed} 家公司（保留人类批注）"
                            ),
                            UiLang::En => format!(
                                "Themes synced · {clusters} archetype clusters across {indexed} companies (human notes preserved)"
                            ),
                        });
                    }
                    Err(err) => {
                        self.themes_error.set(Some(err.clone()));
                        self.set_error_notice(format!(
                            "{}: {err}",
                            l.t("同步主题图谱失败", "Theme sync failed")
                        ));
                    }
                }
            },
        );
    }

    pub fn open_theme_member(self, ticker: &str) {
        self.open_inbox_signal_target(ticker);
    }

    pub fn reload_industries_from_disk(self) {
        let paths = self.paths.get_untracked();
        let list = industry::list_industries(&paths).unwrap_or_default();
        let cur = self.selected_industry_slug.get_untracked();
        let next_slug = if let Some(slug) = cur {
            if list.iter().any(|p| p.slug == slug) {
                Some(slug)
            } else {
                list.first().map(|p| p.slug.clone())
            }
        } else {
            list.first().map(|p| p.slug.clone())
        };
        self.industries.set(list);
        self.selected_industry_slug.set(next_slug);
        self.reload_selected_industry_context();
    }

    pub fn selected_industry(self) -> Option<IndustryProfile> {
        let slug = self.selected_industry_slug.get_untracked()?;
        self.industries
            .with_untracked(|list| list.iter().find(|p| p.slug == slug).cloned())
    }

    pub fn select_industry(self, slug: &str) {
        self.selected_industry_slug.set(Some(slug.to_owned()));
        self.industry_live_perspectives.set(BTreeMap::new());
        self.reload_selected_industry_context();
    }

    pub fn reload_selected_industry_context(self) {
        let Some(profile) = self.selected_industry() else {
            self.industry_peer_rows.set(Vec::new());
            self.industry_primer_md.set(None);
            self.industry_human_note_draft.set(String::new());
            return;
        };
        let paths = self.paths.get_untracked();
        self.industry_human_note_draft
            .set(profile.human_notes.clone());
        self.industry_anchor_low_draft.set(
            profile
                .multiple_low
                .map(|d| d.round_dp(1).normalize().to_string())
                .unwrap_or_default(),
        );
        self.industry_anchor_high_draft.set(
            profile
                .multiple_high
                .map(|d| d.round_dp(1).normalize().to_string())
                .unwrap_or_default(),
        );
        self.industry_primer_md
            .set(industry::load_industry_primer_md(&paths, &profile.slug));

        if !can_open_duckdb_read_only(
            self.universe_syncing.get_untracked(),
            self.inbox_catching_up.get_untracked(),
        ) {
            self.industry_peer_rows.set(Vec::new());
            return;
        }
        let rows = if let Ok(Some(db)) = LakeDb::open_read_only_if_exists(&paths) {
            db.load_universe_rows_by_tickers(&profile.bound_tickers)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.industry_peer_rows.set(rows);
    }

    pub fn open_or_create_industry_from_topic(self, raw_topic: &str) {
        let trimmed = raw_topic.trim();
        let l = self.lang.get_untracked();
        if trimmed.is_empty() {
            self.set_info_notice(
                l.t(
                    "请输入行业名称或关键词再发起研究",
                    "Enter an industry topic first",
                )
                .to_owned(),
            );
            return;
        }
        let paths = self.paths.get_untracked();
        match industry::create_or_open_from_topic(&paths, trimmed) {
            Ok(profile) => {
                self.industry_new_topic_input.set(String::new());
                self.selected_industry_slug.set(Some(profile.slug.clone()));
                self.reload_industries_from_disk();
                self.open_tab(WorkspaceTab::Industry);
                let n = profile.bound_tickers.len();
                self.set_info_notice(match l {
                    UiLang::Zh => format!(
                        "已打开行业研究「{}」（自动匹配 {n} 家代表公司），可点击启动 Alan 4 视角深研",
                        profile.title
                    ),
                    UiLang::En => format!(
                        "Opened Industry Study \"{}\" (matched {n} peer tickers). Ready for Alan 4-Lens Research.",
                        profile.title
                    ),
                });
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("创建行业研究失败", "Failed to create study")
                ));
            }
        }
    }

    pub fn open_or_create_industry_for_company(
        self,
        ticker: &str,
        company_name: &str,
        sic: Option<u32>,
        sic_desc: Option<&str>,
    ) {
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match industry::create_or_open_from_company(&paths, ticker, company_name, sic, sic_desc) {
            Ok(profile) => {
                self.selected_industry_slug.set(Some(profile.slug.clone()));
                self.reload_industries_from_disk();
                self.open_tab(WorkspaceTab::Industry);
                self.set_info_notice(match l {
                    UiLang::Zh => format!(
                        "已打开 {ticker} 所在行业「{}」（已绑定 {} 家代表公司）",
                        profile.title,
                        profile.bound_tickers.len()
                    ),
                    UiLang::En => format!(
                        "Opened industry \"{}\" for {ticker} ({} representative peers bound)",
                        profile.title,
                        profile.bound_tickers.len()
                    ),
                });
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("打开行业研究失败", "Failed to open industry study")
                ));
            }
        }
    }

    pub fn open_or_create_industry_from_theme(self, theme: &ThemeCluster) {
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match industry::create_or_open_from_theme(&paths, theme) {
            Ok(profile) => {
                self.selected_industry_slug.set(Some(profile.slug.clone()));
                self.reload_industries_from_disk();
                self.open_tab(WorkspaceTab::Industry);
                self.set_info_notice(match l {
                    UiLang::Zh => format!(
                        "已将主题「{}」升级为独立行业研究项目（{} 家代表公司）",
                        profile.title,
                        profile.bound_tickers.len()
                    ),
                    UiLang::En => format!(
                        "Promoted theme \"{}\" to Industry Study ({} bound companies)",
                        profile.title,
                        profile.bound_tickers.len()
                    ),
                });
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("转换主题失败", "Failed to promote theme")
                ));
            }
        }
    }

    pub fn set_selected_industry_status(self, status: IndustryStatus) {
        let Some(slug) = self.selected_industry_slug.get_untracked() else {
            return;
        };
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match industry::set_industry_status(&paths, &slug, status) {
            Ok(updated) => {
                self.reload_industries_from_disk();
                let msg = match (status, l) {
                    (IndustryStatus::InsideCircle, UiLang::Zh) => format!(
                        "「{}」已毕业进入【能力圈内】· 下次事件流追平起，该行业公司将按您的估值锚进入击球区雷达",
                        updated.title
                    ),
                    (IndustryStatus::InsideCircle, UiLang::En) => format!(
                        "\"{}\" graduated to Inside Circle · Bound companies now qualify for Circle-gated Strike signals",
                        updated.title
                    ),
                    (IndustryStatus::TooHard, UiLang::Zh) => format!(
                        "已将「{}」标为 Too Hard · 系统将屏蔽该行业个股的击球区与量化候选提醒",
                        updated.title
                    ),
                    (IndustryStatus::TooHard, UiLang::En) => format!(
                        "Marked \"{}\" as Too Hard · Strike and discovery signals suppressed for this industry",
                        updated.title
                    ),
                    (IndustryStatus::Studying, UiLang::Zh) => {
                        format!("已将「{}」恢复为「学习中」状态", updated.title)
                    }
                    (IndustryStatus::Studying, UiLang::En) => {
                        format!("Set \"{}\" back to Studying", updated.title)
                    }
                };
                self.set_info_notice(msg);
            }
            Err(err) => {
                self.set_error_notice(format!("{err:#}"));
            }
        }
    }

    pub fn set_selected_industry_anchor(self, low: Option<Decimal>, high: Option<Decimal>) {
        let Some(slug) = self.selected_industry_slug.get_untracked() else {
            return;
        };
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match industry::set_industry_valuation_anchor(&paths, &slug, low, high) {
            Ok(updated) => {
                self.reload_industries_from_disk();
                let lbl = updated
                    .valuation_anchor_label()
                    .unwrap_or_else(|| l.t("已清除", "Cleared").to_owned());
                self.set_info_notice(match l {
                    UiLang::Zh => format!("已保存「{}」人类估值锚：{lbl}", updated.title),
                    UiLang::En => {
                        format!("Saved valuation anchor for \"{}\": {lbl}", updated.title)
                    }
                });
            }
            Err(err) => {
                self.set_error_notice(format!("{err:#}"));
            }
        }
    }

    pub fn add_ticker_to_selected_industry(self, raw_ticker: &str) {
        let Some(slug) = self.selected_industry_slug.get_untracked() else {
            return;
        };
        let trimmed = raw_ticker.trim();
        if trimmed.is_empty() {
            return;
        }
        let paths = self.paths.get_untracked();
        match industry::add_bound_ticker(&paths, &slug, trimmed) {
            Ok(_) => {
                self.industry_add_ticker_input.set(String::new());
                self.reload_industries_from_disk();
            }
            Err(err) => self.set_error_notice(format!("{err:#}")),
        }
    }

    pub fn remove_ticker_from_selected_industry(self, ticker: &str) {
        let Some(slug) = self.selected_industry_slug.get_untracked() else {
            return;
        };
        let paths = self.paths.get_untracked();
        match industry::remove_bound_ticker(&paths, &slug, ticker) {
            Ok(_) => self.reload_industries_from_disk(),
            Err(err) => self.set_error_notice(format!("{err:#}")),
        }
    }

    pub fn toggle_selected_industry_feynman(self, idx: usize) {
        let Some(slug) = self.selected_industry_slug.get_untracked() else {
            return;
        };
        let paths = self.paths.get_untracked();
        match industry::toggle_feynman_check(&paths, &slug, idx) {
            Ok(_) => self.reload_industries_from_disk(),
            Err(err) => self.set_error_notice(format!("{err:#}")),
        }
    }

    pub fn save_selected_industry_human_notes(self, note: &str) {
        let Some(slug) = self.selected_industry_slug.get_untracked() else {
            return;
        };
        let paths = self.paths.get_untracked();
        let l = self.lang.get_untracked();
        match industry::save_human_notes(&paths, &slug, note) {
            Ok(_) => {
                self.reload_industries_from_disk();
                self.set_info_notice(
                    l.t(
                        "已保存人类行业笔记（同步受保护的 ## Human Notes）",
                        "Saved human industry notes (protected ## Human Notes synced)",
                    )
                    .to_owned(),
                );
            }
            Err(err) => self.set_error_notice(format!("{err:#}")),
        }
    }

    pub fn start_industry_deep_research(self) {
        let l = self.lang.get_untracked();
        if self.industry_research_running.get_untracked()
            || self.deep_research_running.get_untracked()
        {
            self.set_info_notice(
                l.t(
                    "已有 Alan 深研任务正在运行，请稍候",
                    "Another Alan research run is active",
                )
                .to_owned(),
            );
            return;
        }
        let Some(profile) = self.selected_industry() else {
            return;
        };
        if self.universe_syncing.get_untracked() || self.inbox_catching_up.get_untracked() {
            self.set_info_notice(
                l.t(
                    "后台湖仓同步中，请稍候完成后再启动行业深研",
                    "Wait for lakehouse sync to finish before starting industry research",
                )
                .to_owned(),
            );
            return;
        }

        self.industry_research_running.set(true);
        self.industry_research_slug.set(Some(profile.slug.clone()));
        self.industry_live_perspectives.set(BTreeMap::new());
        self.industry_research_progress.set(Some(match l {
            UiLang::Zh => format!("正在准备「{}」4 视角行业深研…", profile.title),
            UiLang::En => format!("Preparing 4-lens research for \"{}\"…", profile.title),
        }));

        let root = self.paths.get_untracked().repo_root;
        let slug = profile.slug.clone();
        run_streamed_blocking(
            move |tokio_tx| {
                let paths = WorkspacePaths::from_root(root);
                let (std_tx, std_rx) = std::sync::mpsc::channel::<IndustryResearchEvent>();
                let fwd_tx = tokio_tx.clone();
                let fwd = std::thread::spawn(move || {
                    while let Ok(ev) = std_rx.recv() {
                        if fwd_tx.send(ev).is_err() {
                            break;
                        }
                    }
                });
                if let Err(err) = industry_research::run_industry_deep_research(
                    &paths,
                    &slug,
                    None,
                    Some(std_tx.clone()),
                ) {
                    let _ = std_tx.send(IndustryResearchEvent::Failed {
                        run_id: String::new(),
                        slug,
                        error: format!("{err:#}"),
                    });
                }
                drop(std_tx);
                let _ = fwd.join();
            },
            move |ev| self.apply_industry_research_event(ev),
            move || {
                self.industry_research_running.set(false);
            },
        );
    }

    fn apply_industry_research_event(self, ev: IndustryResearchEvent) {
        let l = self.lang.get_untracked();
        match ev {
            IndustryResearchEvent::Started { slug, .. } => {
                self.industry_research_progress.set(Some(match l {
                    UiLang::Zh => {
                        format!("正在检查/拉取「{slug}」代表公司 10-K 与启动 4 视角 Alan…")
                    }
                    UiLang::En => {
                        format!("Bootstrapping 10-K filings & Alan 4-lens for \"{slug}\"…")
                    }
                }));
            }
            IndustryResearchEvent::Fetching10K { ticker, idx, total } => {
                self.industry_research_progress.set(Some(match l {
                    UiLang::Zh => {
                        format!("正在拉取代表公司 10-K（{idx}/{total}）：{ticker} (0 LLM)…")
                    }
                    UiLang::En => format!("Fetching peer 10-K ({idx}/{total}): {ticker} (0 LLM)…"),
                }));
            }
            IndustryResearchEvent::PerspectiveStarted { perspective, label } => {
                self.industry_live_perspectives.update(|map| {
                    map.entry(perspective).or_default();
                });
                self.industry_research_progress.set(Some(match l {
                    UiLang::Zh => format!("Alan 视角运行中：{label}…"),
                    UiLang::En => format!("Alan lens running: {label}…"),
                }));
            }
            IndustryResearchEvent::PerspectiveDelta { perspective, chunk } => {
                self.industry_live_perspectives.update(|map| {
                    map.entry(perspective).or_default().push_str(&chunk);
                });
            }
            IndustryResearchEvent::ToolStarted { name, title, .. } => {
                self.industry_research_progress
                    .set(Some(format!("{name}: {title}")));
            }
            IndustryResearchEvent::PerspectiveFinished { .. } => {}
            IndustryResearchEvent::Completed(outcome) => {
                self.industry_research_running.set(false);
                self.industry_research_progress.set(None);
                let cost = outcome.cost_usd_str;
                self.set_info_notice(match l {
                    UiLang::Zh => format!(
                        "行业深研完成（拉取 {} 家 10-K · {} in / {} out · ${cost}）· 已写入活手册",
                        outcome.sec_10k_fetched_count, outcome.input_tokens, outcome.output_tokens,
                    ),
                    UiLang::En => format!(
                        "Industry research complete ({} 10-Ks fetched · {} in / {} out · ${cost})",
                        outcome.sec_10k_fetched_count, outcome.input_tokens, outcome.output_tokens,
                    ),
                });
                self.reload_industries_from_disk();
                self.reload_budget_from_lake();
            }
            IndustryResearchEvent::Failed { error, .. } => {
                self.industry_research_running.set(false);
                self.industry_research_progress.set(None);
                self.set_error_notice(error);
            }
        }
    }

    pub fn start_industry_batch_10k_fetch(self) {
        let l = self.lang.get_untracked();
        if self.industry_batch_10k_running.get_untracked()
            || self.universe_syncing.get_untracked()
            || self.inbox_catching_up.get_untracked()
        {
            return;
        }
        if self.selected_industry().is_none() {
            return;
        }
        let targets: Vec<(String, String)> = self.industry_peer_rows.with_untracked(|rows| {
            rows.iter()
                .take(5)
                .map(|r| (r.ticker.clone(), r.cik.clone()))
                .collect()
        });
        if targets.is_empty() {
            self.set_info_notice(
                l.t(
                    "代表公司尚未在本地 L0 湖仓找到 CIK，请先同步全美股 L0",
                    "Bound companies not yet in L0 lakehouse — sync L0 first",
                )
                .to_owned(),
            );
            return;
        }
        self.industry_batch_10k_running.set(true);
        let count = targets.len();
        self.set_info_notice(match l {
            UiLang::Zh => {
                format!("正在后台拉取 {count} 家代表公司最近 SEC 10-K 原文（0 Flagship Token）…")
            }
            UiLang::En => {
                format!("Fetching latest SEC 10-K Item 1/1A for {count} peers (0 Flagship Tokens)…")
            }
        });

        let root = self.paths.get_untracked().repo_root;
        run_blocking_result(
            move || {
                let paths = WorkspacePaths::from_root(root);
                let mut ok_tickers = Vec::new();
                let mut total_chunks = 0usize;
                for (ticker, cik) in targets {
                    if let Ok(rep) = events::fetch_company_10k_profile(&paths, &ticker, &cik) {
                        ok_tickers.push(rep.ticker);
                        total_chunks += rep.chunks_indexed;
                    }
                }
                (ok_tickers, total_chunks)
            },
            move |(ok_tickers, chunks)| {
                self.industry_batch_10k_running.set(false);
                let l = self.lang.get_untracked();
                let tickers = ok_tickers.join(", ");
                self.set_info_notice(match l {
                    UiLang::Zh => {
                        format!("代表公司 10-K 原文同步完成：{tickers}（新增 {chunks} 条片段）")
                    }
                    UiLang::En => format!("Fetched 10-K profiles for {tickers} (+{chunks} chunks)"),
                });
                self.reload_selected_industry_context();
            },
        );
    }

    // ── Deep Research & EmbeddedAlan ─────────────────────────────────────────

    pub fn run_deep_research_selected(self) {
        let ticker = self.selected_ticker.get_untracked();
        if ticker.trim().is_empty() {
            let l = self.lang.get_untracked();
            self.set_info_notice(
                l.t(
                    "当前能力圈为空，请先从「全市场雷达」或「简报收件箱」添加标的",
                    "Your Circle is empty — add a company from Universe Radar or Briefing Inbox first",
                )
                .to_owned(),
            );
            return;
        }
        self.open_company(&ticker, CompanyViewTab::DeepResearchAlan);
        self.start_deep_research(&ticker, None, "menu");
    }

    pub fn start_deep_research(self, ticker: &str, focus: Option<String>, trigger: &str) {
        let upper = ticker.trim().to_ascii_uppercase();
        if upper.is_empty() {
            return;
        }
        let l = self.lang.get_untracked();
        if self.deep_research_running.get_untracked() {
            self.set_info_notice(
                l.t(
                    "Deep Research 任务正在执行中，请等待当前运行完成",
                    "Deep Research is already running; wait for completion.",
                )
                .to_owned(),
            );
            return;
        }
        if self.universe_syncing.get_untracked() {
            self.set_info_notice(
                l.t(
                    "全美股 L0 湖仓正在同步，请完成后再发起 Deep Research",
                    "US universe L0 lakehouse is syncing; start Deep Research after it finishes.",
                )
                .to_owned(),
            );
            return;
        }

        self.deep_research_running.set(true);
        self.deep_research_ticker.set(Some(upper.clone()));
        self.deep_research_run_id.set(None);
        self.deep_research_budget_blocked.set(None);
        self.deep_research_perspectives
            .set(default_deep_research_perspectives());
        self.deep_research_status_note.set(Some(match l {
            UiLang::Zh => format!("正在准备 {upper} · 4 视角并行深研…"),
            UiLang::En => format!("Preparing {upper} · 4-perspective Deep Research…"),
        }));

        let root = self.paths.get_untracked().repo_root;
        let req = DeepResearchRequest {
            ticker: upper,
            focus,
            trigger: trigger.to_owned(),
        };

        run_streamed_blocking(
            move |tokio_tx| {
                let paths = WorkspacePaths::from_root(root);
                let (std_tx, std_rx) = std::sync::mpsc::channel::<DeepResearchEvent>();
                let fwd_tx = tokio_tx.clone();
                let fwd = std::thread::spawn(move || {
                    while let Ok(ev) = std_rx.recv() {
                        if fwd_tx.send(ev).is_err() {
                            break;
                        }
                    }
                });
                if let Err(err) = research::run_deep_research(&paths, &req, Some(std_tx.clone())) {
                    let _ = std_tx.send(DeepResearchEvent::Failed {
                        run_id: String::new(),
                        ticker: req.ticker,
                        error: format!("{err:#}"),
                    });
                }
                drop(std_tx);
                let _ = fwd.join();
            },
            move |ev| {
                let l = self.lang.get_untracked();
                let mut running = self.deep_research_running.get_untracked();
                let mut run_id = self.deep_research_run_id.get_untracked();
                let mut status_note = self.deep_research_status_note.get_untracked();
                let mut budget_blocked = self.deep_research_budget_blocked.get_untracked();
                let mut perspectives = self.deep_research_perspectives.get_untracked();
                let mut last_result = self.deep_research_last_result.get_untracked();
                let mut tape = self.agent_tape.get_untracked();

                let (reload_ctx, banner) = apply_deep_research_event_compat(
                    &mut running,
                    &mut run_id,
                    &mut status_note,
                    &mut budget_blocked,
                    &mut perspectives,
                    &mut last_result,
                    &mut tape,
                    &ev,
                    l,
                );

                self.deep_research_running.set(running);
                self.deep_research_run_id.set(run_id);
                self.deep_research_status_note.set(status_note);
                self.deep_research_budget_blocked.set(budget_blocked);
                self.deep_research_perspectives.set(perspectives);
                self.deep_research_last_result.set(last_result);
                self.agent_tape.set(tape);

                if let Some(b) = banner {
                    self.notice.set(Some(b));
                }
                if reload_ctx {
                    let target = self
                        .deep_research_ticker
                        .get_untracked()
                        .unwrap_or_else(|| self.selected_ticker.get_untracked());
                    self.reload_research_desk_context(&target);
                }
            },
            move || {
                self.deep_research_running.set(false);
            },
        );
    }

    fn ensure_alan_handle(self) -> M3InterrogatorHandle {
        if let Some(existing) = self.alan_handle.with(|opt| opt.clone()) {
            existing.refresh_fs();
            return existing;
        }
        let notify = Arc::new(Notify::new());
        let handle = M3InterrogatorHandle::start_shared_credentials_notified(
            self.paths.get_untracked(),
            Arc::clone(&notify),
        );
        self.alan_handle.replace(Some(handle.clone()));
        spawn_notify_listener(notify, move || {
            self.drain_alan_events();
        });
        handle
    }

    fn next_agent_seq(self) -> usize {
        let cur = self.agent_seq.get();
        self.agent_seq.replace(cur + 1);
        cur
    }

    fn append_tape(
        self,
        call_id: Option<String>,
        kind: TuringTapeKind,
        summary: String,
        detail: Option<String>,
        status: String,
    ) {
        self.agent_tape.update(|tape| {
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id,
                kind,
                summary,
                detail,
                status,
            });
        });
    }

    pub fn submit_alan_question(
        self,
        ticker: &str,
        name: &str,
        bucket: Bucket,
        raw_question: String,
    ) {
        let question = raw_question.trim().to_owned();
        if question.is_empty() {
            return;
        }
        let l = self.lang.get_untracked();
        self.agent_input_draft.set(String::new());
        self.alan_reply_target.set(AlanReplyTarget::Company);
        self.agent_session_ticker.set(Some(ticker.to_owned()));
        self.agent_streaming.set(true);
        self.agent_status_hint.set(Some(
            l.t("正在连接 Alan…", "Connecting to Alan…").to_owned(),
        ));

        let short_q: String = question.chars().take(72).collect();
        self.append_tape(
            None,
            TuringTapeKind::Input,
            format!("{ticker} · {short_q}"),
            Some(question.clone()),
            "sent".to_owned(),
        );

        let user_seq = self.next_agent_seq();
        let asst_seq = self.next_agent_seq();
        self.agent_messages.update(|msgs| {
            msgs.push(AgentUiMessage {
                id: format!("user-{user_seq}"),
                role: AgentMessageRole::User,
                raw_text: question.clone(),
                text: question.clone(),
                tokens: Vec::new(),
                streaming: false,
            });
            msgs.push(AgentUiMessage {
                id: format!("assistant-{asst_seq}"),
                role: AgentMessageRole::Assistant,
                raw_text: String::new(),
                text: String::new(),
                tokens: Vec::new(),
                streaming: true,
            });
        });

        let handle = self.ensure_alan_handle();
        handle.submit(ticker, name, bucket, question);
    }

    pub fn start_feynman_interrogation(self, ticker: &str, name: &str, bucket: Bucket) {
        let l = self.lang.get_untracked();
        let starter = match l {
            UiLang::Zh => format!(
                "请使用 m3_report 与 m3_dossier 获取 {ticker}（{name}）的确定性财务报告与现有审讯档案，指出当前估值与护城河中最脆弱的核心假设，并向我发起一轮费曼审讯追问。"
            ),
            UiLang::En => format!(
                "Please use m3_report and m3_dossier to fetch the deterministic financial report and existing interrogation dossier for {ticker} ({name}), identify the most fragile assumptions in current valuation and moat, and initiate a round of Feynman interrogation."
            ),
        };
        self.submit_alan_question(ticker, name, bucket, starter);
    }

    pub fn submit_industry_alan_question(self, raw_question: &str) {
        let question = raw_question.trim().to_owned();
        if question.is_empty() {
            return;
        }
        let Some(profile) = self.selected_industry() else {
            return;
        };
        let primer = self.industry_primer_md.get_untracked().unwrap_or_default();
        let full_prompt = compose_industry_tutor_turn(&profile, &primer, &question);
        let l = self.lang.get_untracked();

        self.industry_alan_input.set(String::new());
        self.alan_reply_target
            .set(AlanReplyTarget::Industry(profile.slug.clone()));
        self.industry_alan_streaming.set(true);
        self.industry_alan_status_hint.set(Some(
            l.t("Alan 行业导师思考中…", "Alan industry tutor thinking…")
                .to_owned(),
        ));

        let user_seq = self.next_agent_seq();
        let asst_seq = self.next_agent_seq();
        self.industry_alan_messages.update(|msgs| {
            msgs.push(AgentUiMessage {
                id: format!("ind-u-{user_seq}"),
                role: AgentMessageRole::User,
                raw_text: question.clone(),
                text: question.clone(),
                tokens: Vec::new(),
                streaming: false,
            });
            msgs.push(AgentUiMessage {
                id: format!("ind-a-{asst_seq}"),
                role: AgentMessageRole::Assistant,
                raw_text: String::new(),
                text: String::new(),
                tokens: Vec::new(),
                streaming: true,
            });
        });

        let handle = self.ensure_alan_handle();
        handle.submit_prompt(question, full_prompt);
    }

    pub fn trigger_industry_feynman_quiz(self) {
        let Some(profile) = self.selected_industry() else {
            return;
        };
        let primer = self.industry_primer_md.get_untracked().unwrap_or_default();
        let full_prompt = compose_feynman_quiz_turn(&profile, &primer);
        let l = self.lang.get_untracked();
        let display = match l {
            UiLang::Zh => format!("对「{}」发起费曼通关自测（4 道尖锐追问）", profile.title),
            UiLang::En => format!(
                "Start Feynman Mastery Quiz for \"{}\" (4 sharp questions)",
                profile.title
            ),
        };

        self.alan_reply_target
            .set(AlanReplyTarget::Industry(profile.slug.clone()));
        self.industry_alan_streaming.set(true);
        self.industry_alan_status_hint.set(Some(
            l.t("正在生成费曼通关考题…", "Generating Feynman quiz…")
                .to_owned(),
        ));

        let user_seq = self.next_agent_seq();
        let asst_seq = self.next_agent_seq();
        self.industry_alan_messages.update(|msgs| {
            msgs.push(AgentUiMessage {
                id: format!("ind-fq-u-{user_seq}"),
                role: AgentMessageRole::User,
                raw_text: display.clone(),
                text: display.clone(),
                tokens: Vec::new(),
                streaming: false,
            });
            msgs.push(AgentUiMessage {
                id: format!("ind-fq-a-{asst_seq}"),
                role: AgentMessageRole::Assistant,
                raw_text: String::new(),
                text: String::new(),
                tokens: Vec::new(),
                streaming: true,
            });
        });

        let handle = self.ensure_alan_handle();
        handle.submit_prompt(display, full_prompt);
    }

    pub fn interrupt_alan(self) {
        self.alan_handle.with(|opt| {
            if let Some(h) = opt {
                h.interrupt();
            }
        });
        let l = self.lang.get_untracked();
        self.agent_streaming.set(false);
        self.industry_alan_streaming.set(false);
        self.agent_status_hint.set(Some(
            l.t("已中断当前审讯", "Interrogation interrupted")
                .to_owned(),
        ));
        self.agent_messages.update(|msgs| {
            if let Some(last) = msgs
                .iter_mut()
                .rev()
                .find(|m| m.role == AgentMessageRole::Assistant)
            {
                last.streaming = false;
            }
        });
    }

    pub fn reset_alan_session(self) {
        self.alan_handle.with(|opt| {
            if let Some(h) = opt {
                h.reset();
            }
        });
        self.agent_messages.set(Vec::new());
        self.industry_alan_messages.set(Vec::new());
        self.agent_tape.set(Vec::new());
        self.agent_expanded_evidence.set(None);
        self.agent_expanded_tape.set(None);
        self.agent_streaming.set(false);
        self.industry_alan_streaming.set(false);
        self.agent_status_hint.set(None);
        self.industry_alan_status_hint.set(None);
    }

    fn drain_alan_events(self) {
        let events = self
            .alan_handle
            .with(|opt| opt.as_ref().map(|h| h.drain_events()).unwrap_or_default());
        if events.is_empty() {
            return;
        }
        let paths = self.paths.get_untracked();
        let focus_ticker = self
            .agent_session_ticker
            .get_untracked()
            .unwrap_or_else(|| self.selected_ticker.get_untracked());
        let reports = self.reports.get_untracked();
        let mut dossier_map = HashMap::new();
        for rep in &reports {
            if rep.ticker == focus_ticker {
                dossier_map.insert(rep.ticker.clone(), self.selected_dossier.get_untracked());
            } else if let Ok(d) = store::load_dossier(&paths, &rep.ticker) {
                dossier_map.insert(rep.ticker.clone(), d);
            }
        }

        let target = self.alan_reply_target.get_untracked();
        let l = self.lang.get_untracked();
        let (mut streaming, mut status_hint, mut messages) = match target {
            AlanReplyTarget::Company => (
                self.agent_streaming.get_untracked(),
                self.agent_status_hint.get_untracked(),
                self.agent_messages.get_untracked(),
            ),
            AlanReplyTarget::Industry(_) => (
                self.industry_alan_streaming.get_untracked(),
                self.industry_alan_status_hint.get_untracked(),
                self.industry_alan_messages.get_untracked(),
            ),
        };
        let mut tape = self.agent_tape.get_untracked();
        let mut seq = self.agent_seq.get();
        let mut reload_disk = false;

        for ev in &events {
            if apply_interrogator_event(
                &mut streaming,
                &mut status_hint,
                &mut messages,
                &mut tape,
                &mut seq,
                ev,
                &focus_ticker,
                &reports,
                &dossier_map,
                l,
            ) {
                reload_disk = true;
            }
        }

        self.agent_seq.replace(seq);
        self.agent_tape.set(tape);
        match target {
            AlanReplyTarget::Company => {
                self.agent_streaming.set(streaming);
                self.agent_status_hint.set(status_hint);
                self.agent_messages.set(messages);
            }
            AlanReplyTarget::Industry(_) => {
                self.industry_alan_streaming.set(streaming);
                self.industry_alan_status_hint.set(status_hint);
                self.industry_alan_messages.set(messages);
            }
        }
        if reload_disk {
            self.reload_from_disk();
        }
    }

    // ── Software Updates ─────────────────────────────────────────────────────

    pub fn start_update_check(self, notify_when_up_to_date: bool) {
        let l = self.lang.get_untracked();
        if self.update_applying.get_untracked() {
            if notify_when_up_to_date {
                self.set_info_notice(
                    l.t(
                        "正在安装更新，请稍候…",
                        "Update install already in progress…",
                    )
                    .to_owned(),
                );
            }
            return;
        }
        let target_channel = self.update_channel.get_untracked();
        if self.update_checking.get_untracked()
            && self.update_check_channel.get_untracked() == Some(target_channel)
        {
            if notify_when_up_to_date {
                self.set_info_notice(
                    l.t("正在检查更新，请稍候…", "Update check already in progress…")
                        .to_owned(),
                );
            }
            return;
        }
        let req_seq = self.update_check_seq.get_untracked().wrapping_add(1);
        self.update_check_seq.set(req_seq);
        self.update_check_channel.set(Some(target_channel));
        self.update_checking.set(true);
        if notify_when_up_to_date {
            self.set_info_notice(match l {
                UiLang::Zh => format!("正在检查 {} 通道更新…", target_channel.as_str()),
                UiLang::En => format!("Checking {} channel for updates…", target_channel.as_str()),
            });
        }
        run_blocking_or_err(
            move || {
                update::run_update(&UpdateOptions {
                    check_only: true,
                    channel: Some(target_channel),
                    ..Default::default()
                })
                .map_err(|e| format!("{e:#}"))
            },
            move |outcome: Result<Result<UpdateReport, String>, String>| {
                self.apply_update_check_outcome(
                    req_seq,
                    target_channel,
                    notify_when_up_to_date,
                    outcome,
                );
            },
        );
    }

    pub(crate) fn apply_update_check_outcome(
        self,
        req_seq: u64,
        target_channel: ReleaseChannel,
        notify_when_up_to_date: bool,
        outcome: Result<Result<UpdateReport, String>, String>,
    ) {
        if self.update_check_seq.get_untracked() != req_seq {
            return;
        }
        self.update_checking.set(false);
        self.update_check_channel.set(None);
        let res = match outcome {
            Ok(inner) => inner,
            Err(worker_err) => Err(worker_err),
        };
        let l = self.lang.get_untracked();
        match res {
            Ok(report) => {
                if self.update_channel.get_untracked() != target_channel {
                    return;
                }
                if let Ok(ch) = ReleaseChannel::parse(&report.channel) {
                    self.update_channel.set(ch);
                }
                if report.update_available {
                    let ch = ReleaseChannel::parse(&report.channel)
                        .unwrap_or_else(|_| self.update_channel.get_untracked());
                    self.update_notice.set(Some(PassiveUpdateNotice {
                        latest_version: report.target_version.clone(),
                        channel: ch,
                        tag: report.tag.clone(),
                        is_channel_switch: report.previous_channel.is_some(),
                        release_url: report.release_url.clone(),
                    }));
                    self.set_info_notice(match l {
                        UiLang::Zh => format!(
                            "发现可用更新 v{}（{}），点击更新按钮立即安装",
                            report.target_version, report.channel
                        ),
                        UiLang::En => format!(
                            "Update available: v{} ({}) — click Update to install",
                            report.target_version, report.channel
                        ),
                    });
                } else {
                    self.update_notice.set(None);
                    if notify_when_up_to_date {
                        self.set_info_notice(match l {
                            UiLang::Zh => format!(
                                "当前已是 {} 通道最新版本 (v{})",
                                report.channel, report.current_version
                            ),
                            UiLang::En => format!(
                                "Up to date on {} channel (v{})",
                                report.channel, report.current_version
                            ),
                        });
                    }
                }
            }
            Err(err) => {
                if notify_when_up_to_date {
                    self.set_error_notice(format!(
                        "{}: {err}",
                        l.t("检查更新失败", "Update check failed")
                    ));
                }
            }
        }
    }

    pub fn start_update_apply(self) {
        let l = self.lang.get_untracked();
        if self.update_applying.get_untracked() {
            return;
        }
        if self.update_checking.get_untracked() {
            self.set_info_notice(
                l.t("正在检查更新，请稍候…", "Checking for updates…")
                    .to_owned(),
            );
            return;
        }
        let next_seq = self.update_check_seq.get_untracked().wrapping_add(1);
        self.update_check_seq.set(next_seq);
        let target_channel = self.update_channel.get_untracked();
        let initial = update::UpdateProgress::for_stage(update::UpdateStage::Resolving, 0, None);
        self.update_applying.set(true);
        self.update_progress
            .set(Some(crate::format::format_update_progress(&initial, l)));
        self.update_progress_detail.set(Some(initial));
        if self
            .notice
            .with_untracked(|n| n.as_ref().is_some_and(|(_, err)| !*err))
        {
            self.clear_notice();
        }

        enum UpdateApplyEvent {
            Progress(update::UpdateProgress),
            Finished(Box<Result<UpdateReport, String>>),
        }

        run_streamed_blocking(
            move |tx| {
                let res = update::run_update_with_progress(
                    &UpdateOptions {
                        check_only: false,
                        channel: Some(target_channel),
                        ..Default::default()
                    },
                    |progress| {
                        let _ = tx.send(UpdateApplyEvent::Progress(progress));
                    },
                )
                .map_err(|e| format!("{e:#}"));
                let _ = tx.send(UpdateApplyEvent::Finished(Box::new(res)));
            },
            move |ev| match ev {
                UpdateApplyEvent::Progress(p) => {
                    let cur_lang = self.lang.get_untracked();
                    self.update_progress
                        .set(Some(crate::format::format_update_progress(&p, cur_lang)));
                    self.update_progress_detail.set(Some(p));
                }
                UpdateApplyEvent::Finished(res) => {
                    let res = *res;
                    self.update_applying.set(false);
                    self.update_progress.set(None);
                    self.update_progress_detail.set(None);
                    let l = self.lang.get_untracked();
                    match res {
                        Ok(report) => {
                            if let Ok(ch) = ReleaseChannel::parse(&report.channel) {
                                self.update_channel.set(ch);
                            }
                            self.update_notice.set(None);
                            if report.updated {
                                self.update_applied_version
                                    .set(Some(report.target_version.clone()));
                                self.set_info_notice(match l {
                                    UiLang::Zh => format!(
                                        "已安装 M3 v{}（{}），点击「重启」以生效",
                                        report.target_version, report.channel
                                    ),
                                    UiLang::En => format!(
                                        "Installed M3 v{} ({}) — click Restart to apply",
                                        report.target_version, report.channel
                                    ),
                                });
                            } else {
                                self.set_info_notice(match l {
                                    UiLang::Zh => {
                                        format!("当前已是最新版本 (v{})", report.current_version)
                                    }
                                    UiLang::En => format!(
                                        "Already running the latest version (v{})",
                                        report.current_version
                                    ),
                                });
                            }
                        }
                        Err(err) => {
                            self.set_error_notice(format!(
                                "{}: {err}",
                                l.t("安装更新失败", "Update install failed")
                            ));
                        }
                    }
                }
            },
            move || {
                if self.update_applying.get_untracked() {
                    self.update_applying.set(false);
                    self.update_progress.set(None);
                    self.update_progress_detail.set(None);
                }
            },
        );
    }

    pub fn switch_update_channel(self, channel: ReleaseChannel) {
        let l = self.lang.get_untracked();
        if self.update_applying.get_untracked() {
            self.set_info_notice(
                l.t(
                    "正在安装更新，请等待完成后再切换通道…",
                    "Update install in progress — please wait before switching channels…",
                )
                .to_owned(),
            );
            return;
        }
        match update::channel::write_configured_channel(channel) {
            Ok(()) => {
                let next_seq = self.update_check_seq.get_untracked().wrapping_add(1);
                self.update_check_seq.set(next_seq);
                self.update_checking.set(false);
                self.update_check_channel.set(None);
                self.update_channel.set(channel);
                self.update_notice.set(None);
                self.start_update_check(true);
            }
            Err(err) => {
                self.set_error_notice(format!(
                    "{}: {err:#}",
                    l.t("切换更新通道失败", "Failed to switch channel")
                ));
            }
        }
    }

    /// Collects Circle tags & counts from current reports.
    pub fn circle_tags(self) -> Vec<(String, usize)> {
        self.reports.with(|reps| collect_circle_tags(reps))
    }

    /// Builds `PaletteItem` entries for the `⌘K` Command Palette.
    pub fn build_palette_items(self) -> Vec<PaletteItem> {
        crate::palette::build_palette_items_from_store(self)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_interrogator_event(
    streaming: &mut bool,
    status_hint: &mut Option<String>,
    messages: &mut Vec<AgentUiMessage>,
    tape: &mut Vec<TuringTapeEntry>,
    seq_counter: &mut usize,
    ev: &InterrogatorEvent,
    focus_ticker: &str,
    reports: &[MungerReport],
    dossiers: &HashMap<String, Option<Dossier>>,
    lang: UiLang,
) -> bool {
    match ev {
        InterrogatorEvent::Ready | InterrogatorEvent::UserSubmitted { .. } => {
            if status_hint
                .as_deref()
                .is_some_and(|h| h.contains("正在连接") || h.contains("Connecting"))
            {
                *status_hint = None;
            }
            false
        }
        InterrogatorEvent::ToolStarted {
            call_id,
            name,
            title,
        } => {
            *status_hint = Some(match lang {
                UiLang::Zh => format!("正在执行 {name}…"),
                UiLang::En => format!("Running {name}…"),
            });
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id: Some(call_id.clone()),
                kind: TuringTapeKind::Tool,
                summary: if title.is_empty() || title == name {
                    format!("{name} {focus_ticker}")
                } else {
                    format!("{name} · {title}")
                },
                detail: None,
                status: "running".to_owned(),
            });
            false
        }
        InterrogatorEvent::ToolProgress { call_id, preview } => {
            if let Some(row) = tape
                .iter_mut()
                .rev()
                .find(|r| r.call_id.as_deref() == Some(call_id.as_str()))
            {
                row.detail = Some(preview.clone());
            }
            false
        }
        InterrogatorEvent::ToolFinished {
            call_id,
            name,
            status,
            preview,
        } => {
            let blocked_by_redline =
                preview.contains("红线拦截") || preview.to_ascii_lowercase().contains("redline");
            if let Some(row) = tape
                .iter_mut()
                .rev()
                .find(|r| r.call_id.as_deref() == Some(call_id.as_str()))
            {
                row.status = if blocked_by_redline {
                    "denied".to_owned()
                } else {
                    status.clone()
                };
                if blocked_by_redline {
                    row.kind = TuringTapeKind::Policy;
                }
                if !preview.is_empty() {
                    row.detail = Some(preview.clone());
                }
            } else {
                let seq = tape.len() + 1;
                tape.push(TuringTapeEntry {
                    seq,
                    call_id: Some(call_id.clone()),
                    kind: if blocked_by_redline {
                        TuringTapeKind::Policy
                    } else {
                        TuringTapeKind::Tool
                    },
                    summary: format!("{name} {focus_ticker}"),
                    detail: (!preview.is_empty()).then(|| preview.clone()),
                    status: if blocked_by_redline {
                        "denied".to_owned()
                    } else {
                        status.clone()
                    },
                });
            }
            name == "m3_update_draft" && status == "completed" && !blocked_by_redline
        }
        InterrogatorEvent::TextDelta {
            chunk, is_final, ..
        } => {
            *status_hint = None;
            if let Some(last) = messages
                .iter_mut()
                .rev()
                .find(|m| m.role == AgentMessageRole::Assistant && m.streaming)
            {
                last.raw_text.push_str(chunk);
                let (cleaned, tokens) =
                    parse_acme_evidence_tokens(&last.raw_text, focus_ticker, reports, dossiers);
                last.text = cleaned;
                last.tokens = tokens;
                if *is_final {
                    last.streaming = false;
                }
            } else {
                let id = *seq_counter;
                *seq_counter += 1;
                let (cleaned, tokens) =
                    parse_acme_evidence_tokens(chunk, focus_ticker, reports, dossiers);
                messages.push(AgentUiMessage {
                    id: format!("assistant-{id}"),
                    role: AgentMessageRole::Assistant,
                    raw_text: chunk.clone(),
                    text: cleaned,
                    tokens,
                    streaming: !is_final,
                });
            }
            false
        }
        InterrogatorEvent::TurnDone { error, .. } => {
            *streaming = false;
            *status_hint = None;
            let mut token_count = 0usize;
            if let Some(last) = messages
                .iter_mut()
                .rev()
                .find(|m| m.role == AgentMessageRole::Assistant)
            {
                last.streaming = false;
                token_count = last.tokens.len();
            }
            if let Some(err) = error {
                if messages
                    .last()
                    .is_some_and(|m| m.role == AgentMessageRole::Assistant && m.text.is_empty())
                {
                    messages.pop();
                }
                let seq = tape.len() + 1;
                tape.push(TuringTapeEntry {
                    seq,
                    call_id: None,
                    kind: TuringTapeKind::Error,
                    summary: lang
                        .t("审讯未能完成", "Interrogation did not complete")
                        .to_owned(),
                    detail: Some(err.clone()),
                    status: "error".to_owned(),
                });
                let id = *seq_counter;
                *seq_counter += 1;
                messages.push(AgentUiMessage {
                    id: format!("note-{id}"),
                    role: AgentMessageRole::SystemNote,
                    raw_text: err.clone(),
                    text: err.clone(),
                    tokens: Vec::new(),
                    streaming: false,
                });
                false
            } else {
                if messages
                    .last()
                    .is_some_and(|m| m.role == AgentMessageRole::Assistant && m.text.is_empty())
                {
                    messages.pop();
                }
                let seq = tape.len() + 1;
                tape.push(TuringTapeEntry {
                    seq,
                    call_id: None,
                    kind: TuringTapeKind::Out,
                    summary: match lang {
                        UiLang::Zh => format!("回答完成 · 引用 Acme 证据 {token_count} 项"),
                        UiLang::En => {
                            format!("Answer complete · {token_count} Acme evidence citations")
                        }
                    },
                    detail: None,
                    status: "done".to_owned(),
                });
                true
            }
        }
        InterrogatorEvent::ResetDone => {
            *streaming = false;
            *status_hint = None;
            messages.clear();
            tape.clear();
            false
        }
        InterrogatorEvent::Error { message } => {
            *streaming = false;
            *status_hint = None;
            if messages
                .last()
                .is_some_and(|m| m.role == AgentMessageRole::Assistant && m.text.is_empty())
            {
                messages.pop();
            }
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id: None,
                kind: TuringTapeKind::Error,
                summary: lang.t("Alan 状态提示", "Alan status notice").to_owned(),
                detail: Some(message.clone()),
                status: "error".to_owned(),
            });
            let id = *seq_counter;
            *seq_counter += 1;
            messages.push(AgentUiMessage {
                id: format!("err-{id}"),
                role: AgentMessageRole::SystemNote,
                raw_text: message.clone(),
                text: message.clone(),
                tokens: Vec::new(),
                streaming: false,
            });
            false
        }
    }
}

/// Fetches the `AppStore` from the current NSH component context.
pub fn use_app_store() -> Option<AppStore> {
    use_context::<AppStore>()
}
