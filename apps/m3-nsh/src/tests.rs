//! Unit tests & Phase 1 spike verification for `m3-nsh`.

use crate::format::{
    civil_ym_from_ms, civil_ymd_from_ms, compute_kline_domain, filter_inbox_signals,
    filter_palette_items, fmt_money_compact, fmt_pct_clean, fmt_volume_compact,
    parse_research_memo,
};
use crate::platform_desktop::build_app_menus;
use crate::stores::{
    AppStore, BasketsViewMode, ChartTimeframe, CompanyViewTab, InboxFilter, PaletteAction,
    PaletteItem, UniversePreset, UniverseSearchMode, WorkspaceTab,
};
use crate::theme::{BG_ROW_ALT, BG_SELECTED_ROW, BG_VIEWPORT, EMERALD_FG};
use crate::views::chart::{
    compute_kline_canvas_bounds, compute_kline_valuation_overlay, kline_hover_bar_index,
    kline_price_to_y, select_kline_bars,
};
use crate::views::universe::{build_scatter_model, hit_test_scatter_point, scatter_point_px};
use m3_core::i18n::UiLang;
use m3_core::lake::UniverseRow;
use m3_core::lake_l1::{SignalInboxRow, SignalKind, SignalStatus};
use m3_core::model::{
    Bucket, DataSource, MungerReport, OhlcBar, PriceHistorySnapshot, PriceSnapshot,
    StrikeZoneStatus, ValuationAssumptions, ValuationSnapshot,
};
use m3_core::store::WorkspacePaths;
use m3_core::update::ReleaseChannel;
use nsh_core::element::{Runtime, create_root};
use nsh_core::prelude::*;
use rust_decimal::Decimal;
use std::collections::HashSet;
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
        &[unpriced, outlier, circle, strike, screened],
        &fresh_strikes,
        Some("0000769397"),
    )
    .expect("scatter model");

    assert_eq!(model.points.len(), 4);
    assert!(model.ref_roic_norm.is_some());
    assert!(model.ref_fcf_norm.is_some());
    assert_eq!(model.strike_count, 1);
    assert_eq!(model.watch_count, 1);
    assert!(model.clipped_count >= 1);
    assert!(model.x_max <= 80.0);
    assert!(model.y_max <= 25.0);

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
