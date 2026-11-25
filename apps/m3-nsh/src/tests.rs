//! Unit tests & Phase 1 spike verification for `m3-nsh`.

use crate::format::{
    apply_deep_research_event_compat, budget_watermark_summary_for, civil_ym_from_ms,
    civil_ymd_from_ms, collect_circle_tags, compact_sidebar_version_label, compute_kline_domain,
    compute_today_pulse, count_inbox_filter, filter_inbox_signals, filter_palette_items,
    filter_reports_for_basket, fmt_money_compact, fmt_money_compact_opt, fmt_pct_clean,
    fmt_time_cn_for, fmt_volume_compact, format_theme_member_distance, format_theme_member_roic,
    format_update_progress, parse_research_memo, resolve_active_inbox_signal,
    resolve_sidebar_dock_notice_for, should_skip_auto_catchup, should_skip_auto_catchup_with,
    signal_kind_badge_for, signal_status_badge_for, sort_reports, step_inbox_selection,
};
use crate::palette::{build_palette_items_from_store, execute_palette_action};
use crate::platform_desktop::build_app_menus;
use crate::stores::app_store::apply_interrogator_event;
use crate::stores::types::{
    AgentMessageRole, AgentUiMessage, PerspectiveUiStatus, SortCol, StatusQuickFilter,
    UniverseViewState, apply_ui_row_guards, default_deep_research_perspectives,
};
use crate::stores::{
    AppStore, BasketsViewMode, ChartTimeframe, CompanyViewTab, InboxFilter, InboxViewState,
    PaletteAction, PaletteItem, UniversePreset, UniverseSearchMode, WorkspaceTab,
};
use crate::theme::{
    BG_ROW_ALT, BG_SELECTED_ROW, BG_VIEWPORT, CORAL_FG, EMERALD_FG, SKY_FG, VIOLET_FG,
    bucket_colors, industry_status_colors, journal_action_colors, share_delta_color, status_colors,
    status_dot_color,
};
use crate::views::chart::{
    compute_kline_canvas_bounds, compute_kline_valuation_overlay, kline_hover_bar_index,
    kline_price_to_y, select_kline_bars,
};
use crate::views::company::roic_unavailable_label;
use crate::views::onboarding::{is_fresh_workspace_for_onboarding, starter_pack_summary_line};
use crate::views::universe::{
    build_scatter_model, hit_test_scatter_point, is_lake_empty, scatter_point_px,
};
use m3_core::alan_interrogator::{InterrogatorEvent, TuringTapeKind};
use m3_core::budget::{BudgetConfig, ThemeClusterMember};
use m3_core::i18n::UiLang;
use m3_core::industry::IndustryStatus;
use m3_core::lake::{UniverseRow, UniverseSortBy, UniverseSummary};
use m3_core::lake_l1::{SignalEvidence, SignalInboxRow, SignalKind, SignalStatus};
use m3_core::lake_l2::{BudgetStatusReport, MonthlyBudgetUsage};
use m3_core::model::{
    Bucket, DataSource, JournalAction, MungerReport, OhlcBar, PriceHistorySnapshot, PriceSnapshot,
    StrikeZoneStatus, ValuationAssumptions, ValuationSnapshot,
};
use m3_core::research::{DeepResearchEvent, DeepResearchResult, PerspectiveOutcome};
use m3_core::store::WorkspacePaths;
use m3_core::update::ReleaseChannel;
use nsh_core::element::{Runtime, create_root};
use nsh_core::prelude::*;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

fn run_with_reactive_runtime<T>(setup: impl FnOnce() -> T, check: impl FnOnce(T)) {
    let runtime = Runtime::default();
    let _entered = runtime.enter();
    let (owner, made) = create_root(setup);
    check(made);
    owner.dispose();
}

fn temp_workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("m3_nsh_test_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("workbench"));
    dir
}

