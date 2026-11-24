use super::{MungerWorkbench, NavView, components::*, format::*, theme::*};
use crate::model::{Bucket, MungerReport};
use crate::update;
use gpui::{
    Context, MouseButton, MouseDownEvent, WindowControlArea, div, prelude::*, px, rgb, rgba,
};

impl MungerWorkbench {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let nav_items = [
            (
                NavView::Baskets,
                "总览",
                "⌘1",
                self.active_view == NavView::Baskets && self.basket_filter.is_none(),
                INK_HERO,
            ),
            (
                NavView::Universe,
                "全市场雷达",
                "⌘5",
                self.active_view == NavView::Universe,
                SKY_FG,
            ),
            (
                NavView::Journal,
                "决策账本",
                "⌘4",
                self.active_view == NavView::Journal,
                GOLD_FG,
            ),
        ];

        div()
            .w(px(224.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(BG_SHELL))
            // Immersive Traffic-Light Header Zone + Pixel-Art M3 Monogram
            .child(
                div()
                    .h(px(40.))
                    .flex_shrink_0()
                    .pl(px(88.))
                    .pr_3()
                    .flex()
                    .items_center()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, ev: &MouseDownEvent, window, _| {
                            if ev.click_count == 2 {
                                window.titlebar_double_click();
                            }
                        }),
                    )
                    .child(pixel_m3_mark(1.4)),
            )
            // Single Command Palette Trigger Bar
            .child(
                div().px_2p5().pb_2().child(
                    div()
                        .id("sidebar-cmd-trigger")
                        .px_2p5()
                        .py_1p5()
                        .rounded_md()
                        .bg(rgb(0x13131B))
                        .border_1()
                        .border_color(rgb(0x222230))
                        .hover(|s| s.bg(rgb(0x1A1A26)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .justify_between()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_palette(window);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(11.))
                                .text_color(rgb(INK_MUTED))
                                .child("> 跳转 / 命令…"),
                        )
                        .child(kbd("⌘K", &self.fonts)),
                ),
            )
            // Portfolio-Level Workspaces (Overview + Decision Journal)
            .child(
                div()
                    .px_2p5()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        div()
                            .px_2()
                            .mb_1()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED))
                            .child("工作台"),
                    )
                    .children(nav_items.into_iter().map(
                        |(view, title, key, selected, rail_col)| {
                            div()
                                .id(gpui::SharedString::from(format!("nav-{key}")))
                                .px_2()
                                .py_1p5()
                                .mb_0p5()
                                .rounded_md()
                                .cursor_pointer()
                                .when(selected, |d| d.bg(rgba(RAIL_SELECTED)))
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                .flex()
                                .items_center()
                                .justify_between()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if view == NavView::Baskets {
                                        this.focus_basket(None);
                                    } else {
                                        this.switch_view(view);
                                    }
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(2.))
                                                .h(px(13.))
                                                .rounded_full()
                                                .when(selected, |d| d.bg(rgb(rail_col))),
                                        )
                                        .child(
                                            div()
                                                .font_family(self.fonts.ui.clone())
                                                .text_xs()
                                                .font_weight(if selected {
                                                    gpui::FontWeight::SEMIBOLD
                                                } else {
                                                    gpui::FontWeight::NORMAL
                                                })
                                                .text_color(rgb(if selected {
                                                    INK_HERO
                                                } else {
                                                    INK_SECONDARY
                                                }))
                                                .child(title),
                                        ),
                                )
                                .child(kbd(key, &self.fonts))
                        },
                    )),
            )
            // Unified Scrollable Basket Tree (Click header to filter basket; click ticker to open workspace)
            .child(
                div()
                    .id("rail-companies")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2p5()
                    .py_2p5()
                    .child(
                        div()
                            .px_2()
                            .mb_1p5()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED))
                            .child("芒格篮子"),
                    )
                    .children(Bucket::display_order().into_iter().map(|b| {
                        let members: Vec<&MungerReport> =
                            self.reports.iter().filter(|r| r.bucket == b).collect();
                        let cnt = members.len();
                        let (fg, _, _) = bucket_colors(b);
                        let bkt_active =
                            self.active_view == NavView::Baskets && self.basket_filter == Some(b);

                        div()
                            .mb_2()
                            .child(
                                div()
                                    .id(gpui::SharedString::from(format!("rail-bkt-{}", b.code())))
                                    .px_2()
                                    .py_1()
                                    .mb_0p5()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(bkt_active, |d| d.bg(rgba(RAIL_SELECTED)))
                                    .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                    .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if bkt_active {
                                            this.focus_basket(None);
                                        } else {
                                            this.focus_basket(Some(b));
                                        }
                                        cx.notify();
                                    }))
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
                                                    .bg(rgb(fg)),
                                            )
                                            .child(
                                                div()
                                                    .font_family(self.fonts.mono_term.clone())
                                                    .text_size(px(10.))
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_color(rgb(if bkt_active {
                                                        fg
                                                    } else {
                                                        INK_PRIMARY
                                                    }))
                                                    .child(b.code()),
                                            )
                                            .child(
                                                div()
                                                    .font_family(self.fonts.ui.clone())
                                                    .text_size(px(11.))
                                                    .text_color(rgb(if bkt_active {
                                                        INK_HERO
                                                    } else {
                                                        INK_MUTED
                                                    }))
                                                    .child(b.short_label()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(10.))
                                            .text_color(rgb(if bkt_active {
                                                INK_HERO
                                            } else {
                                                INK_MUTED
                                            }))
                                            .child(format!("{cnt}")),
                                    ),
                            )
                            .children(members.into_iter().map(|rep| {
                                let is_selected = self.selected_ticker == rep.ticker
                                    && matches!(
                                        self.active_view,
                                        NavView::Dossier | NavView::Agent
                                    );
                                let dot_col = status_dot_color(rep.status);
                                let ticker = rep.ticker.clone();
                                let price_str =
                                    fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency);
                                div()
                                    .id(gpui::SharedString::from(format!("pick-{}", rep.ticker)))
                                    .pl_3p5()
                                    .pr_2()
                                    .py_1()
                                    .mb_0p5()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .when(is_selected, |d| d.bg(rgba(RAIL_SELECTED)))
                                    .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                    .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_company_contextual(&ticker);
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .w(px(5.))
                                                    .h(px(5.))
                                                    .rounded_full()
                                                    .bg(rgb(dot_col)),
                                            )
                                            .child(
                                                div()
                                                    .font_family(self.fonts.mono_num.clone())
                                                    .text_xs()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(if is_selected {
                                                        INK_HERO
                                                    } else {
                                                        INK_PRIMARY
                                                    }))
                                                    .child(rep.ticker.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_num.clone())
                                            .text_size(px(11.))
                                            .text_color(rgb(if is_selected {
                                                INK_SECONDARY
                                            } else {
                                                INK_RAIL_PRICE
                                            }))
                                            .child(price_str),
                                    )
                            }))
                    })),
            )
    }

    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let bench_val = match &self.benchmark {
            Some(b) => format!("{:.2}", b.price),
            None => "—".into(),
        };
        let sync_label = match &self.sync_status {
            Some(s) => fmt_time_cn(Some(s.synced_at_ms)),
            None => "未同步".into(),
        };
        let selected_name = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .map(|r| r.name.clone())
            .unwrap_or_default();

        let breadcrumb = match self.active_view {
            NavView::Baskets => match (self.basket_filter, &self.circle_filter) {
                (Some(b), Some(tag)) => {
                    format!("总览 / {} ({}) / #{tag}", b.code(), b.short_label())
                }
                (Some(b), None) => format!("总览 / {} ({})", b.code(), b.short_label()),
                (None, Some(tag)) => format!("总览 / #{tag}"),
                (None, None) => "总览".to_owned(),
            },
            NavView::Dossier | NavView::Agent => {
                format!("{} · {}", self.selected_ticker, selected_name)
            }
            NavView::Journal => "决策账本".to_owned(),
            NavView::Universe => "全市场雷达 · US Lakehouse L0".to_owned(),
        };

        div()
            .h(px(40.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(BG_TOOLBAR))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .when(
                        matches!(self.active_view, NavView::Dossier | NavView::Agent),
                        |d| {
                            d.child(
                                div()
                                    .id("tb-back")
                                    .px_2()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .active(|s| s.bg(rgb(BG_ACTIVE)))
                                    .cursor_pointer()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_SECONDARY))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.switch_view(NavView::Baskets);
                                        cx.notify();
                                    }))
                                    .child("←"),
                            )
                        },
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_PRIMARY))
                            .child(breadcrumb),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, ev: &MouseDownEvent, window, _| {
                            if ev.click_count == 2 {
                                window.titlebar_double_click();
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(self.update_applied_version.clone(), |d, ver| {
                        d.child(
                            div()
                                .id("update-restart-pill")
                                .px_2p5()
                                .py_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgba(EMERALD_BG))
                                .border_1()
                                .border_color(rgba(EMERALD_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .active(|s| s.bg(rgb(BG_ACTIVE)))
                                .font_family(self.fonts.mono_num.clone())
                                .text_size(px(11.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(EMERALD_FG))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.restart_after_update(cx);
                                }))
                                .child(format!("已更新 v{ver} · 重启")),
                        )
                    })
                    .when(
                        self.update_applied_version.is_none() && self.update_applying,
                        |d| {
                            d.child(
                                div()
                                    .id("update-applying-pill")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_sm()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child("更新中…"),
                            )
                        },
                    )
                    .when_some(
                        if self.update_applied_version.is_none() && !self.update_applying {
                            self.update_notice.clone()
                        } else {
                            None
                        },
                        |d, notice| {
                            d.child(
                                div()
                                    .id("update-available-pill")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .bg(rgba(SKY_BG))
                                    .border_1()
                                    .border_color(rgba(SKY_BORDER))
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .active(|s| s.bg(rgb(BG_ACTIVE)))
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(SKY_FG))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_update_apply(cx);
                                    }))
                                    .child(format!("↑ v{}", notice.latest_version)),
                            )
                        },
                    )
                    .child(
                        div()
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(BG_SURFACE))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child("S&P 500"),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_PRIMARY))
                                    .child(bench_val),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .px_2()
                            .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(
                                if self.sync_status.is_some() {
                                    EMERALD_FG
                                } else {
                                    INK_MUTED
                                },
                            )))
                            .child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(11.))
                                    .text_color(rgb(INK_MUTED))
                                    .child(sync_label),
                            ),
                    )
                    .child(
                        div()
                            .id("sync-btn")
                            .px_2p5()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .border_1()
                            .border_color(rgb(if self.syncing {
                                BORDER_SUBTLE
                            } else {
                                BORDER_PILL
                            }))
                            .bg(rgb(BG_WELL))
                            .hover(|s| s.bg(rgb(0x21212F)))
                            .active(|s| s.bg(rgb(BG_ACTIVE)))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_sync(cx);
                            }))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(if self.syncing {
                                        INK_MUTED
                                    } else {
                                        INK_PRIMARY
                                    }))
                                    .child(if self.syncing {
                                        "同步中…"
                                    } else {
                                        "同步"
                                    }),
                            )
                            .child(kbd("⌘R", &self.fonts)),
                    ),
            )
    }

    pub(super) fn render_statusline(&self, cx: &mut Context<Self>) -> gpui::Div {
        let (dot_col, legend) = if self.palette_open {
            (SKY_FG, "↑/↓ 选择 · ↵ 执行 · esc 关闭命令面板")
        } else {
            match self.active_view {
                NavView::Baskets => (
                    EMERALD_FG,
                    "v 列表/看板 · 1-4 切篮子 · 0 全部 · j/k 移动 · ↵ 打开 · ⌘K 命令",
                ),
                NavView::Dossier | NavView::Agent => (
                    if self.active_view == NavView::Agent {
                        VIOLET_FG
                    } else {
                        EMERALD_FG
                    },
                    "1 财报K线 · 2 Alan · 3 XBRL · t TTM/FY · j/k 换股 · p Prompt · c CLI · esc 返回",
                ),
                NavView::Journal => (
                    GOLD_FG,
                    "/ 或 ⌘K 命令面板 · c 复制 journal CLI · esc 返回总览",
                ),
                NavView::Universe => (
                    SKY_FG,
                    "/ 搜索 · j/k 选择 · ↵ 打开已入池研报 · r 刷新 · ⌘K 命令 · esc 返回总览",
                ),
            }
        };

        div()
            .h(px(24.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .justify_between()
            .bg(rgb(BG_STATUSLINE))
            .border_t_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(dot_col)))
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(11.))
                            .text_color(rgb(INK_MUTED))
                            .truncate()
                            .child(legend),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("status-version-pill")
                            .px_1p5()
                            .py_0p5()
                            .rounded_xs()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(RAIL_HOVER)))
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(10.))
                            .text_color(rgb(INK_MUTED))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_update_check(true, cx);
                            }))
                            .child(if self.update_checking {
                                "检查更新中…".to_owned()
                            } else {
                                format!(
                                    "v{} · {}",
                                    update::current_binary_version(),
                                    self.update_channel.as_str()
                                )
                            }),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.mono_term.clone())
                            .text_size(px(10.))
                            .text_color(rgb(INK_MUTED))
                            .child(match self.active_view {
                                NavView::Agent => "EMBEDDED ALAN · PAPER ONLY",
                                NavView::Universe => "DUCKDB L0 · ZERO-LLM",
                                _ => "ZERO-LLM · PAPER ONLY",
                            }),
                    ),
            )
    }
}
