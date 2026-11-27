//! Company Dossier overlay (`1 Financials & Memo`, `2 XBRL Ledger`).

use super::agent::cross_company_context_strip;
use super::chart::chart_placeholder;
use crate::components::{
    action_button, animate_shell_spring, badge_pill, card_title, concept_hint_static, icon_button,
    icon_label_button, inspector_kv, kv_line, mono_right, obsidian_card, prop_section, prose_block,
    ratio_bar, roic_sparkline_for, section_header, seg_group, seg_tab, stepper_inline,
    styled_input, v_divider,
};
use crate::format::{
    civil_ym_from_ms, currency_prefix, fmt_money_compact, fmt_money_compact_opt, fmt_pct_clean,
    fmt_price, fmt_shares_compact, primary_row, primary_val,
};
use crate::icons::{ICON_ARROW_LEFT, ICON_CHEVRON_LEFT, ICON_CHEVRON_RIGHT, ICON_CLOSE, icon};
use crate::platform_desktop::{copy_text_with_notice, open_external_url};
use crate::stores::{
    AlanCanvasTab, AlanScope, AppStore, ChartTimeframe, CompanyViewTab, DeepResearchPerspectiveUi,
    MemoSection, PerspectiveUiStatus, WorkspaceTab, default_deep_research_perspectives,
};
use crate::theme::*;
use m3_core::calculator;
use m3_core::edgar;
use m3_core::i18n::UiLang;
use m3_core::model::{
    AnnualMetricRow, Bucket, MungerReport, ValuationSnapshot, WarningFlag, WarningSeverity,
    primary_circle_tag, secondary_circle_detail,
};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

pub(crate) fn roic_unavailable_label(roic_note: Option<&str>) -> String {
    match roic_note {
        Some("net_cash_exceeds_book_capital") => "Net Cash > Book Capital".into(),
        Some("total_debt_unavailable") => "— (Debt Unavailable)".into(),
        Some("cash_and_st_unavailable") => "— (Cash Unavailable)".into(),
        Some("tax_expense_unavailable") => "— (Tax Unavailable)".into(),
        Some(note) => format!("— ({note})"),
        None => "—".into(),
    }
}

fn active_report(store: AppStore) -> Option<MungerReport> {
    let ticker = store.selected_ticker.get();
    store
        .reports
        .with(|reps| reps.iter().find(|r| r.ticker == ticker).cloned())
}

fn parse_sic_from_circle(circle: &str) -> Option<u32> {
    let marker = "SIC ";
    let idx = circle.find(marker)?;
    let rest = &circle[idx + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

fn sync_decimal_input_effect(sig: Signal<Decimal>, text_sig: Signal<String>) {
    effect(move || {
        let val = sig.get().normalize();
        let parsed = text_sig
            .get_untracked()
            .trim()
            .parse::<Decimal>()
            .ok()
            .map(|d| d.normalize());
        if parsed != Some(val) {
            text_sig.set(val.to_string());
        }
    });
}

/// Renders the Company Dossier / Research Desk overlay.
pub fn company_view(store: AppStore) -> impl IntoElement {
    let mult_text = signal(
        store
            .sandbox_multiple
            .get_untracked()
            .normalize()
            .to_string(),
    );
    let ret_text = signal(
        store
            .sandbox_target_return
            .get_untracked()
            .normalize()
            .to_string(),
    );
    let gro_text = signal(
        store
            .sandbox_expected_growth
            .get_untracked()
            .normalize()
            .to_string(),
    );
    let metric_draft = signal(String::new());
    sync_decimal_input_effect(store.sandbox_multiple, mult_text);
    sync_decimal_input_effect(store.sandbox_target_return, ret_text);
    sync_decimal_input_effect(store.sandbox_expected_growth, gro_text);

    view()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(rich_top_bar(store))
        .child(dynamic(move || {
            let l = store.lang.get();
            let Some(rep) = active_report(store) else {
                return empty_company_state(store, l).into_element();
            };

            let body_pane = match store.company_view_tab.get() {
                CompanyViewTab::FinancialsMemo | CompanyViewTab::DeepResearchAlan => scroll()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(financials_memo_body(
                        store,
                        &rep,
                        mult_text,
                        ret_text,
                        gro_text,
                        metric_draft,
                    ))
                    .into_element(),
                CompanyViewTab::XbrlInspector => scroll()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(xbrl_inspector_body(store, &rep))
                    .into_element(),
            };

            view()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .flex_row()
                .overflow_hidden()
                .child(body_pane)
                .child(properties_rail(store))
                .into_element()
        }))
}

fn empty_company_state(store: AppStore, l: UiLang) -> View {
    let ticker = store.selected_ticker.get();
    let clean_ticker = ticker.trim().to_ascii_uppercase();
    if clean_ticker.is_empty() {
        return view().p(px(24.0)).child(
            obsidian_card()
                .p(px(24.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    text(l.t("未选择公司", "No Company Selected"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(
                    text(l.t(
                        "当前能力圈中暂无已选标的，请先从「全市场雷达」将公司加入能力圈，或在「能力圈总览」中选择一家标的。",
                        "No holding is selected in your Circle. Promote a company from Universe Radar or pick one in Circle Overview.",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(18.0))
                    .text_color(rgb(INK_SECONDARY)),
                )
                .child(view().pt(px(4.0)).flex().flex_row().gap(px(8.0)).child(
                    action_button(
                        l.t("前往全市场雷达", "Open Universe Radar"),
                        true,
                        move || store.open_tab(WorkspaceTab::Universe),
                    ),
                )),
        );
    }

    let paths = store.paths.get_untracked();
    let cached_rep =
        m3_core::store::read_json::<MungerReport>(&paths.computed_report_path(&clean_ticker)).ok();
    let dossier = store
        .selected_dossier
        .get()
        .filter(|d| d.ticker.eq_ignore_ascii_case(&clean_ticker))
        .or_else(|| {
            m3_core::store::load_dossier(&paths, &clean_ticker)
                .ok()
                .flatten()
        });
    let uni_row = store
        .universe_selected_row
        .get()
        .filter(|r| r.ticker.eq_ignore_ascii_case(&clean_ticker));

    let name_hint = cached_rep
        .as_ref()
        .map(|r| r.name.clone())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            dossier
                .as_ref()
                .map(|d| d.name.clone())
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| {
            uni_row
                .as_ref()
                .map(|r| r.name.clone())
                .filter(|s| !s.trim().is_empty())
        });
    let cik_hint = cached_rep.as_ref().and_then(|r| r.cik.clone()).or_else(|| {
        uni_row
            .as_ref()
            .map(|r| r.cik.clone())
            .filter(|s| !s.trim().is_empty())
    });
    let circle_hint = cached_rep
        .as_ref()
        .map(|r| r.circle.clone())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            uni_row
                .as_ref()
                .and_then(|r| r.sic_description.clone())
                .filter(|s| !s.trim().is_empty())
        });

    let heading = match name_hint.as_deref() {
        Some(name) => format!(
            "{} · {} ({})",
            l.t("尚未加入能力圈", "Not Yet in Circle"),
            clean_ticker,
            name
        ),
        None => format!(
            "{} · {}",
            l.t("尚未加入能力圈", "Not Yet in Circle"),
            clean_ticker
        ),
    };

    let mut card = obsidian_card()
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            text(heading)
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        )
        .child(
            text(l.t(
                "该标的尚未写入你的能力圈观察池（WATCH / IN / OUT / TOO HARD）。加入 [WATCH] 后即可查看完整 5 年财务底表、K 线估值沙盒、4 视角深研与 XBRL 溯源。",
                "This ticker is not in your Circle watchlist yet. Add it to [WATCH] to unlock the full 5-year financials, K-line valuation sandbox, 4-lens deep research, and XBRL provenance.",
            ))
            .font_family(FONT_UI)
            .text_size(px(TEXT_XS))
            .line_height(px(18.0))
            .text_color(rgb(INK_SECONDARY)),
        );

    if let Some(rep) = cached_rep.as_ref() {
        let (roic, _) = fmt_pct_clean(rep.roic_5y_avg_pct, false);
        let (gm, _) = fmt_pct_clean(rep.gross_margin_5y_avg_pct, false);
        let (share_d, _) = fmt_pct_clean(rep.share_change_3y_pct, true);
        let warns = rep.warnings.len();
        let warn_col = if warns > 0 { GOLD_FG } else { EMERALD_FG };
        card = card.child(
            view()
                .p(px(12.0))
                .rounded_md()
                .bg(rgb(BG_SURFACE))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(16.0))
                .child(kv_line(l.t("5Y ROIC", "5Y ROIC"), roic, INK_PRIMARY))
                .child(kv_line(
                    l.t("5Y 毛利率", "5Y Gross Margin"),
                    gm,
                    INK_PRIMARY,
                ))
                .child(kv_line(
                    l.t("3Y 股本变化", "3Y Share Δ"),
                    share_d,
                    INK_PRIMARY,
                ))
                .child(kv_line(
                    l.t("预警项", "Warnings"),
                    format!("{warns}"),
                    warn_col,
                )),
        );
    }

    if let Some(d) = dossier.as_ref()
        && !d.three_sentence_business.trim().is_empty()
    {
        card = card.child(
            view()
                .p(px(12.0))
                .rounded_md()
                .bg(rgb(BG_SURFACE))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    text(l.t("商业模式档案摘要", "Dossier Business Summary"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(INK_PRIMARY)),
                )
                .child(
                    text(d.three_sentence_business.trim().to_owned())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .line_height(px(18.0))
                        .text_color(rgb(INK_SECONDARY)),
                ),
        );
    }

    let promote_ticker = clean_ticker.clone();
    let promote_name = name_hint.clone();
    let promote_cik = cik_hint.clone();
    let promote_circle = circle_hint.clone();

    let ask_ticker = clean_ticker.clone();
    let ask_anchor_label = name_hint
        .as_ref()
        .map(|n| format!("{clean_ticker} · {n}"))
        .unwrap_or_else(|| clean_ticker.clone());
    let ask_seed = match l {
        UiLang::Zh => format!(
            "请结合 /mnt/m3/dossiers/{clean_ticker}.json 与 /mnt/m3/computed/{clean_ticker}.json，帮我快速评估 {clean_ticker} 的商业模式与 5 年 ROIC，值不值得加入 WATCH 观察池？"
        ),
        UiLang::En => format!(
            "Inspect /mnt/m3/dossiers/{clean_ticker}.json and /mnt/m3/computed/{clean_ticker}.json and summarize whether {clean_ticker} merits adding to WATCH."
        ),
    };

    card = card.child(
        view()
            .pt(px(4.0))
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(8.0))
            .child(action_button(
                l.t(
                    "+ WATCH · 加入观察池并查看完整研报",
                    "+ WATCH · Add & Open Full Desk",
                ),
                true,
                move || {
                    store.promote_ticker_to_watch(
                        &promote_ticker,
                        promote_name.as_deref(),
                        promote_cik.as_deref(),
                        promote_circle.as_deref(),
                    );
                },
            ))
            .child(action_button(
                l.t("问 Alan ↗", "Ask Alan ↗"),
                false,
                move || {
                    animate_shell_spring();
                    store.open_alan_with_anchor(
                        AlanScope::Company(ask_ticker.clone()),
                        &ask_anchor_label,
                        &ask_seed,
                    );
                },
            ))
            .child(action_button(
                l.t("前往全市场雷达", "Open Universe Radar"),
                false,
                move || store.open_tab(WorkspaceTab::Universe),
            )),
    );

    view().p(px(24.0)).child(card)
}

fn rich_top_bar(store: AppStore) -> View {
    view()
        .min_h(px(42.0))
        .px(px(18.0))
        .py(px(6.0))
        .bg(rgb(BG_SURFACE))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(dynamic(move || {
            let l = store.lang.get();
            let cur_tab = store.company_view_tab.get();
            let rep_opt = active_report(store);

            let mut left_group = view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(8.0))
                .child(icon_label_button(
                    ICON_ARROW_LEFT,
                    l.t("返回", "Back"),
                    false,
                    move || {
                        store.close_company_overlay();
                    },
                ))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .child(icon_button(ICON_CHEVRON_LEFT, 12.0, move || {
                            store.step_selected_company(-1);
                        }))
                        .child(icon_button(ICON_CHEVRON_RIGHT, 12.0, move || {
                            store.step_selected_company(1);
                        })),
                );

            let mut right_group = view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(8.0));

            if let Some(rep) = rep_opt {
                let use_ttm = store.use_ttm_basis.get();
                let val_opt = primary_val(&rep, use_ttm);
                let cur_price = rep.price.as_ref().map(|p| p.price);
                let oe_ps = val_opt.and_then(|v| v.owner_earnings_per_share);
                let mult = store.sandbox_multiple.get();
                let ret = store.sandbox_target_return.get();
                let gro = store.sandbox_expected_growth.get();
                let sb = calculator::evaluate_sandbox(oe_ps, cur_price, mult, ret, gro);
                let active_strike = sb
                    .fair_value
                    .or_else(|| val_opt.and_then(|v| v.strike_price));
                let dist_live = match (cur_price, active_strike) {
                    (Some(p), Some(s)) if s > Decimal::ZERO => {
                        Some(((p - s) / s) * Decimal::from(100))
                    }
                    _ => val_opt.and_then(|v| v.distance_to_strike_pct),
                };
                let (dist_str, _) = fmt_pct_clean(dist_live, true);
                let (st_fg, st_bg, st_br) = if matches!(rep.bucket, Bucket::Out | Bucket::TooHard) {
                    status_colors(rep.status)
                } else if rep.bucket == Bucket::Watch
                    && dist_live.is_some_and(|d| d <= Decimal::ZERO)
                {
                    (GOLD_FG, GOLD_BG, GOLD_BORDER)
                } else if rep.bucket == Bucket::Core
                    && dist_live.is_some_and(|d| d <= Decimal::ZERO)
                {
                    (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                } else {
                    status_colors(rep.status)
                };
                let mos_label = if matches!(rep.bucket, Bucket::Out | Bucket::TooHard) {
                    rep.status.label_for(l).to_owned()
                } else {
                    match dist_live {
                        Some(d) if d <= Decimal::ZERO && rep.bucket == Bucket::Core => {
                            format!("{} ({dist_str})", l.t("击球区", "Strike Zone"))
                        }
                        Some(d) if d <= Decimal::ZERO && rep.bucket == Bucket::Watch => {
                            format!("{} ({dist_str})", l.t("观察池折价", "Watch Discount"))
                        }
                        Some(d) if d <= Decimal::ZERO => {
                            format!("{}: {dist_str}", l.t("距参考", "vs Ref"))
                        }
                        Some(_) => format!("{}: {dist_str}", l.t("距击球", "vs Strike")),
                        None => rep.status.label_for(l).to_owned(),
                    }
                };

                left_group = left_group
                    .child(
                        text(rep.ticker.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_LG))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_HERO)),
                    )
                    .child(
                        text(rep.name.clone())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(INK_SECONDARY)),
                    )
                    .child(
                        text(fmt_price(cur_price, &rep.currency))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_MD))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_PRIMARY)),
                    )
                    .child(badge_pill(mos_label, st_fg, st_bg, st_br))
                    .child(v_divider())
                    .child(
                        seg_group().children(Bucket::display_order().into_iter().map(|b| {
                            let active = rep.bucket == b;
                            let (bfg, bbg, _) = bucket_colors(b);
                            let t_move = rep.ticker.clone();
                            seg_tab(
                                b.code(),
                                active,
                                active.then_some((bfg, bbg)),
                                true,
                                move || {
                                    if !active {
                                        store.request_bucket_change(&t_move, b);
                                    }
                                },
                            )
                        })),
                    );

                let ind_ticker = rep.ticker.clone();
                let ind_name = rep.name.clone();
                let ind_circle = rep.circle.clone();
                let peers_sic = store.selected_peers.with(|p| {
                    p.as_ref().and_then(|pc| {
                        if pc.target.ticker.eq_ignore_ascii_case(&ind_ticker) {
                            Some((pc.target.sic, pc.target.sic_description.clone()))
                        } else {
                            None
                        }
                    })
                });
                right_group = right_group.child(action_button(
                    l.t("研究所属行业", "Research Industry"),
                    false,
                    move || {
                        let (sic, sic_desc) = match &peers_sic {
                            Some((s, d)) => (*s, d.clone()),
                            None => (
                                parse_sic_from_circle(&ind_circle),
                                (!ind_circle.trim().is_empty()).then(|| ind_circle.clone()),
                            ),
                        };
                        store.open_or_create_industry_for_company(
                            &ind_ticker,
                            &ind_name,
                            sic,
                            sic_desc.as_deref(),
                        );
                    },
                ));

                right_group = right_group.child(
                    seg_group()
                        .child(seg_tab(
                            "TTM",
                            use_ttm,
                            Some((EMERALD_FG, EMERALD_BG)),
                            true,
                            move || {
                                if !store.use_ttm_basis.get_untracked() {
                                    store.toggle_basis();
                                }
                            },
                        ))
                        .child(seg_tab(
                            "FY",
                            !use_ttm,
                            Some((SKY_FG, SKY_BG)),
                            true,
                            move || {
                                if store.use_ttm_basis.get_untracked() {
                                    store.toggle_basis();
                                }
                            },
                        )),
                );
            }

            right_group = right_group.child(
                seg_group()
                    .child(seg_tab(
                        l.t("1 财报与活研报", "1 Financials & Memo"),
                        cur_tab != CompanyViewTab::XbrlInspector,
                        None,
                        false,
                        move || store.company_view_tab.set(CompanyViewTab::FinancialsMemo),
                    ))
                    .child(seg_tab(
                        l.t("2 XBRL 底表", "2 XBRL Ledger"),
                        cur_tab == CompanyViewTab::XbrlInspector,
                        None,
                        false,
                        move || store.company_view_tab.set(CompanyViewTab::XbrlInspector),
                    )),
            );

            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap(px(12.0))
                .child(left_group)
                .child(right_group)
        }))
}

