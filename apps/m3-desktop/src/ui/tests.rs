use super::agent::{AgentPanelSlice, apply_interrogator_event};
use super::format::*;
use super::theme::*;
use super::*;
use crate::alan_interrogator::{InterrogatorEvent, TuringTapeKind};
use crate::model::{DataSource, OhlcBar, StrikeZoneStatus, ValuationAssumptions, ValuationSnapshot};
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