#[test]
fn store_state_transitions_and_navigation() {
    let root = temp_workspace("store_nav");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            assert_eq!(store.active_tab.get(), WorkspaceTab::Inbox);
            assert!(!store.company_open.get());

            store.open_tab(WorkspaceTab::Universe);
            assert_eq!(store.active_tab.get(), WorkspaceTab::Universe);

            store.apply_universe_preset(UniversePreset::HighRoicCompounders);
            let st = store.universe_state.get();
            assert_eq!(st.preset, UniversePreset::HighRoicCompounders);
            assert_eq!(st.min_roic_5y, Some(Decimal::from(20)));

            store.set_universe_search_mode(UniverseSearchMode::Similar);
            assert_eq!(
                store.universe_state.get().search_mode,
                UniverseSearchMode::Similar
            );

            store.open_company("AAPL", CompanyViewTab::DeepResearchAlan);
            assert!(store.company_open.get());
            assert_eq!(store.selected_ticker.get(), "AAPL");
            assert_eq!(
                store.company_view_tab.get(),
                CompanyViewTab::DeepResearchAlan
            );

            store.close_company_overlay();
            assert!(!store.company_open.get());

            assert_eq!(store.baskets_view_mode.get(), BasketsViewMode::List);
            store.toggle_baskets_view();
            assert_eq!(store.baskets_view_mode.get(), BasketsViewMode::Board);
            assert_eq!(store.active_tab.get(), WorkspaceTab::Baskets);

            store.toggle_palette();
            assert!(store.palette_open.get());
            store.close_palette();
            assert!(!store.palette_open.get());
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn formatting_and_memo_parser_helpers() {
    assert_eq!(
        fmt_money_compact(Decimal::from(2_500_000_000i64), "USD"),
        "$2.50B"
    );
    assert_eq!(
        fmt_pct_clean(Some(Decimal::new(1234, 2)), true),
        ("+12.34%".into(), 1)
    );
    assert_eq!(
        fmt_pct_clean(Some(Decimal::new(1, 3)), true),
        ("0.00%".into(), 0)
    );

    let raw = "# AAPL Living Memo\n\n- **Updated**: `2026-11-24`\n\n## Moat (moat)\nStrong ecosystem.\n\n## Human Notes\nKeep watching FCF yield.\n";
    let parsed = parse_research_memo(raw);
    assert_eq!(parsed.title, "AAPL Living Memo");
    assert_eq!(parsed.metadata.len(), 1);
    assert_eq!(
        parsed.human_notes.as_deref(),
        Some("Keep watching FCF yield.")
    );
}

#[test]
fn palette_and_inbox_filtering() {
    let items = vec![
        PaletteItem {
            group: "DOSSIER".into(),
            title: "AAPL Apple Inc. [CORE]".into(),
            subtitle: "#consumer · Strike".into(),
            shortcut: Some("↵"),
            dot_color: EMERALD_FG,
            action: PaletteAction::OpenDossier("AAPL".into()),
        },
        PaletteItem {
            group: "VIEW".into(),
            title: "Universe Radar".into(),
            subtitle: "DuckDB screening".into(),
            shortcut: Some("⌘2"),
            dot_color: EMERALD_FG,
            action: PaletteAction::SwitchTab(WorkspaceTab::Universe),
        },
    ];
    let filtered = filter_palette_items(&items, "aapl core");
    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered[0].action,
        PaletteAction::OpenDossier("AAPL".into())
    );

    let sigs = vec![
        SignalInboxRow {
            signal_id: "s1".into(),
            kind: SignalKind::Strike,
            status: SignalStatus::Unread,
            primary_ticker: "AAPL".into(),
            tickers: vec!["AAPL".into()],
            title: "Strike".into(),
            summary: String::new(),
            evidence: Vec::new(),
            score: 1.0,
            event_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        },
        SignalInboxRow {
            signal_id: "s2".into(),
            kind: SignalKind::Discovery,
            status: SignalStatus::Archived,
            primary_ticker: "MSFT".into(),
            tickers: vec!["MSFT".into()],
            title: "Old".into(),
            summary: String::new(),
            evidence: Vec::new(),
            score: 0.5,
            event_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        },
    ];
    assert_eq!(filter_inbox_signals(&sigs, InboxFilter::Unread).len(), 1);
    assert_eq!(filter_inbox_signals(&sigs, InboxFilter::Archived).len(), 1);
    assert_eq!(filter_inbox_signals(&sigs, InboxFilter::All).len(), 2);
}

