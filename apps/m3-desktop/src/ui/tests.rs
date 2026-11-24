use super::agent::{
    AgentPanelSlice, DeepResearchUiSlice, PerspectiveUiStatus, apply_deep_research_event,
    apply_interrogator_event, default_deep_research_perspectives, parse_research_memo,
};
use super::company::roic_unavailable_label;
use super::format::*;
use super::inbox::{
    InboxFilter, InboxViewState, budget_watermark_summary, count_inbox_filter,
    filter_inbox_signals, resolve_active_inbox_signal, should_skip_auto_catchup,
    should_skip_auto_catchup_with, signal_kind_badge, signal_status_badge, step_inbox_selection,
};
use super::theme::*;
use super::themes::{format_theme_member_distance, format_theme_member_roic};
use super::universe::{
    UniversePreset, UniverseSearchMode, UniverseViewState, apply_ui_row_guards,
    build_scatter_model, format_watch_promote_banner, is_lake_empty, resolve_active_universe_row,
};
use super::*;
use crate::alan_interrogator::{InterrogatorEvent, TuringTapeKind};
use crate::budget::{BudgetConfig, ThemeClusterMember};
use crate::lake::{UniverseRow, UniverseSortBy, UniverseSummary};
use crate::lake_l1::{SignalEvidence, SignalInboxRow, SignalKind, SignalStatus};
use crate::lake_l2::{BudgetStatusReport, MonthlyBudgetUsage};
use crate::model::{
    DataSource, OhlcBar, StrikeZoneStatus, ValuationAssumptions, ValuationSnapshot,
};
use crate::research::{DeepResearchEvent, DeepResearchResult, PerspectiveOutcome};
use rust_decimal::Decimal;
use std::collections::HashMap;

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
            strict_owner_earnings: Some(Decimal::ZERO),
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
        rows_desc
            .iter()
            .map(|r| r.ticker.as_str())
            .collect::<Vec<_>>(),
        vec!["AAPL", "PTC", "DSY.PA"]
    );

    let mut rows_asc = vec![&b, &a, &c];
    sort_reports(&mut rows_asc, SortCol::Roic, true);
    assert_eq!(
        rows_asc
            .iter()
            .map(|r| r.ticker.as_str())
            .collect::<Vec<_>>(),
        vec!["PTC", "AAPL", "DSY.PA"]
    );

    let mut rows_margin = vec![&b, &a, &c];
    sort_reports(&mut rows_margin, SortCol::Margin, true);
    assert_eq!(
        rows_margin
            .iter()
            .map(|r| r.ticker.as_str())
            .collect::<Vec<_>>(),
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
        compute_kline_domain(&bars, Some(Decimal::from(120)), Some(Decimal::from(145))).unwrap();
    assert!(ymin < 120.0);
    assert!(ymax > 240.0);
    assert_eq!(maxv, 8200);

    let (ymin2, ymax2, _) = compute_kline_domain(&bars, Some(Decimal::from(30)), None).unwrap();
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

#[test]
fn alan_interrogator_tape_and_evidence_wire_cleanly() {
    let rep = make_rep(
        "AAPL",
        Bucket::Core,
        "消费生态",
        Some(Decimal::from(150)),
        Some(Decimal::from(15)),
    );
    let reports = vec![rep];
    let dossiers = HashMap::new();
    let mut streaming = true;
    let mut status_hint = Some("正在连接…".to_string());
    let mut messages = vec![AgentUiMessage {
        id: "a1".into(),
        role: AgentMessageRole::Assistant,
        raw_text: String::new(),
        text: String::new(),
        tokens: Vec::new(),
        streaming: true,
    }];
    let mut tape = Vec::new();
    let mut seq = 2usize;

    let events = [
        InterrogatorEvent::ToolStarted {
            call_id: "c1".into(),
            name: "m3_report".into(),
            title: "AAPL".into(),
        },
        InterrogatorEvent::ToolFinished {
            call_id: "c1".into(),
            name: "m3_report".into(),
            status: "completed".into(),
            preview: "{\"ticker\":\"AAPL\"}".into(),
        },
        InterrogatorEvent::ToolStarted {
            call_id: "c2".into(),
            name: "m3_update_draft".into(),
            title: "guard".into(),
        },
        InterrogatorEvent::ToolFinished {
            call_id: "c2".into(),
            name: "m3_update_draft".into(),
            status: "error".into(),
            preview: "红线拦截：human_judgment 不可写入".into(),
        },
        InterrogatorEvent::TextDelta {
            submission_id: Some("s1".into()),
            chunk: "已核对 [report:AAPL] 与 [xbrl:AAPL]。".into(),
            is_final: false,
        },
        InterrogatorEvent::TurnDone {
            submission_id: "s1".into(),
            completed: true,
            error: None,
        },
    ];

    let mut reload = false;
    for ev in &events {
        let mut slice = AgentPanelSlice {
            streaming: &mut streaming,
            status_hint: &mut status_hint,
            messages: &mut messages,
            tape: &mut tape,
            seq: &mut seq,
        };
        if apply_interrogator_event(&mut slice, ev, "AAPL", &reports, &dossiers) {
            reload = true;
        }
    }

    assert!(reload);
    assert!(!streaming);
    assert_eq!(tape.len(), 3);
    assert_eq!(tape[0].kind, TuringTapeKind::Tool);
    assert_eq!(tape[0].status, "completed");
    assert_eq!(tape[1].kind, TuringTapeKind::Policy);
    assert_eq!(tape[1].status, "denied");
    assert_eq!(tape[2].kind, TuringTapeKind::Out);
    assert_eq!(messages[0].tokens.len(), 2);
    assert_eq!(messages[0].tokens[0].id, "report:AAPL");
    assert_eq!(messages[0].tokens[1].id, "xbrl:AAPL");
}

fn make_universe_row(
    cik: &str,
    ticker: &str,
    years_of_data: usize,
    roic: Option<i64>,
    fcf_yield: Option<i64>,
    distance: Option<i64>,
    in_watchlist: bool,
) -> UniverseRow {
    UniverseRow {
        ticker: ticker.into(),
        cik: cik.into(),
        name: format!("{ticker} Corp."),
        exchange: "NASDAQ".into(),
        sic: Some(7372),
        sic_description: Some("Prepackaged Software".into()),
        is_financial: false,
        latest_filing_end: Some("2025-12-31".into()),
        years_of_data,
        quality_score: Decimal::from(85),
        in_price_pool: fcf_yield.is_some(),
        in_watchlist,
        bucket_opt: in_watchlist.then_some(Bucket::Watch),
        latest_price: fcf_yield.map(|_| Decimal::from(100)),
        strike_price: distance.map(|_| Decimal::from(90)),
        distance_to_strike_pct: distance.map(Decimal::from),
        strict_fcf_yield_pct: fcf_yield.map(Decimal::from),
        roic_5y_avg_pct: roic.map(Decimal::from),
        roic_5y_stddev_pct: Some(Decimal::from(2)),
        gross_margin_5y_avg_pct: Some(Decimal::from(68)),
        gross_margin_5y_stddev_pct: Some(Decimal::from(1)),
        share_change_3y_pct: Some(Decimal::from(-3)),
        sbc_to_ocf_pct: Some(Decimal::from(8)),
        net_debt_to_oe: Some(Decimal::from(0)),
        strict_owner_earnings: Some(Decimal::from(1_000_000_000i64)),
        warnings_count: 0,
        warnings_json: "[]".into(),
        raw_sha256: Some("deadbeef".into()),
    }
}

#[test]
fn universe_view_state_presets_and_toggles_build_accurate_filters() {
    let mut state = UniverseViewState::default();
    assert!(state.is_default());
    assert!(state.require_5y_history);
    let f0 = state.to_lake_filter();
    assert!(f0.exclude_financials);
    assert_eq!(f0.min_years_of_data, Some(5));
    assert_eq!(f0.sort_by, UniverseSortBy::QualityScore);
    assert!(!f0.sort_asc);
    assert!(f0.query.is_none());

    state.toggle_require_5y_history();
    assert!(!state.require_5y_history);
    assert!(!state.is_default());
    assert_eq!(state.to_lake_filter().min_years_of_data, None);

    state.query = "  software ".into();
    state.apply_preset(UniversePreset::HighRoicCompounders);
    assert!(state.require_5y_history);
    assert!(!state.is_default());
    let f1 = state.to_lake_filter();
    assert_eq!(f1.min_roic_5y, Some(Decimal::from(20)));
    assert_eq!(f1.min_years_of_data, Some(5));
    assert!(f1.positive_strict_oe_only);
    assert_eq!(f1.sort_by, UniverseSortBy::Roic5yAvg);
    assert_eq!(f1.query.as_deref(), Some("software"));

    state.apply_preset(UniversePreset::LowSbcCashCows);
    assert!(state.require_5y_history);
    let f2 = state.to_lake_filter();
    assert_eq!(f2.min_roic_5y, Some(Decimal::from(15)));
    assert_eq!(f2.max_sbc_to_ocf, Some(Decimal::from(10)));
    assert_eq!(f2.max_share_change_3y, Some(Decimal::from(2)));
    assert_eq!(f2.max_net_debt_to_oe, Some(Decimal::from(3)));
    assert_eq!(f2.min_years_of_data, Some(5));
    assert_eq!(f2.sort_by, UniverseSortBy::StrictFcfYield);

    state.apply_preset(UniversePreset::StrikeZone);
    assert!(state.require_5y_history);
    state.toggle_exclude_financials();
    let f3 = state.to_lake_filter();
    assert!(!f3.exclude_financials);
    assert_eq!(f3.min_years_of_data, Some(5));
    assert!(f3.strike_zone_only);
    assert!(f3.in_price_pool_only);
    assert_eq!(f3.sort_by, UniverseSortBy::DistanceToStrike);
    assert!(f3.sort_asc);

    state.toggle_sort(UniverseSortBy::DistanceToStrike);
    assert!(!state.sort_asc);
    state.toggle_sort(UniverseSortBy::Ticker);
    assert!(state.sort_asc);

    state.reset();
    assert!(state.is_default());
    assert!(state.exclude_financials);
    assert!(state.require_5y_history);

    let short_hist =
        make_universe_row("0000000099", "NEWIPO", 3, Some(30), Some(6), Some(5), false);
    let full_hist = make_universe_row("0000000100", "MATURE", 5, Some(30), Some(6), Some(5), false);
    let guarded = apply_ui_row_guards(vec![short_hist.clone(), full_hist.clone()], true);
    assert_eq!(guarded.len(), 1);
    assert_eq!(guarded[0].cik, "0000000100");
    assert_eq!(
        apply_ui_row_guards(vec![short_hist, full_hist], false).len(),
        2
    );
}

#[test]
fn universe_empty_lake_detection_handles_missing_and_zero_company_states() {
    assert!(is_lake_empty(false, None));
    assert!(is_lake_empty(true, None));
    assert!(is_lake_empty(true, Some(&UniverseSummary::default())));
    assert!(!is_lake_empty(
        true,
        Some(&UniverseSummary {
            total_companies: 120,
            ..UniverseSummary::default()
        })
    ));
}

#[test]
fn universe_scatter_model_clamps_outliers_and_prioritizes_selected_point() {
    let empty_strikes = HashSet::new();
    let unpriced = make_universe_row("0000000001", "NOPRICE", 5, Some(25), None, None, false);
    assert!(build_scatter_model(std::slice::from_ref(&unpriced), &empty_strikes, None).is_none());

    let normal = make_universe_row("0000769397", "ADSK", 8, Some(32), Some(6), Some(15), true);
    let strike = make_universe_row(
        "0000000002",
        "CHEAP",
        6,
        Some(22),
        Some(9),
        Some(-12),
        false,
    );
    let stale_strike = make_universe_row(
        "0000000004",
        "STALE",
        6,
        Some(20),
        Some(8),
        Some(-18),
        false,
    );
    let outlier = make_universe_row(
        "0000000003",
        "HYPER",
        5,
        Some(250),
        Some(45),
        Some(5),
        false,
    );

    let fresh_strikes = HashSet::from([strike.cik.clone()]);
    let model = build_scatter_model(
        &[unpriced, outlier, normal, strike, stale_strike],
        &fresh_strikes,
        Some("0000769397"),
    )
    .unwrap();
    assert_eq!(model.points.len(), 4);
    assert!(model.ref_roic_norm.is_some());
    assert!(model.ref_fcf_norm.is_some());
    assert!(model.clipped_count >= 1);
    assert!(model.x_max <= 80.0);
    assert!(model.y_max <= 25.0);

    // Selected point must be painted last so crosshair + halo sit on top.
    let last = model.points.last().unwrap();
    assert_eq!(last.cik, "0000769397");
    assert_eq!(last.ticker, "ADSK");
    assert!(last.is_selected);
    assert!(last.in_watchlist);

    let cheap_pt = model.points.iter().find(|p| p.ticker == "CHEAP").unwrap();
    assert!(cheap_pt.in_strike_zone);
    let stale_pt = model.points.iter().find(|p| p.ticker == "STALE").unwrap();
    assert!(!stale_pt.in_strike_zone);

    let hyper = model.points.iter().find(|p| p.ticker == "HYPER").unwrap();
    assert!(hyper.clipped);
    assert!((0.0..=1.0).contains(&hyper.x_norm));
    assert!((0.0..=1.0).contains(&hyper.y_norm));
}

#[test]
fn universe_palette_action_matches_chinese_and_english_queries() {
    let items = vec![
        PaletteItem {
            group: "VIEW".into(),
            title: "全市场雷达 · US Lakehouse L0".into(),
            subtitle: "DuckDB 全美股筛选、ROIC × FCF 散点图与一键入观察池".into(),
            shortcut: Some("⌘5"),
            dot_color: SKY_FG,
            action: PaletteAction::SwitchView(NavView::Universe),
        },
        PaletteItem {
            group: "DATA".into(),
            title: "同步全美股 L0 湖仓 (SEC Bulk + 价格分层)".into(),
            subtitle: "下载 companyfacts.zip".into(),
            shortcut: None,
            dot_color: SKY_FG,
            action: PaletteAction::SyncUniverseNow,
        },
    ];

    let view_hits = filter_palette_items(&items, "全市场雷达");
    assert_eq!(view_hits.len(), 1);
    assert_eq!(
        view_hits[0].action,
        PaletteAction::SwitchView(NavView::Universe)
    );

    let l0_hits = filter_palette_items(&items, "l0");
    assert_eq!(l0_hits.len(), 2);
}

#[test]
fn fmt_money_compact_opt_and_roic_note_render_unavailable_values_as_dash() {
    assert_eq!(fmt_money_compact_opt(None, "USD"), "—");
    assert_eq!(fmt_money_compact_opt(Some(Decimal::ZERO), "USD"), "$0");
    assert_eq!(
        fmt_money_compact_opt(Some(Decimal::from(1_500_000_000i64)), "USD"),
        "$1.50B"
    );
    assert_eq!(
        fmt_money_compact_opt(Some(Decimal::from(-25_000_000i64)), "USD"),
        "-$25.0M"
    );

    assert_eq!(
        roic_unavailable_label(Some("total_debt_unavailable")),
        "— (Debt Unavailable)"
    );
    assert_eq!(
        roic_unavailable_label(Some("cash_and_st_unavailable")),
        "— (Cash Unavailable)"
    );
    assert_eq!(
        roic_unavailable_label(Some("tax_expense_unavailable")),
        "— (Tax Unavailable)"
    );
    assert_eq!(
        roic_unavailable_label(Some("net_cash_exceeds_book_capital")),
        "Net Cash > Book Capital"
    );
    assert_eq!(roic_unavailable_label(None), "—");
}

#[test]
fn resolve_active_universe_row_prefers_matching_drawer_snapshot_copy() {
    let mut table_row =
        make_universe_row("0000320193", "AAPL", 8, Some(45), Some(4), Some(10), true);
    table_row.latest_price = Some(Decimal::from(100));
    let other_row = make_universe_row("0000769397", "ADSK", 8, Some(32), Some(6), Some(15), false);

    let mut drawer_row = table_row.clone();
    drawer_row.latest_price = Some(Decimal::from(112));

    let rows = vec![table_row.clone(), other_row.clone()];
    let resolved = resolve_active_universe_row(Some("0000320193"), Some(&drawer_row), &rows)
        .expect("should match drawer snapshot row");
    assert_eq!(resolved.cik, "0000320193");
    assert_eq!(resolved.latest_price, Some(Decimal::from(112)));

    let mismatched_drawer =
        resolve_active_universe_row(Some("0000769397"), Some(&drawer_row), &rows)
            .expect("should fall back to matching row in table when drawer cik differs");
    assert_eq!(mismatched_drawer.cik, "0000769397");
    assert_eq!(mismatched_drawer.ticker, "ADSK");
}

#[test]
fn format_watch_promote_banner_surfaces_duckdb_and_reload_errors() {
    let ok = format_watch_promote_banner("ADSK", "Autodesk", None, None, None, false);
    assert!(!ok.1);
    assert!(ok.0.contains("ADSK"));

    let syncing = format_watch_promote_banner("ADSK", "Autodesk", None, None, None, true);
    assert!(!syncing.1);
    assert!(syncing.0.contains("后台全市场同步完成后"));

    let lake_err =
        format_watch_promote_banner("ADSK", "Autodesk", None, Some("duckdb locked"), None, false);
    assert!(lake_err.1);
    assert!(lake_err.0.contains("duckdb locked"));

    let reload_err =
        format_watch_promote_banner("ADSK", "Autodesk", None, None, Some("bad snapshot"), false);
    assert!(reload_err.1);
    assert!(reload_err.0.contains("bad snapshot"));

    let disk_err = format_watch_promote_banner(
        "ADSK",
        "Autodesk",
        Some("report corrupt"),
        None,
        None,
        false,
    );
    assert!(disk_err.1);
    assert!(disk_err.0.contains("report corrupt"));
}

fn make_inbox_signal(
    id: &str,
    kind: SignalKind,
    status: SignalStatus,
    ticker: &str,
    score: f64,
) -> SignalInboxRow {
    SignalInboxRow {
        signal_id: id.into(),
        kind,
        score,
        primary_ticker: ticker.into(),
        tickers: vec![ticker.into()],
        title: format!("{ticker} signal {id}"),
        summary: format!("Summary for {ticker}"),
        evidence: vec![SignalEvidence {
            chunk_id: Some(format!("c-{id}")),
            ticker: ticker.into(),
            doc_type: "10-K".into(),
            section: "item_1".into(),
            filing_date: Some("2026-03-01".into()),
            title: "Business".into(),
            quote: "Recurring CAD software subscription revenue.".into(),
            source_url: "https://www.sec.gov/example".into(),
            similarity: 0.88,
        }],
        status,
        event_id: None,
        created_at_ms: 1_795_400_000_000,
        updated_at_ms: 1_795_400_000_000,
    }
}

#[test]
fn inbox_filters_navigation_and_auto_catchup_guards_work() {
    let s1 = make_inbox_signal(
        "s1",
        SignalKind::Discovery,
        SignalStatus::Unread,
        "PTC",
        0.92,
    );
    let s2 = make_inbox_signal("s2", SignalKind::Ripple, SignalStatus::Unread, "ADSK", 0.85);
    let s3 = make_inbox_signal("s3", SignalKind::Strike, SignalStatus::Watch, "CDNS", 0.80);
    let s4 = make_inbox_signal(
        "s4",
        SignalKind::RedFlag,
        SignalStatus::Unread,
        "INTC",
        0.78,
    );
    let s5 = make_inbox_signal(
        "s5",
        SignalKind::Discovery,
        SignalStatus::Archived,
        "IBM",
        0.50,
    );
    let signals = vec![s1, s2, s3, s4, s5];

    assert_eq!(count_inbox_filter(&signals, InboxFilter::Unread), 3);
    assert_eq!(count_inbox_filter(&signals, InboxFilter::All), 5);
    assert_eq!(count_inbox_filter(&signals, InboxFilter::Discovery), 1);
    assert_eq!(count_inbox_filter(&signals, InboxFilter::Ripple), 1);
    assert_eq!(
        count_inbox_filter(&signals, InboxFilter::StrikeAndRedFlag),
        2
    );
    assert_eq!(count_inbox_filter(&signals, InboxFilter::Archived), 1);

    let unread = filter_inbox_signals(&signals, InboxFilter::Unread);
    let (idx0, active0) = resolve_active_inbox_signal(&unread, None, 0).unwrap();
    assert_eq!(idx0, 0);
    assert_eq!(active0.signal_id, "s1");

    let (idx1, id1) = step_inbox_selection(&unread, Some(&active0.signal_id), idx0, 1).unwrap();
    assert_eq!(idx1, 1);
    assert_eq!(id1, "s2");

    let (idx2, id2) = step_inbox_selection(&unread, Some(&id1), idx1, 5).unwrap();
    assert_eq!(idx2, 2);
    assert_eq!(id2, "s4");

    let (idx_back, id_back) = step_inbox_selection(&unread, Some(&id2), idx2, -10).unwrap();
    assert_eq!(idx_back, 0);
    assert_eq!(id_back, "s1");

    assert_eq!(signal_kind_badge(SignalKind::Ripple).0, "隐空间传导");
    assert_eq!(
        signal_status_badge(SignalStatus::Researching).0,
        "Alan 研判"
    );

    let state = InboxViewState::default();
    assert_eq!(state.filter, InboxFilter::Unread);

    assert!(should_skip_auto_catchup(true));
    assert!(should_skip_auto_catchup_with(true, None, true));
    assert!(should_skip_auto_catchup_with(false, None, false));
    assert!(should_skip_auto_catchup_with(false, Some("1"), true));
    assert!(should_skip_auto_catchup_with(false, Some("true"), true));
    assert!(!should_skip_auto_catchup_with(false, Some("0"), true));
    assert!(!should_skip_auto_catchup_with(false, None, true));
}

#[test]
fn universe_similar_search_state_and_palette_actions_wire_cleanly() {
    let mut state = UniverseViewState::default();
    assert_eq!(state.search_mode, UniverseSearchMode::Keyword);
    assert!(!state.similar_as_ticker);

    state.apply_preset(UniversePreset::LowSbcCashCows);
    state.start_similar_for_ticker("adsk");
    assert_eq!(state.search_mode, UniverseSearchMode::Similar);
    assert!(state.similar_as_ticker);
    assert_eq!(state.query, "ADSK");
    assert!(!state.is_default());

    let sim_filter = state.to_similar_filter();
    assert_eq!(sim_filter.min_roic_5y, Some(Decimal::from(15)));
    assert_eq!(sim_filter.max_sbc_to_ocf, Some(Decimal::from(10)));
    assert!(sim_filter.exclude_financials);
    assert!(sim_filter.positive_strict_oe_only);
    assert_eq!(sim_filter.limit, 50);

    state.set_search_mode(UniverseSearchMode::Keyword);
    assert_eq!(state.search_mode, UniverseSearchMode::Keyword);
    assert!(!state.similar_as_ticker);

    let items = vec![
        PaletteItem {
            group: "VIEW".into(),
            title: "简报收件箱 · Event-Driven Inbox".into(),
            subtitle: "新浮现、隐空间传导与击球红灯待审讯信号".into(),
            shortcut: Some("⌘1"),
            dot_color: EMERALD_FG,
            action: PaletteAction::SwitchView(NavView::Inbox),
        },
        PaletteItem {
            group: "DATA".into(),
            title: "追平事件流 · L1 SEC Filings / News / 语义索引".into(),
            subtitle: "0 Flagship Token".into(),
            shortcut: Some("r"),
            dot_color: EMERALD_FG,
            action: PaletteAction::CatchupEvents,
        },
        PaletteItem {
            group: "SEARCH".into(),
            title: "混合语义相似搜索 · Lookalike Discovery".into(),
            subtitle: "按商业模式描述或 Ticker 检索".into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::FocusSimilarSearch,
        },
    ];

    assert_eq!(
        filter_palette_items(&items, "inbox")[0].action,
        PaletteAction::SwitchView(NavView::Inbox)
    );
    assert_eq!(
        filter_palette_items(&items, "追平")[0].action,
        PaletteAction::CatchupEvents
    );
    assert_eq!(
        filter_palette_items(&items, "lookalike")[0].action,
        PaletteAction::FocusSimilarSearch
    );
}

#[test]
fn milestone_c_memo_parser_deep_research_events_budget_themes_and_palette() {
    let md = "\
# ADSK · Autodesk, Inc. — Living Research Memo

- **Run ID**: `run-42`
- **Updated Date**: 2026-11-24
- **Trigger**: `manual`
- **Model**: `connection-service-default`
- **Usage**: `1200` in / `400` out tokens · `$0.0096` USD

## Deterministic Financial Snapshot

| 指标 | 确定性数值 | 来源 |
| --- | --- | --- |
| 5Y ROIC 均值 | 28.00% | `calculator.rs` |

## 护城河 (moat)

High switching costs across AEC CAD workflows.

## 会计排雷 (accounting)

Watch SBC dilution relative to OCF.

## 隐空间同行 (latent_peers)

Closely aligned with PTC and SNPS.

## 5年逆向验尸 (premortem)

Cloud migration churn or pricing backlash.

## Evidence & Citations

1. **[ADSK]** `10-K · item_1` (2026-03-10, sim `0.95`): \"AEC design platform\" — https://www.sec.gov/adsk

## Human Notes

Keep watching SBC ratio before adding size.
";

    let parsed = parse_research_memo(md);
    assert_eq!(parsed.title, "ADSK · Autodesk, Inc. — Living Research Memo");
    assert_eq!(parsed.metadata.len(), 5);
    assert_eq!(parsed.metadata[0], ("Run ID".into(), "run-42".into()));
    assert_eq!(parsed.citations.len(), 1);
    assert!(parsed.citations[0].contains("ADSK"));
    assert_eq!(
        parsed.human_notes.as_deref(),
        Some("Keep watching SBC ratio before adding size.")
    );
    assert!(parsed.sections.iter().any(|s| s.id == "moat"));
    assert!(parsed.sections.iter().any(|s| s.id == "premortem"));

    // Deep Research streaming state transitions (completed + budget-blocked)
    let mut running = false;
    let mut run_id = None;
    let mut status_note = None;
    let mut budget_blocked = None;
    let mut perspectives = default_deep_research_perspectives();
    let mut last_result = None;
    let mut tape = Vec::new();

    let events = [
        DeepResearchEvent::Started {
            run_id: "run-1".into(),
            ticker: "ADSK".into(),
        },
        DeepResearchEvent::PerspectiveStarted {
            perspective: "moat".into(),
            label: "护城河".into(),
        },
        DeepResearchEvent::ToolStarted {
            perspective: "moat".into(),
            name: "m3_peer_percentiles".into(),
            title: "ADSK".into(),
        },
        DeepResearchEvent::ToolFinished {
            perspective: "moat".into(),
            name: "m3_peer_percentiles".into(),
            status: "completed".into(),
        },
        DeepResearchEvent::PerspectiveDelta {
            perspective: "moat".into(),
            chunk: "Strong moat.".into(),
        },
        DeepResearchEvent::PerspectiveFinished {
            perspective: "moat".into(),
            summary_chars: 12,
        },
        DeepResearchEvent::Completed {
            result: Box::new(DeepResearchResult {
                run_id: "run-1".into(),
                ticker: "ADSK".into(),
                trigger: "manual".into(),
                status: "completed".into(),
                model: "test-model".into(),
                memo_path: Some("workbench/memos/ADSK.md".into()),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: Decimal::new(45, 4),
                perspectives: vec![PerspectiveOutcome {
                    perspective: "moat".into(),
                    title: "护城河".into(),
                    content: "Strong moat.".into(),
                    tools_used: vec!["m3_peer_percentiles".into()],
                    input_tokens: 500,
                    output_tokens: 200,
                }],
                citations: Vec::new(),
                error: None,
                finished_at_ms: 1_795_500_000_000,
            }),
        },
    ];
    let mut last_effect = None;
    for ev in &events {
        let mut slice = DeepResearchUiSlice {
            running: &mut running,
            run_id: &mut run_id,
            status_note: &mut status_note,
            budget_blocked: &mut budget_blocked,
            perspectives: &mut perspectives,
            last_result: &mut last_result,
            tape: &mut tape,
        };
        last_effect = Some(apply_deep_research_event(&mut slice, ev));
    }
    assert!(!running);
    assert!(last_effect.unwrap().reload_context);
    assert_eq!(perspectives[0].status, PerspectiveUiStatus::Completed);
    assert_eq!(perspectives[0].input_tokens, 500);
    assert_eq!(tape.last().unwrap().kind, TuringTapeKind::Out);

    // Budget-blocked event
    let mut slice = DeepResearchUiSlice {
        running: &mut running,
        run_id: &mut run_id,
        status_note: &mut status_note,
        budget_blocked: &mut budget_blocked,
        perspectives: &mut perspectives,
        last_result: &mut last_result,
        tape: &mut tape,
    };
    let blocked_eff = apply_deep_research_event(
        &mut slice,
        &DeepResearchEvent::BudgetBlocked {
            run_id: "run-2".into(),
            ticker: "PTC".into(),
            reason: "月度预算已用尽".into(),
        },
    );
    assert!(blocked_eff.reload_context);
    assert!(blocked_eff.banner.unwrap().1);
    assert_eq!(budget_blocked.as_deref(), Some("月度预算已用尽"));
    assert_eq!(tape.last().unwrap().kind, TuringTapeKind::Policy);

    // Inbox budget watermark summary
    let (default_lbl, default_col) = budget_watermark_summary(None);
    assert_eq!(default_lbl, "本月 AI 预算 $0.00 / $10.00");
    assert_eq!(default_col, EMERALD_FG);

    let exhausted_report = BudgetStatusReport {
        config: BudgetConfig::default(),
        usage: MonthlyBudgetUsage {
            month_utc: "2026-11".into(),
            total_runs: 3,
            completed_runs: 3,
            blocked_runs: 0,
            failed_runs: 0,
            total_input_tokens: 100_000,
            total_output_tokens: 50_000,
            total_cost_usd: Decimal::new(1000, 2),
            weekly_auto_runs: 0,
        },
        remaining_monthly_usd: Decimal::ZERO,
        remaining_monthly_tokens: 1_850_000,
        auto_trigger_remaining_weekly: 0,
    };
    let (ex_lbl, ex_col) = budget_watermark_summary(Some(&exhausted_report));
    assert_eq!(ex_lbl, "本月 AI 预算 $10.00 / $10.00");
    assert_eq!(ex_col, CORAL_FG);

    // Themes member formatting
    let member = ThemeClusterMember {
        ticker: "ADSK".into(),
        cik: "0000769397".into(),
        name: "Autodesk".into(),
        sic: Some(7372),
        sic_description: Some("Prepackaged Software".into()),
        roic_5y_avg_pct: Some(Decimal::from(28)),
        distance_to_strike_pct: Some(Decimal::new(-55, 1)),
        in_watchlist: true,
    };
    assert_eq!(format_theme_member_roic(&member), "ROIC 28%");
    assert_eq!(format_theme_member_distance(&member), "-5.50%");

    // Milestone C Command Palette items
    let c_palette = vec![
        PaletteItem {
            group: "RESEARCH".into(),
            title: "Deep Research · 4 视角并行深研 (ADSK)".into(),
            subtitle: "受预算硬门控".into(),
            shortcut: None,
            dot_color: VIOLET_FG,
            action: PaletteAction::RunDeepResearch("ADSK".into()),
        },
        PaletteItem {
            group: "BUDGET".into(),
            title: "Budget Status · 查看本月 AI 预算与配额".into(),
            subtitle: "budget.json".into(),
            shortcut: None,
            dot_color: EMERALD_FG,
            action: PaletteAction::ShowBudgetStatus,
        },
        PaletteItem {
            group: "THEMES".into(),
            title: "Sync Themes · 同步跨公司商业原型主题图谱".into(),
            subtitle: "themes.json".into(),
            shortcut: Some("s"),
            dot_color: SKY_FG,
            action: PaletteAction::SyncThemes,
        },
    ];
    assert_eq!(
        filter_palette_items(&c_palette, "deep research")[0].action,
        PaletteAction::RunDeepResearch("ADSK".into())
    );
    assert_eq!(
        filter_palette_items(&c_palette, "budget status")[0].action,
        PaletteAction::ShowBudgetStatus
    );
    assert_eq!(
        filter_palette_items(&c_palette, "sync themes")[0].action,
        PaletteAction::SyncThemes
    );
}