fn financials_memo_body(
    store: AppStore,
    rep: &MungerReport,
    mult_text: Signal<String>,
    ret_text: Signal<String>,
    gro_text: Signal<String>,
    metric_draft: Signal<String>,
) -> View {
    let l = store.lang.get();
    let use_ttm = store.use_ttm_basis.get();
    let row_opt = primary_row(rep, use_ttm);
    let val_opt = primary_val(rep, use_ttm);

    let mut canvas_col = view().p(px(20.0)).flex().flex_col().gap(px(16.0));

    if !rep.warnings.is_empty() {
        canvas_col = canvas_col.child(render_warnings_with_alan_anchors(
            store,
            &rep.ticker,
            &rep.warnings,
            l,
        ));
    }

    if let Some(reason) = &rep.too_hard_reason {
        let tk_reason = rep.ticker.clone();
        let reason_seed = match l {
            UiLang::Zh => format!(
                "请帮我复核 {} 的能力圈边界理由「{}」，哪些事实改变后才值得重新评估？",
                rep.ticker, reason
            ),
            UiLang::En => format!(
                "Help review the boundary rationale for {}: '{}'. What primary facts would need to change to revisit?",
                rep.ticker, reason
            ),
        };
        canvas_col = canvas_col.child(
            view()
                .px(px(16.0))
                .py(px(12.0))
                .rounded_lg()
                .bg(rgba(SLATE_BG))
                .border_1()
                .border_color(rgba(SLATE_BORDER))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    view().flex_1().min_w(px(0.0)).child(
                        text(format!("{}: {reason}", l.t("边界理由", "Boundary Note")))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_BASE))
                            .line_height(px(20.0))
                            .text_color(rgb(INK_PRIMARY)),
                    ),
                )
                .child(ask_alan_anchor_button(
                    store,
                    tk_reason,
                    l.t("边界理由", "Boundary Note").to_owned(),
                    reason_seed,
                    l,
                )),
        );
    }

    // 1. Intrinsic Value K-Line Card + Charlie Sandbox
    canvas_col = canvas_col.child(kline_and_sandbox_card(
        store, rep, row_opt, val_opt, mult_text, ret_text, gro_text,
    ));

    // 2. Deterministic KPI Cards
    canvas_col = canvas_col.child(deterministic_kpi_section(store, rep, row_opt, val_opt));

    // 3. Cross-Company Context Strip (Peers + Lookalike)
    canvas_col = canvas_col.child(cross_company_context_strip(store));

    // 4. Historical Multi-Year Financials Table
    canvas_col = canvas_col.child(render_multi_year_table(rep, l));

    // 5. Living Research Memo & Folded 4-Lens Deep Research (async trigger, zero inline chat)
    canvas_col = canvas_col.child(living_memo_and_deep_research_card(store, &rep.ticker));

    // 6. Human Judgment · Feynman Gate & Pre-Falsification Metrics CRUD
    canvas_col = canvas_col.child(human_judgment_card(store, &rep.ticker, metric_draft));

    canvas_col
}

fn ask_alan_anchor_button(
    store: AppStore,
    ticker: String,
    anchor_label: String,
    seed_prompt: String,
    l: UiLang,
) -> View {
    view()
        .flex_shrink_0()
        .px(px(8.0))
        .py(px(3.0))
        .rounded_xs()
        .cursor_pointer()
        .bg(rgba(VIOLET_BG))
        .border_1()
        .border_color(rgba(VIOLET_BORDER))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .on_press(move || {
            animate_shell_spring();
            store.open_alan_with_anchor(
                AlanScope::Company(ticker.clone()),
                &anchor_label,
                &seed_prompt,
            );
        })
        .child(
            text(l.t("问 Alan ↗", "Ask Alan ↗"))
                .font_family(FONT_UI)
                .text_size(px(TEXT_2XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(VIOLET_FG))
                .number_of_lines(1),
        )
}

fn render_warnings_with_alan_anchors(
    store: AppStore,
    ticker: &str,
    warnings: &[WarningFlag],
    lang: UiLang,
) -> View {
    let norm_ticker = ticker.trim().to_ascii_uppercase();
    view()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .children(warnings.iter().map(|w| {
            let (fg, bg, br) = match w.severity {
                WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
            };
            let sev_label = match w.severity {
                WarningSeverity::Red => lang.t("红灯预警", "Red Flag"),
                WarningSeverity::Amber => lang.t("黄灯关注", "Watch Flag"),
                WarningSeverity::Info => lang.t("提示", "Info"),
            };
            let seed_prompt = match lang {
                UiLang::Zh => format!(
                    "请核查 {} 的{}「{}」：{}。结合一手 XBRL 口径与 10-K 附注判断是否存在结构性会计/经营风险。",
                    norm_ticker, sev_label, w.title, w.detail
                ),
                UiLang::En => format!(
                    "Investigate {} {} '{}': {}. Inspect primary XBRL and 10-K footnotes to evaluate structural risk.",
                    norm_ticker, sev_label, w.title, w.detail
                ),
            };
            view()
                .rounded_md()
                .bg(rgba(bg))
                .border_1()
                .border_color(rgba(br))
                .px(px(12.0))
                .py(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(10.0))
                        .child(badge_pill(sev_label.to_owned(), fg, bg, br))
                        .child(
                            text(w.title.clone())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(fg)),
                        )
                        .child(
                            text(format!("— {}", w.detail))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(INK_PRIMARY)),
                        ),
                )
                .child(ask_alan_anchor_button(
                    store,
                    norm_ticker.clone(),
                    format!("{sev_label} · {}", w.title),
                    seed_prompt,
                    lang,
                ))
        }))
}

