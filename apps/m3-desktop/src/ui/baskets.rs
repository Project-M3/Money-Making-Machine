use super::{
    BasketsViewMode, MungerWorkbench, NavView, SortCol, StatusQuickFilter, components::*,
    format::*, theme::*,
};
use crate::model::{Bucket, primary_circle_tag, secondary_circle_detail};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;

impl MungerWorkbench {
    pub(super) fn render_baskets_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let tags = collect_circle_tags(&self.reports);
        let pulse = compute_today_pulse(&self.reports);
        let has_any_filter = self.basket_filter.is_some()
            || self.circle_filter.is_some()
            || self.status_filter != StatusQuickFilter::All;

        let mut root = div().flex().flex_col();

        // Row 1: 38px Today's Pulse Strip (Self-Speaking Routine Beat 1) + [List | Board]
        let pulse_quiet = pulse.strike_tickers.is_empty()
            && pulse.red_core_tickers.is_empty()
            && pulse.watch_count == 0;

        let mut pulse_left = div()
            .flex()
            .items_center()
            .gap_2p5()
            .min_w_0()
            .child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_size(px(10.))
                    .text_color(rgb(INK_MUTED))
                    .child(l.t("今日信号", "TODAY'S PULSE")),
            )
            .child(v_divider());

        if self.reports.is_empty() {
            pulse_left = pulse_left.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(INK_MUTED)))
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child(l.t(
                                "能力圈为空 · 从全市场雷达或收件箱挑选首批标的",
                                "Circle is empty · pick holdings from Universe Radar or Inbox",
                            )),
                    ),
            );
        } else if pulse_quiet {
            pulse_left = pulse_left.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(EMERALD_FG)))
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_SECONDARY))
                            .child(l.t(
                                "今日无球可挥 · 耐心等待",
                                "No fat pitch today · wait patiently",
                            )),
                    ),
            );
        } else {
            if let Some(first_strike) = pulse.strike_tickers.first().cloned() {
                let list_txt = pulse.strike_tickers.join(", ");
                pulse_left = pulse_left.child(
                    div()
                        .id("pulse-strike")
                        .px_2()
                        .py_0p5()
                        .rounded_xs()
                        .bg(rgba(EMERALD_BG))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_company(&first_strike, Some(NavView::Dossier));
                            cx.notify();
                        }))
                        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(EMERALD_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(EMERALD_FG))
                                .child(format!("{}: {list_txt}", l.t("击球区", "Strike"))),
                        ),
                );
            } else {
                pulse_left = pulse_left.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child(l.t("0 击球", "0 Strike")),
                );
            }

            pulse_left = pulse_left.child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child("·"),
            );

            if let Some(first_red) = pulse.red_core_tickers.first().cloned() {
                let list_txt = pulse.red_core_tickers.join(", ");
                pulse_left = pulse_left.child(
                    div()
                        .id("pulse-red")
                        .px_2()
                        .py_0p5()
                        .rounded_xs()
                        .bg(rgba(CORAL_BG))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_company(&first_red, Some(NavView::Dossier));
                            cx.notify();
                        }))
                        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(CORAL_FG)))
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(CORAL_FG))
                                .child(format!("{}: {list_txt}", l.t("红灯复核", "Red Review"))),
                        ),
                );
            } else {
                pulse_left = pulse_left.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child(l.t("核心池无红灯", "0 Core Warnings")),
                );
            }

            pulse_left = pulse_left.child(
                div()
                    .font_family(self.fonts.mono_term.clone())
                    .text_xs()
                    .text_color(rgb(INK_MUTED))
                    .child("·"),
            );

            if pulse.watch_count > 0 {
                let watch_cnt = pulse.watch_count;
                pulse_left = pulse_left.child(
                    div()
                        .id("pulse-watch")
                        .px_2()
                        .py_0p5()
                        .rounded_xs()
                        .bg(rgba(SKY_BG))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(SKY_FG))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.focus_basket(Some(Bucket::Watch));
                            cx.notify();
                        }))
                        .child(match l {
                            crate::i18n::UiLang::Zh => format!("○ 待分拣 {watch_cnt} 家"),
                            crate::i18n::UiLang::En => format!("○ {watch_cnt} to triage"),
                        }),
                );
            } else {
                pulse_left = pulse_left.child(
                    div()
                        .font_family(self.fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child(l.t("待审讯已清空", "Watch cleared")),
                );
            }
        }

        root = root.child(
            div()
                .h(px(38.))
                .px_5()
                .bg(rgb(BG_SURFACE))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(pulse_left)
                .child(
                    seg_group()
                        .child(seg_tab(
                            "bkt-mode-list",
                            l.t("列表", "List"),
                            self.baskets_view_mode == BasketsViewMode::List,
                            None,
                            false,
                            &self.fonts,
                            cx.listener(|this, _, _, cx| {
                                this.baskets_view_mode = BasketsViewMode::List;
                                this.cursor_index = 0;
                                cx.notify();
                            }),
                        ))
                        .child(seg_tab(
                            "bkt-mode-board",
                            l.t("看板", "Board"),
                            self.baskets_view_mode == BasketsViewMode::Board,
                            None,
                            false,
                            &self.fonts,
                            cx.listener(|this, _, _, cx| {
                                this.baskets_view_mode = BasketsViewMode::Board;
                                this.cursor_index = 0;
                                cx.notify();
                            }),
                        ))
                        .child(div().px_1().child(kbd("v", &self.fonts))),
                ),
        );

        // Row 2: 36px Basket Focus Tabs + Quiet Status Filters
        let total_cnt = self.reports.len();
        root = root.child(
            div()
                .h(px(36.))
                .px_5()
                .bg(rgb(BG_ROW_ALT))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child({
                            let active = self.basket_filter.is_none();
                            div()
                                .id("tab-bkt-all")
                                .px_2()
                                .py_0p5()
                                .rounded_xs()
                                .cursor_pointer()
                                .when(active, |d| {
                                    d.bg(rgb(BG_HOVER))
                                        .border_1()
                                        .border_color(rgb(BORDER_PILL))
                                })
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .flex()
                                .items_center()
                                .gap_1()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.focus_basket(None);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .font_weight(if active {
                                            gpui::FontWeight::SEMIBOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(rgb(if active { INK_HERO } else { INK_MUTED }))
                                        .child(l.t("全部", "All")),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(if active {
                                            INK_PRIMARY
                                        } else {
                                            INK_MUTED
                                        }))
                                        .child(format!("{total_cnt}")),
                                )
                        })
                        .children(Bucket::display_order().into_iter().map(|b| {
                            let (fg, bg, br) = bucket_colors(b);
                            let active = self.basket_filter == Some(b);
                            let cnt = self.reports.iter().filter(|r| r.bucket == b).count();
                            div()
                                .id(gpui::SharedString::from(format!("tab-bkt-{}", b.code())))
                                .px_2()
                                .py_0p5()
                                .rounded_xs()
                                .cursor_pointer()
                                .when(active, |d| d.bg(rgba(bg)).border_1().border_color(rgba(br)))
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .flex()
                                .items_center()
                                .gap_1()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if active {
                                        this.focus_basket(None);
                                    } else {
                                        this.focus_basket(Some(b));
                                    }
                                    cx.notify();
                                }))
                                .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(fg)))
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_size(px(11.))
                                        .font_weight(if active {
                                            gpui::FontWeight::BOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(rgb(if active { fg } else { INK_SECONDARY }))
                                        .child(b.code()),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(if active { INK_HERO } else { INK_MUTED }))
                                        .child(format!("{cnt}")),
                                )
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .children(
                            [StatusQuickFilter::StrikeOnly, StatusQuickFilter::WarnedOnly]
                                .into_iter()
                                .map(|sf| {
                                    let active = self.status_filter == sf;
                                    div()
                                        .id(gpui::SharedString::from(format!("sf-{}", sf.label())))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_xs()
                                        .cursor_pointer()
                                        .when(active, |d| {
                                            d.bg(rgb(BG_HOVER))
                                                .border_1()
                                                .border_color(rgb(BORDER_PILL))
                                        })
                                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .text_color(rgb(if active { INK_HERO } else { INK_MUTED }))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_status_filter(sf);
                                            cx.notify();
                                        }))
                                        .child(sf.label_for(l))
                                }),
                        )
                        .when(has_any_filter, |d| {
                            d.child(
                                div()
                                    .id("clear-all-filters")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_xs()
                                    .bg(rgba(GOLD_BG))
                                    .border_1()
                                    .border_color(rgba(GOLD_BORDER))
                                    .cursor_pointer()
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(GOLD_FG))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.basket_filter = None;
                                        this.circle_filter = None;
                                        this.status_filter = StatusQuickFilter::All;
                                        this.cursor_index = 0;
                                        cx.notify();
                                    }))
                                    .child(l.t("× 重置", "× Reset")),
                            )
                        }),
                ),
        );

        // Bare Quiet #能力圈 Tag Line (no background strip)
        if !tags.is_empty() {
            root = root.child(
                div()
                    .px_5()
                    .pt_3()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_1p5()
                    .children(tags.into_iter().map(|(tag, cnt)| {
                        let active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_click = tag.clone();
                        circle_tag_button(
                            format!("tag-bar-{tag}"),
                            tag,
                            active,
                            true,
                            Some(cnt),
                            &self.fonts,
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_circle_filter(&tag_click);
                                cx.notify();
                            }),
                        )
                    })),
            );
        }

        if self.reports.is_empty() && !has_any_filter {
            return root.child(
                div().px_5().pt_4().pb_6().child(
                    card_shell()
                        .p_6()
                        .flex()
                        .flex_col()
                        .gap_3p5()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .w(px(7.))
                                        .h(px(7.))
                                        .rounded_full()
                                        .bg(rgb(EMERALD_FG)),
                                )
                                .child(
                                    div()
                                        .font_family(self.fonts.ui.clone())
                                        .text_base()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO))
                                        .child(l.t(
                                            "尚未建立个人能力圈",
                                            "Your Circle is Empty",
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_xs()
                                .line_height(px(19.))
                                .text_color(rgb(INK_SECONDARY))
                                .child(l.t(
                                    "M3 不会预置任何持仓或观察名单。你可以先同步全美股 L0 湖仓，从全市场雷达或简报收件箱中挑选真正读懂的公司加入能力圈（CORE / WATCH / OUT / TOO HARD），也可以在终端通过 CLI 自行维护。",
                                    "M3 ships with zero pre-loaded holdings or watchlists. Sync the US L0 lakehouse, then promote businesses you understand from Universe Radar or Briefing Inbox into your Circle (CORE / WATCH / OUT / TOO HARD).",
                                )),
                        )
                        .child(
                            div()
                                .pt_1()
                                .flex()
                                .items_center()
                                .gap_2p5()
                                .child(
                                    div()
                                        .id("empty-circle-go-universe")
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
                                        .child(l.t(
                                            "前往全市场雷达 (⌘2)",
                                            "Open Universe Radar (⌘2)",
                                        )),
                                )
                                .child(
                                    div()
                                        .id("empty-circle-go-inbox")
                                        .px_3()
                                        .py_1p5()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgb(BORDER_PILL))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_PRIMARY))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.switch_view(NavView::Inbox);
                                            cx.notify();
                                        }))
                                        .child(l.t(
                                            "打开简报收件箱 (⌘1)",
                                            "Open Briefing Inbox (⌘1)",
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .pt_1()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(11.))
                                .text_color(rgb(INK_MUTED))
                                .child(l.t(
                                    "CLI: m3 add AAPL --bucket watch --circle \"消费生态\"",
                                    "CLI: m3 add AAPL --bucket watch --circle \"Ecosystem\"",
                                )),
                        ),
                ),
            );
        }

        if self.baskets_view_mode == BasketsViewMode::Board {
            return root.child(self.render_baskets_board(cx));
        }

        // Collapsible Grouped Basket Cards
        let mut linear_idx: usize = 0;
        let nd_label = l.t("净债/OE", "NetDebt/OE");
        let status_label = l.t("状态", "Status");
        let headers: &[(&str, f32, bool, Option<SortCol>)] = &[
            (
                l.t("标的 / 能力圈", "Holding / Circle"),
                0.,
                false,
                Some(SortCol::Ticker),
            ),
            (l.t("现价", "Price"), 90., true, Some(SortCol::Price)),
            (l.t("击球价", "Strike"), 90., true, None),
            (
                l.t("距击球价", "vs Strike"),
                90.,
                true,
                Some(SortCol::Margin),
            ),
            ("5Y ROIC", 84., true, Some(SortCol::Roic)),
            (
                l.t("严苛 FCF%", "Strict FCF%"),
                88.,
                true,
                Some(SortCol::FcfYield),
            ),
            (
                l.t("3Y 股本Δ", "3Y ShareΔ"),
                84.,
                true,
                Some(SortCol::ShareDelta),
            ),
            (nd_label, 82., true, Some(SortCol::NetDebt)),
            (status_label, 112., false, None),
        ];

        let mut cards = div().px_5().pt_3().pb_5().flex().flex_col().gap_3();
        for bucket in self.visible_baskets_order() {
            let (fg, _, _) = bucket_colors(bucket);
            let is_collapsed =
                self.basket_filter.is_none() && self.collapsed_baskets.contains(&bucket);
            let rows = filter_reports_for_basket(
                &self.reports,
                bucket,
                self.circle_filter.as_deref(),
                self.status_filter,
                self.sort_col,
                self.sort_asc,
            );
            let row_cnt = rows.len();
            let collapsed_preview = if is_collapsed && !rows.is_empty() {
                Some(
                    rows.iter()
                        .map(|r| r.ticker.as_str())
                        .collect::<Vec<_>>()
                        .join(" · "),
                )
            } else {
                None
            };

            let mut block = card_shell().child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "bkt-hdr-{}",
                        bucket.code()
                    )))
                    .px_4()
                    .py_2p5()
                    .bg(rgb(BG_WELL))
                    .when(!is_collapsed, |d| {
                        d.border_b_1().border_color(rgb(BORDER_HAIRLINE))
                    })
                    .flex()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.basket_filter.is_none() {
                            if this.collapsed_baskets.contains(&bucket) {
                                this.collapsed_baskets.remove(&bucket);
                            } else {
                                this.collapsed_baskets.insert(bucket);
                            }
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .when(self.basket_filter.is_none(), |d| {
                                d.child(
                                    div()
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .child(if is_collapsed { "▸" } else { "▾" }),
                                )
                            })
                            .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(fg)))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_term.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(fg))
                                    .child(bucket.code()),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(bucket.short_label_for(l)),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(format!("{row_cnt}")),
                            ),
                    )
                    .when_some(collapsed_preview, |d, prev| {
                        d.child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .text_color(rgb(INK_MUTED))
                                .child(prev),
                        )
                    }),
            );

            if !is_collapsed {
                if rows.is_empty() {
                    let empty_msg = if has_any_filter {
                        match l {
                            crate::i18n::UiLang::Zh => format!(
                                "当前筛选下 [{} · {}] 暂无匹配标的。",
                                bucket.code(),
                                bucket.short_label_for(l)
                            ),
                            crate::i18n::UiLang::En => format!(
                                "No matching holdings in [{} · {}] for active filters.",
                                bucket.code(),
                                bucket.short_label_for(l)
                            ),
                        }
                    } else {
                        match l {
                            crate::i18n::UiLang::Zh => format!(
                                "当前 [{} · {}] 暂无标的。",
                                bucket.code(),
                                bucket.short_label_for(l)
                            ),
                            crate::i18n::UiLang::En => format!(
                                "No holdings in [{} · {}] yet.",
                                bucket.code(),
                                bucket.short_label_for(l)
                            ),
                        }
                    };
                    block = block.child(
                        div()
                            .px_4()
                            .py_3p5()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(empty_msg),
                    );
                } else if matches!(bucket, Bucket::Core | Bucket::Watch) {
                    block = block.child(
                        div()
                            .flex()
                            .items_center()
                            .px_4()
                            .py_2()
                            .bg(rgb(BG_ROW_ALT))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .children(headers.iter().map(|(label, w, right, sort_opt)| {
                                let is_sorted = sort_opt.is_some_and(|c| c == self.sort_col);
                                let arrow = if is_sorted {
                                    if self.sort_asc { " ↑" } else { " ↓" }
                                } else {
                                    ""
                                };
                                let is_nd = *label == nd_label;
                                let is_status = *label == status_label;
                                div()
                                    .id(gpui::SharedString::from(format!(
                                        "hdr-{}-{label}",
                                        bucket.code()
                                    )))
                                    .px_1p5()
                                    .when(is_nd, |d| d.pr_4())
                                    .when(is_status, |d| d.pl_3())
                                    .flex()
                                    .items_center()
                                    .when(*right, |d| d.justify_end())
                                    .gap_1()
                                    .when(*w > 0., |d| d.w(px(*w)).flex_shrink_0())
                                    .when(*w == 0., |d| d.flex_1().min_w_0())
                                    .when_some(*sort_opt, |d, col| {
                                        d.cursor_pointer()
                                            .hover(|s| s.text_color(rgb(INK_PRIMARY)))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle_sort(col);
                                                cx.notify();
                                            }))
                                    })
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_size(px(11.))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(if is_sorted {
                                                INK_PRIMARY
                                            } else {
                                                INK_MUTED
                                            }))
                                            .child(*label),
                                    )
                                    .when(!arrow.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .font_family(self.fonts.mono_term.clone())
                                                .text_size(px(10.))
                                                .text_color(rgb(INK_HERO))
                                                .child(arrow),
                                        )
                                    })
                            })),
                    );

                    for (local_idx, rep) in rows.into_iter().enumerate() {
                        let global_idx = linear_idx;
                        linear_idx += 1;

                        let val = primary_val(rep, true);
                        let (s_fg, s_bg, s_br) = status_colors(rep.status);
                        let dist_opt = val.and_then(|v| v.distance_to_strike_pct);
                        let (dist_str, _) = fmt_pct_clean(dist_opt, true);
                        let dist_col = match dist_opt {
                            Some(d) if d <= Decimal::ZERO => EMERALD_FG,
                            Some(_) => INK_SECONDARY,
                            None => INK_MUTED,
                        };
                        let (roic_str, _) = fmt_pct_clean(rep.roic_5y_avg_pct, false);
                        let roic_col = match rep.roic_5y_avg_pct {
                            Some(v) if v >= Decimal::from(15) => INK_PRIMARY,
                            Some(_) => GOLD_FG,
                            None => INK_MUTED,
                        };
                        let yield_opt = val.and_then(|v| v.strict_fcf_yield_mcap_pct);
                        let (yield_str, yield_pol) = fmt_pct_clean(yield_opt, false);
                        let yield_col = match yield_opt {
                            Some(_) if yield_pol < 0 => CORAL_FG,
                            Some(_) => INK_PRIMARY,
                            None => INK_MUTED,
                        };
                        let (share_str, share_pol) = fmt_pct_clean(rep.share_change_3y_pct, true);
                        let share_col = share_delta_color(rep.share_change_3y_pct, share_pol);
                        let nd_oe = rep
                            .net_debt_to_owner_earnings
                            .map(|v| format!("{v:.2}x"))
                            .unwrap_or_else(|| "—".into());
                        let nd_col = if rep.net_debt_to_owner_earnings.is_some() {
                            INK_SECONDARY
                        } else {
                            INK_MUTED
                        };

                        let is_cursor = global_idx == self.cursor_index;
                        let row_bg = if is_cursor {
                            BG_SELECTED_ROW
                        } else if local_idx % 2 == 0 {
                            BG_VIEWPORT
                        } else {
                            BG_ROW_ALT
                        };
                        let ticker = rep.ticker.clone();
                        let tag = primary_circle_tag(&rep.circle);
                        let tag_active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_for_click = tag.clone();
                        let sub_detail = secondary_circle_detail(&rep.circle);

                        block = block.child(
                            div()
                                .id(gpui::SharedString::from(format!("wl-row-{}", rep.ticker)))
                                .relative()
                                .flex()
                                .items_center()
                                .px_4()
                                .py_2p5()
                                .bg(rgb(row_bg))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .active(|s| s.bg(rgb(BG_ACTIVE)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cursor_index = global_idx;
                                    this.select_company(&ticker, Some(NavView::Dossier));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .px_1p5()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.mono_num.clone())
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_color(rgb(INK_HERO))
                                                        .child(rep.ticker.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.ui.clone())
                                                        .text_xs()
                                                        .text_color(rgb(INK_PRIMARY))
                                                        .truncate()
                                                        .child(rep.name.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .mt_1()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(circle_tag_button(
                                                    format!("row-tag-{}", rep.ticker),
                                                    tag,
                                                    tag_active,
                                                    false,
                                                    None,
                                                    &self.fonts,
                                                    cx.listener(move |this, _, _, cx| {
                                                        this.toggle_circle_filter(&tag_for_click);
                                                        cx.notify();
                                                    }),
                                                ))
                                                .when_some(sub_detail, |d, det| {
                                                    d.child(
                                                        div()
                                                            .font_family(self.fonts.ui.clone())
                                                            .text_size(px(11.))
                                                            .text_color(rgb(INK_MUTED))
                                                            .truncate()
                                                            .child(det),
                                                    )
                                                }),
                                        ),
                                )
                                .child(mono_right(
                                    fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency),
                                    90.,
                                    INK_HERO,
                                    &self.fonts,
                                ))
                                .child(mono_right(
                                    fmt_price(val.and_then(|v| v.strike_price), &rep.currency),
                                    90.,
                                    if val.and_then(|v| v.strike_price).is_some() {
                                        INK_SECONDARY
                                    } else {
                                        INK_MUTED
                                    },
                                    &self.fonts,
                                ))
                                .child(mono_right(dist_str, 90., dist_col, &self.fonts))
                                .child(mono_right(roic_str, 84., roic_col, &self.fonts))
                                .child(mono_right(yield_str, 88., yield_col, &self.fonts))
                                .child(mono_right(share_str, 84., share_col, &self.fonts))
                                .child(
                                    div()
                                        .w(px(82.))
                                        .flex_shrink_0()
                                        .pl_1p5()
                                        .pr_4()
                                        .text_right()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_xs()
                                        .text_color(rgb(nd_col))
                                        .truncate()
                                        .child(nd_oe),
                                )
                                .child(
                                    div()
                                        .w(px(112.))
                                        .flex_shrink_0()
                                        .pl_3()
                                        .pr_1p5()
                                        .flex()
                                        .items_center()
                                        .child(badge(
                                            rep.status.label_for(l).to_owned(),
                                            s_fg,
                                            s_bg,
                                            s_br,
                                            &self.fonts,
                                        )),
                                ),
                        );
                    }
                } else {
                    // OUT & TOO HARD: Compact Decision Specimen Cards
                    for (local_idx, rep) in rows.into_iter().enumerate() {
                        let global_idx = linear_idx;
                        linear_idx += 1;

                        let is_cursor = global_idx == self.cursor_index;
                        let row_bg = if is_cursor {
                            BG_SELECTED_ROW
                        } else if local_idx % 2 == 0 {
                            BG_VIEWPORT
                        } else {
                            BG_ROW_ALT
                        };
                        let ticker = rep.ticker.clone();
                        let tag = primary_circle_tag(&rep.circle);
                        let tag_active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_for_click = tag.clone();
                        let latest_row = rep.ttm_row.as_ref().or_else(|| rep.annual_rows.last());
                        let price_txt =
                            fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency);
                        let oe_txt = fmt_money_compact_opt(
                            latest_row.and_then(|r| r.strict_owner_earnings),
                            &rep.currency,
                        );

                        block = block.child(
                            div()
                                .id(gpui::SharedString::from(format!("sp-row-{}", rep.ticker)))
                                .relative()
                                .px_4()
                                .py_3()
                                .bg(rgb(row_bg))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cursor_index = global_idx;
                                    this.select_company(&ticker, Some(NavView::Dossier));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_3()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.mono_num.clone())
                                                        .text_sm()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_color(rgb(INK_HERO))
                                                        .child(rep.ticker.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.ui.clone())
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .text_color(rgb(INK_PRIMARY))
                                                        .child(rep.name.clone()),
                                                )
                                                .child(circle_tag_button(
                                                    format!("sp-tag-{}", rep.ticker),
                                                    tag,
                                                    tag_active,
                                                    false,
                                                    None,
                                                    &self.fonts,
                                                    cx.listener(move |this, _, _, cx| {
                                                        this.toggle_circle_filter(&tag_for_click);
                                                        cx.notify();
                                                    }),
                                                )),
                                        )
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_num.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_SECONDARY))
                                                .child(format!(
                                                    "{} {price_txt} · {} {oe_txt}",
                                                    l.t("现价", "Price"),
                                                    l.t("严苛 OE", "Strict OE"),
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .mt_1p5()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(13.))
                                        .line_height(px(20.))
                                        .text_color(rgb(INK_SECONDARY))
                                        .child(rep.too_hard_reason.clone().unwrap_or_else(|| {
                                            l.t("已拒绝或能力圈外。", "Rejected or Too Hard.")
                                                .into()
                                        })),
                                ),
                        );
                    }
                }
            }

            cards = cards.child(block);
        }

        root.child(cards)
    }

    pub(super) fn render_baskets_board(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let board_order = [Bucket::Watch, Bucket::Core, Bucket::Out, Bucket::TooHard];
        let mut linear_idx: usize = 0;

        div()
            .px_5()
            .pt_3()
            .pb_5()
            .flex()
            .gap_3()
            .children(board_order.into_iter().map(|bucket| {
                let (fg, bg, br) = bucket_colors(bucket);
                let is_focused = self.basket_filter == Some(bucket);
                let rows = filter_reports_for_basket(
                    &self.reports,
                    bucket,
                    self.circle_filter.as_deref(),
                    self.status_filter,
                    self.sort_col,
                    self.sort_asc,
                );
                let cnt = rows.len();

                let col = div()
                    .flex_1()
                    .min_w_0()
                    .rounded_lg()
                    .bg(rgb(BG_ROW_ALT))
                    .border_1()
                    .border_color(rgba(if is_focused {
                        br
                    } else {
                        BORDER_SUBTLE << 8 | 0xFF
                    }))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_3()
                            .py_2p5()
                            .bg(rgba(if is_focused { bg } else { 0x161620FF }))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(fg)))
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(fg))
                                            .child(bucket.code()),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .child(bucket.short_label_for(l)),
                                    ),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(format!("{cnt}")),
                            ),
                    );

                let mut body = div().p_2p5().flex().flex_col().gap_2();
                if rows.is_empty() {
                    body = body.child(
                        div()
                            .p_3()
                            .rounded_md()
                            .bg(rgb(BG_VIEWPORT))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED))
                            .child(l.t("暂无标的", "Empty")),
                    );
                } else {
                    for rep in rows {
                        let global_idx = linear_idx;
                        linear_idx += 1;

                        let is_cursor = global_idx == self.cursor_index;
                        let val = primary_val(rep, true);
                        let (s_fg, s_bg, s_br) = status_colors(rep.status);
                        let price_txt =
                            fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency);
                        let dist_txt =
                            fmt_pct_clean(val.and_then(|v| v.distance_to_strike_pct), true).0;
                        let roic_txt = fmt_pct_clean(rep.roic_5y_avg_pct, false).0;
                        let tag = primary_circle_tag(&rep.circle);
                        let tag_active = self.circle_filter.as_deref() == Some(tag.as_str());
                        let tag_click = tag.clone();
                        let ticker = rep.ticker.clone();

                        body = body.child(
                            div()
                                .id(gpui::SharedString::from(format!("brd-card-{}", rep.ticker)))
                                .p_2p5()
                                .rounded_md()
                                .bg(rgb(if is_cursor {
                                    BG_SELECTED_ROW
                                } else {
                                    BG_SURFACE
                                }))
                                .border_1()
                                .border_color(rgb(if is_cursor { INK_HERO } else { BORDER_SUBTLE }))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cursor_index = global_idx;
                                    this.select_company(&ticker, Some(NavView::Dossier));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_1p5()
                                                .child(
                                                    div()
                                                        .w(px(6.))
                                                        .h(px(6.))
                                                        .rounded_full()
                                                        .bg(rgb(status_dot_color(rep.status))),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(self.fonts.mono_num.clone())
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::BOLD)
                                                        .text_color(rgb(INK_HERO))
                                                        .child(rep.ticker.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_num.clone())
                                                .text_xs()
                                                .text_color(rgb(INK_PRIMARY))
                                                .child(price_txt),
                                        ),
                                )
                                .child(
                                    div()
                                        .mt_0p5()
                                        .font_family(self.fonts.ui.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_SECONDARY))
                                        .truncate()
                                        .child(rep.name.clone()),
                                )
                                .child(match bucket {
                                    Bucket::Core | Bucket::Watch => div()
                                        .mt_2()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .font_family(self.fonts.mono_num.clone())
                                                .text_size(px(10.))
                                                .text_color(rgb(INK_MUTED))
                                                .truncate()
                                                .child(format!(
                                                    "{} {dist_txt} · ROIC {roic_txt}",
                                                    l.t("边际", "Diff")
                                                )),
                                        )
                                        .child(badge(
                                            rep.status.label_for(l).to_owned(),
                                            s_fg,
                                            s_bg,
                                            s_br,
                                            &self.fonts,
                                        )),
                                    Bucket::Out | Bucket::TooHard => div()
                                        .mt_2()
                                        .font_family(self.fonts.ui.clone())
                                        .text_size(px(11.))
                                        .line_height(px(16.))
                                        .text_color(rgb(INK_MUTED))
                                        .child(rep.too_hard_reason.clone().unwrap_or_else(|| {
                                            l.t("已拒绝或能力圈外", "Rejected or Too Hard").into()
                                        })),
                                })
                                .child(
                                    div()
                                        .mt_2()
                                        .pt_1p5()
                                        .border_t_1()
                                        .border_color(rgb(BORDER_HAIRLINE))
                                        .flex()
                                        .items_center()
                                        .child(circle_tag_button(
                                            format!("brd-tag-{}", rep.ticker),
                                            tag,
                                            tag_active,
                                            false,
                                            None,
                                            &self.fonts,
                                            cx.listener(move |this, _, _, cx| {
                                                this.toggle_circle_filter(&tag_click);
                                                cx.notify();
                                            }),
                                        )),
                                ),
                        );
                    }
                }

                col.child(body)
            }))
    }
}
