use super::{
    MungerWorkbench, NavView, components::*, format::*, inbox::budget_watermark_summary_for,
    theme::*,
};
use gpui::{
    Context, MouseButton, MouseDownEvent, WindowControlArea, div, prelude::*, px, rgb, rgba,
};
use m3_core::i18n::UiLang;
use m3_core::model::{Bucket, MungerReport};
use m3_core::update;

/// Compresses rolling beta tags like `0.3.1-beta.20261124.6` into `v0.3.1-beta.6`
/// so version badges never clip inside the 224px sidebar dock.
pub(super) fn compact_sidebar_version_label(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('v');
    if let Some((base, rest)) = trimmed.split_once("-beta.")
        && let Some((_, build)) = rest.rsplit_once('.')
        && !build.is_empty()
    {
        return format!("v{base}-beta.{build}");
    }
    format!("v{trimmed}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SidebarDockNotice {
    pub tag: &'static str,
    pub message: String,
    pub dot_color: u32,
    pub border_color: u32,
    pub dismissible: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_sidebar_dock_notice_for(
    banner: Option<&(String, bool)>,
    universe_syncing: bool,
    universe_progress: Option<&str>,
    inbox_catching_up: bool,
    inbox_progress: Option<&str>,
    deep_research_running: bool,
    industry_research_running: bool,
    industry_progress: Option<&str>,
    syncing: bool,
    themes_syncing: bool,
    update_checking: bool,
    update_applying: bool,
    lang: UiLang,
) -> Option<SidebarDockNotice> {
    if let Some((msg, true)) = banner {
        return Some(SidebarDockNotice {
            tag: "ERROR",
            message: msg.clone(),
            dot_color: CORAL_FG,
            border_color: CORAL_BORDER,
            dismissible: true,
        });
    }
    if update_applying {
        return Some(SidebarDockNotice {
            tag: "UPDATING",
            message: lang
                .t(
                    "正在下载并校验新版 M3.app…",
                    "Downloading and verifying M3.app update…",
                )
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if update_checking {
        let message = match banner {
            Some((msg, false)) => msg.clone(),
            _ => lang
                .t("正在检查新版本…", "Checking for updates…")
                .to_owned(),
        };
        return Some(SidebarDockNotice {
            tag: "UPDATE",
            message,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if universe_syncing {
        return Some(SidebarDockNotice {
            tag: "LAKE SYNC",
            message: universe_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    lang.t(
                        "正在后台同步全美股 L0 确定性湖仓…",
                        "Syncing US L0 deterministic lakehouse…",
                    )
                })
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if inbox_catching_up {
        return Some(SidebarDockNotice {
            tag: "CATCH-UP",
            message: inbox_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| lang.t("正在追平 SEC 事件流…", "Catching up L1 SEC event feed…"))
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if industry_research_running {
        return Some(SidebarDockNotice {
            tag: "INDUSTRY L2",
            message: industry_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    lang.t(
                        "Alan 4 视角行业深研运行中…",
                        "Running 4-perspective industry research…",
                    )
                })
                .to_owned(),
            dot_color: VIOLET_FG,
            border_color: VIOLET_BORDER,
            dismissible: false,
        });
    }
    if deep_research_running {
        return Some(SidebarDockNotice {
            tag: "ALAN L2",
            message: lang
                .t(
                    "4 视角并行深研运行中…",
                    "Running 4-perspective Deep Research…",
                )
                .to_owned(),
            dot_color: VIOLET_FG,
            border_color: VIOLET_BORDER,
            dismissible: false,
        });
    }
    if syncing {
        return Some(SidebarDockNotice {
            tag: "SYNCING",
            message: lang
                .t(
                    "正在同步核心池行情与财务档案…",
                    "Syncing Circle quotes and SEC financials…",
                )
                .to_owned(),
            dot_color: EMERALD_FG,
            border_color: EMERALD_BORDER,
            dismissible: false,
        });
    }
    if themes_syncing {
        return Some(SidebarDockNotice {
            tag: "THEMES",
            message: lang
                .t(
                    "正在同步商业原型主题图谱…",
                    "Syncing business archetype graph…",
                )
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if let Some((msg, false)) = banner {
        return Some(SidebarDockNotice {
            tag: "NOTICE",
            message: msg.clone(),
            dot_color: EMERALD_FG,
            border_color: EMERALD_BORDER,
            dismissible: true,
        });
    }
    None
}

#[allow(clippy::too_many_arguments, dead_code)]
pub(super) fn resolve_sidebar_dock_notice_from(
    banner: Option<&(String, bool)>,
    universe_syncing: bool,
    universe_progress: Option<&str>,
    inbox_catching_up: bool,
    inbox_progress: Option<&str>,
    deep_research_running: bool,
    syncing: bool,
    themes_syncing: bool,
    update_checking: bool,
    update_applying: bool,
) -> Option<SidebarDockNotice> {
    resolve_sidebar_dock_notice_for(
        banner,
        universe_syncing,
        universe_progress,
        inbox_catching_up,
        inbox_progress,
        deep_research_running,
        false,
        None,
        syncing,
        themes_syncing,
        update_checking,
        update_applying,
        UiLang::Zh,
    )
}

impl MungerWorkbench {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let unread_count = self
            .inbox_summary
            .as_ref()
            .map(|s| s.unread_signals)
            .unwrap_or(0);
        let nav_items = [
            (
                NavView::Inbox,
                l.t("简报收件箱", "Briefing Inbox"),
                "⌘1",
                self.active_view == NavView::Inbox,
                EMERALD_FG,
            ),
            (
                NavView::Universe,
                l.t("全市场雷达", "Universe Radar"),
                "⌘2",
                self.active_view == NavView::Universe,
                SKY_FG,
            ),
            (
                NavView::Industry,
                l.t("行业研究", "Industry Research"),
                "⌘3",
                self.active_view == NavView::Industry,
                VIOLET_FG,
            ),
            (
                NavView::Baskets,
                l.t("能力圈总览", "Circle"),
                "⌘4",
                self.active_view == NavView::Baskets && self.basket_filter.is_none(),
                INK_HERO,
            ),
            (
                NavView::Journal,
                l.t("决策账本", "Decision Journal"),
                "⌘5",
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
                                .child(l.t("> 跳转 / 命令…", "> Jump / Command…")),
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
                            .child(l.t("工作台", "WORKBENCH")),
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
                                        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(
                                            if selected { rail_col } else { BORDER_STRONG },
                                        )))
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
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .when(view == NavView::Inbox && unread_count > 0, |d| {
                                            d.child(
                                                div()
                                                    .px_1p5()
                                                    .py_0p5()
                                                    .rounded_full()
                                                    .bg(rgba(EMERALD_BG))
                                                    .border_1()
                                                    .border_color(rgba(EMERALD_BORDER))
                                                    .font_family(self.fonts.mono_num.clone())
                                                    .text_size(px(10.))
                                                    .font_weight(gpui::FontWeight::BOLD)
                                                    .text_color(rgb(EMERALD_FG))
                                                    .child(format!("{unread_count}")),
                                            )
                                        })
                                        .child(kbd(key, &self.fonts)),
                                )
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
                            .child(l.t("能力圈", "CIRCLE")),
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
                                                    .child(b.short_label_for(l)),
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
            // Settings navigation row above the bottom dock
            .child(
                div()
                    .px_2p5()
                    .py_1p5()
                    .border_t_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child({
                        let active = self.active_view == NavView::Settings;
                        div()
                            .id("nav-settings")
                            .px_2()
                            .py_1p5()
                            .rounded_md()
                            .cursor_pointer()
                            .when(active, |d| d.bg(rgba(RAIL_SELECTED)))
                            .hover(|s| s.bg(rgba(RAIL_HOVER)))
                            .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                            .flex()
                            .items_center()
                            .justify_between()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switch_view(NavView::Settings);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(5.))
                                            .h(px(5.))
                                            .rounded_full()
                                            .bg(rgb(if active { INK_HERO } else { BORDER_STRONG })),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .font_weight(if active {
                                                gpui::FontWeight::SEMIBOLD
                                            } else {
                                                gpui::FontWeight::NORMAL
                                            })
                                            .text_color(rgb(if active {
                                                INK_HERO
                                            } else {
                                                INK_SECONDARY
                                            }))
                                            .child(l.t("设置", "Settings")),
                                    ),
                            )
                            .child(kbd("⌘,", &self.fonts))
                    }),
            )
            .child(self.render_sidebar_status_dock(cx))
    }

    fn resolve_sidebar_dock_notice(&self) -> Option<SidebarDockNotice> {
        resolve_sidebar_dock_notice_for(
            self.banner.as_ref(),
            self.universe_syncing,
            self.universe_sync_progress.as_deref(),
            self.inbox_catching_up,
            self.inbox_catchup_progress.as_deref(),
            self.deep_research_running,
            self.industry_research_running,
            self.industry_research_progress.as_deref(),
            self.syncing,
            self.themes_syncing,
            self.update_checking,
            self.update_applying,
            self.lang,
        )
    }

    fn render_sidebar_status_dock(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let notice = self.resolve_sidebar_dock_notice();
        let total_companies = self
            .universe_summary
            .as_ref()
            .map(|s| s.total_companies)
            .unwrap_or(0);
        let lake_label = if total_companies > 0 {
            match l {
                UiLang::Zh => format!("L0 {total_companies} 家"),
                UiLang::En => format!("L0 {total_companies}"),
            }
        } else {
            l.t("L0 未同步", "L0 Unsynced").to_owned()
        };
        let (budget_text, budget_col) =
            budget_watermark_summary_for(self.budget_report.as_ref(), l);
        let budget_compact = budget_text
            .strip_prefix("本月 AI 预算 ")
            .or_else(|| budget_text.strip_prefix("AI Budget "))
            .unwrap_or(&budget_text)
            .to_owned();

        let mut dock = div()
            .flex_shrink_0()
            .px_2p5()
            .pt_2()
            .pb_2p5()
            .border_t_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .bg(rgb(BG_SHELL))
            .flex()
            .flex_col()
            .gap_2();

        if let Some(n) = notice {
            dock = dock.child(
                div()
                    .rounded_md()
                    .bg(rgb(BG_SURFACE))
                    .border_1()
                    .border_color(rgba(n.border_color))
                    .p_2p5()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
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
                                            .bg(rgb(n.dot_color)),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_size(px(10.))
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(n.dot_color))
                                            .child(n.tag),
                                    ),
                            )
                            .when(n.dismissible, |d| {
                                d.child(
                                    div()
                                        .id("sidebar-dock-dismiss")
                                        .px_1()
                                        .rounded_xs()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                        .font_family(self.fonts.mono_term.clone())
                                        .text_xs()
                                        .text_color(rgb(INK_MUTED))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.banner = None;
                                            cx.notify();
                                        }))
                                        .child("×"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .text_color(rgb(INK_PRIMARY))
                            .child(n.message),
                    ),
            );
        }

        let version_badge = if let Some(ver) = &self.update_applied_version {
            let short_ver = compact_sidebar_version_label(ver);
            div()
                .id("sidebar-update-restart")
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .px_1p5()
                .py_0p5()
                .rounded_xs()
                .cursor_pointer()
                .bg(rgba(EMERALD_BG))
                .border_1()
                .border_color(rgba(EMERALD_BORDER))
                .font_family(self.fonts.mono_num.clone())
                .text_size(px(10.))
                .text_color(rgb(EMERALD_FG))
                .truncate()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.restart_after_update(cx);
                }))
                .child(format!("{} {short_ver}", l.t("重启", "Restart")))
        } else if let Some(notice) = &self.update_notice {
            let short_ver = compact_sidebar_version_label(&notice.latest_version);
            div()
                .id("sidebar-update-apply")
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .px_1p5()
                .py_0p5()
                .rounded_xs()
                .cursor_pointer()
                .bg(rgba(SKY_BG))
                .border_1()
                .border_color(rgba(SKY_BORDER))
                .font_family(self.fonts.mono_num.clone())
                .text_size(px(10.))
                .text_color(rgb(SKY_FG))
                .truncate()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.start_update_apply(cx);
                }))
                .child(format!("↑ {short_ver}"))
        } else {
            let short_ver = compact_sidebar_version_label(update::current_binary_version());
            div()
                .id("sidebar-update-check")
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .py_0p5()
                .rounded_xs()
                .cursor_pointer()
                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                .font_family(self.fonts.mono_num.clone())
                .text_size(px(10.))
                .text_color(rgb(INK_MUTED))
                .truncate()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.start_update_check(true, cx);
                }))
                .child(if self.update_checking {
                    l.t("检查更新中…", "Checking…").to_owned()
                } else {
                    short_ver
                })
        };

        let channel_label = self.update_channel.as_str().to_owned();

        dock.child(
            div()
                .px_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(div().w(px(5.)).h(px(5.)).flex_shrink_0().rounded_full().bg(
                                    rgb(if total_companies > 0 {
                                        EMERALD_FG
                                    } else {
                                        INK_MUTED
                                    }),
                                ))
                                .child(
                                    div()
                                        .font_family(self.fonts.mono_num.clone())
                                        .text_size(px(10.))
                                        .text_color(rgb(INK_SECONDARY))
                                        .truncate()
                                        .child(lake_label),
                                ),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_family(self.fonts.mono_num.clone())
                                .text_size(px(10.))
                                .text_color(rgb(budget_col))
                                .child(budget_compact),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(version_badge)
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_family(self.fonts.mono_term.clone())
                                .text_size(px(10.))
                                .text_color(rgb(INK_MUTED))
                                .child(channel_label),
                        ),
                ),
        )
    }

    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let bench_val = match &self.benchmark {
            Some(b) => format!("{:.2}", b.price),
            None => "—".into(),
        };
        let sync_label = match &self.sync_status {
            Some(s) => fmt_time_cn_for(Some(s.synced_at_ms), l),
            None => l.t("未同步", "Unsynced").into(),
        };
        let selected_name = self
            .reports
            .iter()
            .find(|r| r.ticker == self.selected_ticker)
            .map(|r| r.name.clone())
            .unwrap_or_default();

        let breadcrumb = match self.active_view {
            NavView::Inbox => l
                .t(
                    "简报收件箱 · Event-Driven L1",
                    "Briefing Inbox · Event-Driven L1",
                )
                .to_owned(),
            NavView::Baskets => match (self.basket_filter, &self.circle_filter) {
                (Some(b), Some(tag)) => {
                    format!(
                        "{} / {} ({}) / #{tag}",
                        l.t("能力圈总览", "Circle"),
                        b.code(),
                        b.short_label_for(l)
                    )
                }
                (Some(b), None) => format!(
                    "{} / {} ({})",
                    l.t("能力圈总览", "Circle"),
                    b.code(),
                    b.short_label_for(l)
                ),
                (None, Some(tag)) => format!("{} / #{tag}", l.t("能力圈总览", "Circle")),
                (None, None) => l.t("能力圈总览", "Circle Overview").to_owned(),
            },
            NavView::Dossier | NavView::Agent => {
                if self.selected_ticker.is_empty() {
                    "Research Desk".to_owned()
                } else {
                    format!(
                        "{} · {} · Research Desk",
                        self.selected_ticker, selected_name
                    )
                }
            }
            NavView::Industry => {
                if let Some(ind) = self.selected_industry() {
                    format!(
                        "{} · {} ({})",
                        l.t("行业研究", "Industry Research"),
                        ind.title,
                        ind.status.label_for(l)
                    )
                } else {
                    l.t(
                        "行业研究工作台 · Alan Deep Research",
                        "Industry Research Studio · Alan Deep Research",
                    )
                    .to_owned()
                }
            }
            NavView::Journal => l
                .t("决策账本 · Decision Journal", "Decision Journal")
                .to_owned(),
            NavView::Universe => l
                .t(
                    "全市场雷达 · US Lakehouse L0/L1",
                    "Universe Radar · US Lakehouse L0/L1",
                )
                .to_owned(),
            NavView::Settings => l
                .t(
                    "设置 · Preferences & Guardrails",
                    "Settings · Preferences & Guardrails",
                )
                .to_owned(),
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
                                .child(format!(
                                    "{} v{ver} · {}",
                                    l.t("已更新", "Updated"),
                                    l.t("重启", "Restart")
                                )),
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
                                    .child(l.t("更新中…", "Updating…")),
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
                                        l.t("同步中…", "Syncing…")
                                    } else {
                                        l.t("同步", "Sync")
                                    }),
                            )
                            .child(kbd("⌘R", &self.fonts)),
                    ),
            )
    }

    pub(super) fn render_statusline(&self) -> gpui::Div {
        let l = self.lang;
        let (dot_col, legend) = if self.palette_open {
            (
                SKY_FG,
                l.t(
                    "↑/↓ 选择 · ↵ 执行 · esc 关闭命令面板",
                    "↑/↓ select · ↵ run · esc close palette",
                ),
            )
        } else {
            match self.active_view {
                NavView::Inbox => (
                    EMERALD_FG,
                    l.t(
                        "j/k 移动 · ↵ 打开 · w 观察 · x TooHard · e 归档 · u 未读 · d Alan · r 追平 · ⌘K 命令",
                        "j/k move · ↵ open · w watch · x too hard · e archive · u unread · d Alan · r catch-up · ⌘K commands",
                    ),
                ),
                NavView::Baskets => (
                    EMERALD_FG,
                    l.t(
                        "v 列表/看板 · 1-4 切分组 · 0 全部 · j/k 移动 · ↵ 打开 · ⌘K 命令",
                        "v list/board · 1-4 tier · 0 all · j/k move · ↵ open · ⌘K commands",
                    ),
                ),
                NavView::Dossier | NavView::Agent => (
                    if self.active_view == NavView::Agent {
                        VIOLET_FG
                    } else {
                        EMERALD_FG
                    },
                    l.t(
                        "1 财报与活研报 · 2 Deep Research & Alan · 3 XBRL · t TTM/FY · j/k 换股 · p Prompt · c CLI · esc 返回",
                        "1 Financials & Memo · 2 Deep Research & Alan · 3 XBRL · t TTM/FY · j/k switch · p Prompt · c CLI · esc back",
                    ),
                ),
                NavView::Industry => (
                    VIOLET_FG,
                    l.t(
                        "新建行业 / 选中代表公司 / Alan 4 视角深研 + Web 检索 + 费曼通关进能力圈 · s 同步 SIC 聚类 · ⌘K 命令",
                        "New Study / Bound Peers / Alan 4-Lens + Web Search + Feynman Graduation · s sync SIC clusters · ⌘K commands",
                    ),
                ),
                NavView::Journal => (
                    GOLD_FG,
                    l.t(
                        "/ 或 ⌘K 命令面板 · c 复制 journal CLI · esc 返回总览",
                        "/ or ⌘K palette · c copy journal CLI · esc back to Circle",
                    ),
                ),
                NavView::Universe => (
                    SKY_FG,
                    l.t(
                        "/ 搜索 · j/k 选择 · ↵ 打开已入池研报 · r 刷新 · ⌘K 命令 · esc 返回",
                        "/ search · j/k select · ↵ open dossier · r refresh · ⌘K palette · esc back",
                    ),
                ),
                NavView::Settings => (
                    SKY_FG,
                    l.t(
                        "中/英双语即时切换 · 工作区目录 · 更新通道 · L2 预算护栏 · esc 返回",
                        "Live Zh/En switch · Workspace · Updates · L2 Budget Guardrails · esc back",
                    ),
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
            .gap_4()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(5.))
                            .h(px(5.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(dot_col)),
                    )
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
                div().flex_shrink_0().flex().items_center().child(
                    div()
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .child(match self.active_view {
                            NavView::Inbox => "L0/L1 · 0 FLAGSHIP TOKEN",
                            NavView::Agent | NavView::Industry => "EMBEDDED ALAN · PAPER ONLY",
                            NavView::Universe => "DUCKDB L0/L1 · ZERO-LLM",
                            _ => "ZERO-LLM · PAPER ONLY",
                        }),
                ),
            )
    }
}