fn perspective_title_for(id: &str, fallback_cn: &str, lang: UiLang) -> String {
    match (lang, id) {
        (UiLang::En, "moat") => "Moat".to_owned(),
        (UiLang::En, "accounting") => "Accounting".to_owned(),
        (UiLang::En, "latent_peers") => "Latent Peers".to_owned(),
        (UiLang::En, "premortem") => "5Y Pre-Mortem".to_owned(),
        _ => fallback_cn.to_owned(),
    }
}

fn living_memo_and_deep_research_card(store: AppStore, ticker: &str) -> View {
    let norm_ticker = ticker.trim().to_ascii_uppercase();
    obsidian_card().child(dynamic(move || {
        let l = store.lang.get();
        let active_ticker = {
            let sel = store.selected_ticker.get();
            if sel.trim().is_empty() {
                norm_ticker.clone()
            } else {
                sel
            }
        };
        let memo_rel = format!("workbench/memos/{active_ticker}.md");
        let raw_opt = store.selected_memo_raw.get();
        let parsed_opt = store.selected_memo_parsed.get();
        let running = store.deep_research_running.get();
        let active_for_ticker = store
            .deep_research_ticker
            .get()
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case(&active_ticker));
        let blocked_reason = if active_for_ticker {
            store.deep_research_budget_blocked.get()
        } else {
            None
        };
        let status_note = if active_for_ticker {
            store.deep_research_status_note.get()
        } else {
            None
        };
        let last_completed = active_for_ticker
            && store
                .deep_research_last_result
                .with(|r| r.as_ref().is_some_and(|x| x.status == "completed"));

        let status_pill = if active_for_ticker && running {
            Some(badge_pill(
                l.t("4 视角并行生成中…", "4-Lens Running…"),
                VIOLET_FG,
                VIOLET_BG,
                VIOLET_BORDER,
            ))
        } else if blocked_reason.is_some() {
            Some(badge_pill(
                l.t("预算闸门已拦截", "Budget Blocked"),
                CORAL_FG,
                CORAL_BG,
                CORAL_BORDER,
            ))
        } else if last_completed {
            Some(badge_pill(
                l.t("4 视角已完成", "4-Lens Complete"),
                EMERALD_FG,
                EMERALD_BG,
                EMERALD_BORDER,
            ))
        } else {
            None
        };

        let mut actions = view()
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .gap(px(8.0));
        if let Some(md) = raw_opt {
            let t_copy = active_ticker.clone();
            actions = actions.child(action_button(
                l.t("复制 Markdown", "Copy Markdown"),
                false,
                move || {
                    copy_text_with_notice(store, md.clone(), &format!("Memo {t_copy}"));
                },
            ));
        }
        let t_refresh = active_ticker.clone();
        actions = actions.child(action_button(l.t("刷新", "Refresh"), false, move || {
            store.reload_research_desk_context(&t_refresh);
            let cur_l = store.lang.get_untracked();
            store.set_info_notice(match cur_l {
                UiLang::Zh => format!("已从磁盘刷新 {t_refresh} Memo"),
                UiLang::En => format!("Reloaded {t_refresh} Memo from disk"),
            });
        }));
        let memo_seed = match l {
            UiLang::Zh => format!(
                "请结合 {} 的 Living Research Memo 与一手 10-K 原文，挑出当前最关键的护城河假设与 5 年证伪风险。",
                active_ticker
            ),
            UiLang::En => format!(
                "Review {}'s Living Research Memo and primary 10-K filings to stress-test the core moat assumptions and 5-year falsification risks.",
                active_ticker
            ),
        };
        actions = actions
            .child(ask_alan_anchor_button(
                store,
                active_ticker.clone(),
                l.t("活研报总览", "Living Memo").to_owned(),
                memo_seed,
                l,
            ))
            .child(action_button(
                if active_for_ticker && running {
                    l.t("4 视角深研中…", "4-Lens Running…")
                } else {
                    l.t("生成 / 刷新 4 视角深研", "Generate / Refresh 4-Lens Research")
                },
                !(active_for_ticker && running),
                move || store.run_deep_research_selected(),
            ));

        let mut title_row = view()
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .gap(px(8.0))
            .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(EMERALD_FG)))
            .child(
                text(format!("Living Research Memo · {memo_rel}"))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO)),
            );
        if let Some(pill) = status_pill {
            title_row = title_row.child(pill);
        }

        let header = view()
            .px(px(16.0))
            .py(px(10.0))
            .bg(rgb(BG_WELL))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(12.0))
            .child(title_row)
            .child(actions);

        let mut card = view().flex().flex_col().child(header);

        if let Some(reason) = blocked_reason {
            card = card.child(
                view()
                    .px(px(16.0))
                    .py(px(10.0))
                    .bg(rgba(CORAL_BG))
                    .border_b_1()
                    .border_color(rgba(CORAL_BORDER))
                    .child(
                        text(match l {
                            UiLang::Zh => {
                                format!("预算硬闸门已拦截（0 LLM Token 消耗）：{reason}")
                            }
                            UiLang::En => {
                                format!("Hard budget gate blocked (0 LLM tokens used): {reason}")
                            }
                        })
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(CORAL_FG)),
                    ),
            );
        } else if let Some(note) = status_note {
            card = card.child(
                view()
                    .px(px(16.0))
                    .py(px(8.0))
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        text(note)
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_SECONDARY)),
                    ),
            );
        }

        if active_for_ticker && (running || last_completed) {
            let perspectives: Vec<DeepResearchPerspectiveUi> = {
                let live = store.deep_research_perspectives.get();
                if live.is_empty() {
                    default_deep_research_perspectives()
                } else {
                    live
                }
            };
            card = card.child(
                view()
                    .px(px(16.0))
                    .py(px(10.0))
                    .bg(rgb(BG_SURFACE))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .children(perspectives.into_iter().map(|p| {
                        let (fg, bg, br) = match p.status {
                            PerspectiveUiStatus::Pending => (INK_MUTED, BG_WELL, BORDER_SUBTLE),
                            PerspectiveUiStatus::Running => (VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
                            PerspectiveUiStatus::Completed => {
                                (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                            }
                            PerspectiveUiStatus::Failed => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                        };
                        view()
                            .flex_1()
                            .min_w(px(160.0))
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded_sm()
                            .bg(rgb(BG_VIEWPORT))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(6.0))
                            .child(
                                text(perspective_title_for(&p.id, &p.title_cn, l))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(INK_HERO)),
                            )
                            .child(badge_pill(p.status.label_for(l), fg, bg, br))
                    })),
            );
        }

        let Some(memo) = parsed_opt else {
            return card.child(
                view().p(px(16.0)).child(
                    text(match l {
                        UiLang::Zh => format!(
                            "尚未生成 {memo_rel}。点击右上角「生成 / 刷新 4 视角深研」或运行 `m3 research run {active_ticker}` 后，完整 4 视角报告与一手 10-K 引用将直接呈现在此（进度同步显示于顶部消息盒）。"
                        ),
                        UiLang::En => format!(
                            "{memo_rel} has not been generated yet. Click 'Generate / Refresh 4-Lens Research' or run `m3 research run {active_ticker}` to fold the 4-lens report and 10-K citations here (progress streams in the topbar Message Box)."
                        ),
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(19.0))
                    .text_color(rgb(INK_MUTED)),
                ),
            );
        };

        if !memo.metadata.is_empty() {
            card = card.child(
                view()
                    .px(px(16.0))
                    .py(px(8.0))
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_wrap()
                    .gap(px(12.0))
                    .children(memo.metadata.iter().map(|(k, v)| {
                        view()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                text(format!("{k}:"))
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                text(v.clone())
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_PRIMARY)),
                            )
                    })),
            );
        }

        let perspective_sections: Vec<&MemoSection> = memo
            .sections
            .iter()
            .filter(|s| matches!(s.id.as_str(), "moat" | "accounting" | "latent_peers" | "premortem"))
            .collect();
        let display_sections: Vec<&MemoSection> = if perspective_sections.is_empty() {
            memo.sections.iter().collect()
        } else {
            perspective_sections
        };

        let mut body = view().p(px(16.0)).flex().flex_col().gap(px(12.0));
        for sec in display_sections {
            let sec_seed = match l {
                UiLang::Zh => format!(
                    "围绕 {} Living Memo 中的「{}」章节，请结合一手 10-K 证据核验关键论据与潜在盲区。",
                    active_ticker, sec.heading
                ),
                UiLang::En => format!(
                    "Focusing on the '{}' section of {}'s Living Memo, verify key claims and blind spots against primary 10-K filings.",
                    sec.heading, active_ticker
                ),
            };
            body = body.child(
                view()
                    .p(px(12.0))
                    .rounded_sm()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        view()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .child(
                                text(sec.heading.clone())
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                            )
                            .child(ask_alan_anchor_button(
                                store,
                                active_ticker.clone(),
                                sec.heading.clone(),
                                sec_seed,
                                l,
                            )),
                    )
                    .child(
                        text(sec.body.clone())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .line_height(px(20.0))
                            .text_color(rgb(INK_SECONDARY))
                            .selectable(true),
                    ),
            );
        }

        if !memo.citations.is_empty() {
            body = body.child(
                view()
                    .p(px(12.0))
                    .rounded_sm()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        text(format!(
                            "{} ({})",
                            l.t("10-K 原文与一手引用证据", "10-K & PRIMARY CITATIONS"),
                            memo.citations.len()
                        ))
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(SKY_FG)),
                    )
                    .children(memo.citations.iter().take(12).map(|cite| {
                        let cite_seed = match l {
                            UiLang::Zh => format!(
                                "请核对 {} 这条 10-K / 一手引用证据原文及上下文含义：{}",
                                active_ticker, cite
                            ),
                            UiLang::En => format!(
                                "Verify the context and implications of this 10-K / primary citation for {}: {}",
                                active_ticker, cite
                            ),
                        };
                        view()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(8.0))
                            .child(
                                view().flex_1().min_w(px(0.0)).child(
                                    text(cite.clone())
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .line_height(px(17.0))
                                        .text_color(rgb(INK_SECONDARY))
                                        .selectable(true),
                                ),
                            )
                            .child(ask_alan_anchor_button(
                                store,
                                active_ticker.clone(),
                                l.t("10-K 原文引用", "10-K Excerpt").to_owned(),
                                cite_seed,
                                l,
                            ))
                    })),
            );
        }

        if let Some(notes) = &memo.human_notes {
            body = body.child(
                view()
                    .p(px(12.0))
                    .rounded_sm()
                    .bg(rgba(GOLD_BG))
                    .border_1()
                    .border_color(rgba(GOLD_BORDER))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        text(l.t(
                            "人类主权区（深度研究永不覆盖）",
                            "## Human Notes (Human-Owned · Never Overwritten by Deep Research)",
                        ))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(GOLD_FG)),
                    )
                    .child(
                        text(notes.clone())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .line_height(px(19.0))
                            .text_color(rgb(INK_PRIMARY))
                            .selectable(true),
                    ),
            );
        }

        card.child(body)
    }))
}

