use super::{
    DossierTab, MungerWorkbench, NavView, components::*, format::*, theme::*,
};
use crate::{
    calculator,
    model::{
        AnnualMetricRow, Bucket, MungerReport, SandboxResult, StrikeZoneStatus, ValuationSnapshot,
        WarningSeverity, primary_circle_tag, secondary_circle_detail,
    },
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;

impl MungerWorkbench {
    pub(super) fn render_company_shell(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(rep) = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
        else {
            return div()
                .p_6()
                .font_family(self.fonts.ui.clone())
                .text_color(rgb(INK_SECONDARY))
                .child("未选择公司");
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
        let prompt_str = build_alan_interrogation_prompt(&rep.ticker, &rep.name, rep.bucket);

        // Unified 44px Sub-header: Routine Tabs + Single Contextual Control
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
                            "击球区".into(),
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
                                "1 财报与 K 线",
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
                                "2 Alan 审讯台",
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
                                "3 XBRL 底表",
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
                                        .child("复制审讯 Prompt"),
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
        if !rep.warnings.is_empty() {
            canvas_el = canvas_el.child(render_warning_panel(&rep.warnings, &self.fonts));
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
                    .child(format!("边界理由：{reason}")),
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
                    .map(|v| format!("总资本 {v:.1}% (净现金 > 账面)"))
                    .unwrap_or_else(|| "—".into()),
            };
            let gm_display = match (rep.gross_margin_5y_avg_pct, rep.gross_margin_5y_stddev_pct) {
                (Some(avg), Some(std)) => format!("{avg:.1}% ± {std:.1}%"),
                _ => fmt_pct_clean(row.gross_margin_pct, false).0,
            };
            let (s3_str, s3_pol) = fmt_pct_clean(rep.share_change_3y_pct, true);
            let s3_col = share_delta_color(rep.share_change_3y_pct, s3_pol);
            let (s5_str, s5_pol) = fmt_pct_clean(rep.share_change_5y_pct, true);
            let s5_col = if s5_pol == 0 { INK_MUTED } else { INK_SECONDARY };

            let (ym_str, ym_pol) =
                fmt_pct_clean(val_opt.and_then(|v| v.strict_fcf_yield_mcap_pct), false);
            let (ye_str, _) =
                fmt_pct_clean(val_opt.and_then(|v| v.strict_fcf_yield_ev_pct), false);
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
                    fmt_money_compact(y.ocf, &rep.currency),
                    fmt_money_compact(y.capex, &rep.currency),
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
                            .child(card_title("护城河 & 股本质量", "5Y 稳定性", &self.fonts))
                            .child(kv_line("5Y ROIC", roic_display, INK_HERO, &self.fonts))
                            .child(kv_line("5Y 毛利率", gm_display, INK_PRIMARY, &self.fonts))
                            .child(kv_line(
                                "当期净负债",
                                fmt_money_compact(row.net_debt, &rep.currency),
                                if row.net_debt <= Decimal::ZERO {
                                    EMERALD_FG
                                } else {
                                    INK_PRIMARY
                                },
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "净负债 / 严苛 OE",
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
                            .child(kv_line("3Y 稀释股本Δ", s3_str, s3_col, &self.fonts))
                            .child(kv_line("5Y 稀释股本Δ", s5_str, s5_col, &self.fonts))
                            .when(!rep.annual_rows.is_empty(), |d| {
                                d.child(roic_sparkline(&rep.annual_rows, &self.fonts))
                            }),
                    )
                    .child(
                        card_shell()
                            .flex_1()
                            .min_w_0()
                            .p_4()
                            .child(card_title("现金流真相", &row.label, &self.fonts))
                            .child(kv_line(
                                "经营现金流 (OCF)",
                                fmt_money_compact(row.ocf, &rep.currency),
                                INK_HERO,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "严苛 OE (OCF−CapEx−SBC)",
                                fmt_money_compact(row.strict_owner_earnings, &rep.currency),
                                if row.strict_owner_earnings > Decimal::ZERO {
                                    EMERALD_FG
                                } else {
                                    CORAL_FG
                                },
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "严苛 FCF% (市值 / EV)",
                                format!("{ym_str} / {ye_str}"),
                                if ym_pol < 0 { CORAL_FG } else { INK_PRIMARY },
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                "CapEx 开支".into(),
                                fmt_money_compact(row.capex, &rep.currency),
                                row.capex,
                                row.ocf,
                                GOLD_FG,
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                "SBC 激励".into(),
                                fmt_money_compact(row.sbc, &rep.currency),
                                row.sbc,
                                row.ocf,
                                CORAL_FG,
                                &self.fonts,
                            ))
                            .child(ratio_bar(
                                "剩余严苛 OE".into(),
                                fmt_money_compact(row.strict_owner_earnings, &rep.currency),
                                row.strict_owner_earnings.max(Decimal::ZERO),
                                row.ocf,
                                EMERALD_FG,
                                &self.fonts,
                            ))
                            .child(kv_line(
                                "回购 / SBC",
                                format!(
                                    "{} / {}",
                                    fmt_money_compact(row.buybacks, &rep.currency),
                                    fmt_money_compact(row.sbc, &rep.currency)
                                ),
                                INK_SECONDARY,
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
                    .child(format!(
                        "{} 暂无结构化财报数据；如需接入计算器，可录入 workbench/manual_financials/{}.json。",
                        rep.ticker, rep.ticker
                    )),
            );
        }

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
                                            .child("人类判词 · 费曼防线"),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .child("仅本人确认 · AI 草稿见「2 Alan 审讯台」"),
                                    ),
                            )
                            .child(if human.pending_human_review {
                                badge("待答辩".into(), GOLD_FG, GOLD_BG, GOLD_BORDER, &self.fonts)
                            } else {
                                badge(
                                    "已确认".into(),
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
                            .child(div().flex_1().min_w_0().child(prose_block(
                                "我的定性判词 (my_stance)",
                                human.my_stance.as_deref().unwrap_or("（待填写）"),
                                &self.fonts,
                            )))
                            .child(div().flex_1().min_w_0().child(prose_block(
                                "分歧权重判断 (weight_rationale)",
                                human.weight_rationale.as_deref().unwrap_or("（待填写）"),
                                &self.fonts,
                            ))),
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
                                            .child("事前证伪指标 (falsification_metrics)"),
                                    )
                                    .when(human.falsification_metrics.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .mt_1()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_MUTED))
                                                .child("（尚未设置）"),
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
                                        .child(format!("核验时间 {checked}")),
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
        let basis_tag = val_opt
            .map(|v| format!("{} · {}", v.basis_label, v.period_end))
            .unwrap_or_else(|| "待录入".into());
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
            ("○", "待录财报".into(), INK_MUTED)
        } else if red_cnt > 0 {
            ("●", format!("红灯 {red_cnt}"), CORAL_FG)
        } else if amber_cnt > 0 {
            ("○", format!("黄灯 {amber_cnt}"), GOLD_FG)
        } else {
            ("●", "无红灯".into(), EMERALD_FG)
        };

        let pending_feynman = self
            .selected_dossier
            .as_ref()
            .is_none_or(|d| d.human_judgment.pending_human_review);

        let (strike_dot, strike_lbl, strike_col) = match dist_live {
            Some(d) if d <= Decimal::ZERO => ("●", "击球区".into(), EMERALD_FG),
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
                                    .child("现价"),
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
                        "沙盒击球价",
                        fmt_price(active_strike, &rep.currency),
                        if active_strike.is_some() {
                            EMERALD_FG
                        } else {
                            INK_MUTED
                        },
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "距击球价",
                        dist_live_str,
                        match dist_live {
                            Some(d) if d <= Decimal::ZERO => EMERALD_FG,
                            Some(_) => GOLD_FG,
                            None => INK_MUTED,
                        },
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "市值",
                        val_opt
                            .and_then(|v| v.market_cap)
                            .map(|m| fmt_money_compact(m, &rep.currency))
                            .unwrap_or_else(|| "—".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line(
                        "企业价值 (EV)",
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
                prop_section("DECISION GATE · 决策检查", &self.fonts)
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
                                    .child("财务体检"),
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
                                    .child("费曼答辩"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(if pending_feynman {
                                        GOLD_FG
                                    } else {
                                        EMERALD_FG
                                    }))
                                    .child(if pending_feynman {
                                        "○ 待答辩 →"
                                    } else {
                                        "● 已确认"
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
                                    .child("击球价格"),
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
            // 3. Basket 2x2 Switcher
            .child(
                prop_section("BASKET · 归入篮子", &self.fonts)
                    .child(
                        div()
                            .flex()
                            .gap_1p5()
                            .children([Bucket::Core, Bucket::Watch].into_iter().map(|b| {
                                self.render_prop_bucket_btn(&rep.ticker, rep.bucket, b, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1p5()
                            .children([Bucket::Out, Bucket::TooHard].into_iter().map(|b| {
                                self.render_prop_bucket_btn(&rep.ticker, rep.bucket, b, cx)
                            })),
                    ),
            )
            // 4. Circle Tag Linkage
            .child(
                prop_section("LABELS · 能力圈", &self.fonts).child(
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
                prop_section("PROVENANCE · 溯源", &self.fonts)
                    .child(kv_line(
                        "CIK",
                        rep.cik.clone().unwrap_or_else(|| "MANUAL".into()),
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(kv_line("口径", basis_tag, INK_SECONDARY, &self.fonts))
                    .child(kv_line(
                        "最近财报",
                        rep.latest_filing_end
                            .clone()
                            .unwrap_or_else(|| "待录入".into()),
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
            .id(gpui::SharedString::from(format!("prop-bkt-{}", target.code())))
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
                .child("暂无多年结构化财务底表。");
        }
        let fin_cols: &[(&str, f32, bool)] = &[
            ("期间", 122., false),
            ("营收", 84., true),
            ("毛利率", 68., true),
            ("ROIC", 68., true),
            ("OCF", 84., true),
            ("CapEx", 80., true),
            ("SBC", 74., true),
            ("严苛 OE", 88., true),
            ("回购", 80., true),
            ("稀释股本", 0., true),
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
                            .child("多年确定性财务底表（Rust 解析自 SEC EDGAR XBRL）"),
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
                    row_el.bg(rgb(if i % 2 == 0 {
                        BG_SURFACE
                    } else {
                        BG_ROW_ALT
                    }))
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
                        fmt_money_compact(r.sbc, &rep.currency),
                        74.,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.strict_owner_earnings, &rep.currency),
                        88.,
                        if r.strict_owner_earnings > Decimal::ZERO {
                            EMERALD_FG
                        } else {
                            CORAL_FG
                        },
                        &self.fonts,
                    ))
                    .child(mono_right(
                        fmt_money_compact(r.buybacks, &rep.currency),
                        80.,
                        INK_SECONDARY,
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
                        .child(format!("XBRL 溯源: {tag_summary}")),
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
        let sha_full = rep
            .raw_sha256
            .as_deref()
            .unwrap_or("— (无一手 XBRL 源文件)");
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
                    .unwrap_or_else(|| "Net Cash > Book Capital".into());
                let mcap = v
                    .market_cap
                    .map(|m| fmt_money_compact(m, &rep.currency))
                    .unwrap_or_else(|| "—".into());
                let yield_s = fmt_pct_clean(v.strict_fcf_yield_mcap_pct, false).0;
                let imp_s = fmt_pct_clean(v.implied_growth_for_target_pct, true).0;
                vec![
                    (
                        "严苛股东盈余",
                        format!(
                            "Strict_OE = OCF({}) - CapEx({}) - SBC({}) = {}",
                            fmt_money_compact(r.ocf, &rep.currency),
                            fmt_money_compact(r.capex, &rep.currency),
                            fmt_money_compact(r.sbc, &rep.currency),
                            fmt_money_compact(r.strict_owner_earnings, &rep.currency)
                        ),
                    ),
                    (
                        "投入资本回报率",
                        format!(
                            "ROIC = NOPAT({}) / IC(Debt {} + Eq {} - Cash {} = {}) = {}",
                            fmt_money_compact(r.nopat, &rep.currency),
                            fmt_money_compact(r.total_debt, &rep.currency),
                            fmt_money_compact(r.equity, &rep.currency),
                            fmt_money_compact(r.cash_and_st, &rep.currency),
                            fmt_money_compact(r.invested_capital, &rep.currency),
                            roic_str
                        ),
                    ),
                    (
                        "保守击球价格",
                        format!(
                            "Strike = OE_per_share({oe_ps}) * Multiple_low({}x) = {}",
                            rep.assumptions.multiple_low,
                            fmt_price(v.strike_price, &rep.currency)
                        ),
                    ),
                    (
                        "反向隐含增速",
                        format!(
                            "Implied_g = Target({}%) - Strict_OE/MCap({} / {} = {yield_s}) = {imp_s}",
                            rep.assumptions.target_return_pct,
                            fmt_money_compact(v.strict_owner_earnings, &rep.currency),
                            mcap,
                        ),
                    ),
                ]
            }
            (Some(r), None) => vec![(
                "严苛股东盈余",
                format!(
                    "Strict_OE = OCF({}) - CapEx({}) - SBC({}) = {}",
                    fmt_money_compact(r.ocf, &rep.currency),
                    fmt_money_compact(r.capex, &rep.currency),
                    fmt_money_compact(r.sbc, &rep.currency),
                    fmt_money_compact(r.strict_owner_earnings, &rep.currency)
                ),
            )],
            _ => vec![(
                "STATUS",
                "无一手结构化财务事实（绝不由 LLM 补全）。".into(),
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
                            .child(inspector_kv(
                                "RAW_SHA256",
                                sha_full.to_owned(),
                                &self.fonts,
                            ))
                            .child(inspector_kv(
                                "FILING_END",
                                rep.latest_filing_end
                                    .clone()
                                    .unwrap_or_else(|| "—".into()),
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
                            .bg(rgb(if i % 2 == 0 {
                                BG_SURFACE
                            } else {
                                BG_ROW_ALT
                            }))
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
