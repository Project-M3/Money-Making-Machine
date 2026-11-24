use super::{ChartTimeframe, MungerWorkbench, components::*, format::*, theme::*};
use crate::model::{AnnualMetricRow, MungerReport, OhlcBar, SandboxResult};
use gpui::{Context, bounds, canvas, div, fill, point, prelude::*, px, rgb, rgba, size};
use rust_decimal::Decimal;

impl MungerWorkbench {
    pub(super) fn render_kline_card(
        &self,
        rep: &MungerReport,
        row_opt: Option<&AnnualMetricRow>,
        strike_price: Option<Decimal>,
        sandbox: &SandboxResult,
        sandbox_dirty: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let bars: Vec<OhlcBar> = match self.chart_timeframe {
            ChartTimeframe::FiveYearWeekly => self
                .selected_history_wk
                .as_ref()
                .map(|h| h.bars.clone())
                .unwrap_or_default(),
            ChartTimeframe::ThreeYearWeekly => self
                .selected_history_wk
                .as_ref()
                .map(|h| {
                    let keep = 156usize.min(h.bars.len());
                    h.bars[h.bars.len() - keep..].to_vec()
                })
                .unwrap_or_default(),
            ChartTimeframe::OneYearDaily => self
                .selected_history_1d
                .as_ref()
                .map(|h| h.bars.clone())
                .unwrap_or_default(),
        };

        let fv_high = strike_price.and_then(|s| {
            if self.sandbox_multiple > Decimal::ZERO {
                Some(s * (self.sandbox_multiple + Decimal::from(4)) / self.sandbox_multiple)
            } else {
                None
            }
        });
        let domain = compute_kline_domain(&bars, strike_price, fv_high);

        let last_bar = bars.last();
        let ohlc_readout = match last_bar {
            Some(b) => {
                let pfx = currency_prefix(&rep.currency);
                format!(
                    "O {pfx}{:.2}   H {pfx}{:.2}   L {pfx}{:.2}   C {pfx}{:.2}",
                    b.open, b.high, b.low, b.close
                )
            }
            None => l.t("暂无本地 K 线缓存", "No local K-line cache").into(),
        };

        let peak_high = bars
            .iter()
            .map(|b| b.high)
            .fold(Decimal::ZERO, Decimal::max);
        let drawdown_pct = match (last_bar, peak_high) {
            (Some(b), pk) if pk > Decimal::ZERO => Some(((b.close - pk) / pk) * Decimal::from(100)),
            _ => None,
        };
        let (dd_str, _) = fmt_pct_clean(drawdown_pct, true);

        let cur_price = rep.price.as_ref().map(|p| p.price);
        let dist_to_strike = match (cur_price, strike_price) {
            (Some(p), Some(s)) if s > Decimal::ZERO => Some(((p - s) / s) * Decimal::from(100)),
            _ => None,
        };
        let (strike_dist_str, _) = fmt_pct_clean(dist_to_strike, true);

        let strike_f64 = strike_price.and_then(dec_to_f64);
        let fv_high_f64 = fv_high.and_then(dec_to_f64);
        let strike_below_visible = match (domain, strike_f64) {
            (Some((y_min, _, _)), Some(s)) => s < y_min,
            _ => false,
        };

        let date_ticks: Vec<String> = if bars.is_empty() {
            Vec::new()
        } else {
            let n = bars.len();
            [0, n / 4, n / 2, (n * 3) / 4, n - 1]
                .into_iter()
                .map(|idx| civil_ym_from_ms(bars[idx].ts_ms))
                .collect()
        };

        let mut card = card_shell().p_4().flex().flex_col().gap_3().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            seg_group().children(
                                [
                                    ChartTimeframe::FiveYearWeekly,
                                    ChartTimeframe::ThreeYearWeekly,
                                    ChartTimeframe::OneYearDaily,
                                ]
                                .into_iter()
                                .map(|tf| {
                                    seg_tab(
                                        format!("tf-{}", tf.label()),
                                        tf.label_for(l),
                                        self.chart_timeframe == tf,
                                        None,
                                        true,
                                        &self.fonts,
                                        cx.listener(move |this, _, _, cx| {
                                            this.chart_timeframe = tf;
                                            cx.notify();
                                        }),
                                    )
                                }),
                            ),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .text_color(rgb(INK_SECONDARY))
                                .child(ohlc_readout),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap_1()
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .text_color(rgb(INK_MUTED))
                                        .child(l.t("高点回撤", "Drawdown")),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .child(dd_str),
                                ),
                        )
                        .child(v_divider())
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .gap_1()
                                .child(div().w(px(8.)).h(px(2.)).bg(rgb(EMERALD_FG)))
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .text_color(rgb(INK_MUTED))
                                        .child(l.t("击球线", "Strike")),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(if strike_price.is_some() {
                                            EMERALD_FG
                                        } else {
                                            INK_MUTED
                                        }))
                                        .child(fmt_price(strike_price, &rep.currency)),
                                )
                                .when(dist_to_strike.is_some(), |d| {
                                    d.child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(11.))
                                            .text_color(rgb(INK_MUTED))
                                            .child(format!("({strike_dist_str})")),
                                    )
                                }),
                        ),
                ),
        );

        if let Some((y_min, y_max, max_vol)) = domain {
            let pfx = currency_prefix(&rep.currency);
            let y_ticks = [
                format!("{pfx}{:.2}", y_max),
                format!("{pfx}{:.2}", y_min + (y_max - y_min) * 0.667),
                format!("{pfx}{:.2}", y_min + (y_max - y_min) * 0.333),
                format!("{pfx}{:.2}", y_min),
            ];
            let bars_paint = bars.clone();

            card = card
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .h(px(228.))
                                .rounded_md()
                                .bg(rgb(BG_VIEWPORT))
                                .border_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .overflow_hidden()
                                .child(
                                    canvas(
                                        |_, _, _| (),
                                        move |b, _, window, _| {
                                            let ox = f32::from(b.origin.x);
                                            let oy = f32::from(b.origin.y);
                                            let w = f32::from(b.size.width);
                                            let h = f32::from(b.size.height);
                                            if w <= 20.0 || h <= 40.0 || bars_paint.is_empty() {
                                                return;
                                            }
                                            let pad_x = 10.0f32;
                                            let plot_w = (w - pad_x * 2.0).max(10.0);
                                            let price_top = oy + 8.0;
                                            let price_h = h * 0.72;
                                            let vol_top = oy + h * 0.78;
                                            let vol_h = h * 0.19;
                                            let y_span = (y_max - y_min).max(1e-6);
                                            let price_to_y = |p: f64| -> f32 {
                                                let ratio = ((p - y_min) / y_span).clamp(0.0, 1.0);
                                                price_top + price_h - (ratio as f32) * price_h
                                            };

                                            for frac in [0.0f32, 0.333, 0.667, 1.0] {
                                                let gy = price_top + price_h * frac;
                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(px(ox), px(gy)),
                                                        size(px(w), px(1.)),
                                                    ),
                                                    rgb(BORDER_HAIRLINE),
                                                ));
                                            }

                                            if let Some(s) = strike_f64 {
                                                let band_top = fv_high_f64.unwrap_or(s);
                                                if band_top >= y_min && s <= y_max {
                                                    let y_hi = price_to_y(band_top.min(y_max));
                                                    let y_lo = price_to_y(s.max(y_min));
                                                    let band_h = (y_lo - y_hi).max(2.0);
                                                    window.paint_quad(fill(
                                                        bounds(
                                                            point(px(ox), px(y_hi)),
                                                            size(px(w), px(band_h)),
                                                        ),
                                                        rgba(EMERALD_BAND),
                                                    ));
                                                }
                                            }

                                            let n = bars_paint.len() as f32;
                                            let step = plot_w / n;
                                            let body_w = (step * 0.62).clamp(1.2, 8.0);
                                            let max_v_f = (max_vol as f64).max(1.0);

                                            for (idx, bar) in bars_paint.iter().enumerate() {
                                                let cx_p = ox + pad_x + (idx as f32 + 0.5) * step;
                                                let (Some(o), Some(hi), Some(lo), Some(c)) = (
                                                    dec_to_f64(bar.open),
                                                    dec_to_f64(bar.high),
                                                    dec_to_f64(bar.low),
                                                    dec_to_f64(bar.close),
                                                ) else {
                                                    continue;
                                                };
                                                let is_up = c >= o;
                                                let col = if is_up { EMERALD_FG } else { CORAL_FG };
                                                let y_hi = price_to_y(hi);
                                                let y_lo = price_to_y(lo);
                                                let y_o = price_to_y(o);
                                                let y_c = price_to_y(c);

                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(px(cx_p - 0.5), px(y_hi)),
                                                        size(px(1.0), px((y_lo - y_hi).max(1.0))),
                                                    ),
                                                    rgb(col),
                                                ));
                                                let body_top = y_o.min(y_c);
                                                let body_h = (y_o - y_c).abs().max(1.2);
                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(
                                                            px(cx_p - body_w * 0.5),
                                                            px(body_top),
                                                        ),
                                                        size(px(body_w), px(body_h)),
                                                    ),
                                                    rgb(col),
                                                ));

                                                let vh = ((bar.volume as f64 / max_v_f)
                                                    * vol_h as f64)
                                                    as f32;
                                                let vh = vh.max(1.0).min(vol_h);
                                                let vy = vol_top + (vol_h - vh);
                                                window.paint_quad(fill(
                                                    bounds(
                                                        point(px(cx_p - body_w * 0.5), px(vy)),
                                                        size(px(body_w), px(vh)),
                                                    ),
                                                    rgba(if is_up {
                                                        EMERALD_VOL
                                                    } else {
                                                        CORAL_VOL
                                                    }),
                                                ));
                                            }

                                            if let Some(s) = strike_f64
                                                && s >= y_min
                                                && s <= y_max
                                            {
                                                let sy = price_to_y(s);
                                                let mut dx = ox;
                                                while dx < ox + w {
                                                    let seg_w = (ox + w - dx).min(6.0);
                                                    window.paint_quad(fill(
                                                        bounds(
                                                            point(px(dx), px(sy - 0.75)),
                                                            size(px(seg_w), px(1.5)),
                                                        ),
                                                        rgba(EMERALD_LINE),
                                                    ));
                                                    dx += 10.0;
                                                }
                                            }
                                        },
                                    )
                                    .size_full(),
                                )
                                .when(strike_below_visible, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .bottom_2()
                                            .right_2()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_xs()
                                            .bg(rgba(EMERALD_BG))
                                            .border_1()
                                            .border_color(rgba(EMERALD_BORDER))
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(10.))
                                            .text_color(rgb(EMERALD_FG))
                                            .child(format!(
                                                "↓ {} {} ({})",
                                                l.t("击球线", "Strike"),
                                                fmt_price(strike_price, &rep.currency),
                                                l.t("低于区间底价", "below visible floor")
                                            )),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .w(px(72.))
                                .h(px(228.))
                                .flex_shrink_0()
                                .py_1()
                                .flex()
                                .flex_col()
                                .justify_between()
                                .items_end()
                                .children(y_ticks.into_iter().map(|lbl| {
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(INK_MUTED))
                                        .child(lbl)
                                })),
                        ),
                )
                .child(
                    div()
                        .pr(px(80.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .children(date_ticks.into_iter().map(|dt| {
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_size(px(10.))
                                .text_color(rgb(INK_MUTED))
                                .child(dt)
                        })),
                );
        } else {
            card = card.child(
                div()
                    .h(px(120.))
                    .rounded_md()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2p5()
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(match l {
                                crate::i18n::UiLang::Zh => {
                                    format!("尚未同步 {} 的历史 K 线缓存", rep.ticker)
                                }
                                crate::i18n::UiLang::En => {
                                    format!("No historical K-line cache synced for {}", rep.ticker)
                                }
                            }),
                    )
                    .child(
                        div()
                            .id("kline-sync-now")
                            .px_3()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_PILL))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_PRIMARY))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_sync(cx);
                            }))
                            .child(l.t("同步行情 K 线", "Sync Quotes & K-Lines")),
                    ),
            );
        }

        // Docked 1-Row Inline Charlie Sandbox Footer
        if row_opt.is_some() {
            let (sb_imp_str, _) = fmt_pct_clean(sandbox.implied_growth_for_target_pct, true);
            let (sb_exp_str, _) = fmt_pct_clean(sandbox.expected_total_return_pct, false);

            card = card.child(
                div()
                    .pt_2p5()
                    .border_t_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_3()
                            .child(stepper_inline(
                                l.t("倍数", "Mult"),
                                format!("{}x", self.sandbox_multiple),
                                "sb-mult",
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_multiple > Decimal::from(5) {
                                        this.sandbox_multiple -= Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_multiple < Decimal::from(45) {
                                        this.sandbox_multiple += Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(stepper_inline(
                                l.t("目标", "Target"),
                                format!("{}%", self.sandbox_target_return),
                                "sb-ret",
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_target_return > Decimal::from(5) {
                                        this.sandbox_target_return -= Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_target_return < Decimal::from(25) {
                                        this.sandbox_target_return += Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(stepper_inline(
                                l.t("增速", "Growth"),
                                format!("{}%", self.sandbox_expected_growth),
                                "sb-gro",
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_expected_growth > Decimal::from(-10) {
                                        this.sandbox_expected_growth -= Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                                cx.listener(|this, _, _, cx| {
                                    if this.sandbox_expected_growth < Decimal::from(25) {
                                        this.sandbox_expected_growth += Decimal::ONE;
                                    }
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .child(l.t("隐含增速", "Implied Growth")),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(GOLD_FG))
                                            .child(sb_imp_str),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .text_color(rgb(INK_MUTED))
                                            .child(l.t("预期年化", "Expected IRR")),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child(sb_exp_str),
                                    ),
                            )
                            .when(sandbox_dirty, |d| {
                                d.child(
                                    div()
                                        .id("reset-sandbox")
                                        .px_2()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(rgb(BG_VIEWPORT))
                                        .border_1()
                                        .border_color(rgb(BORDER_STRONG))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reset_sandbox_to_saved();
                                            cx.notify();
                                        }))
                                        .child(l.t("重置", "Reset")),
                                )
                            })
                            .child(
                                div()
                                    .id("save-sandbox")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .when(sandbox_dirty, |d| {
                                        d.bg(rgb(CTA_BG))
                                            .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                            .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                                            .text_color(rgb(CTA_INK))
                                    })
                                    .when(!sandbox_dirty, |d| {
                                        d.bg(rgb(BG_WELL))
                                            .border_1()
                                            .border_color(rgb(BORDER_PILL))
                                            .hover(|s| s.bg(rgb(BG_HOVER)))
                                            .text_color(rgb(INK_MUTED))
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_sandbox_assumptions();
                                        cx.notify();
                                    }))
                                    .child(if sandbox_dirty {
                                        l.t("保存假设", "Save Assumptions")
                                    } else {
                                        l.t("已保存", "Saved")
                                    }),
                            ),
                    ),
            );
        }

        card
    }
}