fn human_judgment_card(store: AppStore, ticker: &str, metric_draft: Signal<String>) -> View {
    let norm_ticker = ticker.trim().to_ascii_uppercase();
    let ticker_for_body = norm_ticker.clone();
    let ticker_for_input = norm_ticker.clone();
    let ticker_for_btn = norm_ticker;

    obsidian_card()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(dynamic(move || {
            let l = store.lang.get();
            let tk = ticker_for_body.clone();
            let tk_feynman = tk.clone();
            let dossier_opt = store.selected_dossier.get();
            let (pending_review, my_stance, weight_rationale, metrics, checked_at) =
                match dossier_opt.as_ref() {
                    Some(d) => (
                        d.human_judgment.pending_human_review,
                        d.human_judgment.my_stance.clone(),
                        d.human_judgment.weight_rationale.clone(),
                        d.human_judgment.falsification_metrics.clone(),
                        d.human_judgment.feynman_checked_at.clone(),
                    ),
                    None => (true, None, None, Vec::new(), None),
                };

            let mut suggested_from_alan: Vec<String> = Vec::new();
            store.signal_triages.with(|triages| {
                for tr in triages {
                    if !tr.ticker.eq_ignore_ascii_case(&tk) {
                        continue;
                    }
                    for m in &tr.suggested_metrics {
                        let trimmed = m.trim();
                        if !trimmed.is_empty()
                            && !metrics.iter().any(|e| e.trim() == trimmed)
                            && !suggested_from_alan.iter().any(|s| s == trimmed)
                        {
                            suggested_from_alan.push(trimmed.to_owned());
                        }
                    }
                }
            });

            let status_badge = if let Some(checked) = checked_at.as_deref() {
                badge_pill(
                    match l {
                        UiLang::Zh => format!("已费曼核验 · {checked}"),
                        UiLang::En => format!("Feynman Checked · {checked}"),
                    },
                    EMERALD_FG,
                    EMERALD_BG,
                    EMERALD_BORDER,
                )
            } else if !pending_review {
                badge_pill(
                    l.t("已确认", "Verified").to_owned(),
                    EMERALD_FG,
                    EMERALD_BG,
                    EMERALD_BORDER,
                )
            } else {
                badge_pill(
                    l.t("待 3 轮费曼答辩", "Pending 3-Round Feynman").to_owned(),
                    GOLD_FG,
                    GOLD_BG,
                    GOLD_BORDER,
                )
            };

            let mut metrics_list = view().flex().flex_col().gap(px(6.0));
            if metrics.is_empty() {
                metrics_list = metrics_list.child(
                    text(l.t(
                        "（尚未设置事前证伪指标；L1.5 巡检将回退至事前验尸与确定性红灯检查）",
                        "(No falsification metrics yet; L1.5 watchdog falls back to pre-mortem & deterministic warnings)",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                );
            } else {
                for (idx, m) in metrics.into_iter().enumerate() {
                    let tk_rm = tk.clone();
                    metrics_list = metrics_list.child(
                        view()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded_sm()
                            .bg(rgb(BG_VIEWPORT))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(10.0))
                            .child(
                                view().flex_1().min_w(px(0.0)).child(
                                    text(format!("• {m}"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_BASE))
                                        .line_height(px(20.0))
                                        .text_color(rgb(INK_PRIMARY)),
                                ),
                            )
                            .child(
                                view()
                                    .flex_shrink_0()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .rounded_xs()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .hover(|s| s.bg(rgba(CORAL_BG)))
                                    .cursor_pointer()
                                    .on_press(move || {
                                        store.remove_falsification_metric(&tk_rm, idx);
                                    })
                                    .child(
                                        text(l.t("移除", "Remove"))
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_2XS))
                                            .text_color(rgb(INK_MUTED)),
                                    ),
                            ),
                    );
                }
            }

            let mut card_col = view()
                .flex()
                .flex_col()
                .gap(px(14.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .flex_wrap()
                        .gap(px(8.0))
                        .pb(px(10.0))
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_baseline()
                                .gap(px(10.0))
                                .child(
                                    text(
                                        l.t("人类判词 · 费曼防线", "Human Judgment · Feynman Gate"),
                                    )
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_SM))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(
                                    text(l.t(
                                        "仅本人确认 · AI 审讯与答辩在右侧 Alan 画布",
                                        "Human-Owned Only · AI defense & drafts live in Alan Canvas",
                                    ))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                                ),
                        )
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(status_badge)
                                .child(action_button(
                                    l.t("开启 3 轮费曼答辩", "Start 3-Round Feynman Defense"),
                                    pending_review || checked_at.is_none(),
                                    move || store.start_graduation_feynman(&tk_feynman),
                                )),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .gap(px(20.0))
                        .child(
                            view().flex_1().min_w(px(0.0)).child(prose_block(
                                l.t("我的定性判词 (my_stance)", "My Stance (my_stance)"),
                                my_stance
                                    .as_deref()
                                    .unwrap_or_else(|| l.t("（待填写）", "(pending)")),
                            )),
                        )
                        .child(
                            view().flex_1().min_w(px(0.0)).child(prose_block(
                                l.t(
                                    "分歧权重判断 (weight_rationale)",
                                    "Sizing / Weight Rationale (weight_rationale)",
                                ),
                                weight_rationale
                                    .as_deref()
                                    .unwrap_or_else(|| l.t("（待填写）", "(pending)")),
                            )),
                        ),
                )
                .child(
                    view()
                        .pt(px(10.0))
                        .border_t_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    text(l.t(
                                        "事前证伪指标 (falsification_metrics)",
                                        "Pre-Falsification Metrics (falsification_metrics)",
                                    ))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(if let Some(checked) = checked_at {
                                    view().child(
                                        text(format!("{} {checked}", l.t("核验时间", "Checked")))
                                            .font_family(FONT_MONO_NUM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(EMERALD_FG)),
                                    )
                                } else {
                                    view()
                                }),
                        )
                        .child(metrics_list),
                );

            if !suggested_from_alan.is_empty() {
                card_col = card_col.child(
                    view()
                        .p(px(10.0))
                        .rounded_sm()
                        .bg(rgba(VIOLET_BG))
                        .border_1()
                        .border_color(rgba(VIOLET_BORDER))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            text(l.t(
                                "Alan 巡检建议的事前证伪指标（需本人点击采纳）：",
                                "Alan Watchdog Suggested Falsification Metrics (human adoption only):",
                            ))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(VIOLET_FG)),
                        )
                        .child(view().flex().flex_col().gap(px(6.0)).children(
                            suggested_from_alan.into_iter().map(|m| {
                                let m_adopt = m.clone();
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.0))
                                    .child(
                                        view().flex_1().min_w(px(0.0)).child(
                                            text(format!("+ {m}"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_PRIMARY)),
                                        ),
                                    )
                                    .child(action_button(
                                        l.t("采纳", "Adopt"),
                                        false,
                                        move || store.adopt_suggested_falsification_metric(&m_adopt),
                                    ))
                            }),
                        )),
                );
            }

            card_col
        }))
        // Inline CRUD add input mounted OUTSIDE dynamic() so IME & focus stay intact
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    styled_input(
                        move || metric_draft.get(),
                        move || {
                            store
                                .lang
                                .get()
                                .t(
                                    "新增可量化证伪指标（例如：连续两季毛利率 < 62% 或 SBC/OCF > 15%）…",
                                    "Add testable falsification metric (e.g. Gross margin < 62% for 2Q)…",
                                )
                                .to_owned()
                        },
                        false,
                    )
                    .flex_1()
                    .min_w(px(0.0))
                    .on_change(move |val: &str| metric_draft.set(val.to_owned()))
                    .on_submit(move |val: &str| {
                        let trimmed = val.trim().to_owned();
                        if !trimmed.is_empty() {
                            store.add_falsification_metric(&ticker_for_input, &trimmed);
                            metric_draft.set(String::new());
                        }
                    }),
                )
                .child(action_button(
                    move || {
                        store
                            .lang
                            .get()
                            .t("添加证伪指标", "Add Metric")
                            .to_owned()
                    },
                    false,
                    move || {
                        let trimmed = metric_draft.get_untracked().trim().to_owned();
                        if !trimmed.is_empty() {
                            store.add_falsification_metric(&ticker_for_btn, &trimmed);
                            metric_draft.set(String::new());
                        }
                    },
                )),
        )
}

