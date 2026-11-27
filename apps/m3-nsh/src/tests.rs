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
use crate::icons;
use crate::palette::{
    build_palette_items_from_store, execute_palette_action, visible_palette_items_for,
};
use crate::platform_desktop::{SMOKED_GLASS_BLUR_RADIUS, build_app_menus, main_window_attributes};
use crate::stores::app_store::apply_interrogator_event;
use crate::stores::types::{
    AgentMessageRole, AgentUiMessage, PerspectiveUiStatus, SortCol, StatusQuickFilter,
    UniverseViewState, apply_ui_row_guards, default_deep_research_perspectives,
};
use crate::stores::{
    AlanCanvasTab, AlanScope, AlanStageMode, AppStore, BasketsViewMode, ChartTimeframe,
    CompanyViewTab, InboxFilter, InboxViewState, PaletteAction, PaletteItem, SettingsTab,
    UniversePreset, UniverseSearchMode, WorkspaceTab,
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

fn run_with_reactive_runtime<T>(
    setup: impl FnOnce() -> T + 'static,
    check: impl FnOnce(T) + 'static,
) {
    use nsh_core::element::{RustCall, RustOp, ToJs};
    use nsh_core::{ApproxTextEngine, Core, Op};
    use std::cell::RefCell;

    let mut core = Core::new(ApproxTextEngine, size(px(800.0), px(600.0)));
    let payload = RefCell::new(Some((setup, check)));
    core.apply(Op::Rust(RustOp {
        call: 1,
        run: RustCall::Op(Box::new(move |_| {
            if let Some((setup_fn, check_fn)) = payload.borrow_mut().take() {
                let (owner, made) = create_root(setup_fn);
                check_fn(made);
                owner.dispose();
            }
            Ok(Box::new(()) as Box<dyn ToJs>)
        })),
    }))
    .unwrap();
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

            store.open_company("AAPL", CompanyViewTab::FinancialsMemo);
            assert!(store.company_open.get());
            assert_eq!(store.selected_ticker.get(), "AAPL");
            assert_eq!(store.company_view_tab.get(), CompanyViewTab::FinancialsMemo);
            store.company_view_tab.set(CompanyViewTab::XbrlInspector);
            assert_eq!(store.company_view_tab.get(), CompanyViewTab::XbrlInspector);

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
    let empty_buckets = HashMap::new();
    let unpriced = make_universe_row("0000000001", "NOPRICE", 5, Some(25), None, None, false);
    assert!(
        build_scatter_model(
            std::slice::from_ref(&unpriced),
            &empty_strikes,
            &empty_buckets,
            None,
        )
        .is_none()
    );

    let circle = make_universe_row("0000769397", "ADSK", 8, Some(32), Some(6), Some(15), true);
    let core_strike = make_universe_row(
        "0000000002",
        "COREHIT",
        6,
        Some(22),
        Some(9),
        Some(-12),
        true,
    );
    let watch_discount = make_universe_row(
        "0000000006",
        "WATCHDISC",
        6,
        Some(24),
        Some(8),
        Some(-10),
        true,
    );
    let non_circle_discount =
        make_universe_row("0000000007", "CHEAP", 6, Some(21), Some(8), Some(-8), false);
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

    let fresh_strikes = HashSet::from([
        core_strike.cik.clone(),
        watch_discount.cik.clone(),
        non_circle_discount.cik.clone(),
    ]);
    let live_buckets = HashMap::from([
        ("ADSK".to_owned(), Bucket::Watch),
        ("COREHIT".to_owned(), Bucket::Core),
        ("WATCHDISC".to_owned(), Bucket::Watch),
    ]);
    let model = build_scatter_model(
        &[
            unpriced,
            outlier,
            circle,
            core_strike,
            watch_discount,
            non_circle_discount,
            stale_strike,
            screened,
        ],
        &fresh_strikes,
        &live_buckets,
        Some("0000769397"),
    )
    .expect("scatter model");

    assert_eq!(model.points.len(), 7);
    assert!(model.ref_roic_norm.is_some());
    assert!(model.ref_fcf_norm.is_some());
    assert_eq!(model.strike_count, 1);
    assert_eq!(model.watch_count, 3);
    assert_eq!(model.discount_count, 2);
    assert!(model.clipped_count >= 1);
    assert!(model.x_max <= 80.0);
    assert!(model.y_max <= 25.0);
    let core_pt = model.points.iter().find(|p| p.ticker == "COREHIT").unwrap();
    assert!(core_pt.in_strike_zone);
    assert!(!core_pt.in_watch_discount);
    let watch_pt = model
        .points
        .iter()
        .find(|p| p.ticker == "WATCHDISC")
        .unwrap();
    assert!(!watch_pt.in_strike_zone);
    assert!(watch_pt.in_watch_discount);
    let cheap_pt = model.points.iter().find(|p| p.ticker == "CHEAP").unwrap();
    assert!(!cheap_pt.in_strike_zone);
    assert!(cheap_pt.in_ref_discount);
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
    assert_eq!(UniversePreset::StrikeZone.label_for(en), "≤18x Discount");
    assert_eq!(UniverseSearchMode::Similar.label_for(en), "Lookalike");
    assert_eq!(
        UniverseSearchMode::Similar.label_for(UiLang::Zh),
        "相似公司"
    );

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
        move |store| {
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
                        project_id: None,
                        location: None,
                        has_secret: true,
                        masked_hint: Some("sk-o••••9876".to_owned()),
                        is_default: true,
                    },
                    AlanConnectionProfileSummary {
                        id: "work-oai".to_owned(),
                        provider: "openai_responses".to_owned(),
                        model: Some("gpt-5.4".to_owned()),
                        base_url: Some("https://proxy.example/v1".to_owned()),
                        project_id: None,
                        location: None,
                        has_secret: true,
                        masked_hint: Some("sk-p••••4321".to_owned()),
                        is_default: false,
                    },
                    AlanConnectionProfileSummary {
                        id: "vertex-prod".to_owned(),
                        provider: "google_gemini_generate_content".to_owned(),
                        model: Some("gemini-2.5-pro".to_owned()),
                        base_url: None,
                        project_id: Some("m3-gcp".to_owned()),
                        location: Some("us-central1".to_owned()),
                        has_secret: true,
                        masked_hint: Some("ADC · m3-gcp".to_owned()),
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

            // Switching to Vertex AI loads project_id & location
            store.select_alan_conn_provider(AlanConnectionProvider::GoogleVertex);
            assert_eq!(
                store.alan_conn_profile_id_draft.get_untracked(),
                "vertex-prod"
            );
            assert_eq!(store.alan_conn_project_id_draft.get_untracked(), "m3-gcp");
            assert_eq!(
                store.alan_conn_location_draft.get_untracked(),
                "us-central1"
            );

            // Switching to unconfigured ChatGPT & Magpie uses clean defaults
            store
                .alan_conn_profile_id_draft
                .set("openai-main".to_owned());
            store.alan_conn_model_draft.set("gpt-5.4".to_owned());
            store.select_alan_conn_provider(AlanConnectionProvider::Chatgpt);
            assert_eq!(
                store.alan_conn_profile_id_draft.get_untracked(),
                "chatgpt-main"
            );
            assert_eq!(store.alan_conn_model_draft.get_untracked(), "gpt-5.3-codex");
            store.select_alan_conn_provider(AlanConnectionProvider::Magpie);
            assert_eq!(
                store.alan_conn_profile_id_draft.get_untracked(),
                "magpie-main"
            );
            assert_eq!(store.alan_conn_model_draft.get_untracked(), "default");
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

#[test]
fn async_bridge_tasks_survive_transient_owner_disposal() {
    use crate::stores::async_bridge::{run_blocking_or_err, run_streamed_blocking};
    use nsh_core::element::{RustCall, RustOp, ToJs};
    use nsh_core::{ApproxTextEngine, Core, Op};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    fn run_in_core(core: &mut Core<ApproxTextEngine>, call: u32, f: impl FnOnce() + 'static) {
        let slot = RefCell::new(Some(f));
        core.apply(Op::Rust(RustOp {
            call,
            run: RustCall::Op(Box::new(move |_| {
                if let Some(cb) = slot.borrow_mut().take() {
                    cb();
                }
                Ok(Box::new(()) as Box<dyn ToJs>)
            })),
        }))
        .unwrap();
    }

    fn pump_until(core: &mut Core<ApproxTextEngine>, flag: &AtomicBool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !flag.load(Ordering::Acquire) && Instant::now() < deadline {
            core.run_component_tasks();
            if !flag.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(
            flag.load(Ordering::Acquire),
            "timed out waiting for async_bridge callback"
        );
    }

    let mut core = Core::new(ApproxTextEngine, size(px(800.0), px(600.0)));

    // Scenario A: Live transient owner (e.g. a `dynamic(...)` branch) is disposed before worker finishes.
    let (gate_a_tx, gate_a_rx) = mpsc::channel::<()>();
    let done_a = Arc::new(AtomicBool::new(false));
    let done_a_cb = done_a.clone();
    let sig_a = Rc::new(RefCell::new(None::<Signal<Option<Result<i32, String>>>>));
    let sig_a_setup = sig_a.clone();
    run_in_core(&mut core, 1, move || {
        let target = signal(None);
        *sig_a_setup.borrow_mut() = Some(target);
        let (transient_owner, ()) = create_root(|| {
            run_blocking_or_err(
                move || {
                    let _ = gate_a_rx.recv();
                    42
                },
                move |res| {
                    target.set(Some(res));
                    done_a_cb.store(true, Ordering::Release);
                },
            );
        });
        transient_owner.dispose();
    });
    gate_a_tx.send(()).unwrap();
    pump_until(&mut core, &done_a);
    let outcome_a = Rc::new(RefCell::new(None));
    let outcome_a_read = outcome_a.clone();
    let target_a = sig_a.borrow().unwrap();
    run_in_core(&mut core, 2, move || {
        *outcome_a_read.borrow_mut() = target_a.get_untracked();
    });
    assert_eq!(outcome_a.borrow().as_ref(), Some(&Ok(42)));

    // Scenario B: Caller runs inside an already-disposed owner (synchronous signal flush invalidated it).
    let done_b = Arc::new(AtomicBool::new(false));
    let done_b_cb = done_b.clone();
    let sig_b = Rc::new(RefCell::new(
        None::<Signal<Option<Result<&'static str, String>>>>,
    ));
    let sig_b_setup = sig_b.clone();
    run_in_core(&mut core, 3, move || {
        let target = signal(None);
        *sig_b_setup.borrow_mut() = Some(target);
        let (disposed_owner, ()) = create_root(|| {});
        disposed_owner.dispose();
        disposed_owner.run(|| {
            run_blocking_or_err(
                || "ok",
                move |res| {
                    target.set(Some(res));
                    done_b_cb.store(true, Ordering::Release);
                },
            );
        });
    });
    pump_until(&mut core, &done_b);
    let outcome_b = Rc::new(RefCell::new(None));
    let outcome_b_read = outcome_b.clone();
    let target_b = sig_b.borrow().unwrap();
    run_in_core(&mut core, 4, move || {
        *outcome_b_read.borrow_mut() = target_b.get_untracked();
    });
    assert_eq!(outcome_b.borrow().as_ref(), Some(&Ok("ok")));

    // Scenario C: Streamed blocking task survives transient owner disposal.
    let (gate_c_tx, gate_c_rx) = mpsc::channel::<()>();
    let done_c = Arc::new(AtomicBool::new(false));
    let done_c_cb = done_c.clone();
    let sig_c = Rc::new(Cell::new(None::<(Signal<Vec<i32>>, Signal<bool>)>));
    let sig_c_setup = sig_c.clone();
    run_in_core(&mut core, 5, move || {
        let events = signal(Vec::new());
        let finished = signal(false);
        sig_c_setup.set(Some((events, finished)));
        let (transient_owner, ()) = create_root(|| {
            run_streamed_blocking(
                move |tx| {
                    let _ = gate_c_rx.recv();
                    let _ = tx.send(10);
                    let _ = tx.send(20);
                },
                move |ev| {
                    events.update(|v| v.push(ev));
                },
                move || {
                    finished.set(true);
                    done_c_cb.store(true, Ordering::Release);
                },
            );
        });
        transient_owner.dispose();
    });
    gate_c_tx.send(()).unwrap();
    pump_until(&mut core, &done_c);
    let result_c = Rc::new(RefCell::new((Vec::new(), false)));
    let result_c_read = result_c.clone();
    let (events_c, finished_c) = sig_c.get().unwrap();
    run_in_core(&mut core, 6, move || {
        *result_c_read.borrow_mut() = (events_c.get_untracked(), finished_c.get_untracked());
    });
    assert_eq!(*result_c.borrow(), (vec![10, 20], true));
}

#[test]
fn settings_tabs_labels_default_and_switching() {
    use m3_core::alan_connection::AlanConnectionStatus;

    assert_eq!(
        SettingsTab::all(),
        [SettingsTab::General, SettingsTab::Ai, SettingsTab::Data]
    );
    assert_eq!(SettingsTab::General.label_for(UiLang::Zh), "常规");
    assert_eq!(SettingsTab::General.label_for(UiLang::En), "General");
    assert_eq!(SettingsTab::Ai.label_for(UiLang::Zh), "AI");
    assert_eq!(SettingsTab::Ai.label_for(UiLang::En), "AI");
    assert_eq!(SettingsTab::Data.label_for(UiLang::Zh), "数据");
    assert_eq!(SettingsTab::Data.label_for(UiLang::En), "Data");

    let root = temp_workspace("settings_tabs");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            assert_eq!(store.settings_tab.get_untracked(), SettingsTab::General);

            // Pre-populate cached signals so tab switching does not spawn background NSH tasks
            // inside `run_with_reactive_runtime` (which has no Core task executor).
            store.alan_conn_status.set(Some(AlanConnectionStatus {
                default_profile: None,
                profiles: Vec::new(),
            }));
            store.budget_report.set(Some(BudgetStatusReport {
                config: BudgetConfig::default(),
                usage: MonthlyBudgetUsage {
                    month_utc: "2026-11".into(),
                    total_runs: 0,
                    completed_runs: 0,
                    blocked_runs: 0,
                    failed_runs: 0,
                    total_input_tokens: 0,
                    total_output_tokens: 0,
                    total_cost_usd: Decimal::ZERO,
                    weekly_auto_runs: 0,
                },
                remaining_monthly_usd: Decimal::new(1000, 2),
                remaining_monthly_tokens: 2_000_000,
                auto_trigger_remaining_weekly: 3,
            }));
            store.storage_scanning.set(true);

            store.set_settings_tab(SettingsTab::Ai);
            assert_eq!(store.settings_tab.get_untracked(), SettingsTab::Ai);

            store.set_settings_tab(SettingsTab::Data);
            assert_eq!(store.settings_tab.get_untracked(), SettingsTab::Data);

            store.set_settings_tab(SettingsTab::General);
            assert_eq!(store.settings_tab.get_untracked(), SettingsTab::General);

            assert!(!store.settings_open.get_untracked());
            store.open_settings();
            assert!(store.settings_open.get_untracked());
            assert_eq!(
                store.active_tab.get_untracked(),
                WorkspaceTab::Inbox,
                "opening Settings modal must preserve the underlying workspace tab"
            );
            store.toggle_settings();
            assert!(!store.settings_open.get_untracked());
            store.open_tab(WorkspaceTab::Settings);
            assert!(store.settings_open.get_untracked());
            store.close_settings();
            assert!(!store.settings_open.get_untracked());
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn topbar_message_box_prioritizes_tasks_progress_and_dismissal() {
    use crate::format::{TopbarMessageInputs, build_topbar_messages};
    use crate::stores::{TopbarMessageAction, TopbarMessageStatus};
    use m3_core::update::PassiveUpdateNotice;

    let root = temp_workspace("topbar_message_box");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            // Shell toggles start closed/expanded and toggle cleanly.
            assert!(!store.sidebar_collapsed.get_untracked());
            assert!(!store.message_box_open.get_untracked());
            store.toggle_sidebar();
            assert!(store.sidebar_collapsed.get_untracked());
            store.toggle_message_box();
            assert!(store.message_box_open.get_untracked());
            // Opening Command Palette auto-closes Message Box popover.
            store.toggle_palette();
            assert!(store.palette_open.get_untracked());
            assert!(!store.message_box_open.get_untracked());

            // Stored messages deduplicate consecutive identical messages and support dismiss/clear.
            store.clear_messages();
            store.set_info_notice("Saved config".to_owned());
            store.set_info_notice("Saved config".to_owned());
            assert_eq!(store.messages.with_untracked(|v| v.len()), 1);
            store.set_error_notice("Network timeout".to_owned());
            assert_eq!(store.messages.with_untracked(|v| v.len()), 2);

            let update_notice = PassiveUpdateNotice {
                latest_version: "0.5.0".into(),
                channel: ReleaseChannel::Beta,
                tag: "beta".into(),
                is_channel_switch: false,
                release_url: None,
            };
            let stored_msgs = store.messages.get_untracked();

            // Active update + running sync + stored notices keep strict priority ordering and progress.
            let synthesized = build_topbar_messages(&TopbarMessageInputs {
                lang: UiLang::Zh,
                update_applied_version: None,
                update_applying: true,
                update_progress: Some("正在下载 v0.5.0 (4.2 MB / 10.0 MB)"),
                update_progress_fraction: Some(0.42),
                update_checking: false,
                update_notice: Some(&update_notice),
                universe_syncing: false,
                universe_progress: None,
                universe_progress_fraction: None,
                inbox_catching_up: false,
                inbox_progress: None,
                deep_research_running: true,
                deep_research_ticker: Some("COST"),
                industry_research_running: false,
                industry_progress: None,
                syncing: true,
                sync_stage: Some("更新行情快照"),
                sync_progress: Some(0.75),
                themes_syncing: false,
                storage_scanning: false,
                messages: &stored_msgs,
            });
            assert_eq!(synthesized.len(), 5);
            assert_eq!(synthesized[0].tag, "UPDATING");
            assert_eq!(synthesized[0].status, TopbarMessageStatus::Running);
            assert_eq!(synthesized[0].progress, Some(0.42));
            assert!(!synthesized[0].dismissible);

            assert_eq!(synthesized[1].tag, "ALAN L2");
            assert!(synthesized[1].title.contains("COST"));

            assert_eq!(synthesized[2].tag, "SYNCING");
            assert_eq!(synthesized[2].progress, Some(0.75));

            assert_eq!(synthesized[3].tag, "ERROR");
            assert_eq!(synthesized[3].status, TopbarMessageStatus::Error);
            assert!(synthesized[3].dismissible);

            assert_eq!(synthesized[4].tag, "NOTICE");
            assert_eq!(synthesized[4].status, TopbarMessageStatus::Info);

            // Applied update offers RestartAfterUpdate action at the top.
            let applied = build_topbar_messages(&TopbarMessageInputs {
                lang: UiLang::En,
                update_applied_version: Some("0.5.0"),
                update_applying: false,
                update_progress: None,
                update_progress_fraction: None,
                update_checking: false,
                update_notice: None,
                universe_syncing: false,
                universe_progress: None,
                universe_progress_fraction: None,
                inbox_catching_up: false,
                inbox_progress: None,
                deep_research_running: false,
                deep_research_ticker: None,
                industry_research_running: false,
                industry_progress: None,
                syncing: false,
                sync_stage: None,
                sync_progress: None,
                themes_syncing: false,
                storage_scanning: false,
                messages: &[],
            });
            assert_eq!(applied.len(), 1);
            assert_eq!(applied[0].status, TopbarMessageStatus::ActionRequired);
            assert_eq!(
                applied[0].action,
                Some(TopbarMessageAction::RestartAfterUpdate)
            );

            // Dismissing one stored message removes only that entry; clearing removes all.
            let first_id = stored_msgs[0].id;
            store.dismiss_message(first_id);
            assert_eq!(store.messages.with_untracked(|v| v.len()), 1);
            store.clear_messages();
            assert!(store.messages.with_untracked(|v| v.is_empty()));
            assert!(store.notice.get_untracked().is_none());
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn iconmcp_svg_assets_are_valid_current_color_markup() {
    let all_icons = [
        ("ICON_SIDEBAR_LEFT", icons::ICON_SIDEBAR_LEFT),
        ("ICON_ARROW_LEFT", icons::ICON_ARROW_LEFT),
        ("ICON_SEARCH", icons::ICON_SEARCH),
        ("ICON_BELL", icons::ICON_BELL),
        ("ICON_REFRESH", icons::ICON_REFRESH),
        ("ICON_SETTINGS", icons::ICON_SETTINGS),
        ("ICON_INBOX", icons::ICON_INBOX),
        ("ICON_RADAR", icons::ICON_RADAR),
        ("ICON_LAYERS", icons::ICON_LAYERS),
        ("ICON_TARGET", icons::ICON_TARGET),
        ("ICON_JOURNAL", icons::ICON_JOURNAL),
        ("ICON_CLOSE", icons::ICON_CLOSE),
        ("ICON_ARROW_CIRCLE_UP", icons::ICON_ARROW_CIRCLE_UP),
        ("ICON_CHECK", icons::ICON_CHECK),
        ("ICON_CIRCLE_CHECK", icons::ICON_CIRCLE_CHECK),
        ("ICON_ALERT", icons::ICON_ALERT),
        ("ICON_ENTER", icons::ICON_ENTER),
        ("ICON_CHEVRON_DOWN", icons::ICON_CHEVRON_DOWN),
        ("ICON_CHEVRON_RIGHT", icons::ICON_CHEVRON_RIGHT),
    ];

    for (name, markup) in all_icons {
        assert!(markup.starts_with("<svg "), "{name} must start with <svg");
        assert!(markup.ends_with("</svg>"), "{name} must end with </svg>");
        assert!(
            markup.contains("currentColor"),
            "{name} must inherit currentColor"
        );
    }
}

#[test]
fn main_window_attributes_uses_transparent_window_for_dark_frosted_glass() {
    let attrs = main_window_attributes();
    assert_eq!(attrs.transparent, Some(true));
    assert_eq!(
        attrs.backdrop,
        Some(nsh_core::desktop::Backdrop::None),
        "window backdrop must be None so M3SmokedGlassView + deep obsidian tint shine through cleanly"
    );
    assert!(
        (8.0..=20.0).contains(&SMOKED_GLASS_BLUR_RADIUS),
        "blur radius must stay low for crystal-clear translucency"
    );
    let glass_alpha = crate::theme::BG_SHELL_GLASS & 0xFF;
    assert!(
        (0xB0..0xE8).contains(&glass_alpha),
        "BG_SHELL_GLASS alpha must stay deep obsidian yet translucent"
    );
}

#[test]
fn cmd_held_defaults_false_and_palette_groups_reset_on_close() {
    let root = temp_workspace("m3_nsh_cmd_held_and_groups");
    let paths = WorkspacePaths::from_root(root.clone());
    run_with_reactive_runtime(
        move || AppStore::new(paths),
        |store| {
            assert!(!store.cmd_held.get_untracked());
            store.cmd_held.set(true);
            assert!(store.cmd_held.get_untracked());
            store.cmd_held.set(false);
            assert!(!store.cmd_held.get_untracked());

            store.open_palette();
            assert!(store.palette_open.get_untracked());
            assert!(
                store
                    .palette_expanded_groups
                    .with_untracked(|g| g.is_empty())
            );
            store.toggle_palette_group("ACTIONS");
            assert!(
                store
                    .palette_expanded_groups
                    .with_untracked(|g| g.contains("ACTIONS"))
            );
            store.toggle_palette_group("ACTIONS");
            assert!(
                store
                    .palette_expanded_groups
                    .with_untracked(|g| !g.contains("ACTIONS"))
            );

            store.toggle_palette_group("CIRCLE");
            store.close_palette();
            assert!(!store.palette_open.get_untracked());
            assert!(
                store
                    .palette_expanded_groups
                    .with_untracked(|g| g.is_empty())
            );
        },
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn visible_palette_items_respects_silicon_group_expansion() {
    let items = vec![
        PaletteItem {
            group: "VIEW".into(),
            title: "Inbox".into(),
            subtitle: "Signals".into(),
            shortcut: Some("⌘1"),
            dot_color: EMERALD_FG,
            action: PaletteAction::SwitchTab(WorkspaceTab::Inbox),
        },
        PaletteItem {
            group: "VIEW".into(),
            title: "Circle Overview".into(),
            subtitle: "Circle triage".into(),
            shortcut: Some("⌘4"),
            dot_color: SKY_FG,
            action: PaletteAction::FocusBasket(None),
        },
        PaletteItem {
            group: "DATA".into(),
            title: "Sync".into(),
            subtitle: "Sync financials".into(),
            shortcut: Some("⌘R"),
            dot_color: SKY_FG,
            action: PaletteAction::SyncNow,
        },
        PaletteItem {
            group: "DATA".into(),
            title: "Catchup".into(),
            subtitle: "Catchup L1".into(),
            shortcut: None,
            dot_color: SKY_FG,
            action: PaletteAction::CatchupEvents,
        },
        PaletteItem {
            group: "DOSSIER".into(),
            title: "COST Costco".into(),
            subtitle: "#Retail".into(),
            shortcut: Some("↵"),
            dot_color: EMERALD_FG,
            action: PaletteAction::OpenDossier("COST".into()),
        },
        PaletteItem {
            group: "DOSSIER".into(),
            title: "ODFL Old Dominion".into(),
            subtitle: "#Freight".into(),
            shortcut: Some("↵"),
            dot_color: SKY_FG,
            action: PaletteAction::OpenDossier("ODFL".into()),
        },
    ];

    let empty_exp = std::collections::HashSet::new();
    let vis_all = visible_palette_items_for(&items, "", &empty_exp);
    assert_eq!(
        vis_all.len(),
        6,
        "All 2 Views + 2 Actions + 2 Circles show without folding"
    );
    assert_eq!(vis_all[1].action, PaletteAction::FocusBasket(None));

    let vis_query = visible_palette_items_for(&items, "catchup", &empty_exp);
    assert_eq!(vis_query.len(), 1);
    assert_eq!(vis_query[0].title, "Catchup");
}

#[test]
fn mcp_config_snippets_and_progressive_universe_format_work() {
    use crate::format::{
        McpClientTarget, TopbarMessageInputs, build_mcp_config_snippet, build_topbar_messages,
        format_universe_sync_progress,
    };
    use m3_core::universe::{UniverseSyncProgress, UniverseSyncStage};

    let ws_default = std::path::PathBuf::from("/Users/test/default");
    assert_eq!(
        build_mcp_config_snippet(McpClientTarget::ClaudeCode, &ws_default, false),
        "claude mcp add m3 -- m3 mcp"
    );
    assert_eq!(
        build_mcp_config_snippet(McpClientTarget::Codex, &ws_default, false),
        "[mcp_servers.m3]\ncommand = \"m3\"\nargs = [\"mcp\"]"
    );

    let ws_custom = std::path::PathBuf::from("/Users/test/My M3's Folder");
    let claude_custom = build_mcp_config_snippet(McpClientTarget::ClaudeCode, &ws_custom, true);
    assert!(claude_custom.contains("--workspace"));
    assert!(claude_custom.contains("My M3"));

    let codex_custom = build_mcp_config_snippet(McpClientTarget::Codex, &ws_custom, true);
    assert!(codex_custom.contains("[mcp_servers.m3]"));
    assert!(codex_custom.contains("\"--workspace\""));

    let json_custom = build_mcp_config_snippet(McpClientTarget::JsonClients, &ws_custom, true);
    let parsed: serde_json::Value = serde_json::from_str(&json_custom).unwrap();
    assert_eq!(parsed["mcpServers"]["m3"]["command"], "m3");
    assert_eq!(
        parsed["mcpServers"]["m3"]["args"],
        serde_json::json!(["mcp", "--workspace", "/Users/test/My M3's Folder"])
    );

    let dl_prog = UniverseSyncProgress {
        stage: UniverseSyncStage::DownloadSec,
        fraction: 0.30,
        message: "downloading".to_owned(),
        downloaded_bytes: Some(50 * 1024 * 1024),
        total_bytes: Some(200 * 1024 * 1024),
        parsed_companies: None,
        quotes_done: None,
        quotes_total: None,
        lake_ready: false,
    };
    let zh = format_universe_sync_progress(&dl_prog, UiLang::Zh);
    let en = format_universe_sync_progress(&dl_prog, UiLang::En);
    assert!(zh.contains("30%"));
    assert!(en.contains("30%"));

    let msgs = build_topbar_messages(&TopbarMessageInputs {
        lang: UiLang::Zh,
        update_applied_version: None,
        update_applying: false,
        update_progress: None,
        update_progress_fraction: None,
        update_checking: false,
        update_notice: None,
        universe_syncing: true,
        universe_progress: Some(&zh),
        universe_progress_fraction: Some(0.72),
        inbox_catching_up: false,
        inbox_progress: None,
        deep_research_running: false,
        deep_research_ticker: None,
        industry_research_running: false,
        industry_progress: None,
        syncing: false,
        sync_stage: None,
        sync_progress: None,
        themes_syncing: false,
        storage_scanning: false,
        messages: &[],
    });
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].tag, "LAKE SYNC");
    assert_eq!(msgs[0].progress, Some(0.72));
}

#[test]
fn bucket_display_order_and_remove_company_from_circle_work() {
    use m3_core::model::{CompanyEntry, WatchlistFile};
    use m3_core::store::save_watchlist;

    assert_eq!(
        Bucket::display_order(),
        [Bucket::Watch, Bucket::Core, Bucket::Out, Bucket::TooHard]
    );

    let root = temp_workspace("remove_circle");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");
    let wl = WatchlistFile {
        schema_version: 1,
        benchmark_symbol: "^GSPC".into(),
        default_target_return_pct: Decimal::from(10),
        companies: vec![CompanyEntry {
            ticker: "HRMY".into(),
            name: "Harmony Biosciences".into(),
            bucket: Bucket::Watch,
            source: DataSource::SecEdgar,
            cik: Some("0001802665".into()),
            currency: "USD".into(),
            circle: "Pharma".into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(18),
                multiple_high: Decimal::from(22),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        }],
    };
    save_watchlist(&paths, &wl).expect("save watchlist");
    let dossier_path = paths.dossier_path("HRMY");
    std::fs::create_dir_all(dossier_path.parent().unwrap()).unwrap();
    std::fs::write(&dossier_path, "{}").unwrap();
    let dossier_check = dossier_path.clone();

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            assert_eq!(store.reports.with_untracked(|r| r.len()), 1);
            store.open_company("HRMY", CompanyViewTab::FinancialsMemo);
            assert!(store.company_open.get_untracked());

            store.remove_company_from_circle("HRMY");
            assert!(!store.company_open.get_untracked());
            assert!(store.reports.with_untracked(|r| r.is_empty()));
            assert!(dossier_check.exists());
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn watch_to_core_soft_gate_and_confirm_direct_graduation_work() {
    use m3_core::model::{CompanyEntry, WatchlistFile};
    use m3_core::store::{apply_feynman_sign_off, load_watchlist, save_watchlist};

    let root = temp_workspace("soft_gate_core");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");
    let wl = WatchlistFile {
        schema_version: 1,
        benchmark_symbol: "^GSPC".into(),
        default_target_return_pct: Decimal::from(10),
        companies: vec![CompanyEntry {
            ticker: "HRMY".into(),
            name: "Harmony Biosciences".into(),
            bucket: Bucket::Watch,
            source: DataSource::SecEdgar,
            cik: Some("0001802665".into()),
            currency: "USD".into(),
            circle: "Pharma".into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(18),
                multiple_high: Decimal::from(22),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        }],
    };
    save_watchlist(&paths, &wl).expect("save watchlist");

    let paths_check = paths.clone();
    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            // Missing dossier / feynman_checked_at triggers Soft Gate without moving bucket.
            store.request_bucket_change("HRMY", Bucket::Core);
            assert_eq!(
                store.pending_graduation_ticker.get_untracked().as_deref(),
                Some("HRMY")
            );
            let wl_before = load_watchlist(&paths_check).expect("load watchlist");
            assert_eq!(wl_before.companies[0].bucket, Bucket::Watch);

            // Explicit human override moves directly to Core and clears modal.
            store.confirm_direct_graduation("HRMY");
            assert_eq!(store.pending_graduation_ticker.get_untracked(), None);
            let wl_after = load_watchlist(&paths_check).expect("load watchlist");
            assert_eq!(wl_after.companies[0].bucket, Bucket::Core);

            // Move back to Watch, complete Feynman sign-off, then promote directly without gate.
            store.request_bucket_change("HRMY", Bucket::Watch);
            assert_eq!(store.pending_graduation_ticker.get_untracked(), None);
            apply_feynman_sign_off(
                &paths_check,
                "HRMY",
                "Durable narcolepsy franchise".into(),
                vec!["Wakix Rx volume declines YoY".into()],
                "2026-11-26".into(),
            )
            .expect("sign off");

            store.request_bucket_change("HRMY", Bucket::Core);
            assert_eq!(store.pending_graduation_ticker.get_untracked(), None);
            let wl_final = load_watchlist(&paths_check).expect("load watchlist");
            assert_eq!(wl_final.companies[0].bucket, Bucket::Core);
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn falsification_metric_crud_persists_deduplicated_metrics() {
    use m3_core::store::load_dossier;

    let root = temp_workspace("falsification_crud");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");
    let paths_check = paths.clone();

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            store.selected_ticker.set("AAPL".to_owned());
            store.add_falsification_metric("AAPL", "Gross margin < 42% for 2 quarters");
            store.add_falsification_metric("AAPL", "Gross margin < 42% for 2 quarters");
            store.add_falsification_metric("AAPL", "SBC / OCF > 15%");

            let dos = load_dossier(&paths_check, "AAPL")
                .expect("load dossier")
                .expect("dossier created");
            assert_eq!(
                dos.human_judgment.falsification_metrics,
                vec![
                    "Gross margin < 42% for 2 quarters".to_owned(),
                    "SBC / OCF > 15%".to_owned(),
                ]
            );
            assert_eq!(
                store
                    .selected_dossier
                    .get_untracked()
                    .map(|d| d.human_judgment.falsification_metrics),
                Some(vec![
                    "Gross margin < 42% for 2 quarters".to_owned(),
                    "SBC / OCF > 15%".to_owned(),
                ])
            );

            store.remove_falsification_metric("AAPL", 0);
            let dos_after = load_dossier(&paths_check, "AAPL")
                .expect("load dossier")
                .expect("dossier exists");
            assert_eq!(
                dos_after.human_judgment.falsification_metrics,
                vec!["SBC / OCF > 15%".to_owned()]
            );
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn inbox_view_mode_and_noise_collapse_defaults_and_toggle() {
    use crate::stores::InboxViewMode;
    use crate::stores::app_store::resolve_default_inbox_view_mode;

    assert_eq!(
        resolve_default_inbox_view_mode(false),
        InboxViewMode::Stream
    );
    assert_eq!(
        resolve_default_inbox_view_mode(true),
        InboxViewMode::Briefing
    );

    let root = temp_workspace("inbox_defaults");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            // Fresh workspace with 0 briefings opens Stream by default so Inbox is not empty.
            assert_eq!(store.inbox_view_mode.get_untracked(), InboxViewMode::Stream);
            assert!(!store.inbox_noise_expanded.get_untracked());

            store.set_inbox_view_mode(InboxViewMode::Briefing);
            assert_eq!(
                store.inbox_view_mode.get_untracked(),
                InboxViewMode::Briefing
            );

            store.toggle_inbox_noise_expanded();
            assert!(store.inbox_noise_expanded.get_untracked());
            store.toggle_inbox_noise_expanded();
            assert!(!store.inbox_noise_expanded.get_untracked());
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn landing_onboarding_auto_open_skip_and_step_navigation_work() {
    use crate::stores::OnboardingStep;
    use crate::stores::app_store::should_auto_open_landing_onboarding;

    assert!(should_auto_open_landing_onboarding(false, true, false));
    assert!(!should_auto_open_landing_onboarding(true, true, false));
    assert!(!should_auto_open_landing_onboarding(false, false, false));
    assert!(!should_auto_open_landing_onboarding(false, true, true));

    let root = temp_workspace("landing_onboarding");
    let paths = WorkspacePaths::from_root(root.clone());

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            assert!(!store.landing_onboarding_open.get_untracked());
            store.open_landing_onboarding();
            assert!(store.landing_onboarding_open.get_untracked());
            assert_eq!(
                store.landing_onboarding_step.get_untracked(),
                OnboardingStep::WelcomeConcepts
            );

            store.set_landing_onboarding_step(OnboardingStep::ConnectAlan);
            assert_eq!(
                store.landing_onboarding_step.get_untracked(),
                OnboardingStep::ConnectAlan
            );

            store.complete_landing_onboarding();
            assert!(!store.landing_onboarding_open.get_untracked());
            assert!(store.onboarding_completed.get_untracked());
            assert!(!store.should_show_onboarding());
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn save_proactive_settings_persists_to_budget_json() {
    use m3_core::budget::load_budget_config;

    let root = temp_workspace("proactive_settings");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");
    let paths_check = paths.clone();

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            store.save_proactive_settings(false, 20, 2, 12);
            let cfg = load_budget_config(&paths_check).expect("load budget.json");
            assert!(!cfg.proactive_triage_enabled);
            assert_eq!(cfg.daily_triage_quota, 20);
            assert_eq!(cfg.daily_briefing_summary_quota, 2);
            assert_eq!(cfg.background_refresh_hours, 12);
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tray_template_icon_and_quiet_status_runs_match_spec() {
    use crate::icons::ICON_TRAY_M3;
    use crate::platform_desktop::tray_status_runs;

    assert!(ICON_TRAY_M3.contains("viewBox=\"0 0 18 18\""));
    assert!(ICON_TRAY_M3.contains("fill=\"#000000\""));
    assert!(!ICON_TRAY_M3.contains("#0B0B0E"));

    assert!(tray_status_runs(0, 0).is_empty());
    assert_eq!(tray_status_runs(1, 0).len(), 1);
    assert_eq!(tray_status_runs(0, 1).len(), 1);
    assert_eq!(tray_status_runs(2, 3).len(), 3);
}

#[test]
fn strike_zone_counts_and_filters_require_core_in_bucket_only() {
    use crate::platform_desktop::count_circle_tray_badges;
    use m3_core::proactive::{SignalTriageRecord, TriageVerdict};

    let mut core_hit = make_rep("CORE1", Bucket::Core, "Tech", Some(Decimal::from(25)), None);
    core_hit.status = StrikeZoneStatus::InStrikeZone;

    let mut watch_hit = make_rep(
        "WATCH1",
        Bucket::Watch,
        "Tech",
        Some(Decimal::from(25)),
        None,
    );
    watch_hit.status = StrikeZoneStatus::InStrikeZone;

    let reports = vec![core_hit, watch_hit];

    let inbox_signals = vec![
        SignalInboxRow {
            signal_id: "sig-c".into(),
            kind: SignalKind::Strike,
            status: SignalStatus::Unread,
            primary_ticker: "CORE1".into(),
            tickers: vec!["CORE1".into()],
            title: "Strike".into(),
            summary: "Core alert".into(),
            evidence: vec![],
            score: 0.9,
            event_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        },
        SignalInboxRow {
            signal_id: "sig-w".into(),
            kind: SignalKind::Strike,
            status: SignalStatus::Unread,
            primary_ticker: "WATCH1".into(),
            tickers: vec!["WATCH1".into()],
            title: "Watch".into(),
            summary: "Watch alert".into(),
            evidence: vec![],
            score: 0.9,
            event_id: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        },
    ];
    let triages = vec![
        SignalTriageRecord {
            signal_id: "sig-c".into(),
            ticker: "CORE1".into(),
            verdict: TriageVerdict::FalsificationAlert,
            headline: "Core falsification".into(),
            rationale: "test".into(),
            matched_metrics: vec![],
            suggested_metrics: vec![],
            quotes: vec![],
            fallback_no_metrics_warning: false,
            model_used: "mock".into(),
            tokens_used: 0,
            cost_usd: Decimal::ZERO,
            created_at_ms: 1,
        },
        SignalTriageRecord {
            signal_id: "sig-w".into(),
            ticker: "WATCH1".into(),
            verdict: TriageVerdict::FalsificationAlert,
            headline: "Watch falsification".into(),
            rationale: "test".into(),
            matched_metrics: vec![],
            suggested_metrics: vec![],
            quotes: vec![],
            fallback_no_metrics_warning: false,
            model_used: "mock".into(),
            tokens_used: 0,
            cost_usd: Decimal::ZERO,
            created_at_ms: 1,
        },
    ];

    // Tray counts falsification alerts for WATCH + IN, but Strike Zone ONLY for IN (Core).
    let (alerts, strikes) = count_circle_tray_badges(&reports, &inbox_signals, &triages);
    assert_eq!(alerts, 2);
    assert_eq!(strikes, 1);

    // Basket StrikeOnly filter matches only Core, excluding Watch even if InStrikeZone.
    let core_filtered = filter_reports_for_basket(
        &reports,
        Bucket::Core,
        None,
        StatusQuickFilter::StrikeOnly,
        SortCol::Default,
        false,
    );
    assert_eq!(core_filtered.len(), 1);
    assert_eq!(core_filtered[0].ticker, "CORE1");

    let watch_filtered = filter_reports_for_basket(
        &reports,
        Bucket::Watch,
        None,
        StatusQuickFilter::StrikeOnly,
        SortCol::Default,
        false,
    );
    assert!(watch_filtered.is_empty());
}

#[test]
fn delete_industry_removes_profile_and_clears_selection() {
    let root = temp_workspace("delete_industry_ui");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            store.open_or_create_industry_from_topic("Medical Devices");
            let slug = store
                .selected_industry_slug
                .get_untracked()
                .expect("created industry slug");
            assert_eq!(store.industries.get_untracked().len(), 1);

            store.delete_industry(&slug);
            assert!(store.industries.get_untracked().is_empty());
            assert!(store.selected_industry_slug.get_untracked().is_none());
            assert!(store.industry_delete_confirm_slug.get_untracked().is_none());
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn journal_gui_composer_validates_and_records_entries() {
    use crate::stores::JournalViewTab;

    assert_eq!(JournalViewTab::default(), JournalViewTab::Ledger);

    let root = temp_workspace("journal_gui_composer");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            store.open_journal_form(None);
            assert!(store.journal_form_open.get_untracked());

            // Missing ticker -> validation error
            store.submit_journal_entry();
            assert!(store.journal_form_error.get_untracked().is_some());

            // Missing rationale -> validation error
            store.journal_draft_ticker.set("cost".into());
            store.submit_journal_entry();
            assert!(store.journal_form_error.get_untracked().is_some());

            // PaperBuy with missing/invalid shares -> validation error
            store.journal_draft_action.set(JournalAction::PaperBuy);
            store.journal_draft_note.set("Durable club moat".into());
            store.submit_journal_entry();
            assert!(store.journal_form_error.get_untracked().is_some());

            // Valid Wait entry records into append-only journal and resets form
            store.journal_draft_action.set(JournalAction::Wait);
            store
                .journal_draft_falsification
                .set("Renewal drops below 90%".into());
            store.submit_journal_entry();

            assert!(!store.journal_form_open.get_untracked());
            assert!(store.journal_form_error.get_untracked().is_none());
            assert!(store.journal_draft_ticker.get_untracked().is_empty());
            let j = store.journal.get_untracked();
            assert_eq!(j.entries.len(), 1);
            assert_eq!(j.entries[0].ticker, "COST");
            assert_eq!(j.entries[0].action, JournalAction::Wait);
            assert_eq!(j.entries[0].rationale, "Durable club moat");
            assert_eq!(
                j.entries[0].falsification_trigger,
                "Renewal drops below 90%"
            );
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn alan_stage_mode_and_shortcut_transitions_work() {
    let root = temp_workspace("alan_stage_shortcuts");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            // Global launch default is Closed on overview tabs.
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);
            assert!(!store.alan_stage_mode.get_untracked().is_open());
            assert_eq!(
                store.alan_canvas_tab.get_untracked(),
                AlanCanvasTab::Interrogate
            );
            assert!(!store.alan_context_pinned.get_untracked());

            // ⌘L toggles Closed <-> Split.
            store.toggle_alan_split();
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);
            assert!(store.alan_stage_mode.get_untracked().is_open());

            // ⇧⌘L toggles Split <-> Immersive.
            store.toggle_alan_immersive();
            assert_eq!(
                store.alan_stage_mode.get_untracked(),
                AlanStageMode::Immersive
            );

            // Esc in Immersive steps down to Split without closing Company Desk.
            store.open_company("COST", CompanyViewTab::FinancialsMemo);
            store.toggle_alan_immersive();
            assert_eq!(
                store.alan_stage_mode.get_untracked(),
                AlanStageMode::Immersive
            );
            assert!(store.company_open.get_untracked());

            if store.alan_stage_mode.get_untracked() == AlanStageMode::Immersive {
                store.set_alan_stage_mode(AlanStageMode::Split);
            }
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);
            assert!(store.company_open.get_untracked());

            // ⌘L from Immersive also returns to Split; ⌘L from Split closes to Closed.
            store.toggle_alan_immersive();
            assert_eq!(
                store.alan_stage_mode.get_untracked(),
                AlanStageMode::Immersive
            );
            store.toggle_alan_split();
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);
            store.toggle_alan_split();
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);

            // Direct ⇧⌘L from Closed enters Immersive; close_alan_canvas returns to Closed.
            store.toggle_alan_immersive();
            assert_eq!(
                store.alan_stage_mode.get_untracked(),
                AlanStageMode::Immersive
            );
            store.close_alan_canvas();
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn alan_split_preference_remembered_in_deep_research_vs_overview_tabs() {
    let root = temp_workspace("alan_split_pref");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            // Overview tabs default Closed and do not remember Split across tab switches.
            assert!(!store.is_deep_research_view());
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);
            store.toggle_alan_split();
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);
            assert!(!store.alan_deep_research_split_pref.get_untracked());

            store.open_tab(WorkspaceTab::Universe);
            assert!(!store.is_deep_research_view());
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);

            // Entering Industry Studio (deep research view) starts Closed until user opens Split.
            store.open_tab(WorkspaceTab::Industry);
            assert!(store.is_deep_research_view());
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);

            // Opening Split in Industry remembers preference for Company Desk & Industry Studio.
            store.toggle_alan_split();
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);
            assert!(store.alan_deep_research_split_pref.get_untracked());

            // Switching to an overview tab hides Alan back to Closed.
            store.open_tab(WorkspaceTab::Baskets);
            assert!(!store.is_deep_research_view());
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);
            assert!(store.alan_deep_research_split_pref.get_untracked());

            // Entering Company Desk restores remembered Split preference.
            store.open_company("AAPL", CompanyViewTab::FinancialsMemo);
            assert!(store.is_deep_research_view());
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);

            // Closing Split in Company Desk updates preference to false.
            store.toggle_alan_split();
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);
            assert!(!store.alan_deep_research_split_pref.get_untracked());

            store.close_company_overlay();
            store.open_tab(WorkspaceTab::Industry);
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Closed);
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn alan_pin_lock_vs_auto_follow_scope_and_anchor_handoff() {
    let root = temp_workspace("alan_pin_scope");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            // Unpinned auto-follow tracks Inbox -> Universe -> Industry -> Company.
            assert_eq!(
                store.alan_scope.get_untracked(),
                AlanScope::Briefing("latest".into())
            );

            store.open_tab(WorkspaceTab::Universe);
            assert_eq!(store.alan_scope.get_untracked(), AlanScope::Portfolio);

            store.open_tab(WorkspaceTab::Industry);
            store.open_or_create_industry_from_topic("Specialty Pharma");
            let ind_slug = store
                .selected_industry_slug
                .get_untracked()
                .expect("industry slug");
            assert_eq!(
                store.alan_scope.get_untracked(),
                AlanScope::Industry(ind_slug.clone())
            );

            store.open_company("HRMY", CompanyViewTab::FinancialsMemo);
            assert_eq!(
                store.alan_scope.get_untracked(),
                AlanScope::Company("HRMY".into())
            );

            // Open with anchor sets Split, scope, anchor label, and prefilled prompt.
            store.open_alan_with_anchor(
                AlanScope::Company("HRMY".into()),
                "Red Flag · SBC",
                "Inspect HRMY SBC/OCF trend",
            );
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);
            assert_eq!(
                store.alan_anchor_label.get_untracked().as_deref(),
                Some("Red Flag · SBC")
            );
            assert_eq!(
                store.agent_input_draft.get_untracked(),
                "Inspect HRMY SBC/OCF trend"
            );

            // Pinning freezes AlanScope and keeps Split open even across overview tabs.
            store.toggle_alan_pin();
            assert!(store.alan_context_pinned.get_untracked());

            store.close_company_overlay();
            store.open_tab(WorkspaceTab::Journal);
            assert_eq!(
                store.alan_scope.get_untracked(),
                AlanScope::Company("HRMY".into())
            );
            assert_eq!(store.alan_stage_mode.get_untracked(), AlanStageMode::Split);

            // Unpinning immediately snaps AlanScope back to active view context.
            store.toggle_alan_pin();
            assert!(!store.alan_context_pinned.get_untracked());
            assert_eq!(store.alan_scope.get_untracked(), AlanScope::Portfolio);

            // Explicit Feynman handoff for Industry switches scope and Feynman tab.
            store.open_alan_for_scope(
                AlanScope::Industry(ind_slug.clone()),
                AlanCanvasTab::Feynman,
            );
            assert_eq!(
                store.alan_scope.get_untracked(),
                AlanScope::Industry(ind_slug)
            );
            assert_eq!(
                store.alan_canvas_tab.get_untracked(),
                AlanCanvasTab::Feynman
            );
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn control_ipc_dispatch_handles_ping_state_navigate_set_alan_and_actions() {
    use crate::control_ipc::dispatch_app_control_request;
    use m3_core::app_control::AppControlRequest;

    let root = temp_workspace("control_ipc_dispatch");
    let paths = WorkspacePaths::from_root(root.clone());
    m3_core::app_config::ensure_initialized_workspace(&root).expect("init workspace");

    run_with_reactive_runtime(
        move || AppStore::new(paths),
        move |store| {
            // Ping & GetState
            let ping = dispatch_app_control_request(store, AppControlRequest::Ping);
            assert!(ping.ok);
            assert_eq!(ping.message.as_deref(), Some("pong"));
            let ping_state = ping.state.expect("ping state");
            assert_eq!(ping_state.active_tab, "inbox");
            assert_eq!(ping_state.alan_stage_mode, "closed");

            let state_resp = dispatch_app_control_request(store, AppControlRequest::GetState);
            assert!(state_resp.ok);
            assert!(state_resp.message.is_none());
            assert_eq!(
                state_resp.state.expect("get_state").alan_scope,
                "briefing:latest"
            );

            // Navigate: switch tab, open company overlay, close company overlay, select industry
            let nav_uni = dispatch_app_control_request(
                store,
                AppControlRequest::Navigate {
                    tab: Some("universe".to_owned()),
                    ticker: None,
                    industry: None,
                },
            );
            assert!(nav_uni.ok);
            let st = nav_uni.state.expect("nav universe state");
            assert_eq!(st.active_tab, "universe");
            assert!(!st.company_open);

            let nav_company = dispatch_app_control_request(
                store,
                AppControlRequest::Navigate {
                    tab: Some("baskets".to_owned()),
                    ticker: Some("aapl".to_owned()),
                    industry: None,
                },
            );
            assert!(nav_company.ok);
            let st = nav_company.state.expect("nav company state");
            assert_eq!(st.active_tab, "baskets");
            assert!(st.company_open);
            assert_eq!(st.selected_ticker, "AAPL");

            let nav_close_company = dispatch_app_control_request(
                store,
                AppControlRequest::Navigate {
                    tab: None,
                    ticker: Some(String::new()),
                    industry: None,
                },
            );
            assert!(nav_close_company.ok);
            assert!(!nav_close_company.state.expect("closed").company_open);

            let nav_ind = dispatch_app_control_request(
                store,
                AppControlRequest::Navigate {
                    tab: None,
                    ticker: None,
                    industry: Some("specialty-pharma".to_owned()),
                },
            );
            assert!(nav_ind.ok);
            let st = nav_ind.state.expect("nav industry state");
            assert_eq!(st.active_tab, "industry");
            assert_eq!(
                st.selected_industry_slug.as_deref(),
                Some("specialty-pharma")
            );
            assert_eq!(st.alan_scope, "industry:specialty-pharma");

            let nav_settings = dispatch_app_control_request(
                store,
                AppControlRequest::Navigate {
                    tab: Some("settings".to_owned()),
                    ticker: None,
                    industry: None,
                },
            );
            assert!(nav_settings.ok);
            assert_eq!(nav_settings.state.expect("settings").active_tab, "settings");

            let nav_bad = dispatch_app_control_request(
                store,
                AppControlRequest::Navigate {
                    tab: Some("unknown_tab".to_owned()),
                    ticker: None,
                    industry: None,
                },
            );
            assert!(!nav_bad.ok);
            assert!(nav_bad.error.is_some());

            // SetAlan: split, pin, scope, anchor, prompt -> immersive + feynman -> closed
            let alan_split = dispatch_app_control_request(
                store,
                AppControlRequest::SetAlan {
                    mode: Some("split".to_owned()),
                    tab: Some("interrogate".to_owned()),
                    scope: Some("company:cost".to_owned()),
                    pin: Some(true),
                    anchor_label: Some("ROIC".to_owned()),
                    prompt: Some("Inspect Costco moat".to_owned()),
                    submit: false,
                },
            );
            assert!(alan_split.ok);
            let st = alan_split.state.expect("alan split state");
            assert_eq!(st.alan_stage_mode, "split");
            assert_eq!(st.alan_canvas_tab, "interrogate");
            assert_eq!(st.alan_scope, "company:COST");
            assert!(st.alan_context_pinned);
            assert_eq!(
                store.alan_anchor_label.get_untracked().as_deref(),
                Some("ROIC")
            );
            assert_eq!(
                store.agent_input_draft.get_untracked(),
                "Inspect Costco moat"
            );

            let alan_imm = dispatch_app_control_request(
                store,
                AppControlRequest::SetAlan {
                    mode: Some("immersive".to_owned()),
                    tab: Some("feynman".to_owned()),
                    scope: None,
                    pin: None,
                    anchor_label: None,
                    prompt: None,
                    submit: false,
                },
            );
            assert!(alan_imm.ok);
            let st = alan_imm.state.expect("alan immersive state");
            assert_eq!(st.alan_stage_mode, "immersive");
            assert_eq!(st.alan_canvas_tab, "feynman");
            assert_eq!(st.alan_scope, "company:COST");
            assert!(store.feynman_mode_active.get_untracked());

            let alan_closed = dispatch_app_control_request(
                store,
                AppControlRequest::SetAlan {
                    mode: Some("closed".to_owned()),
                    tab: None,
                    scope: Some("portfolio".to_owned()),
                    pin: Some(false),
                    anchor_label: Some(String::new()),
                    prompt: None,
                    submit: false,
                },
            );
            assert!(alan_closed.ok);
            let st = alan_closed.state.expect("alan closed state");
            assert_eq!(st.alan_stage_mode, "closed");
            assert_eq!(st.alan_scope, "portfolio");
            assert!(!st.alan_context_pinned);
            assert!(store.alan_anchor_label.get_untracked().is_none());

            let alan_bad_scope = dispatch_app_control_request(
                store,
                AppControlRequest::SetAlan {
                    mode: None,
                    tab: None,
                    scope: Some("invalid_scope".to_owned()),
                    pin: None,
                    anchor_label: None,
                    prompt: None,
                    submit: false,
                },
            );
            assert!(!alan_bad_scope.ok);
            assert!(alan_bad_scope.error.is_some());

            // TriggerAction: toggle_sidebar, open_palette, close_palette, close_company, reset_alan
            let initial_collapsed = store.sidebar_collapsed.get_untracked();
            let toggle_sb = dispatch_app_control_request(
                store,
                AppControlRequest::TriggerAction {
                    action: "toggle_sidebar".to_owned(),
                    ticker: None,
                },
            );
            assert!(toggle_sb.ok);
            assert_eq!(
                toggle_sb.state.expect("toggle_sidebar").sidebar_collapsed,
                !initial_collapsed
            );

            let open_pal = dispatch_app_control_request(
                store,
                AppControlRequest::TriggerAction {
                    action: "open_palette".to_owned(),
                    ticker: None,
                },
            );
            assert!(open_pal.ok);
            assert!(open_pal.state.expect("open_palette").palette_open);

            let close_pal = dispatch_app_control_request(
                store,
                AppControlRequest::TriggerAction {
                    action: "close_palette".to_owned(),
                    ticker: None,
                },
            );
            assert!(close_pal.ok);
            assert!(!close_pal.state.expect("close_palette").palette_open);

            let _ = dispatch_app_control_request(
                store,
                AppControlRequest::Navigate {
                    tab: None,
                    ticker: Some("MSFT".to_owned()),
                    industry: None,
                },
            );
            assert!(store.company_open.get_untracked());
            let close_co = dispatch_app_control_request(
                store,
                AppControlRequest::TriggerAction {
                    action: "close_company".to_owned(),
                    ticker: None,
                },
            );
            assert!(close_co.ok);
            assert!(!close_co.state.expect("close_company").company_open);

            let reset_alan = dispatch_app_control_request(
                store,
                AppControlRequest::TriggerAction {
                    action: "reset_alan".to_owned(),
                    ticker: None,
                },
            );
            assert!(reset_alan.ok);
            assert_eq!(reset_alan.state.expect("reset_alan").alan_message_count, 0);

            let unknown_act = dispatch_app_control_request(
                store,
                AppControlRequest::TriggerAction {
                    action: "nonexistent_action".to_owned(),
                    ticker: None,
                },
            );
            assert!(!unknown_act.ok);
            assert!(unknown_act.error.is_some());
        },
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn control_ipc_socket_bind_replaces_stale_socket_and_sets_0600() {
    use crate::control_ipc::bind_control_listener;
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener;

    let root = temp_workspace("control_ipc_sock");
    let sock_path = root.join("nested").join("control.sock");
    std::fs::create_dir_all(sock_path.parent().expect("parent")).expect("mkdir");

    // Create a stale unconnected socket file by binding and immediately dropping its listener.
    {
        let _stale = UnixListener::bind(&sock_path).expect("bind stale");
    }
    assert!(sock_path.exists());

    // Binding over the stale socket succeeds and enforces 0600 permissions.
    let live_listener = bind_control_listener(&sock_path).expect("rebind stale socket");
    let mode = std::fs::metadata(&sock_path)
        .expect("stat")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    // Binding while another listener is live refuses to clobber it.
    let err = bind_control_listener(&sock_path).expect_err("must not clobber live socket");
    assert!(err.to_string().contains("already owned"));
    drop(live_listener);

    // Non-socket regular file is never deleted.
    let non_sock = root.join("not_a_socket.txt");
    std::fs::write(&non_sock, "keep me").expect("write");
    let err = bind_control_listener(&non_sock).expect_err("must not remove regular file");
    assert!(err.to_string().contains("non-socket"));
    assert!(non_sock.exists());

    let _ = std::fs::remove_dir_all(&root);
}