#[test]
fn bilingual_application_menus_cover_all_seven_menus() {
    let root = temp_workspace("menus");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            let zh = build_app_menus(UiLang::Zh, ReleaseChannel::Stable, store);
            assert_eq!(zh.len(), 7);
            let en = build_app_menus(UiLang::En, ReleaseChannel::Beta, store);
            assert_eq!(en.len(), 7);
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn spike_virtual_list_6000_and_canvas_bounds_do_not_panic() {
    let runtime = Runtime::default();
    let _entered = runtime.enter();
    let (owner, _) = create_root(|| {
        let rows: Signal<Vec<String>> = signal((0..6000).map(|i| format!("T{i:04}")).collect());
        let selected: Signal<Option<String>> = signal(Some("T0042".to_owned()));
        let list = virtual_list(
            move || rows.with(|r| r.len()),
            move |pos: Position| {
                view()
                    .flex_row()
                    .items_center()
                    .h(px(44.0))
                    .bg(move || {
                        let idx = pos.index();
                        let sel = selected.get();
                        rows.with(|r| match r.get(idx) {
                            Some(t) if sel.as_deref() == Some(t.as_str()) => rgb(BG_SELECTED_ROW),
                            _ if idx.is_multiple_of(2) => rgb(BG_VIEWPORT),
                            _ => rgb(BG_ROW_ALT),
                        })
                    })
                    .child(text(move || {
                        rows.with(|r| r.get(pos.index()).cloned().unwrap_or_default())
                    }))
            },
        )
        .estimated_item_size(px(44.0))
        .keys(move || rows.with(|r| r.clone()));
        let _el = list.into_element();

        let bars = vec![
            OhlcBar {
                ts_ms: 1_700_000_000_000,
                open: Decimal::from(100),
                high: Decimal::from(115),
                low: Decimal::from(95),
                close: Decimal::from(110),
                volume: 1_500_000,
            },
            OhlcBar {
                ts_ms: 1_700_604_800_000,
                open: Decimal::from(110),
                high: Decimal::from(125),
                low: Decimal::from(105),
                close: Decimal::from(120),
                volume: 2_200_000,
            },
        ];
        let domain = compute_kline_domain(&bars, Some(Decimal::from(90)), Some(Decimal::from(130)));
        assert!(domain.is_some());
        let canvas_bounds = compute_kline_canvas_bounds(
            &bars,
            Some(Decimal::from(90)),
            Some(Decimal::from(130)),
            640.0,
            320.0,
        );
        assert_eq!(canvas_bounds.map(|b| b.3), Some(2));
        assert!(compute_kline_canvas_bounds(&bars, None, None, 0.0, 320.0).is_none());

        let _painter = Painter::new()
            .fill_color(rgb(BG_VIEWPORT))
            .fill_rect(0.0, 0.0, 640.0, 320.0)
            .stroke_color(rgb(EMERALD_FG))
            .begin_path()
            .move_to(0.0, 160.0)
            .line_to(640.0, 120.0)
            .stroke();
    });
    owner.dispose();
}

fn make_test_report(oe_per_share: Decimal, cur_price: Decimal) -> MungerReport {
    let val = ValuationSnapshot {
        basis_label: "TTM".into(),
        period_end: "2026-09-30".into(),
        strict_owner_earnings: Some(oe_per_share * Decimal::from(1_000_000)),
        shares_basis: Decimal::from(1_000_000),
        owner_earnings_per_share: Some(oe_per_share),
        market_cap: Some(cur_price * Decimal::from(1_000_000)),
        enterprise_value: Some(cur_price * Decimal::from(1_000_000)),
        strict_fcf_yield_mcap_pct: Some(Decimal::from(5)),
        strict_fcf_yield_ev_pct: Some(Decimal::from(5)),
        fair_value_low: Some(oe_per_share * Decimal::from(15)),
        fair_value_high: Some(oe_per_share * Decimal::from(19)),
        strike_price: Some(oe_per_share * Decimal::from(15)),
        distance_to_strike_pct: Some(Decimal::from(20)),
        implied_growth_for_target_pct: Some(Decimal::from(5)),
        expected_total_return_pct: Some(Decimal::from(13)),
    };
    MungerReport {
        schema_version: 1,
        computed_at_ms: 1_700_000_000_000,
        ticker: "TEST".into(),
        name: "Test Corp".into(),
        bucket: Bucket::Watch,
        status: StrikeZoneStatus::Wait,
        source: DataSource::SecEdgar,
        cik: Some("0000123456".into()),
        currency: "USD".into(),
        circle: "software".into(),
        too_hard_reason: None,
        assumptions: ValuationAssumptions {
            multiple_low: Decimal::from(15),
            multiple_high: Decimal::from(19),
            target_return_pct: Decimal::from(10),
            expected_growth_pct: Decimal::from(8),
            strike_price_override: None,
        },
        price: Some(PriceSnapshot {
            schema_version: 1,
            symbol: "TEST".into(),
            price: cur_price,
            previous_close: Some(cur_price),
            currency: "USD".into(),
            exchange: "NASDAQ".into(),
            market_time_ms: Some(1_700_000_000_000),
            fetched_at_ms: 1_700_000_000_000,
            source_url: "https://example.com".into(),
        }),
        raw_sha256: None,
        latest_filing_end: Some("2026-09-30".into()),
        annual_rows: Vec::new(),
        ttm_row: None,
        latest_ytd: None,
        roic_5y_avg_pct: Some(Decimal::from(24)),
        roic_5y_stddev_pct: None,
        gross_margin_5y_avg_pct: None,
        gross_margin_5y_stddev_pct: None,
        share_change_3y_pct: None,
        share_change_5y_pct: None,
        net_debt: None,
        net_debt_to_owner_earnings: None,
        fy_valuation: None,
        ttm_valuation: Some(val),
        warnings: Vec::new(),
        provenance_summary: Vec::new(),
    }
}

fn make_universe_row(
    cik: &str,
    ticker: &str,
    years: usize,
    roic_pct: Option<i64>,
    fcf_yield_pct: Option<i64>,
    dist_pct: Option<i64>,
    in_watchlist: bool,
) -> UniverseRow {
    UniverseRow {
        ticker: ticker.into(),
        cik: cik.into(),
        name: format!("{ticker} Corp"),
        exchange: "NASDAQ".into(),
        sic: Some(7372),
        sic_description: Some("Software".into()),
        is_financial: false,
        latest_filing_end: Some("2026-09-30".into()),
        years_of_data: years,
        quality_score: Decimal::from(80),
        in_price_pool: fcf_yield_pct.is_some(),
        in_watchlist,
        bucket_opt: in_watchlist.then_some(Bucket::Watch),
        latest_price: fcf_yield_pct.map(|_| Decimal::from(100)),
        strike_price: fcf_yield_pct.map(|_| Decimal::from(90)),
        distance_to_strike_pct: dist_pct.map(Decimal::from),
        strict_fcf_yield_pct: fcf_yield_pct.map(Decimal::from),
        roic_5y_avg_pct: roic_pct.map(Decimal::from),
        roic_5y_stddev_pct: None,
        gross_margin_5y_avg_pct: Some(Decimal::from(65)),
        gross_margin_5y_stddev_pct: None,
        share_change_3y_pct: Some(Decimal::from(-1)),
        sbc_to_ocf_pct: Some(Decimal::from(6)),
        net_debt_to_oe: Some(Decimal::ZERO),
        strict_owner_earnings: Some(Decimal::from(500_000_000)),
        warnings_count: 0,
        warnings_json: "[]".into(),
        raw_sha256: None,
    }
}

#[test]
fn kline_sandbox_overlay_timeframe_slicing_and_hover_mapping() {
    let rep = make_test_report(Decimal::from(5), Decimal::from(100));
    let overlay_18x = compute_kline_valuation_overlay(
        &rep,
        true,
        Decimal::from(18),
        Decimal::from(10),
        Decimal::from(8),
    );
    assert_eq!(overlay_18x.strike_price, Some(Decimal::from(90)));
    assert_eq!(overlay_18x.fv_high, Some(Decimal::from(110)));
    assert_eq!(
        overlay_18x.dist_to_strike_pct,
        Some(((Decimal::from(100) - Decimal::from(90)) / Decimal::from(90)) * Decimal::from(100))
    );

    let overlay_22x = compute_kline_valuation_overlay(
        &rep,
        true,
        Decimal::from(22),
        Decimal::from(10),
        Decimal::from(8),
    );
    assert_eq!(overlay_22x.strike_price, Some(Decimal::from(110)));
    assert_eq!(overlay_22x.fv_high, Some(Decimal::from(130)));
    assert!(
        overlay_22x
            .dist_to_strike_pct
            .is_some_and(|d| d < Decimal::ZERO)
    );

    let wk_bars: Vec<OhlcBar> = (0..220)
        .map(|i| OhlcBar {
            ts_ms: 1_700_000_000_000 + i * 604_800_000,
            open: Decimal::from(90 + (i % 15)),
            high: Decimal::from(95 + (i % 15)),
            low: Decimal::from(85 + (i % 15)),
            close: Decimal::from(92 + (i % 15)),
            volume: 1_250_000,
        })
        .collect();
    let d1_bars = wk_bars[..60].to_vec();
    let wk_snap = PriceHistorySnapshot {
        schema_version: 1,
        symbol: "TEST".into(),
        currency: "USD".into(),
        interval: "1wk".into(),
        range: "5y".into(),
        fetched_at_ms: 1_700_000_000_000,
        source_url: "https://example.com/wk".into(),
        bars: wk_bars,
    };
    let d1_snap = PriceHistorySnapshot {
        schema_version: 1,
        symbol: "TEST".into(),
        currency: "USD".into(),
        interval: "1d".into(),
        range: "1y".into(),
        fetched_at_ms: 1_700_000_000_000,
        source_url: "https://example.com/1d".into(),
        bars: d1_bars,
    };

    assert_eq!(
        select_kline_bars(
            Some(&wk_snap),
            Some(&d1_snap),
            ChartTimeframe::FiveYearWeekly
        )
        .len(),
        220
    );
    assert_eq!(
        select_kline_bars(
            Some(&wk_snap),
            Some(&d1_snap),
            ChartTimeframe::ThreeYearWeekly
        )
        .len(),
        156
    );
    assert_eq!(
        select_kline_bars(Some(&wk_snap), Some(&d1_snap), ChartTimeframe::OneYearDaily).len(),
        60
    );

    // Hover index mapping & Y-projection monotonicity
    assert_eq!(kline_hover_bar_index(-1.0, 500.0, 100), None);
    assert_eq!(kline_hover_bar_index(10.5, 500.0, 100), Some(0));
    assert_eq!(kline_hover_bar_index(495.0, 500.0, 100), Some(99));
    let y_high = kline_price_to_y(130.0, 80.0, 140.0, 228.0);
    let y_low = kline_price_to_y(90.0, 80.0, 140.0, 228.0);
    assert!(y_high < y_low);

    // Date & volume formatting
    assert_eq!(civil_ym_from_ms(1_700_000_000_000), "2023-11");
    assert_eq!(civil_ymd_from_ms(1_700_000_000_000), "2023-11-14");
    assert_eq!(fmt_volume_compact(1_550_000), "1.55M");
    assert_eq!(fmt_volume_compact(42_500), "42.5K");
}

#[test]
fn universe_scatter_coordinate_mapping_thresholds_and_hit_testing() {
    let empty_strikes = HashSet::new();
    let unpriced = make_universe_row("0000000001", "NOPRICE", 5, Some(25), None, None, false);
    assert!(build_scatter_model(std::slice::from_ref(&unpriced), &empty_strikes, None).is_none());

    let circle = make_universe_row("0000769397", "ADSK", 8, Some(32), Some(6), Some(15), true);
    let strike = make_universe_row(
        "0000000002",
        "CHEAP",
        6,
        Some(22),
        Some(9),
        Some(-12),
        false,
    );
    let screened = make_universe_row("0000000003", "POOL", 5, Some(16), Some(4), Some(8), false);
    let stale_strike = make_universe_row(
        "0000000005",
        "STALE",
        6,
        Some(20),
        Some(8),
        Some(-18),
        false,
    );
    let outlier = make_universe_row(
        "0000000004",
        "HYPER",
        5,
        Some(250),
        Some(45),
        Some(5),
        false,
    );

    let fresh_strikes = HashSet::from([strike.cik.clone()]);
    let model = build_scatter_model(
        &[unpriced, outlier, circle, strike, stale_strike, screened],
        &fresh_strikes,
        Some("0000769397"),
    )
    .expect("scatter model");

    assert_eq!(model.points.len(), 5);
    assert!(model.ref_roic_norm.is_some());
    assert!(model.ref_fcf_norm.is_some());
    assert_eq!(model.strike_count, 1);
    assert_eq!(model.watch_count, 1);
    assert!(model.clipped_count >= 1);
    assert!(model.x_max <= 80.0);
    assert!(model.y_max <= 25.0);
    let stale_pt = model.points.iter().find(|p| p.ticker == "STALE").unwrap();
    assert!(!stale_pt.in_strike_zone);
    let hyper = model.points.iter().find(|p| p.ticker == "HYPER").unwrap();
    assert!(hyper.clipped);
    assert!((0.0..=1.0).contains(&hyper.x_norm));
    assert!((0.0..=1.0).contains(&hyper.y_norm));

    // 4% FCF reference line sits at y=4.0 inside [y_min, y_max].
    let expected_ref_fcf = ((4.0 - model.y_min) / (model.y_max - model.y_min)) as f32;
    assert!((model.ref_fcf_norm.unwrap_or(-1.0) - expected_ref_fcf).abs() < 1e-5);

    // Selected point must sit at the end so it paints on top and wins hit-tests.
    let last = model.points.last().expect("selected last");
    assert_eq!(last.cik, "0000769397");
    assert!(last.is_selected);

    let (sel_x, sel_y) = scatter_point_px(last.x_norm, last.y_norm, 600.0, 116.0);
    let hit = hit_test_scatter_point(&model.points, sel_x + 2.0, sel_y - 2.0, 600.0, 116.0, 14.0);
    assert_eq!(hit, Some(model.points.len() - 1));
    assert_eq!(
        hit_test_scatter_point(&model.points, -10.0, 50.0, 600.0, 116.0, 14.0),
        None
    );
}

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
        status: if margin.is_some_and(|d| d <= Decimal::ZERO) {
            StrikeZoneStatus::InStrikeZone
        } else {
            StrikeZoneStatus::Wait
        },
        provenance_summary: Vec::new(),
    }
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
        if apply_interrogator_event(
            &mut streaming,
            &mut status_hint,
            &mut messages,
            &mut tape,
            &mut seq,
            ev,
            "AAPL",
            &reports,
            &dossiers,
            UiLang::Zh,
        ) {
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
fn universe_palette_action_matches_chinese_and_english_queries() {
    let items = vec![
        PaletteItem {
            group: "VIEW".into(),
            title: "全市场雷达 · US Lakehouse L0".into(),
            subtitle: "DuckDB 全美股筛选、ROIC × FCF 散点图与一键入观察池".into(),
            shortcut: Some("⌘2"),
            dot_color: SKY_FG,
            action: PaletteAction::SwitchTab(WorkspaceTab::Universe),
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
        PaletteAction::SwitchTab(WorkspaceTab::Universe)
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

    assert_eq!(
        signal_kind_badge_for(SignalKind::Ripple, UiLang::Zh).0,
        "隐空间传导"
    );
    assert_eq!(
        signal_status_badge_for(SignalStatus::Researching, UiLang::Zh).0,
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
            action: PaletteAction::SwitchTab(WorkspaceTab::Inbox),
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
        PaletteAction::SwitchTab(WorkspaceTab::Inbox)
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
    let mut last_reload = false;
    for ev in &events {
        let (reload, _) = apply_deep_research_event_compat(
            &mut running,
            &mut run_id,
            &mut status_note,
            &mut budget_blocked,
            &mut perspectives,
            &mut last_result,
            &mut tape,
            ev,
            UiLang::Zh,
        );
        last_reload = reload;
    }
    assert!(!running);
    assert!(last_reload);
    assert_eq!(perspectives[0].status, PerspectiveUiStatus::Completed);
    assert_eq!(perspectives[0].input_tokens, 500);
    assert_eq!(tape.last().unwrap().kind, TuringTapeKind::Out);

    let (blocked_reload, blocked_banner) = apply_deep_research_event_compat(
        &mut running,
        &mut run_id,
        &mut status_note,
        &mut budget_blocked,
        &mut perspectives,
        &mut last_result,
        &mut tape,
        &DeepResearchEvent::BudgetBlocked {
            run_id: "run-2".into(),
            ticker: "PTC".into(),
            reason: "月度预算已用尽".into(),
        },
        UiLang::Zh,
    );
    assert!(blocked_reload);
    assert!(blocked_banner.unwrap().1);
    assert_eq!(budget_blocked.as_deref(), Some("月度预算已用尽"));
    assert_eq!(tape.last().unwrap().kind, TuringTapeKind::Policy);

    let (default_lbl, default_col) = budget_watermark_summary_for(None, UiLang::Zh);
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
    let (ex_lbl, ex_col) = budget_watermark_summary_for(Some(&exhausted_report), UiLang::Zh);
    assert_eq!(ex_lbl, "本月 AI 预算 $10.00 / $10.00");
    assert_eq!(ex_col, CORAL_FG);

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

#[test]
fn sidebar_dock_notice_prioritizes_errors_then_active_tasks_then_info() {
    assert!(
        resolve_sidebar_dock_notice_for(
            None,
            false,
            None,
            false,
            None,
            false,
            false,
            None,
            false,
            false,
            false,
            false,
            UiLang::Zh,
        )
        .is_none()
    );

    let info = ("同步完成".to_owned(), false);
    let info_notice = resolve_sidebar_dock_notice_for(
        Some(&info),
        false,
        None,
        false,
        None,
        false,
        false,
        None,
        false,
        false,
        false,
        false,
        UiLang::Zh,
    )
    .unwrap();
    assert_eq!(info_notice.tag, "NOTICE");
    assert!(info_notice.dismissible);

    let sync_notice = resolve_sidebar_dock_notice_for(
        Some(&info),
        true,
        Some("Downloading companyfacts.zip (42%)"),
        false,
        None,
        false,
        false,
        None,
        false,
        false,
        false,
        false,
        UiLang::Zh,
    )
    .unwrap();
    assert_eq!(sync_notice.tag, "LAKE SYNC");
    assert!(!sync_notice.dismissible);
    assert!(sync_notice.message.contains("42%"));

    let checking_banner = ("正在检查 beta 通道更新…".to_owned(), false);
    let update_notice = resolve_sidebar_dock_notice_for(
        Some(&checking_banner),
        false,
        None,
        true,
        Some("catchup"),
        false,
        false,
        None,
        false,
        false,
        true,
        false,
        UiLang::Zh,
    )
    .unwrap();
    assert_eq!(update_notice.tag, "UPDATE");
    assert!(!update_notice.dismissible);
    assert_eq!(update_notice.message, "正在检查 beta 通道更新…");

    let err = ("预算门控拦截".to_owned(), true);
    let err_notice = resolve_sidebar_dock_notice_for(
        Some(&err),
        true,
        Some("progress"),
        true,
        Some("catchup"),
        true,
        true,
        Some("industry"),
        true,
        true,
        true,
        true,
        UiLang::Zh,
    )
    .unwrap();
    assert_eq!(err_notice.tag, "ERROR");
    assert_eq!(err_notice.dot_color, CORAL_FG);
    assert!(err_notice.dismissible);
}

#[test]
fn compact_sidebar_version_label_shortens_rolling_beta_without_truncating_stable() {
    assert_eq!(
        compact_sidebar_version_label("0.3.1-beta.20261124.6"),
        "v0.3.1-beta.6"
    );
    assert_eq!(compact_sidebar_version_label("0.3.0"), "v0.3.0");
}

#[test]
fn bilingual_ui_helpers_and_theme_token_invariants() {
    let en = UiLang::En;
    let notice = resolve_sidebar_dock_notice_for(
        None, false, None, false, None, false, false, None, false, false, false, false, en,
    );
    assert!(notice.is_none());

    let err_banner = ("Sync timeout".to_owned(), true);
    let err_notice = resolve_sidebar_dock_notice_for(
        Some(&err_banner),
        false,
        None,
        true,
        None,
        false,
        false,
        None,
        false,
        false,
        false,
        false,
        en,
    )
    .expect("error notice expected");
    assert_eq!(err_notice.dot_color, CORAL_FG);
    assert!(err_notice.dismissible);

    let syncing_notice = resolve_sidebar_dock_notice_for(
        None, true, None, false, None, false, false, None, false, false, false, false, en,
    )
    .expect("syncing notice expected");
    assert!(syncing_notice.message.contains("Syncing"));

    let (default_lbl, _) = budget_watermark_summary_for(None, en);
    assert_eq!(default_lbl, "AI Budget $0.00 / $10.00");

    let custom_report = BudgetStatusReport {
        config: BudgetConfig::default(),
        usage: MonthlyBudgetUsage {
            month_utc: "2026-11".into(),
            total_runs: 1,
            completed_runs: 1,
            blocked_runs: 0,
            failed_runs: 0,
            total_input_tokens: 1_000,
            total_output_tokens: 500,
            total_cost_usd: Decimal::new(250, 2),
            weekly_auto_runs: 0,
        },
        remaining_monthly_usd: Decimal::new(750, 2),
        remaining_monthly_tokens: 1_998_500,
        auto_trigger_remaining_weekly: 2,
    };
    let (used_lbl, _) = budget_watermark_summary_for(Some(&custom_report), en);
    assert_eq!(used_lbl, "AI Budget $2.50 / $10.00");

    assert_eq!(signal_kind_badge_for(SignalKind::Ripple, en).0, "Ripple");
    assert_eq!(
        signal_status_badge_for(SignalStatus::Researching, en).0,
        "Alan"
    );
    assert_eq!(Bucket::Core.short_label_for(en), "Core Strike");
    assert_eq!(StrikeZoneStatus::InStrikeZone.label_for(en), "Strike Zone");
    assert_eq!(fmt_time_cn_for(None, en), "Never");
    assert_eq!(PerspectiveUiStatus::Pending.label_for(en), "Pending");
    assert_eq!(UniversePreset::StrikeZone.label_for(en), "Strike Zone");
    assert_eq!(UniverseSearchMode::Similar.label_for(en), "Lookalike");

    // Theme token invariants
    assert_eq!(bucket_colors(Bucket::Core).0, EMERALD_FG);
    assert_eq!(status_colors(StrikeZoneStatus::WarningBlocked).0, CORAL_FG);
    assert_eq!(status_dot_color(StrikeZoneStatus::InStrikeZone), EMERALD_FG);
    assert_eq!(journal_action_colors(JournalAction::PaperBuy).0, EMERALD_FG);
    assert_eq!(
        industry_status_colors(IndustryStatus::InsideCircle).0,
        EMERALD_FG
    );
    assert_eq!(share_delta_color(Some(Decimal::from(-1)), -1), EMERALD_FG);
}

#[test]
fn m3_engraving_palette_and_notices_work_bilingually() {
    let root = temp_workspace("m3_engraving");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            store.set_language(UiLang::En);
            let items = build_palette_items_from_store(store);
            let money_hits = filter_palette_items(&items, "money");
            assert!(
                money_hits
                    .iter()
                    .any(|it| it.action == PaletteAction::ShowM3Engraving),
                "Command palette should surface M3 · Money Making Machine on 'money' query"
            );
            let m3_hits = filter_palette_items(&items, "m3");
            assert!(
                m3_hits
                    .iter()
                    .any(|it| it.action == PaletteAction::ShowM3Engraving),
                "Command palette should surface M3 · Money Making Machine on 'm3' query"
            );

            store.show_about_notice();
            let about_en = store.notice.get_untracked().expect("about notice");
            assert!(about_en.0.contains("Money · Making · Machine"));

            execute_palette_action(store, PaletteAction::ShowM3Engraving);
            let engraving_en = store.notice.get_untracked().expect("en engraving");
            assert!(engraving_en.0.contains("M³ = Money · Making · Machine"));
            assert!(engraving_en.0.contains("fat pitches"));

            store.set_language(UiLang::Zh);
            execute_palette_action(store, PaletteAction::ShowM3Engraving);
            let engraving_zh = store.notice.get_untracked().expect("zh engraving");
            assert!(engraving_zh.0.contains("M³ = Money · Making · Machine"));
            assert!(engraving_zh.0.contains("甜区好球"));
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn settings_storage_signals_scan_and_protect_workbench() {
    let root = temp_workspace("settings_storage");
    let paths = WorkspacePaths::from_root(root.clone());
    std::fs::write(paths.watchlist_path(), br#"{"companies":[]}"#).expect("write watchlist");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            let ws = store.paths.get_untracked();
            let rep = m3_core::storage::scan_workspace_storage(&ws, None);
            store.storage_report.set(Some(rep.clone()));

            let fetched = store
                .storage_report
                .get_untracked()
                .expect("storage report");
            assert_eq!(fetched.categories.len(), 5);
            let wb = fetched
                .categories
                .iter()
                .find(|c| c.category == m3_core::storage::StorageCategory::PersonalWorkbench)
                .expect("workbench category");
            assert!(!wb.cleanable);
            assert!(wb.bytes > 0);
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn update_progress_formats_stages_and_bytes_bilingually() {
    use m3_core::update::{UpdateProgress, UpdateStage};

    let dl = UpdateProgress::for_stage(
        UpdateStage::Downloading,
        5 * 1024 * 1024,
        Some(10 * 1024 * 1024),
    );
    let zh_dl = format_update_progress(&dl, UiLang::Zh);
    let en_dl = format_update_progress(&dl, UiLang::En);
    assert!(zh_dl.contains("5.0 MB / 10.0 MB (50%)"));
    assert!(en_dl.contains("5.0 MB / 10.0 MB (50%)"));

    let ver = UpdateProgress::for_stage(UpdateStage::Verifying, 0, None);
    assert!(format_update_progress(&ver, UiLang::Zh).contains("SHA-256"));
    assert!(format_update_progress(&ver, UiLang::En).contains("SHA-256"));

    let ext = UpdateProgress::for_stage(UpdateStage::Extracting, 0, None);
    assert!(format_update_progress(&ext, UiLang::Zh).contains("M3.app"));
    assert!(format_update_progress(&ext, UiLang::En).contains("M3.app"));
}

#[test]
fn onboarding_visibility_and_starter_helpers() {
    let empty_summary = UniverseSummary::default();
    let populated_summary = UniverseSummary {
        total_companies: 6120,
        ..UniverseSummary::default()
    };

    assert!(is_fresh_workspace_for_onboarding(true, None, false));
    assert!(is_fresh_workspace_for_onboarding(
        true,
        Some(&empty_summary),
        false
    ));
    assert!(!is_fresh_workspace_for_onboarding(false, None, false));
    assert!(!is_fresh_workspace_for_onboarding(
        true,
        Some(&populated_summary),
        false
    ));
    assert!(!is_fresh_workspace_for_onboarding(true, None, true));

    for lang in [UiLang::Zh, UiLang::En] {
        let line = starter_pack_summary_line(lang);
        for ticker in ["MCO", "COST", "AAPL", "KO", "VRSN", "DPZ", "AAL", "INTC"] {
            assert!(line.contains(ticker), "missing {ticker} in {line}");
        }
    }

    let root = temp_workspace("onboarding_store");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init empty workspace");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            assert!(store.startup_hydrated.get_untracked());
            assert!(store.should_show_onboarding());

            // Session dismiss hides onboarding without modifying workspace files.
            store.dismiss_onboarding();
            assert!(!store.should_show_onboarding());

            // Resetting dismissal and populating universe_summary hides onboarding.
            store.onboarding_dismissed.set(false);
            assert!(store.should_show_onboarding());
            store.universe_summary.set(Some(populated_summary.clone()));
            assert!(!store.should_show_onboarding());

            // Clearing universe_summary shows onboarding again until starter pack loads.
            store.universe_summary.set(None);
            assert!(store.should_show_onboarding());

            let ws = store.paths.get_untracked();
            let rep = m3_core::starter::apply_starter_pack(&ws).expect("apply starter pack");
            assert_eq!(rep.added_tickers.len(), 8);
            store.reload_from_disk();

            assert!(!store.should_show_onboarding());
            assert_eq!(store.reports.with_untracked(|r| r.len()), 8);
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn settings_alan_connection_drafts_and_provider_switching() {
    use m3_core::alan_connection::{
        AlanConnectionProfileSummary, AlanConnectionProvider, AlanConnectionStatus,
    };

    let root = temp_workspace("settings_alan_conn");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            // Default unconfigured state
            assert!(store.alan_conn_status.get_untracked().is_none());
            assert_eq!(
                store.alan_conn_provider.get_untracked(),
                AlanConnectionProvider::OpenRouter
            );
            assert_eq!(
                store.alan_conn_profile_id_draft.get_untracked(),
                "openrouter-main"
            );

            // Switching provider updates default profile ID and model when still on default values
            store.select_alan_conn_provider(AlanConnectionProvider::OpenAiResponses);
            assert_eq!(
                store.alan_conn_provider.get_untracked(),
                AlanConnectionProvider::OpenAiResponses
            );
            assert_eq!(
                store.alan_conn_profile_id_draft.get_untracked(),
                "openai-main"
            );
            assert_eq!(store.alan_conn_model_draft.get_untracked(), "gpt-5.4");

            // Applying loaded status populates drafts from the active default profile
            let status = AlanConnectionStatus {
                default_profile: Some("my-or".to_owned()),
                profiles: vec![
                    AlanConnectionProfileSummary {
                        id: "my-or".to_owned(),
                        provider: "openrouter".to_owned(),
                        model: Some("anthropic/claude-sonnet-4.6".to_owned()),
                        base_url: Some("https://openrouter.ai/api/v1".to_owned()),
                        has_secret: true,
                        masked_hint: Some("sk-o••••9876".to_owned()),
                        is_default: true,
                    },
                    AlanConnectionProfileSummary {
                        id: "work-oai".to_owned(),
                        provider: "openai_responses".to_owned(),
                        model: Some("gpt-5.4".to_owned()),
                        base_url: Some("https://proxy.example/v1".to_owned()),
                        has_secret: true,
                        masked_hint: Some("sk-p••••4321".to_owned()),
                        is_default: false,
                    },
                ],
            };
            store.apply_alan_connection_status(status);
            assert_eq!(
                store.alan_conn_provider.get_untracked(),
                AlanConnectionProvider::OpenRouter
            );
            assert_eq!(store.alan_conn_profile_id_draft.get_untracked(), "my-or");
            assert_eq!(
                store.alan_conn_model_draft.get_untracked(),
                "anthropic/claude-sonnet-4.6"
            );
            assert_eq!(store.alan_conn_base_url_draft.get_untracked(), "");

            // Switching to OpenAI loads the existing OpenAI profile & custom base URL
            store.select_alan_conn_provider(AlanConnectionProvider::OpenAiResponses);
            assert_eq!(store.alan_conn_profile_id_draft.get_untracked(), "work-oai");
            assert_eq!(store.alan_conn_model_draft.get_untracked(), "gpt-5.4");
            assert_eq!(
                store.alan_conn_base_url_draft.get_untracked(),
                "https://proxy.example/v1"
            );
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn update_check_state_machine_handles_channel_switch_and_stale_results() {
    use m3_core::update::{PassiveUpdateNotice, ReleaseChannel, UpdateReport};

    let root = temp_workspace("update_check_state_machine");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            store.update_channel.set(ReleaseChannel::Beta);
            store.update_checking.set(true);
            store.update_check_channel.set(Some(ReleaseChannel::Beta));
            store.update_check_seq.set(2);

            let stale_report = UpdateReport {
                status: "update_available".into(),
                channel: "stable".into(),
                previous_channel: Some("beta".into()),
                current_version: "0.4.2".into(),
                target_version: "0.4.2".into(),
                tag: "v0.4.2".into(),
                target: "aarch64-apple-darwin".into(),
                update_available: true,
                updated: false,
                checked_only: true,
                app_bundle_path: "/Applications/M3.app".into(),
                cli_link_path: None,
                source_label: "https://example.invalid".into(),
                asset: "m3.tar.gz".into(),
                archive_sha256: "0".repeat(64),
                binary_sha256: None,
                release_url: None,
                signed: false,
                notarized: false,
            };

            // Stale callback from seq=1 must be ignored and must not reset active seq=2 check.
            store.apply_update_check_outcome(1, ReleaseChannel::Stable, true, Ok(Ok(stale_report)));
            assert!(store.update_checking.get_untracked());
            assert_eq!(store.update_channel.get_untracked(), ReleaseChannel::Beta);
            assert!(store.update_notice.get_untracked().is_none());

            // Matching seq worker-panic outcome clears update_checking cleanly and sets error notice.
            store.apply_update_check_outcome(
                2,
                ReleaseChannel::Beta,
                true,
                Err("worker panicked: boom".to_owned()),
            );
            assert!(!store.update_checking.get_untracked());
            assert_eq!(store.update_check_channel.get_untracked(), None);
            assert!(
                store
                    .notice
                    .with_untracked(|n| n.as_ref().is_some_and(|(_, err)| *err))
            );

            // Switching channel while another check is still pending invalidates the older sequence.
            store.update_checking.set(true);
            store.update_check_channel.set(Some(ReleaseChannel::Beta));
            store.update_check_seq.set(3);
            store.update_notice.set(Some(PassiveUpdateNotice {
                latest_version: "0.4.3-beta.1".into(),
                channel: ReleaseChannel::Beta,
                tag: "beta".into(),
                is_channel_switch: false,
                release_url: None,
            }));
            store.update_channel.set(ReleaseChannel::Stable);
            store.update_check_seq.set(4);
            store.update_checking.set(false);
            store.update_check_channel.set(None);
            store.update_notice.set(None);

            // Older seq=3 response arriving later cannot bring back stale notice or change channel.
            store.apply_update_check_outcome(
                3,
                ReleaseChannel::Beta,
                false,
                Err("timed out".to_owned()),
            );
            assert!(!store.update_checking.get_untracked());
            assert_eq!(store.update_channel.get_untracked(), ReleaseChannel::Stable);
            assert!(store.update_notice.get_untracked().is_none());
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}