fn kline_and_sandbox_card(
    store: AppStore,
    rep: &MungerReport,
    row_opt: Option<&AnnualMetricRow>,
    val_opt: Option<&ValuationSnapshot>,
    mult_text: Signal<String>,
    ret_text: Signal<String>,
    gro_text: Signal<String>,
) -> View {
    let l = store.lang.get();
    let cur_price = rep.price.as_ref().map(|p| p.price);
    let oe_per_share = val_opt.and_then(|v| v.owner_earnings_per_share);
    let saved_mult = rep.assumptions.multiple_low;
    let saved_ret = rep.assumptions.target_return_pct;
    let saved_gro = rep.assumptions.expected_growth_pct;
    let currency = rep.currency.clone();
    let currency_hdr = currency.clone();
    let fallback_strike = val_opt.and_then(|v| v.strike_price);

    let active_bars = move || {
        let tf = store.chart_timeframe.get();
        match tf {
            ChartTimeframe::FiveYearWeekly => store
                .selected_history_wk
                .with(|h| h.as_ref().map(|x| x.bars.clone()).unwrap_or_default()),
            ChartTimeframe::ThreeYearWeekly => store.selected_history_wk.with(|h| {
                h.as_ref()
                    .map(|x| {
                        let keep = 156usize.min(x.bars.len());
                        x.bars[x.bars.len() - keep..].to_vec()
                    })
                    .unwrap_or_default()
            }),
            ChartTimeframe::OneYearDaily => store
                .selected_history_1d
                .with(|h| h.as_ref().map(|x| x.bars.clone()).unwrap_or_default()),
        }
    };

    let mut card = obsidian_card()
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        // K-line Header + Timeframe switcher (reactive so switching TF never rebuilds parent)
        .child(dynamic(move || {
            let cur_l = store.lang.get();
            let tf = store.chart_timeframe.get();
            let bars = active_bars();
            let ohlc_readout = match bars.last() {
                Some(b) => {
                    let pfx = currency_prefix(&currency_hdr);
                    format!(
                        "O {pfx}{:.2}   H {pfx}{:.2}   L {pfx}{:.2}   C {pfx}{:.2}",
                        b.open, b.high, b.low, b.close
                    )
                }
                None => cur_l
                    .t("暂无本地 K 线缓存", "No local K-line cache")
                    .to_owned(),
            };
            let peak_high = bars
                .iter()
                .map(|b| b.high)
                .fold(Decimal::ZERO, Decimal::max);
            let drawdown_pct = match (bars.last(), peak_high) {
                (Some(b), pk) if pk > Decimal::ZERO => {
                    Some(((b.close - pk) / pk) * Decimal::from(100))
                }
                _ => None,
            };
            let (dd_str, _) = fmt_pct_clean(drawdown_pct, true);
            let date_range = match (bars.first(), bars.last()) {
                (Some(f), Some(last)) => format!(
                    "{} → {} ({})",
                    civil_ym_from_ms(f.ts_ms),
                    civil_ym_from_ms(last.ts_ms),
                    bars.len()
                ),
                _ => String::new(),
            };

            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap(px(12.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(12.0))
                        .child(
                            seg_group().children(
                                [
                                    ChartTimeframe::FiveYearWeekly,
                                    ChartTimeframe::ThreeYearWeekly,
                                    ChartTimeframe::OneYearDaily,
                                ]
                                .into_iter()
                                .map(|item_tf| {
                                    seg_tab(
                                        item_tf.label_for(cur_l),
                                        tf == item_tf,
                                        None,
                                        true,
                                        move || store.chart_timeframe.set(item_tf),
                                    )
                                }),
                            ),
                        )
                        .child(
                            text(ohlc_readout)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(if date_range.is_empty() {
                            view()
                        } else {
                            view().child(
                                text(date_range)
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED)),
                            )
                        })
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_baseline()
                                .gap(px(4.0))
                                .child(
                                    text(cur_l.t("高点回撤", "Drawdown"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                )
                                .child(
                                    text(dd_str)
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                ),
                        ),
                )
        }))
        // Interactive K-Line & Intrinsic Value Canvas
        .child(chart_placeholder(store));

    // Charlie Sandbox Interactive Valuation Card (inputs mounted outside dynamic!)
    if row_opt.is_some() {
        let cur_for_preview = currency.clone();
        card = card.child(
            view()
                .pt(px(12.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(12.0))
                // Controls row: Steppers + Native IME-safe numeric inputs
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .flex_wrap()
                        .gap(px(12.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .flex_wrap()
                                .gap(px(12.0))
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(stepper_inline(
                                            l.t("倍数", "Mult"),
                                            move || {
                                                format!(
                                                    "{}x",
                                                    store.sandbox_multiple.get().normalize()
                                                )
                                            },
                                            move || {
                                                store.sandbox_multiple.update(|m| {
                                                    if *m > Decimal::from(5) {
                                                        *m -= Decimal::ONE;
                                                    }
                                                });
                                            },
                                            move || {
                                                store.sandbox_multiple.update(|m| {
                                                    if *m < Decimal::from(45) {
                                                        *m += Decimal::ONE;
                                                    }
                                                });
                                            },
                                        ))
                                        .child(
                                            styled_input(
                                                move || mult_text.get(),
                                                move || "18".to_owned(),
                                                true,
                                            )
                                            .w(px(52.0))
                                            .py(px(3.0))
                                            .on_change(move |val: &str| {
                                                mult_text.set(val.to_owned());
                                                if let Ok(d) = val.trim().parse::<Decimal>()
                                                    && d >= Decimal::from(1)
                                                    && d <= Decimal::from(100)
                                                {
                                                    store.sandbox_multiple.set(d);
                                                }
                                            }),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(stepper_inline(
                                            l.t("目标", "Target"),
                                            move || {
                                                format!(
                                                    "{}%",
                                                    store.sandbox_target_return.get().normalize()
                                                )
                                            },
                                            move || {
                                                store.sandbox_target_return.update(|r| {
                                                    if *r > Decimal::from(5) {
                                                        *r -= Decimal::ONE;
                                                    }
                                                });
                                            },
                                            move || {
                                                store.sandbox_target_return.update(|r| {
                                                    if *r < Decimal::from(25) {
                                                        *r += Decimal::ONE;
                                                    }
                                                });
                                            },
                                        ))
                                        .child(
                                            styled_input(
                                                move || ret_text.get(),
                                                move || "10".to_owned(),
                                                true,
                                            )
                                            .w(px(52.0))
                                            .py(px(3.0))
                                            .on_change(move |val: &str| {
                                                ret_text.set(val.to_owned());
                                                if let Ok(d) = val.trim().parse::<Decimal>()
                                                    && d >= Decimal::from(1)
                                                    && d <= Decimal::from(50)
                                                {
                                                    store.sandbox_target_return.set(d);
                                                }
                                            }),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(stepper_inline(
                                            l.t("增速", "Growth"),
                                            move || {
                                                format!(
                                                    "{}%",
                                                    store.sandbox_expected_growth.get().normalize()
                                                )
                                            },
                                            move || {
                                                store.sandbox_expected_growth.update(|g| {
                                                    if *g > Decimal::from(-10) {
                                                        *g -= Decimal::ONE;
                                                    }
                                                });
                                            },
                                            move || {
                                                store.sandbox_expected_growth.update(|g| {
                                                    if *g < Decimal::from(25) {
                                                        *g += Decimal::ONE;
                                                    }
                                                });
                                            },
                                        ))
                                        .child(
                                            styled_input(
                                                move || gro_text.get(),
                                                move || "8".to_owned(),
                                                true,
                                            )
                                            .w(px(52.0))
                                            .py(px(3.0))
                                            .on_change(move |val: &str| {
                                                gro_text.set(val.to_owned());
                                                if let Ok(d) = val.trim().parse::<Decimal>()
                                                    && d >= Decimal::from(-50)
                                                    && d <= Decimal::from(60)
                                                {
                                                    store.sandbox_expected_growth.set(d);
                                                }
                                            }),
                                        ),
                                ),
                        )
                        // Live recalculated preview + Save/Reset buttons
                        .child(dynamic(move || {
                            let cur_l = store.lang.get();
                            let mult = store.sandbox_multiple.get();
                            let ret = store.sandbox_target_return.get();
                            let gro = store.sandbox_expected_growth.get();
                            let sb = calculator::evaluate_sandbox(
                                oe_per_share,
                                cur_price,
                                mult,
                                ret,
                                gro,
                            );
                            let live_strike = sb.fair_value.or(fallback_strike);
                            let live_high = oe_per_share
                                .filter(|v| *v > Decimal::ZERO)
                                .map(|v| v * (mult + Decimal::from(4)));
                            let dirty = mult != saved_mult || ret != saved_ret || gro != saved_gro;
                            let (imp_s, _) = fmt_pct_clean(sb.implied_growth_for_target_pct, true);
                            let (exp_s, _) = fmt_pct_clean(sb.expected_total_return_pct, false);
                            let (dist_s, _) = fmt_pct_clean(sb.distance_to_strike_pct, true);

                            let band_txt = match (live_strike, live_high) {
                                (Some(lo), Some(hi)) => format!(
                                    "{} – {}",
                                    fmt_price(Some(lo), &cur_for_preview),
                                    fmt_price(Some(hi), &cur_for_preview)
                                ),
                                (Some(lo), None) => fmt_price(Some(lo), &cur_for_preview),
                                _ => "—".to_owned(),
                            };

                            let mut row = view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .flex_wrap()
                                .gap(px(12.0))
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_baseline()
                                        .gap(px(4.0))
                                        .child(
                                            text(cur_l.t("击球价", "Strike"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_MUTED)),
                                        )
                                        .child(
                                            text(fmt_price(live_strike, &cur_for_preview))
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_XS))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(rgb(EMERALD_FG)),
                                        )
                                        .child(
                                            text(format!("({dist_s})"))
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_2XS))
                                                .text_color(rgb(INK_MUTED)),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_baseline()
                                        .gap(px(4.0))
                                        .child(
                                            text(cur_l.t("合理区间", "Fair Band"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_MUTED)),
                                        )
                                        .child(
                                            text(band_txt)
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_PRIMARY)),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_baseline()
                                        .gap(px(4.0))
                                        .child(
                                            text(cur_l.t("隐含增速", "Implied g"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_MUTED)),
                                        )
                                        .child(
                                            text(imp_s)
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_XS))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(rgb(GOLD_FG)),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_baseline()
                                        .gap(px(4.0))
                                        .child(
                                            text(cur_l.t("预期年化", "Exp IRR"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_MUTED)),
                                        )
                                        .child(
                                            text(exp_s)
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_XS))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(rgb(INK_HERO)),
                                        ),
                                );

                            if dirty {
                                row = row.child(action_button(
                                    cur_l.t("重置", "Reset"),
                                    false,
                                    move || store.reset_sandbox_to_saved(),
                                ));
                            }
                            row.child(action_button(
                                if dirty {
                                    cur_l.t("保存沙盒", "Save Sandbox")
                                } else {
                                    cur_l.t("已保存", "Saved")
                                },
                                dirty,
                                move || store.save_sandbox_assumptions(),
                            ))
                        })),
                ),
        );
    }

    card
}

