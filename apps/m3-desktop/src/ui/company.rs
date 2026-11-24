use super::{DossierTab, MungerWorkbench, NavView, components::*, format::*, theme::*};
use crate::{
    calculator,
    model::{
        AnnualMetricRow, Bucket, MungerReport, SandboxResult, StrikeZoneStatus, ValuationSnapshot,
        WarningSeverity, primary_circle_tag, secondary_circle_detail,
    },
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;

pub(super) fn roic_unavailable_label(roic_note: Option<&str>) -> String {
    match roic_note {
        Some("net_cash_exceeds_book_capital") => "Net Cash > Book Capital".into(),
        Some("total_debt_unavailable") => "— (Debt Unavailable)".into(),
        Some("cash_and_st_unavailable") => "— (Cash Unavailable)".into(),
        Some("tax_expense_unavailable") => "— (Tax Unavailable)".into(),
        Some(note) => format!("— ({note})"),
        None => "—".into(),
    }
}

impl MungerWorkbench {
    pub(super) fn render_company_shell(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let Some(rep) = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
        else {
            return div().p_6().child(
                card_shell()
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
                            .child(l.t("未选择公司", "No Company Selected")),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .line_height(px(18.))
                            .text_color(rgb(INK_SECONDARY))
                            .child(l.t(
                                "当前能力圈中暂无已选标的，请先从「全市场雷达」将公司加入能力圈，或在「能力圈总览」中选择一家标的。",
                                "No holding is selected in your Circle. Promote a company from Universe Radar or pick one in Circle Overview.",
                            )),
                    )
                    .child(
                        div().pt_1().child(
                            div()
                                .id("empty-dossier-go-universe")
                                .px_3()
                                .py_1p5()
                                .rounded_md()
                                .cursor_pointer()
                                .bg(rgba(SKY_BG))
                                .border_1()
                                .border_color(rgba(SKY_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .active(|s| s.bg(rgb(BG_ACTIVE)))
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(SKY_FG))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.switch_view(NavView::Universe);
                                    cx.notify();
                                }))
                                .child(l.t("前往全市场雷达 (⌘2)", "Open Universe Radar (⌘2)")),
                        ),
                    ),
            );
        };

        let row_opt = primary_row(rep, self.use_ttm_basis);
        let val_opt = primary_val(rep, self.use_ttm_basis);
        let cur_price = rep.price.as_ref().map(|p| p.price);
        let oe_per_share = val_opt.and_then(|v| v.owner_earnings_per_share);
        let sandbox = calculator::evaluate_sandbox(
            oe_per_share,
            cur_price,
            self.sandbox_multiple,
            self.sandbox_target_return,
            self.sandbox_expected_growth,
        );
        let sandbox_dirty = self.sandbox_multiple != rep.assumptions.multiple_low
            || self.sandbox_target_return != rep.assumptions.target_return_pct
            || self.sandbox_expected_growth != rep.assumptions.expected_growth_pct;

        let active_strike = sandbox
            .fair_value
            .or_else(|| val_opt.and_then(|v| v.strike_price));
        let dist_live = match (cur_price, active_strike) {
            (Some(p), Some(s)) if s > Decimal::ZERO => Some(((p - s) / s) * Decimal::from(100)),
            _ => val_opt.and_then(|v| v.distance_to_strike_pct),
        };
        let (dist_live_str, _) = fmt_pct_clean(dist_live, true);

        let is_tab_overview =
            self.active_view == NavView::Dossier && self.dossier_tab == DossierTab::Overview;
        let is_tab_agent = self.active_view == NavView::Agent;
        let is_tab_inspector =
            self.active_view == NavView::Dossier && self.dossier_tab == DossierTab::Inspector;

        let prompt_ticker = rep.ticker.clone();
        let prompt_str = build_alan_interrogation_prompt_for(&rep.ticker, &rep.name, rep.bucket, l);
        let dr_header_ticker = rep.ticker.clone();

        // Unified 44px Sub-header: Routine Tabs + Research Desk Dispatcher + Basis Control
        let subheader = div()
            .h(px(44.))
            .flex_shrink_0()
            .px_5()
            .bg(rgb(BG_SURFACE))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .font_family(self.fonts.mono_num.clone())
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(INK_HERO))
                            .child(rep.ticker.clone()),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_sm()
                            .text_color(rgb(INK_SECONDARY))
                            .child(rep.name.clone()),
                    )
                    .when(rep.status == StrikeZoneStatus::InStrikeZone, |d| {
                        d.child(badge(
                            l.t("击球区", "Strike Zone").into(),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                            &self.fonts,
                        ))
                    })
                    .child(v_divider())
                    .child(
                        seg_group()
                            .child(seg_tab(
                                "tab-overview",
                                l.t("1 财报与活研报", "1 Financials & Memo"),
                                is_tab_overview,
                                None,
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.dossier_tab = DossierTab::Overview;
                                    this.active_view = NavView::Dossier;
                                    cx.notify();
                                }),
                            ))
                            .child(seg_tab(
                                "tab-agent",
                                "2 Deep Research & Alan",
                                is_tab_agent,
                                Some((VIOLET_FG, VIOLET_BG)),
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.active_view = NavView::Agent;
                                    cx.notify();
                                }),
                            ))
                            .child(seg_tab(
                                "tab-inspector",
                                l.t("3 XBRL 底表", "3 XBRL Ledger"),
                                is_tab_inspector,
                                None,
                                false,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.dossier_tab = DossierTab::Inspector;
                                    this.active_view = NavView::Dossier;
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(!is_tab_agent, |d| {
                        d.child(
                            div()
                                .id("comp-quick-deep-research")
                                .px_2p5()
                                .py_1()
                                .rounded_sm()
                                .bg(rgba(VIOLET_BG))
                                .border_1()
                                .border_color(rgba(VIOLET_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(VIOLET_FG))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.active_view = NavView::Agent;
                                    this.start_deep_research(&dr_header_ticker, None, "manual", cx);
                                }))
                                .child(if self.deep_research_running {
                                    l.t("Deep Research 运行中…", "Deep Research Running…")
                                } else {
                                    l.t(
                                        "Deep Research · 4 视角并行",
                                        "Deep Research · 4 Perspectives",
                                    )
                                }),
                        )
                    })
                    .when(is_tab_agent, |d| {
                        d.child(
                            div()
                                .id("ag-copy-prompt")
                                .px_2p5()
                                .py_1()
                                .rounded_sm()
                                .bg(rgba(VIOLET_BG))
                                .border_1()
                                .border_color(rgba(VIOLET_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_cli_command(
                                        prompt_str.clone(),
                                        &format!("Alan Prompt {prompt_ticker}"),
                                        cx,
                                    );
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(VIOLET_FG))
                                        .child(l.t("复制审讯 Prompt", "Copy Prompt")),
                                )
                                .child(kbd("p", &self.fonts)),
                        )
                    })
                    .when(!is_tab_agent && rep.ttm_valuation.is_some(), |d| {
                        d.child(
                            seg_group()
                                .child(seg_tab(
                                    "seg-ttm",
                                    "TTM",
                                    self.use_ttm_basis,
                                    None,
                                    true,
                                    &self.fonts,
                                    cx.listener(|this, _, _, cx| {
                                        this.use_ttm_basis = true;
                                        cx.notify();
                                    }),
                                ))
                                .child(seg_tab(
                                    "seg-fy",
                                    "FY",
                                    !self.use_ttm_basis,
                                    None,
                                    true,
                                    &self.fonts,
                                    cx.listener(|this, _, _, cx| {
                                        this.use_ttm_basis = false;
                                        cx.notify();
                                    }),
                                )),
                        )
                    }),
            );

        // Left Scrollable Canvas
        let mut main_canvas = div()
            .id("company-main-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_4();

        if is_tab_overview {
            main_canvas = self.render_company_overview_body(
                main_canvas,
                rep,
                row_opt,
                val_opt,
                active_strike,
                &sandbox,
                sandbox_dirty,
                cx,
            );
        } else if is_tab_agent {
            main_canvas = self.render_company_agent_body(main_canvas, rep, cx);
        } else {
            main_canvas = main_canvas
                .child(self.render_multi_year_table(rep))
                .child(self.render_inspector_panel(rep, row_opt, val_opt, cx));
        }

        let props_rail = self.render_company_properties_rail(
            rep,
            val_opt,
            cur_price,
            active_strike,
            dist_live,
            dist_live_str,
            cx,
        );

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(subheader)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(main_canvas)
                    .child(props_rail),
            )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_company_overview_body(
        &self,
        mut canvas_el: gpui::Stateful<gpui::Div>,
        rep: &MungerReport,
        row_opt: Option<&AnnualMetricRow>,
        val_opt: Option<&ValuationSnapshot>,
        active_strike: Option<Decimal>,
        sandbox: &SandboxResult,
        sandbox_dirty: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let l = self.lang;
        if !rep.warnings.is_empty() {
            canvas_el = canvas_el.child(render_warning_panel_for(&rep.warnings, l, &self.fonts));
        }

        if let Some(reason) = &rep.too_hard_reason {
            canvas_el = canvas_el.child(
                div()
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .bg(rgba(SLATE_BG))
                    .border_1()
                    .border_color(rgba(SLATE_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(rgb(INK_PRIMARY))
                    .child(format!("{}: {reason}", l.t("边界理由", "Boundary Note"))),
            );
        }

        // Block 1 (HERO): Value-Anchored K-Line + Docked Inline Charlie Sandbox
        canvas_el = canvas_el.child(self.render_kline_card(
            rep,
            row_opt,
            active_strike,
            sandbox,
            sandbox_dirty,
            cx,
        ));

        // Block 2: Core Financial Check (2 side-by-side cards)
        if let Some(row) = row_opt {
            let roic_display = match (rep.roic_5y_avg_pct, rep.roic_5y_stddev_pct) {
                (Some(avg), Some(std)) => format!("{avg:.1}% ± {std:.1}%"),
                _ => row
                    .roic_total_capital_pct
                    .map(|v| match l {
                        crate::i18n::UiLang::Zh => format!("总资本 {v:.1}% (净现金 > 账面)"),
                        crate::i18n::UiLang::En => {
                            format!("Total Cap {v:.1}% (Net Cash > Equity)")
                        }
                    })
                    .unwrap_or_else(|| "—".into()),
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

            let (ym_str, ym_pol) =
                fmt_pct_clean(val_opt.and_then(|v| v.strict_fcf_yield_mcap_pct), false);
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

            canvas_el = canvas_el.child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        card_shell()
                            .flex_1()
                            .min_w_0()
                            .p_4()
                            .child(card_title(
                                l.t("护城河 & 股本质量", "Moat & Share Discipline"),
                                l.t("5Y 稳定性", "5Y Consistency"),
                                &self.fonts,
                            ))
                            .child(kv_line("5Y ROIC", roic_display, INK_HERO, &self.fonts))
                            .child(kv_line(
                                l.t("5Y 毛利率", "5Y Gross Margin"),
                                gm_display,
                                INK_PRIMARY,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                l.t("当期净负债", "Net Debt"),
                                fmt_money_compact_opt(row.net_debt, &rep.currency),
                                match row.net_debt {
                                    Some(v) if v <= Decimal::ZERO => EMERALD_FG,
                                    Some(_) => INK_PRIMARY,
                                    None => INK_MUTED,
                                },
                                &self.fonts,
                            ))
                            .child(kv_line(
                                l.t("净负债 / 严苛 OE", "Net Debt / Strict OE"),
                                rep.net_debt_to_owner_earnings
                                    .map(|v| format!("{v:.2}x"))
                                    .unwrap_or_else(|| "—".into()),
                                if rep.net_debt_to_owner_earnings.is_some() {
                                    INK_SECONDARY
                                } else {
                                    INK_MUTED
                                },
                                &self.fonts,
                            ))
                            .child(kv_line(
                                l.t("3Y 稀释股本Δ", "3Y Diluted Shares Δ"),
                                s3_str,
                                s3_col,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                l.t("5Y 稀释股本Δ", "5Y Diluted Shares Δ"),
                                s5_str,
                                s5_col,
                                &self.fonts,
                            ))
                            .when(!rep.annual_rows.is_empty(), |d| {
                                d.child(roic_sparkline_for(&rep.annual_rows, l, &self.fonts))
                            }),
                    )
                    .child(
                        card_shell()
                            .flex_1()
                            .min_w_0()
                            .p_4()
                            .child(card_title(
                                l.t("现金流真相", "Cash Flow Reality"),
                                &row.label,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                l.t("经营现金流 (OCF)", "Operating Cash Flow (OCF)"),
                                fmt_money_compact(row.ocf, &rep.currency),
                                INK_HERO,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                l.t("严苛 OE (OCF−CapEx−SBC)", "Strict OE (OCF−CapEx−SBC)"),
                                fmt_money_compact_opt(row.strict_owner_earnings, &rep.currency),
                                match row.strict_owner_earnings {
                                    Some(v) if v > Decimal::ZERO => EMERALD_FG,
                                    Some(_) => CORAL_FG,
                                    None => INK_MUTED,
                                },
                                &self.fonts,
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
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                l.t("CapEx 开支", "CapEx").into(),
                                fmt_money_compact(row.capex, &rep.currency),
                                Some(row.capex),
                                row.ocf,
                                GOLD_FG,
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                l.t("SBC 激励", "SBC Dilution").into(),
                                fmt_money_compact_opt(row.sbc, &rep.currency),
                                row.sbc,
                                row.ocf,
                                CORAL_FG,
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                l.t("剩余严苛 OE", "Remaining Strict OE").into(),
                                fmt_money_compact_opt(row.strict_owner_earnings, &rep.currency),
                                row.strict_owner_earnings.map(|v| v.max(Decimal::ZERO)),
                                row.ocf,
                                EMERALD_FG,
                                &self.fonts,
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
                                &self.fonts,
                            ))
                            .when_some(ytd_callout, |d, note| {
                                d.child(
                                    div()
                                        .mt_3()
                                        .px_2p5()
                                        .py_1p5()
                                        .rounded_sm()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgb(BORDER_HAIRLINE))
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .truncate()
                                        .child(note),
                                )
                            }),
                    ),
            );
        } else {
            canvas_el = canvas_el.child(
                div()
                    .p_4()
                    .rounded_lg()
                    .bg(rgba(GOLD_BG))
                    .border_1()
                    .border_color(rgba(GOLD_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_size(px(13.))
                    .line_height(px(21.))
                    .text_color(rgb(GOLD_FG))
                    .child(match l {
                        crate::i18n::UiLang::Zh => format!(
                            "{} 暂无结构化财报数据；如需接入计算器，可录入 workbench/manual_financials/{}.json。",
                            rep.ticker, rep.ticker
                        ),
                        crate::i18n::UiLang::En => format!(
                            "No structured SEC financials for {} yet; add workbench/manual_financials/{}.json if manual entry is needed.",
                            rep.ticker, rep.ticker
                        ),
                    }),
            );
        }

        // Cross-company Peer Percentiles & Lookalike strip + Living Research Memo
        canvas_el = canvas_el
            .child(self.render_cross_company_context_strip(&rep.ticker, cx))
            .child(self.render_living_research_memo_card(&rep.ticker, cx));

        // Block 3: Human Sovereignty & Feynman Gate
        if let Some(dossier) = &self.selected_dossier {
            let human = &dossier.human_judgment;
            canvas_el = canvas_el.child(
                card_shell()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pb_2p5()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_2p5()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child(l.t(
                                                "人类判词 · 费曼防线",
                                                "Human Judgment · Feynman Gate",
                                            )),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .child(l.t(
                                                "仅本人确认 · AI 草稿见「2 Alan 审讯台」",
                                                "Human-Owned Only · AI drafts live under Tab 2",
                                            )),
                                    ),
                            )
                            .child(if human.pending_human_review {
                                badge(
                                    l.t("待答辩", "Pending Review").into(),
                                    GOLD_FG,
                                    GOLD_BG,
                                    GOLD_BORDER,
                                    &self.fonts,
                                )
                            } else {
                                badge(
                                    l.t("已确认", "Verified").into(),
                                    EMERALD_FG,
                                    EMERALD_BG,
                                    EMERALD_BORDER,
                                    &self.fonts,
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(
                                div().flex_1().min_w_0().child(prose_block(
                                    l.t("我的定性判词 (my_stance)", "My Stance (my_stance)"),
                                    human
                                        .my_stance
                                        .as_deref()
                                        .unwrap_or_else(|| l.t("（待填写）", "(pending)")),
                                    &self.fonts,
                                )),
                            )
                            .child(
                                div().flex_1().min_w_0().child(prose_block(
                                    l.t(
                                        "分歧权重判断 (weight_rationale)",
                                        "Sizing / Weight Rationale (weight_rationale)",
                                    ),
                                    human
                                        .weight_rationale
                                        .as_deref()
                                        .unwrap_or_else(|| l.t("（待填写）", "(pending)")),
                                    &self.fonts,
                                )),
                            ),
                    )
                    .child(
                        div()
                            .pt_2p5()
                            .border_t_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_SECONDARY))
                                            .child(l.t(
                                                "事前证伪指标 (falsification_metrics)",
                                                "Pre-Falsification Metrics (falsification_metrics)",
                                            )),
                                    )
                                    .when(human.falsification_metrics.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .mt_1()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_MUTED))
                                                .child(l.t("（尚未设置）", "(none configured)")),
                                        )
                                    })
                                    .children(human.falsification_metrics.iter().map(|m| {
                                        div()
                                            .mt_1()
                                            .font_family(self.fonts.ui.clone())
                                            .text_size(px(13.))
                                            .line_height(px(20.))
                                            .text_color(rgb(INK_PRIMARY))
                                            .child(format!("• {m}"))
                                    })),
                            )
                            .when_some(human.feynman_checked_at.clone(), |d, checked| {
                                d.child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .text_color(rgb(EMERALD_FG))
                                        .child(format!("{} {checked}", l.t("核验时间", "Checked"))),
                                )
                            }),
                    ),
            );
        }

        canvas_el
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_company_properties_rail(
        &self,
        rep: &MungerReport,
        val_opt: Option<&ValuationSnapshot>,
        cur_price: Option<Decimal>,
        active_strike: Option<Decimal>,
        dist_live: Option<Decimal>,
        dist_live_str: String,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let l = self.lang;
        let basis_tag = val_opt
            .map(|v| format!("{} · {}", v.basis_label, v.period_end))
            .unwrap_or_else(|| l.t("待录入", "Pending").into());
        let sha_short = rep
            .raw_sha256
            .as_deref()
            .map(|s| &s[..s.len().min(12)])
            .unwrap_or("—");
        let tag = primary_circle_tag(&rep.circle);
        let tag_for_jump = tag.clone();
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
            ("○", l.t("待录财报", "No Filings").into(), INK_MUTED)
        } else if red_cnt > 0 {
            ("●", format!("{} {red_cnt}", l.t("红灯", "Red")), CORAL_FG)
        } else if amber_cnt > 0 {
            (
                "○",
                format!("{} {amber_cnt}", l.t("黄灯", "Amber")),
                GOLD_FG,
            )
        } else {
            ("●", l.t("无红灯", "Clean").into(), EMERALD_FG)
        };

        let pending_feynman = self
            .selected_dossier
            .as_ref()
            .is_none_or(|d| d.human_judgment.pending_human_review);

        let (strike_dot, strike_lbl, strike_col) = match dist_live {
            Some(d) if d <= Decimal::ZERO => ("●", l.t("击球区", "Strike Zone").into(), EMERALD_FG),
            Some(_) => ("○", dist_live_str.clone(), INK_SECONDARY),
            None => ("○", "—".into(), INK_MUTED),
        };

        div()
            .id("company-props-scroll")
            .w(px(256.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(0x111118))
            .border_l_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .p_4()
            .flex()
            .flex_col()
            .gap_3p5()
            .child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(INK_MUTED))
                    .child("PROPERTIES"),
            )
            // 1. Valuation Anchor (flat directly on rail)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(l.t("现价", "Price")),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(fmt_price(cur_price, &rep.currency)),
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
                        &self.fonts,
                    ))
                    .child(kv_line(
                        l.t("距击球价", "vs Strike"),
                        dist_live_str,
                        match dist_live {
                            Some(d) if d <= Decimal::ZERO => EMERALD_FG,
                            Some(_) => GOLD_FG,
                            None => INK_MUTED,
                        },
                        &self.fonts,
                    ))
                    .child(kv_line(
                        l.t("市值", "Market Cap"),
                        val_opt
                            .and_then(|v| v.market_cap)
                            .map(|m| fmt_money_compact(m, &rep.currency))
                            .unwrap_or_else(|| "—".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        l.t("企业价值 (EV)", "Enterprise Value (EV)"),
                        val_opt
                            .and_then(|v| v.enterprise_value)
                            .map(|m| fmt_money_compact(m, &rep.currency))
                            .unwrap_or_else(|| "—".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    )),
            )
            // 2. Silent 3-Line Munger Decision Gate
            .child(
                prop_section(
                    l.t("DECISION GATE · 决策检查", "DECISION GATE"),
                    &self.fonts,
                )
                .child(
                    div()
                        .id("gate-fin")
                        .flex()
                        .items_center()
                        .justify_between()
                        .py_0p5()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dossier_tab = DossierTab::Overview;
                            this.active_view = NavView::Dossier;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .text_color(rgb(INK_SECONDARY))
                                .child(l.t("财务体检", "Financial Check")),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .text_color(rgb(fin_col))
                                .child(format!("{fin_dot} {fin_lbl}")),
                        ),
                )
                .child(
                    div()
                        .id("gate-feynman")
                        .flex()
                        .items_center()
                        .justify_between()
                        .py_0p5()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.active_view = NavView::Agent;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .text_color(rgb(INK_SECONDARY))
                                .child(l.t("费曼答辩", "Feynman Gate")),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .text_color(rgb(if pending_feynman { GOLD_FG } else { EMERALD_FG }))
                                .child(if pending_feynman {
                                    l.t("○ 待答辩 →", "○ Pending →")
                                } else {
                                    l.t("● 已确认", "● Verified")
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .py_0p5()
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .text_color(rgb(INK_SECONDARY))
                                .child(l.t("击球价格", "Strike Price")),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .text_color(rgb(strike_col))
                                .child(format!("{strike_dot} {strike_lbl}")),
                        ),
                ),
            )
            // 3. Circle 2x2 Switcher
            .child(
                prop_section(l.t("CIRCLE · 能力圈分层", "CIRCLE · TIER"), &self.fonts)
                    .child(
                        div().flex().gap_1p5().children(
                            [Bucket::Core, Bucket::Watch].into_iter().map(|b| {
                                self.render_prop_bucket_btn(&rep.ticker, rep.bucket, b, cx)
                            }),
                        ),
                    )
                    .child(
                        div().flex().gap_1p5().children(
                            [Bucket::Out, Bucket::TooHard].into_iter().map(|b| {
                                self.render_prop_bucket_btn(&rep.ticker, rep.bucket, b, cx)
                            }),
                        ),
                    ),
            )
            // 4. Circle Tag Linkage
            .child(
                prop_section(l.t("LABELS · 能力圈", "LABELS · CIRCLE"), &self.fonts).child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_1p5()
                        .child(circle_tag_button(
                            "prop-tag-jump",
                            tag,
                            false,
                            false,
                            None,
                            &self.fonts,
                            cx.listener(move |this, _, _, cx| {
                                this.jump_to_tag(tag_for_jump.clone());
                                cx.notify();
                            }),
                        ))
                        .when_some(sub_circle, |d, det| {
                            d.child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(det),
                            )
                        }),
                ),
            )
            // 5. Provenance Metadata
            .child(
                prop_section(l.t("PROVENANCE · 溯源", "PROVENANCE"), &self.fonts)
                    .child(kv_line(
                        "CIK",
                        rep.cik.clone().unwrap_or_else(|| "MANUAL".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        l.t("口径", "Basis"),
                        basis_tag,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        l.t("最近财报", "Last Filing"),
                        rep.latest_filing_end
                            .clone()
                            .unwrap_or_else(|| l.t("待录入", "Pending").into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "SHA-256",
                        sha_short.to_owned(),
                        INK_MUTED,
                        &self.fonts,
                    )),
            )
    }

    pub(super) fn render_prop_bucket_btn(
        &self,
        ticker: &str,
        current: Bucket,
        target: Bucket,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let active = current == target;
        let (fg, bg, br) = bucket_colors(target);
        let t = ticker.to_owned();
        div()
            .id(gpui::SharedString::from(format!(
                "prop-bkt-{}",
                target.code()
            )))
            .flex_1()
            .min_w_0()
            .px_2()
            .py_1p5()
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
            .items_center()
            .gap_1p5()
            .on_click(cx.listener(move |this, _, _, cx| {
                if !active {
                    this.move_company_bucket(&t, target, cx);
                }
            }))
            .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(fg)))
            .child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .font_weight(if active {
                        gpui::FontWeight::BOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(rgb(if active { fg } else { INK_SECONDARY }))
                    .truncate()
                    .child(target.code()),
            )
    }

    pub(super) fn render_multi_year_table(&self, rep: &MungerReport) -> gpui::Div {
        let l = self.lang;
        let mut all_rows: Vec<&AnnualMetricRow> = rep.annual_rows.iter().collect();
        if let Some(ttm) = &rep.ttm_row {
            all_rows.push(ttm);
        }
        if all_rows.is_empty() {
            return card_shell()
                .p_4()
                .font_family(self.fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(l.t(
                    "暂无多年结构化财务底表。",
                    "No multi-year financial history yet.",
                ));
        }
        let fin_cols: &[(&str, f32, bool)] = &[
            (l.t("期间", "Period"), 122., false),
            (l.t("营收", "Revenue"), 84., true),
            (l.t("毛利率", "GM%"), 68., true),
            ("ROIC", 68., true),
            ("OCF", 84., true),
            ("CapEx", 80., true),
            ("SBC", 74., true),
            (l.t("严苛 OE", "Strict OE"), 88., true),
            (l.t("回购", "Buyback"), 80., true),
            (l.t("稀释股本", "Shares"), 0., true),
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

        card_shell()
            .child(
                div()
                    .px_4()
                    .py_3()
                    .bg(rgb(BG_WELL))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO))
                            .child(l.t(
                                "多年确定性财务底表（Rust 解析自 SEC EDGAR XBRL）",
                                "Multi-Year Deterministic Ledger (Rust SEC EDGAR XBRL)",
                            )),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(format!(
                                "CIK {} · SHA256 {}",
                                rep.cik.as_deref().unwrap_or("MANUAL"),
                                sha_short
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .px_3()
                    .py_2()
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .children(fin_cols.iter().map(|(lbl, w, right)| {
                        div()
                            .px_1()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED))
                            .when(*right, |d| d.text_right())
                            .when(*w > 0., |d| d.w(px(*w)).flex_shrink_0())
                            .when(*w == 0., |d| d.flex_1().min_w_0())
                            .child(*lbl)
                    })),
            )
            .children(all_rows.into_iter().rev().enumerate().map(|(i, r)| {
                let is_ttm = r.label.starts_with("TTM");
                let row_el = div()
                    .flex()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE));
                let row_el = if is_ttm {
                    row_el.bg(rgba(EMERALD_BG))
                } else {
                    row_el.bg(rgb(if i % 2 == 0 { BG_SURFACE } else { BG_ROW_ALT }))
                };
                let (gm_str, _) = fmt_pct_clean(r.gross_margin_pct, false);
                let (roic_str, _) = fmt_pct_clean(r.roic_pct, false);
                row_el
                    .child(
                        div()
                            .w(px(122.))
                            .flex_shrink_0()
                            .px_1()
                            .font_family(self.fonts.mono_num.clone())
                            .text_xs()
                            .font_weight(if is_ttm {
                                gpui::FontWeight::BOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(rgb(if is_ttm { EMERALD_FG } else { INK_PRIMARY }))
                            .truncate()
                            .child(r.label.clone()),
                    )
                    .child(mono_right(
                        fmt_money_compact(r.revenue, &rep.currency),
                        84.,
                        INK_PRIMARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        gm_str,
                        68.,
                        if r.gross_margin_pct.is_some() {
                            INK_SECONDARY
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        roic_str,
                        68.,
                        match r.roic_pct {
                            Some(v) if v >= Decimal::from(15) => INK_PRIMARY,
                            Some(_) => GOLD_FG,
                            None => INK_MUTED,
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.ocf, &rep.currency),
                        84.,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.capex, &rep.currency),
                        80.,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact_opt(r.sbc, &rep.currency),
                        74.,
                        if r.sbc.is_some() {
                            INK_SECONDARY
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact_opt(r.strict_owner_earnings, &rep.currency),
                        88.,
                        match r.strict_owner_earnings {
                            Some(v) if v > Decimal::ZERO => EMERALD_FG,
                            Some(_) => CORAL_FG,
                            None => INK_MUTED,
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact_opt(r.buybacks, &rep.currency),
                        80.,
                        if r.buybacks.is_some() {
                            INK_SECONDARY
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_1()
                            .text_right()
                            .font_family(self.fonts.mono_num.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child(fmt_shares_compact(r.diluted_shares)),
                    )
            }))
            .when(!tag_summary.is_empty(), |d| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(rgb(BG_WELL))
                        .font_family(self.fonts.mono_term.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .truncate()
                        .child(format!("{}: {tag_summary}", l.t("XBRL 溯源", "XBRL Trace"))),
                )
            })
    }

    pub(super) fn render_inspector_panel(
        &self,
        rep: &MungerReport,
        row_opt: Option<&AnnualMetricRow>,
        val_opt: Option<&ValuationSnapshot>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let sha_full = rep
            .raw_sha256
            .as_deref()
            .unwrap_or_else(|| l.t("— (无一手 XBRL 源文件)", "— (No primary XBRL source)"));
        let raw_file_path = match &rep.cik {
            Some(cik) => format!("data/munger/raw/sec/CIK{cik}.json"),
            None => format!("workbench/manual_financials/{}.json", rep.ticker),
        };
        let report_cmd = format!("./apps/m3-desktop/m3 report {}", rep.ticker);
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

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                card_shell()
                    .p_5()
                    .child(section_header(
                        "Deterministic Rust Arithmetic Trace (rust_decimal)",
                        "ZERO LLM ESTIMATION".into(),
                        &self.fonts,
                    ))
                    .children(formula_lines.into_iter().map(|(lbl, expr)| {
                        div()
                            .py_2()
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .w(px(120.))
                                    .flex_shrink_0()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(lbl),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(expr),
                            )
                    })),
            )
            .child(
                card_shell()
                    .p_5()
                    .child(section_header(
                        "Source Provenance & CLI Verification",
                        format!("CIK {}", rep.cik.as_deref().unwrap_or("MANUAL")),
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(inspector_kv("SOURCE_FILE", raw_file_path, &self.fonts))
                            .child(inspector_kv("RAW_SHA256", sha_full.to_owned(), &self.fonts))
                            .child(inspector_kv(
                                "FILING_END",
                                rep.latest_filing_end.clone().unwrap_or_else(|| "—".into()),
                                &self.fonts,
                            ))
                            .child(inspector_kv(
                                "PRICE_URL",
                                rep.price
                                    .as_ref()
                                    .map(|p| p.source_url.clone())
                                    .unwrap_or_else(|| "—".into()),
                                &self.fonts,
                            )),
                    )
                    .child(
                        div()
                            .mt_4()
                            .pt_3()
                            .border_t_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .id("insp-copy-report")
                                    .px_3()
                                    .py_1p5()
                                    .rounded_sm()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_PILL))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.copy_cli_command(
                                            report_cmd.clone(),
                                            &format!("report {t_report}"),
                                            cx,
                                        );
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_PRIMARY))
                                            .child("Copy CLI"),
                                    )
                                    .child(kbd("c", &self.fonts)),
                            ),
                    ),
            )
            .child(
                card_shell()
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .bg(rgb(BG_WELL))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child("SEC XBRL Concept Mapping (Latest Annual Provenance)"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("{} fields", rep.provenance_summary.len())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .px_4()
                            .py_2()
                            .bg(rgb(BG_ROW_ALT))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED))
                            .child(div().w(px(130.)).child("FIELD"))
                            .child(div().flex_1().min_w_0().child("XBRL_TAG / FORMULA"))
                            .child(div().w(px(110.)).child("PERIOD_END"))
                            .child(div().w(px(110.)).child("FILED"))
                            .child(div().w(px(180.)).child("NOTE")),
                    )
                    .when(rep.provenance_summary.is_empty(), |d| {
                        d.child(
                            div()
                                .p_4()
                                .font_family(self.fonts.mono_term.clone())
                                .text_xs()
                                .text_color(rgb(INK_MUTED))
                                .child("(no XBRL concept provenance records)"),
                        )
                    })
                    .children(rep.provenance_summary.iter().enumerate().map(|(i, p)| {
                        div()
                            .flex()
                            .px_4()
                            .py_2()
                            .bg(rgb(if i % 2 == 0 { BG_SURFACE } else { BG_ROW_ALT }))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.mono_term.clone())
                            .text_xs()
                            .child(
                                div()
                                    .w(px(130.))
                                    .flex_shrink_0()
                                    .text_color(rgb(INK_HERO))
                                    .child(p.field.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(rgb(SKY_FG))
                                    .truncate()
                                    .child(p.source_tag.clone()),
                            )
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_shrink_0()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_color(rgb(INK_SECONDARY))
                                    .child(p.period_end.clone()),
                            )
                            .child(
                                div()
                                    .w(px(110.))
                                    .flex_shrink_0()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_color(rgb(INK_MUTED))
                                    .child(p.filed.clone().unwrap_or_else(|| "—".into())),
                            )
                            .child(
                                div()
                                    .w(px(180.))
                                    .flex_shrink_0()
                                    .text_color(rgb(INK_MUTED))
                                    .truncate()
                                    .child(p.note.clone().unwrap_or_else(|| "direct".into())),
                            )
                    })),
            )
    }
}
