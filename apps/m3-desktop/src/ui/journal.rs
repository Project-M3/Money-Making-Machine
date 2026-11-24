use super::{MungerWorkbench, components::*, format::*, theme::*};
use gpui::{Context, div, prelude::*, px, rgb, rgba};

impl MungerWorkbench {
    pub(super) fn render_journal_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let bench_now = self
            .benchmark
            .as_ref()
            .map(|b| format!("{:.2}", b.price))
            .unwrap_or_else(|| "—".into());
        let bench_start = self
            .journal
            .benchmark_start_price
            .map(|p| format!("{p:.2}"))
            .unwrap_or_else(|| l.t("待锁定", "Unset").into());

        let cols: &[(&str, f32, bool)] = &[
            (l.t("日期", "Date"), 98., false),
            (l.t("标的", "Ticker"), 84., false),
            (l.t("决策动作", "Action"), 112., false),
            (l.t("冻结价格", "Price"), 92., true),
            (l.t("严苛 FCF%", "Strict FCF%"), 88., true),
            ("S&P 500", 88., true),
            (
                l.t("判断依据与事前证伪条件", "Rationale & Pre-Falsification"),
                0.,
                false,
            ),
        ];

        let journal_cmd =
            "./apps/m3-desktop/m3 journal record --ticker SYM --action wait --note \"...\"";

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_5()
                    .py_3()
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
                            .gap_2p5()
                            .child(badge(
                                self.journal.account_note.clone(),
                                EMERALD_FG,
                                EMERALD_BG,
                                EMERALD_BORDER,
                                &self.fonts,
                            ))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(l.t(
                                        "事前冻结决策账本",
                                        "Append-Only Pre-Committed Decision Journal",
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(journal_stat(
                                l.t("模拟现金", "Paper Cash"),
                                format!("${:.2}", self.journal.cash_balance),
                                EMERALD_FG,
                                &self.fonts,
                            ))
                            .child(v_divider())
                            .child(journal_stat(
                                l.t("初始现金", "Starting Cash"),
                                format!("${:.2}", self.journal.starting_cash),
                                INK_HERO,
                                &self.fonts,
                            ))
                            .child(v_divider())
                            .child(journal_stat(
                                "S&P 500",
                                format!("{bench_now} ({} {bench_start})", l.t("起点", "Baseline")),
                                INK_PRIMARY,
                                &self.fonts,
                            ))
                            .child(v_divider())
                            .child(journal_stat(
                                l.t("记录数", "Entries"),
                                format!("{}", self.journal.entries.len()),
                                SLATE_FG,
                                &self.fonts,
                            )),
                    ),
            )
            .child(
                div().p_5().child(
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
                                            "决策记录（workbench/journal.json）",
                                            "Decision Entries (workbench/journal.json)",
                                        )),
                                )
                                .child(
                                    div()
                                        .id("copy-journal-cli")
                                        .px_2p5()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(rgb(BG_SURFACE))
                                        .border_1()
                                        .border_color(rgb(BORDER_PILL))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .cursor_pointer()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy_cli_command(
                                                journal_cmd.into(),
                                                "journal record",
                                                cx,
                                            );
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_SECONDARY))
                                                .child("Copy CLI"),
                                        )
                                        .child(kbd("c", &self.fonts)),
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
                                .children(cols.iter().map(|(lbl, w, right)| {
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
                        .children(self.journal.entries.iter().rev().enumerate().map(|(i, e)| {
                            let (fy_str, _) = fmt_pct_clean(e.strict_fcf_yield_pct, false);
                            let (a_fg, a_bg, a_br) = journal_action_colors(e.action);
                            div()
                                .flex()
                                .px_3()
                                .py_3()
                                .bg(rgb(if i % 2 == 0 { BG_SURFACE } else { BG_ROW_ALT }))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .child(mono_left(
                                    e.recorded_at.clone(),
                                    98.,
                                    INK_SECONDARY,
                                    &self.fonts,
                                ))
                                .child(mono_left(e.ticker.clone(), 84., INK_HERO, &self.fonts))
                                .child(
                                    div()
                                        .w(px(112.))
                                        .flex_shrink_0()
                                        .px_1()
                                        .flex()
                                        .items_start()
                                        .child(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded_xs()
                                                .bg(rgba(a_bg))
                                                .border_1()
                                                .border_color(rgba(a_br))
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_size(px(10.))
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(a_fg))
                                                .child(e.action.label().to_owned()),
                                        ),
                                )
                                .child(mono_right(
                                    e.price
                                        .map(|p| format!("{p:.2}"))
                                        .unwrap_or_else(|| "—".into()),
                                    92.,
                                    if e.price.is_some() {
                                        INK_SECONDARY
                                    } else {
                                        INK_MUTED
                                    },
                                    &self.fonts,
                                ))
                                .child(mono_right(
                                    fy_str,
                                    88.,
                                    if e.strict_fcf_yield_pct.is_some() {
                                        INK_SECONDARY
                                    } else {
                                        INK_MUTED
                                    },
                                    &self.fonts,
                                ))
                                .child(mono_right(
                                    e.benchmark_price
                                        .map(|p| format!("{p:.2}"))
                                        .unwrap_or_else(|| "—".into()),
                                    88.,
                                    INK_MUTED,
                                    &self.fonts,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .px_2()
                                        .child(
                                            div()
                                                .font_family(self.fonts.ui.clone())
                                                .text_size(px(13.))
                                                .line_height(px(20.))
                                                .text_color(rgb(INK_PRIMARY))
                                                .child(e.rationale.clone()),
                                        )
                                        .child(
                                            div()
                                                .mt_1()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_MUTED))
                                                .child(format!(
                                                    "{}: {}",
                                                    l.t("证伪条件", "Falsification Trigger"),
                                                    e.falsification_trigger
                                                )),
                                        ),
                                )
                        })),
                ),
            )
    }
}