fn deterministic_kpi_section(
    store: AppStore,
    rep: &MungerReport,
    row_opt: Option<&AnnualMetricRow>,
    val_opt: Option<&ValuationSnapshot>,
) -> View {
    let l = store.lang.get();
    let Some(row) = row_opt else {
        return view()
            .p(px(16.0))
            .rounded_lg()
            .bg(rgba(GOLD_BG))
            .border_1()
            .border_color(rgba(GOLD_BORDER))
            .child(
                text(match l {
                    UiLang::Zh => format!(
                        "{} 暂无结构化财报数据；如需接入计算器，可录入 workbench/manual_financials/{}.json。",
                        rep.ticker, rep.ticker
                    ),
                    UiLang::En => format!(
                        "No structured SEC financials for {} yet; add workbench/manual_financials/{}.json if manual entry is needed.",
                        rep.ticker, rep.ticker
                    ),
                })
                .font_family(FONT_UI)
                .text_size(px(TEXT_BASE))
                .line_height(px(21.0))
                .text_color(rgb(GOLD_FG)),
            );
    };

    let roic_display = match (rep.roic_5y_avg_pct, rep.roic_5y_stddev_pct) {
        (Some(avg), Some(std)) => format!("{avg:.1}% ± {std:.1}%"),
        _ => row
            .roic_total_capital_pct
            .map(|v| match l {
                UiLang::Zh => format!("总资本 {v:.1}% (净现金 > 账面)"),
                UiLang::En => format!("Total Cap {v:.1}% (Net Cash > Equity)"),
            })
            .unwrap_or_else(|| "—".to_owned()),
    };
    let gm_display = match (rep.gross_margin_5y_avg_pct, rep.gross_margin_5y_stddev_pct) {
        (Some(avg), Some(std)) => format!("{avg:.1}% ± {std:.1}%"),
        _ => fmt_pct_clean(row.gross_margin_pct, false).0,
    };
    let (s3_str, s3_pol) = fmt_pct_clean(rep.share_change_3y_pct, true);
    let s3_col = share_delta_color(rep.share_change_3y_pct, s3_pol);
    let (s5_str, s5_pol) = fmt_pct_clean(rep.share_change_5y_pct, true);
    let s5_col = if s5_pol == 0 {
        INK_MUTED
    } else {
        INK_SECONDARY
    };

    let (ym_str, ym_pol) = fmt_pct_clean(val_opt.and_then(|v| v.strict_fcf_yield_mcap_pct), false);
    let (ye_str, _) = fmt_pct_clean(val_opt.and_then(|v| v.strict_fcf_yield_ev_pct), false);

    let ytd_callout = rep.latest_ytd.as_ref().map(|y| {
        let q_part = match (
            &y.latest_quarter_label,
            y.latest_quarter_ocf,
            y.latest_quarter_capex,
        ) {
            (Some(ql), Some(qo), Some(qc)) => format!(
                " | {ql}: OCF {} vs CapEx {}",
                fmt_money_compact(qo, &rep.currency),
                fmt_money_compact(qc, &rep.currency)
            ),
            _ => String::new(),
        };
        format!(
            "{}: OCF {} vs CapEx {}{}",
            y.period_label,
            fmt_money_compact_opt(y.ocf, &rep.currency),
            fmt_money_compact_opt(y.capex, &rep.currency),
            q_part
        )
    });

    // Quality score from L0 DuckDB PeerComparison if available
    let quality_score_str = store
        .selected_peers
        .get()
        .filter(|p| p.target.ticker.eq_ignore_ascii_case(&rep.ticker))
        .map(|p| format!("{:.1}", p.target.quality_score.round_dp(1)))
        .unwrap_or_else(|| "— (L0)".to_owned());

    let strike_price = val_opt.and_then(|v| v.strike_price);
    let fv_band_str = match (
        val_opt.and_then(|v| v.fair_value_low),
        val_opt.and_then(|v| v.fair_value_high),
    ) {
        (Some(lo), Some(hi)) => format!(
            "{} – {}",
            fmt_price(Some(lo), &rep.currency),
            fmt_price(Some(hi), &rep.currency)
        ),
        (Some(lo), None) => fmt_price(Some(lo), &rep.currency),
        _ => "—".to_owned(),
    };

    let dossier_verdict = store.selected_dossier.with(|d| match d {
        Some(dos) if !dos.human_judgment.pending_human_review => {
            l.t("已费曼核验", "Feynman Verified")
        }
        Some(_) => l.t("待人类答辩", "Pending Human Review"),
        None => l.t("尚无审讯档案", "No Dossier Yet"),
    });

    let mut card1 = obsidian_card()
        .flex_1()
        .min_w(px(0.0))
        .p(px(16.0))
        .child(card_title(
            l.t("护城河 & 股本质量", "Moat & Share Discipline"),
            l.t("5Y 稳定性", "5Y Consistency"),
        ))
        .child(kv_line("5Y ROIC", roic_display, INK_HERO))
        .child(kv_line(
            l.t("5Y 毛利率", "5Y Gross Margin"),
            gm_display,
            INK_PRIMARY,
        ))
        .child(kv_line(
            l.t("质量评分 (L0)", "Quality Score"),
            quality_score_str,
            SKY_FG,
        ))
        .child(kv_line(
            l.t("当期净负债", "Net Debt"),
            fmt_money_compact_opt(row.net_debt, &rep.currency),
            match row.net_debt {
                Some(v) if v <= Decimal::ZERO => EMERALD_FG,
                Some(_) => INK_PRIMARY,
                None => INK_MUTED,
            },
        ))
        .child(kv_line(
            l.t("总债务 / 现金", "Total Debt / Cash"),
            format!(
                "{} / {}",
                fmt_money_compact_opt(row.total_debt, &rep.currency),
                fmt_money_compact_opt(row.cash_and_st, &rep.currency)
            ),
            INK_SECONDARY,
        ))
        .child(kv_line(
            l.t("净负债 / 严苛 OE", "Net Debt / Strict OE"),
            rep.net_debt_to_owner_earnings
                .map(|v| format!("{v:.2}x"))
                .unwrap_or_else(|| "—".to_owned()),
            if rep.net_debt_to_owner_earnings.is_some() {
                INK_SECONDARY
            } else {
                INK_MUTED
            },
        ))
        .child(kv_line(
            l.t("3Y 稀释股本Δ", "3Y Diluted Shares Δ"),
            s3_str,
            s3_col,
        ))
        .child(kv_line(
            l.t("5Y 稀释股本Δ", "5Y Diluted Shares Δ"),
            s5_str,
            s5_col,
        ));
    if !rep.annual_rows.is_empty() {
        card1 = card1.child(roic_sparkline_for(&rep.annual_rows, l));
    }

    let mut card2 = obsidian_card()
        .flex_1()
        .min_w(px(0.0))
        .p(px(16.0))
        .child(card_title(
            l.t("现金流与估值真相", "Cash Flow & Valuation Reality"),
            &row.label,
        ))
        .child(kv_line(
            l.t("经营现金流 (OCF)", "Operating Cash Flow (OCF)"),
            fmt_money_compact(row.ocf, &rep.currency),
            INK_HERO,
        ))
        .child(kv_line(
            l.t("严苛 OE (OCF−CapEx−SBC)", "Strict OE (OCF−CapEx−SBC)"),
            fmt_money_compact_opt(row.strict_owner_earnings, &rep.currency),
            match row.strict_owner_earnings {
                Some(v) if v > Decimal::ZERO => EMERALD_FG,
                Some(_) => CORAL_FG,
                None => INK_MUTED,
            },
        ))
        .child(kv_line(
            l.t("严苛 FCF% (市值 / EV)", "Strict FCF Yield (MCap / EV)"),
            format!("{ym_str} / {ye_str}"),
            if ym_pol < 0 {
                CORAL_FG
            } else if ym_str == "—" && ye_str == "—" {
                INK_MUTED
            } else {
                INK_PRIMARY
            },
        ))
        .child(kv_line(
            l.t("击球价 (Strike Price)", "Strike Price"),
            fmt_price(strike_price, &rep.currency),
            if strike_price.is_some() {
                EMERALD_FG
            } else {
                INK_MUTED
            },
        ))
        .child(kv_line(
            l.t("合理价值区间 (Low–High)", "Fair Value (Low–High)"),
            fv_band_str,
            INK_PRIMARY,
        ))
        .child(kv_line(
            l.t("芒格裁决", "Munger Verdict"),
            format!(
                "{} · {} · {}",
                rep.bucket.code(),
                rep.status.label_for(l),
                dossier_verdict
            ),
            status_colors(rep.status).0,
        ))
        .child(ratio_bar(
            l.t("CapEx 开支", "CapEx").to_owned(),
            fmt_money_compact(row.capex, &rep.currency),
            Some(row.capex),
            row.ocf,
            GOLD_FG,
        ))
        .child(ratio_bar(
            l.t("SBC 激励", "SBC Dilution").to_owned(),
            fmt_money_compact_opt(row.sbc, &rep.currency),
            row.sbc,
            row.ocf,
            CORAL_FG,
        ))
        .child(ratio_bar(
            l.t("剩余严苛 OE", "Remaining Strict OE").to_owned(),
            fmt_money_compact_opt(row.strict_owner_earnings, &rep.currency),
            row.strict_owner_earnings.map(|v| v.max(Decimal::ZERO)),
            row.ocf,
            EMERALD_FG,
        ))
        .child(kv_line(
            l.t("回购 / SBC", "Buybacks / SBC"),
            format!(
                "{} / {}",
                fmt_money_compact_opt(row.buybacks, &rep.currency),
                fmt_money_compact_opt(row.sbc, &rep.currency)
            ),
            if row.buybacks.is_some() || row.sbc.is_some() {
                INK_SECONDARY
            } else {
                INK_MUTED
            },
        ));

    if let Some(note) = ytd_callout {
        card2 = card2.child(
            view()
                .mt(px(10.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded_sm()
                .bg(rgb(BG_WELL))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .child(
                    text(note)
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_SECONDARY))
                        .truncate()
                        .number_of_lines(1),
                ),
        );
    }

    view()
        .flex()
        .flex_row()
        .gap(px(16.0))
        .child(card1)
        .child(card2)
}

fn render_multi_year_table(rep: &MungerReport, l: UiLang) -> View {
    let mut all_rows: Vec<&AnnualMetricRow> = rep.annual_rows.iter().collect();
    if let Some(ttm) = &rep.ttm_row {
        all_rows.push(ttm);
    }
    if all_rows.is_empty() {
        return obsidian_card().p(px(16.0)).child(
            text(l.t(
                "暂无多年结构化财务底表。",
                "No multi-year financial history yet.",
            ))
            .font_family(FONT_UI)
            .text_size(px(TEXT_XS))
            .text_color(rgb(INK_MUTED)),
        );
    }

    let fin_cols: &[(&str, f32, bool)] = &[
        (l.t("期间", "Period"), 116.0, false),
        (l.t("营收", "Revenue"), 82.0, true),
        (l.t("毛利率", "GM%"), 66.0, true),
        ("ROIC", 66.0, true),
        ("OCF", 82.0, true),
        ("CapEx", 76.0, true),
        ("SBC", 72.0, true),
        (l.t("严苛 OE", "Strict OE"), 84.0, true),
        (l.t("回购", "Buyback"), 76.0, true),
        (l.t("稀释股本", "Shares"), 0.0, true),
    ];

    let sha_short = rep
        .raw_sha256
        .as_deref()
        .map(|s| &s[..s.len().min(16)])
        .unwrap_or("—");
    let tag_summary = rep
        .provenance_summary
        .iter()
        .take(5)
        .map(|p| format!("{}:{}", p.field, p.source_tag))
        .collect::<Vec<_>>()
        .join(" · ");

    let mut card = obsidian_card()
        .child(
            view()
                .px(px(16.0))
                .py(px(10.0))
                .bg(rgb(BG_WELL))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            text(l.t("多年财务底表", "Multi-Year Ledger"))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(concept_hint_static(l.t(
                            "直接从 SEC EDGAR XBRL 一手解析的确定性口径，绝不由 LLM 补全。",
                            "Parsed deterministically from primary SEC EDGAR XBRL filings; never LLM-estimated.",
                        ))),
                )
                .child(
                    text(format!(
                        "CIK {} · SHA256 {}",
                        rep.cik.as_deref().unwrap_or("MANUAL"),
                        sha_short
                    ))
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                ),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .px(px(12.0))
                .py(px(8.0))
                .bg(rgb(BG_ROW_ALT))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .children(fin_cols.iter().map(|(lbl, w, right)| {
                    let mut cell = view().px(px(4.0));
                    if *w > 0.0 {
                        cell = cell.w(px(*w)).flex_shrink_0();
                    } else {
                        cell = cell.flex_1().min_w(px(0.0));
                    }
                    let mut t = text((*lbl).to_owned())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_MUTED));
                    if *right {
                        t = t.text_right();
                    }
                    cell.child(t)
                })),
        )
        .children(all_rows.into_iter().rev().enumerate().map(|(i, r)| {
            let is_ttm = r.label.starts_with("TTM");
            let row_bg = if is_ttm {
                rgba(EMERALD_BG)
            } else {
                rgb(if i % 2 == 0 { BG_SURFACE } else { BG_ROW_ALT })
            };
            let (gm_str, _) = fmt_pct_clean(r.gross_margin_pct, false);
            let (roic_str, _) = fmt_pct_clean(r.roic_pct, false);

            view()
                .flex()
                .flex_row()
                .items_center()
                .px(px(12.0))
                .py(px(8.0))
                .bg(row_bg)
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .child(
                    view().w(px(116.0)).flex_shrink_0().px(px(4.0)).child(
                        text(r.label.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .font_weight(if is_ttm {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(rgb(if is_ttm { EMERALD_FG } else { INK_PRIMARY }))
                            .truncate()
                            .number_of_lines(1),
                    ),
                )
                .child(mono_right(
                    fmt_money_compact(r.revenue, &rep.currency),
                    82.0,
                    INK_PRIMARY,
                ))
                .child(mono_right(
                    gm_str,
                    66.0,
                    if r.gross_margin_pct.is_some() {
                        INK_SECONDARY
                    } else {
                        INK_MUTED
                    },
                ))
                .child(mono_right(
                    roic_str,
                    66.0,
                    match r.roic_pct {
                        Some(v) if v >= Decimal::from(15) => INK_PRIMARY,
                        Some(_) => GOLD_FG,
                        None => INK_MUTED,
                    },
                ))
                .child(mono_right(
                    fmt_money_compact(r.ocf, &rep.currency),
                    82.0,
                    INK_SECONDARY,
                ))
                .child(mono_right(
                    fmt_money_compact(r.capex, &rep.currency),
                    76.0,
                    INK_SECONDARY,
                ))
                .child(mono_right(
                    fmt_money_compact_opt(r.sbc, &rep.currency),
                    72.0,
                    if r.sbc.is_some() {
                        INK_SECONDARY
                    } else {
                        INK_MUTED
                    },
                ))
                .child(mono_right(
                    fmt_money_compact_opt(r.strict_owner_earnings, &rep.currency),
                    84.0,
                    match r.strict_owner_earnings {
                        Some(v) if v > Decimal::ZERO => EMERALD_FG,
                        Some(_) => CORAL_FG,
                        None => INK_MUTED,
                    },
                ))
                .child(mono_right(
                    fmt_money_compact_opt(r.buybacks, &rep.currency),
                    76.0,
                    if r.buybacks.is_some() {
                        INK_SECONDARY
                    } else {
                        INK_MUTED
                    },
                ))
                .child(
                    view().flex_1().min_w(px(0.0)).px(px(4.0)).child(
                        text(fmt_shares_compact(r.diluted_shares))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_SECONDARY))
                            .text_right(),
                    ),
                )
        }));

    if !tag_summary.is_empty() {
        card = card.child(
            view().px(px(16.0)).py(px(8.0)).bg(rgb(BG_WELL)).child(
                text(format!("{}: {tag_summary}", l.t("XBRL 溯源", "XBRL Trace")))
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED))
                    .truncate()
                    .number_of_lines(1),
            ),
        );
    }

    card
}

