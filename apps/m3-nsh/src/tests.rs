//! Unit tests & Phase 1 spike verification for `m3-nsh`.

use crate::format::{
    compute_kline_domain, filter_inbox_signals, filter_palette_items, fmt_money_compact,
    fmt_pct_clean, parse_research_memo,
};
use crate::platform_desktop::build_app_menus;
use crate::stores::{
    AppStore, BasketsViewMode, CompanyViewTab, InboxFilter, PaletteAction, PaletteItem,
    UniversePreset, UniverseSearchMode, WorkspaceTab,
};
use crate::theme::{BG_ROW_ALT, BG_SELECTED_ROW, BG_VIEWPORT, EMERALD_FG};
use crate::views::chart::compute_kline_canvas_bounds;
use m3_core::i18n::UiLang;
use m3_core::lake_l1::{SignalInboxRow, SignalKind, SignalStatus};
use m3_core::model::OhlcBar;
use m3_core::store::WorkspacePaths;
use m3_core::update::ReleaseChannel;
use nsh_core::element::{Runtime, create_root};
use nsh_core::prelude::*;
use rust_decimal::Decimal;
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