fn xbrl_inspector_body(store: AppStore, rep: &MungerReport) -> View {
    let l = store.lang.get();
    let use_ttm = store.use_ttm_basis.get();
    let row_opt = primary_row(rep, use_ttm);
    let val_opt = primary_val(rep, use_ttm);

    let sha_full = rep
        .raw_sha256
        .as_deref()
        .unwrap_or_else(|| l.t("— (无一手 XBRL 源文件)", "— (No primary XBRL source)"))
        .to_owned();
    let raw_file_path = match &rep.cik {
        Some(cik) => format!("data/munger/raw/sec/CIK{cik}.json"),
        None => format!("workbench/manual_financials/{}.json", rep.ticker),
    };
    let edgar_url = rep
        .cik
        .as_deref()
        .and_then(|c| edgar::normalize_cik(c).ok())
        .map(|norm| format!("https://www.sec.gov/edgar/browse/?CIK={norm}"));
    let report_cmd = format!("m3 report {}", rep.ticker);
    let t_report = rep.ticker.clone();

    let formula_lines: Vec<(&str, String)> = match (row_opt, val_opt) {
        (Some(r), Some(v)) => {
            let oe_ps = v
                .owner_earnings_per_share
                .map(|x| format!("{}{:.4}/sh", currency_prefix(&rep.currency), x))
                .unwrap_or_else(|| "—".into());
            let roic_str = r
                .roic_pct
                .map(|x| format!("{x:.2}%"))
                .unwrap_or_else(|| roic_unavailable_label(r.roic_note.as_deref()));
            let mcap = fmt_money_compact_opt(v.market_cap, &rep.currency);
            let yield_s = fmt_pct_clean(v.strict_fcf_yield_mcap_pct, false).0;
            let imp_s = fmt_pct_clean(v.implied_growth_for_target_pct, true).0;
            vec![
                (
                    l.t("严苛股东盈余", "Strict Owner Earnings"),
                    format!(
                        "Strict_OE = OCF({}) - CapEx({}) - SBC({}) = {}",
                        fmt_money_compact(r.ocf, &rep.currency),
                        fmt_money_compact(r.capex, &rep.currency),
                        fmt_money_compact_opt(r.sbc, &rep.currency),
                        fmt_money_compact_opt(r.strict_owner_earnings, &rep.currency)
                    ),
                ),
                (
                    l.t("投入资本回报率", "Return on Invested Cap"),
                    format!(
                        "ROIC = NOPAT({}) / IC(Debt {} + Eq {} - Cash {} = {}) = {}",
                        fmt_money_compact_opt(r.nopat, &rep.currency),
                        fmt_money_compact_opt(r.total_debt, &rep.currency),
                        fmt_money_compact(r.equity, &rep.currency),
                        fmt_money_compact_opt(r.cash_and_st, &rep.currency),
                        fmt_money_compact_opt(r.invested_capital, &rep.currency),
                        roic_str
                    ),
                ),
                (
                    l.t("保守击球价格", "Conservative Strike"),
                    format!(
                        "Strike = OE_per_share({oe_ps}) * Multiple_low({}x) = {}",
                        rep.assumptions.multiple_low,
                        fmt_price(v.strike_price, &rep.currency)
                    ),
                ),
                (
                    l.t("反向隐含增速", "Reverse Implied Growth"),
                    format!(
                        "Implied_g = Target({}%) - Strict_OE/MCap({} / {} = {yield_s}) = {imp_s}",
                        rep.assumptions.target_return_pct,
                        fmt_money_compact_opt(v.strict_owner_earnings, &rep.currency),
                        mcap,
                    ),
                ),
            ]
        }
        (Some(r), None) => vec![(
            l.t("严苛股东盈余", "Strict Owner Earnings"),
            format!(
                "Strict_OE = OCF({}) - CapEx({}) - SBC({}) = {}",
                fmt_money_compact(r.ocf, &rep.currency),
                fmt_money_compact(r.capex, &rep.currency),
                fmt_money_compact_opt(r.sbc, &rep.currency),
                fmt_money_compact_opt(r.strict_owner_earnings, &rep.currency)
            ),
        )],
        _ => vec![(
            "STATUS",
            l.t(
                "无一手结构化财务事实（绝不由 LLM 补全）。",
                "No primary structured financial facts (never fabricated by LLM).",
            )
            .into(),
        )],
    };

    let dossier_opt = store.selected_dossier.get();
    let dossier_provenance = dossier_opt
        .as_ref()
        .map(|d| {
            let status = if d.human_judgment.pending_human_review {
                l.t("待人类答辩", "pending_human_review")
            } else {
                l.t("已费曼核验", "verified")
            };
            format!(
                "{} · updated {} · {} ({})",
                d.draft_source,
                d.updated_at,
                status,
                d.human_judgment
                    .feynman_checked_at
                    .as_deref()
                    .unwrap_or("—")
            )
        })
        .unwrap_or_else(|| l.t("— (尚无审讯档案)", "— (No dossier yet)").to_owned());

    let mut prov_actions = view()
        .mt(px(16.0))
        .pt(px(12.0))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(px(8.0))
        .child(action_button(
            l.t("复制 `m3 report` CLI", "Copy CLI"),
            false,
            move || {
                copy_text_with_notice(store, report_cmd.clone(), &format!("report {t_report}"));
            },
        ));

    if let Some(sha_raw) = rep.raw_sha256.clone() {
        let t_sha = rep.ticker.clone();
        prov_actions = prov_actions.child(action_button(
            l.t("复制 SHA-256", "Copy SHA-256"),
            false,
            move || {
                copy_text_with_notice(store, sha_raw.clone(), &format!("{t_sha} SHA-256"));
            },
        ));
    }

    if let Some(url) = edgar_url {
        let url_copy = url.clone();
        let url_open = url.clone();
        let t_url = rep.ticker.clone();
        prov_actions = prov_actions
            .child(action_button(
                l.t("复制 SEC EDGAR 链接", "Copy SEC EDGAR Link"),
                false,
                move || {
                    copy_text_with_notice(store, url_copy.clone(), &format!("{t_url} EDGAR"));
                },
            ))
            .child(action_button(
                l.t("在浏览器打开 SEC EDGAR", "Open SEC EDGAR"),
                true,
                move || {
                    open_external_url(&url_open);
                },
            ));
    }

    view()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(render_multi_year_table(rep, l))
        .child(
            obsidian_card()
                .p(px(20.0))
                .child(section_header(
                    l.t(
                        "确定性 Rust 算术推导 (rust_decimal)",
                        "Deterministic Rust Arithmetic Trace (rust_decimal)",
                    ),
                    l.t("零 LLM 估算", "ZERO LLM ESTIMATION").to_owned(),
                ))
                .children(formula_lines.into_iter().map(|(lbl, expr)| {
                    view()
                        .py(px(8.0))
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(16.0))
                        .child(
                            view().w(px(130.0)).flex_shrink_0().child(
                                text(lbl.to_owned())
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            ),
                        )
                        .child(
                            view().flex_1().min_w(px(0.0)).child(
                                text(expr)
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_PRIMARY)),
                            ),
                        )
                })),
        )
        .child(
            obsidian_card()
                .p(px(20.0))
                .child(section_header(
                    l.t(
                        "一手溯源与 SHA-256 校验",
                        "Source Provenance & SHA-256 Verification",
                    ),
                    format!("CIK {}", rep.cik.as_deref().unwrap_or("MANUAL")),
                ))
                .child(
                    view()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(inspector_kv("SOURCE_FILE", raw_file_path))
                        .child(inspector_kv("RAW_SHA256", sha_full))
                        .child(inspector_kv(
                            "FILING_END",
                            rep.latest_filing_end
                                .clone()
                                .unwrap_or_else(|| "—".to_owned()),
                        ))
                        .child(inspector_kv("DOSSIER_STATE", dossier_provenance))
                        .child(inspector_kv(
                            "PRICE_URL",
                            rep.price
                                .as_ref()
                                .map(|p| p.source_url.clone())
                                .unwrap_or_else(|| "—".to_owned()),
                        )),
                )
                .child(prov_actions),
        )
        .child(
            obsidian_card()
                .child(
                    view()
                        .px(px(16.0))
                        .py(px(12.0))
                        .bg(rgb(BG_WELL))
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            text(l.t(
                                "SEC XBRL 科目映射 (最新年报溯源)",
                                "SEC XBRL Concept Mapping (Latest Annual Provenance)",
                            ))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(format!(
                                "{} {}",
                                rep.provenance_summary.len(),
                                l.t("个字段", "fields")
                            ))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .px(px(16.0))
                        .py(px(8.0))
                        .bg(rgb(BG_ROW_ALT))
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .child(
                            view().w(px(130.0)).flex_shrink_0().child(
                                text("FIELD")
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            ),
                        )
                        .child(
                            view().flex_1().min_w(px(0.0)).child(
                                text("XBRL_TAG / FORMULA")
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            ),
                        )
                        .child(
                            view().w(px(105.0)).flex_shrink_0().child(
                                text("PERIOD_END")
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            ),
                        )
                        .child(
                            view().w(px(105.0)).flex_shrink_0().child(
                                text("FILED")
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            ),
                        )
                        .child(
                            view().w(px(170.0)).flex_shrink_0().child(
                                text("NOTE")
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            ),
                        ),
                )
                .child(if rep.provenance_summary.is_empty() {
                    view().p(px(16.0)).child(
                        text(l.t(
                            "(暂无 XBRL 科目溯源记录)",
                            "(no XBRL concept provenance records)",
                        ))
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                    )
                } else {
                    view().flex().flex_col().children(
                        rep.provenance_summary.iter().enumerate().map(|(i, p)| {
                            view()
                                .flex()
                                .flex_row()
                                .px(px(16.0))
                                .py(px(8.0))
                                .bg(rgb(if i % 2 == 0 { BG_SURFACE } else { BG_ROW_ALT }))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .child(
                                    view().w(px(130.0)).flex_shrink_0().child(
                                        text(p.field.clone())
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(INK_HERO)),
                                    ),
                                )
                                .child(
                                    view().flex_1().min_w(px(0.0)).child(
                                        text(p.source_tag.clone())
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(SKY_FG))
                                            .truncate()
                                            .number_of_lines(1),
                                    ),
                                )
                                .child(
                                    view().w(px(105.0)).flex_shrink_0().child(
                                        text(p.period_end.clone())
                                            .font_family(FONT_MONO_NUM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(INK_SECONDARY)),
                                    ),
                                )
                                .child(
                                    view().w(px(105.0)).flex_shrink_0().child(
                                        text(p.filed.clone().unwrap_or_else(|| "—".to_owned()))
                                            .font_family(FONT_MONO_NUM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(INK_MUTED)),
                                    ),
                                )
                                .child(
                                    view().w(px(170.0)).flex_shrink_0().child(
                                        text(p.note.clone().unwrap_or_else(|| {
                                            l.t("直接映射", "direct").to_owned()
                                        }))
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED))
                                        .truncate()
                                        .number_of_lines(1),
                                    ),
                                )
                        }),
                    )
                }),
        )
}

fn properties_rail(store: AppStore) -> Scroll {
    scroll()
        .w(px(256.0))
        .flex_shrink_0()
        .h_full()
        .bg(rgb(0x111118))
        .border_l_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(dynamic(move || {
            let l = store.lang.get();
            let Some(rep) = active_report(store) else {
                return view();
            };
            let use_ttm = store.use_ttm_basis.get();
            let val_opt = primary_val(&rep, use_ttm);
            let cur_price = rep.price.as_ref().map(|p| p.price);
            let oe_ps = val_opt.and_then(|v| v.owner_earnings_per_share);
            let mult = store.sandbox_multiple.get();
            let ret = store.sandbox_target_return.get();
            let gro = store.sandbox_expected_growth.get();
            let sb = calculator::evaluate_sandbox(oe_ps, cur_price, mult, ret, gro);
            let active_strike = sb
                .fair_value
                .or_else(|| val_opt.and_then(|v| v.strike_price));
            let dist_live = match (cur_price, active_strike) {
                (Some(p), Some(s)) if s > Decimal::ZERO => Some(((p - s) / s) * Decimal::from(100)),
                _ => val_opt.and_then(|v| v.distance_to_strike_pct),
            };
            let (dist_live_str, _) = fmt_pct_clean(dist_live, true);

            let basis_tag = val_opt
                .map(|v| format!("{} · {}", v.basis_label, v.period_end))
                .unwrap_or_else(|| l.t("待录入", "Pending").to_owned());
            let sha_short = rep
                .raw_sha256
                .as_deref()
                .map(|s| &s[..s.len().min(12)])
                .unwrap_or("—");
            let tag = primary_circle_tag(&rep.circle);
            let tag_jump = tag.clone();
            let sub_circle = secondary_circle_detail(&rep.circle);

            let red_cnt = rep
                .warnings
                .iter()
                .filter(|w| w.severity == WarningSeverity::Red)
                .count();
            let amber_cnt = rep
                .warnings
                .iter()
                .filter(|w| w.severity == WarningSeverity::Amber)
                .count();
            let (fin_dot, fin_lbl, fin_col) = if rep.annual_rows.is_empty() {
                ("○", l.t("待录财报", "No Filings").to_owned(), INK_MUTED)
            } else if red_cnt > 0 {
                ("●", format!("{} {red_cnt}", l.t("红灯", "Red")), CORAL_FG)
            } else if amber_cnt > 0 {
                (
                    "○",
                    format!("{} {amber_cnt}", l.t("黄灯", "Amber")),
                    GOLD_FG,
                )
            } else {
                ("●", l.t("无红灯", "Clean").to_owned(), EMERALD_FG)
            };

            let pending_feynman = store.selected_dossier.with(|d| {
                d.as_ref()
                    .is_none_or(|x| x.human_judgment.pending_human_review)
            });
            let (strike_dot, strike_lbl, strike_col) = match dist_live {
                Some(d) if d <= Decimal::ZERO => {
                    ("●", l.t("击球区", "Strike Zone").to_owned(), EMERALD_FG)
                }
                Some(_) => ("○", dist_live_str.clone(), INK_SECONDARY),
                None => ("○", "—".to_owned(), INK_MUTED),
            };

            let bucket_btn = |target: Bucket| {
                let active = rep.bucket == target;
                let (fg, bg, br) = bucket_colors(target);
                let t_move = rep.ticker.clone();
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded_sm()
                    .cursor_pointer()
                    .bg(rgba(if active { bg } else { 0x161620FF }))
                    .border_1()
                    .border_color(rgba(if active {
                        br
                    } else {
                        BORDER_SUBTLE << 8 | 0xFF
                    }))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .on_press(move || {
                        if !active {
                            store.request_bucket_change(&t_move, target);
                        }
                    })
                    .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(rgb(fg)))
                    .child(
                        text(target.code())
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .font_weight(if active {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(rgb(if active { fg } else { INK_SECONDARY }))
                            .truncate()
                            .number_of_lines(1),
                    )
            };

            view()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .child(
                    text("PROPERTIES")
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(INK_MUTED)),
                )
                // 1. Valuation Anchor
                .child(
                    view()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_baseline()
                                .justify_between()
                                .child(
                                    text(l.t("现价", "Price"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                )
                                .child(
                                    text(fmt_price(cur_price, &rep.currency))
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_XL))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(INK_HERO)),
                                ),
                        )
                        .child(kv_line(
                            l.t("沙盒击球价", "Sandbox Strike"),
                            fmt_price(active_strike, &rep.currency),
                            if active_strike.is_some() {
                                EMERALD_FG
                            } else {
                                INK_MUTED
                            },
                        ))
                        .child(kv_line(
                            l.t("距击球价", "vs Strike"),
                            dist_live_str,
                            match dist_live {
                                Some(d) if d <= Decimal::ZERO => EMERALD_FG,
                                Some(_) => GOLD_FG,
                                None => INK_MUTED,
                            },
                        ))
                        .child(kv_line(
                            l.t("市值", "Market Cap"),
                            val_opt
                                .and_then(|v| v.market_cap)
                                .map(|m| fmt_money_compact(m, &rep.currency))
                                .unwrap_or_else(|| "—".to_owned()),
                            INK_SECONDARY,
                        ))
                        .child(kv_line(
                            l.t("企业价值 (EV)", "Enterprise Value (EV)"),
                            val_opt
                                .and_then(|v| v.enterprise_value)
                                .map(|m| fmt_money_compact(m, &rep.currency))
                                .unwrap_or_else(|| "—".to_owned()),
                            INK_SECONDARY,
                        )),
                )
                // 2. Decision Gate
                .child(
                    prop_section(l.t("决策检查", "DECISION GATE"))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .py(px(2.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .on_press(move || {
                                    store.company_view_tab.set(CompanyViewTab::FinancialsMemo);
                                })
                                .child(
                                    text(l.t("财务体检", "Financial Check"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(
                                    text(format!("{fin_dot} {fin_lbl}"))
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(fin_col)),
                                ),
                        )
                        .child({
                            let tk_feynman_rail = rep.ticker.clone();
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .py(px(2.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .on_press(move || {
                                    animate_shell_spring();
                                    store.open_alan_for_scope(
                                        AlanScope::Company(tk_feynman_rail.clone()),
                                        AlanCanvasTab::Feynman,
                                    );
                                })
                                .child(
                                    text(l.t("费曼答辩", "Feynman Gate"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(
                                    text(if pending_feynman {
                                        l.t("○ 待答辩 →", "○ Pending →")
                                    } else {
                                        l.t("● 已确认", "● Verified")
                                    })
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(
                                        if pending_feynman { GOLD_FG } else { EMERALD_FG },
                                    )),
                                )
                        })
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .py(px(2.0))
                                .child(
                                    text(l.t("击球价格", "Strike Price"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(
                                    text(format!("{strike_dot} {strike_lbl}"))
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(strike_col)),
                                ),
                        ),
                )
                // 3. Circle 2x2 Switcher
                .child({
                    let tk_remove = rep.ticker.clone();
                    prop_section(l.t("能力圈分层", "CIRCLE · TIER"))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .gap(px(6.0))
                                .child(bucket_btn(Bucket::Watch))
                                .child(bucket_btn(Bucket::Core)),
                        )
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .gap(px(6.0))
                                .child(bucket_btn(Bucket::Out))
                                .child(bucket_btn(Bucket::TooHard)),
                        )
                        .child(
                            view()
                                .w_full()
                                .px(px(8.0))
                                .py(px(5.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgba(CORAL_BG)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_center()
                                .gap(px(5.0))
                                .on_press(move || store.remove_company_from_circle(&tk_remove))
                                .child(icon(ICON_CLOSE, 11.0).text_color(rgb(INK_MUTED)))
                                .child(
                                    text(l.t("移出能力圈", "Remove from Circle"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED))
                                        .number_of_lines(1),
                                ),
                        )
                })
                // 4. Circle Tag Linkage
                .child(
                    prop_section(l.t("能力圈标签", "LABELS · CIRCLE")).child(
                        view()
                            .flex()
                            .flex_row()
                            .items_center()
                            .flex_wrap()
                            .gap(px(6.0))
                            .child(crate::components::circle_tag_button(
                                tag,
                                false,
                                false,
                                None,
                                move || store.jump_to_tag(tag_jump.clone()),
                            ))
                            .child(if let Some(det) = sub_circle {
                                view().child(
                                    text(det)
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                )
                            } else {
                                view()
                            }),
                    ),
                )
                // 5. Provenance Metadata
                .child(
                    prop_section(l.t("财报溯源", "PROVENANCE"))
                        .child(kv_line(
                            "CIK",
                            rep.cik.clone().unwrap_or_else(|| "MANUAL".to_owned()),
                            INK_SECONDARY,
                        ))
                        .child(kv_line(l.t("口径", "Basis"), basis_tag, INK_SECONDARY))
                        .child(kv_line(
                            l.t("最近财报", "Last Filing"),
                            rep.latest_filing_end
                                .clone()
                                .unwrap_or_else(|| l.t("待录入", "Pending").to_owned()),
                            INK_SECONDARY,
                        ))
                        .child(kv_line("SHA-256", sha_short.to_owned(), INK_MUTED)),
                )
        }))
}
