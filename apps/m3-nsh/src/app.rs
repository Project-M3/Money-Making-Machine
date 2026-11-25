//! Root M3 shell: left sidebar, Circle summary, Status Dock, topbar, shortcuts, and viewport.

use crate::components::{kbd, pixel_m3_mark, progress_bar};
use crate::format::{
    budget_watermark_summary_for, compact_sidebar_version_label, filter_palette_items, fmt_price,
    fmt_time_cn_for, resolve_sidebar_dock_notice_for,
};
use crate::palette::{
    build_palette_items_from_store, command_palette_overlay, execute_palette_action,
};
use crate::platform_desktop::{
    bind_live_application_menu, main_window_attributes, restart_after_update,
    show_company_context_menu, titlebar_left_inset,
};
use crate::stores::{AppStore, CompanyViewTab, WorkspaceTab};
use crate::theme::*;
use crate::views::{
    baskets::baskets_view, company::company_view, inbox::inbox_view, industry::industry_view,
    journal::journal_view, settings::settings_view, universe::universe_view,
};
use m3_core::i18n::UiLang;
use m3_core::lake_l1::SignalStatus;
use m3_core::model::Bucket;
use m3_core::update;
use nsh_core::desktop::WindowKind;
use nsh_core::element::desktop::{on_reopen, window};
use nsh_core::keymap::When;
use nsh_core::prelude::*;

/// Mounts the M3 reactive shell inside NSH 0.7.0's declarative `window()`, configuring desktop
/// chrome, live menus, Dock reopen behavior, and global keyboard shortcuts.
pub fn render_app_root() -> impl IntoElement {
    let store = AppStore::start();
    provide_context(store);
    bind_live_application_menu(store);
    register_keyboard_shortcuts(store);

    let main_open = signal(true);
    on_reopen(move || {
        if !main_open.get_untracked() {
            main_open.set(true);
        }
    });

    window()
        .id("main")
        .kind(WindowKind::Normal)
        .title("M3")
        .default_size(size(px(1380.0), px(880.0)))
        .restore_bounds(true)
        .attributes(main_window_attributes())
        .open(main_open)
        .on_open_change(move |e| main_open.set(e.open))
        .content(move || {
            let root = view()
                .relative()
                .w_full()
                .h_full()
                .flex()
                .flex_row()
                .bg(rgb(BG_SHELL))
                .text_color(rgb(INK_PRIMARY))
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .child(render_sidebar(store))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .pt(px(6.0))
                        .pr(px(6.0))
                        .pb(px(6.0))
                        .bg(rgb(BG_SHELL))
                        .child(
                            view()
                                .w_full()
                                .h_full()
                                .rounded_lg()
                                .bg(rgb(BG_VIEWPORT))
                                .border_1()
                                .border_color(rgb(BORDER_VIEWPORT))
                                .overflow_hidden()
                                .flex()
                                .flex_col()
                                .child(render_topbar(store))
                                .child(render_main_viewport(store))
                                .child(render_statusline(store)),
                        ),
                )
                .child(when(
                    move || store.palette_open.get(),
                    move || command_palette_overlay(store),
                ));

            if os().is_desktop() {
                root
            } else {
                root.p_safe_area()
            }
        })
}

fn render_main_viewport(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .child(dynamic(move || {
            if store.company_open.get() {
                return company_view(store).into_element();
            }
            match store.active_tab.get() {
                WorkspaceTab::Inbox => inbox_view(store).into_element(),
                WorkspaceTab::Universe => universe_view(store).into_element(),
                WorkspaceTab::Industry => industry_view(store).into_element(),
                WorkspaceTab::Baskets => baskets_view(store).into_element(),
                WorkspaceTab::Journal => journal_view(store).into_element(),
                WorkspaceTab::Settings => settings_view(store).into_element(),
            }
        }))
}

fn render_sidebar(store: AppStore) -> impl IntoElement {
    let nav_items = [
        (
            WorkspaceTab::Inbox,
            "简报收件箱",
            "Briefing Inbox",
            "⌘1",
            EMERALD_FG,
        ),
        (
            WorkspaceTab::Universe,
            "全市场雷达",
            "Universe Radar",
            "⌘2",
            SKY_FG,
        ),
        (
            WorkspaceTab::Industry,
            "行业研究",
            "Industry Research",
            "⌘3",
            VIOLET_FG,
        ),
        (
            WorkspaceTab::Baskets,
            "能力圈总览",
            "Circle",
            "⌘4",
            INK_HERO,
        ),
        (
            WorkspaceTab::Journal,
            "决策账本",
            "Decision Journal",
            "⌘5",
            GOLD_FG,
        ),
    ];

    view()
        .w(px(SIDEBAR_WIDTH))
        .flex_shrink_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(rgb(BG_SHELL))
        // Titlebar + traffic-light spacing + M3 pixel mark (triple-click reveals M³ engraving)
        .child({
            let mark_clicks = stored((0u8, None::<std::time::Instant>));
            view()
                .h(px(40.0))
                .flex_shrink_0()
                .pl(move || px(titlebar_left_inset()))
                .pr(px(12.0))
                .flex()
                .flex_row()
                .items_center()
                .window_drag(true)
                .child(
                    view()
                        .window_drag(false)
                        .cursor_pointer()
                        .px(px(4.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .on_press(move || {
                            let now = std::time::Instant::now();
                            let triggered = mark_clicks.with(|(count, last)| {
                                let within = last.is_some_and(|prev| {
                                    now.duration_since(prev)
                                        <= std::time::Duration::from_millis(1400)
                                });
                                *count = if within { count.saturating_add(1) } else { 1 };
                                *last = Some(now);
                                if *count >= 3 {
                                    *count = 0;
                                    true
                                } else {
                                    false
                                }
                            });
                            if triggered {
                                store.show_m3_engraving_notice();
                            }
                        })
                        .child(pixel_m3_mark(1.4)),
                )
        })
        // Command Palette trigger bar
        .child(
            view().px(px(10.0)).pb(px(8.0)).child(
                view()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded_md()
                    .bg(rgb(0x13131B))
                    .border_1()
                    .border_color(rgb(0x222230))
                    .hover(|s| s.bg(rgb(0x1A1A26)))
                    .active(|s| s.bg(rgb(BG_ACTIVE)))
                    .cursor_pointer()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .on_press(move || store.toggle_palette())
                    .child(
                        text(move || {
                            store
                                .lang
                                .get()
                                .t("> 跳转 / 命令…", "> Jump / Command…")
                                .to_owned()
                        })
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                    )
                    .child(kbd("⌘K")),
            ),
        )
        // 5-Tab Workspace Navigation
        .child(
            view()
                .px(px(10.0))
                .py(px(8.0))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .child(
                    view().px(px(8.0)).mb(px(4.0)).child(
                        text(move || store.lang.get().t("工作台", "WORKBENCH").to_owned())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED)),
                    ),
                )
                .children(
                    nav_items
                        .into_iter()
                        .map(|(tab, zh, en, shortcut, rail_col)| {
                            let is_selected = computed(move || {
                                !store.company_open.get()
                                    && store.active_tab.get() == tab
                                    && (tab != WorkspaceTab::Baskets
                                        || store.basket_filter.get().is_none())
                            });
                            view()
                                .px(px(8.0))
                                .py(px(6.0))
                                .mb(px(2.0))
                                .rounded_md()
                                .cursor_pointer()
                                .bg(move || {
                                    if is_selected.get() {
                                        rgba(RAIL_SELECTED)
                                    } else {
                                        rgba(0x00000000)
                                    }
                                })
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .on_press(move || {
                                    if tab == WorkspaceTab::Baskets {
                                        store.focus_basket(None);
                                    } else {
                                        store.open_tab(tab);
                                    }
                                })
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(8.0))
                                        .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(
                                            move || {
                                                rgb(if is_selected.get() {
                                                    rail_col
                                                } else {
                                                    BORDER_STRONG
                                                })
                                            },
                                        ))
                                        .child(
                                            text(move || store.lang.get().t(zh, en).to_owned())
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_SM))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || {
                                                    rgb(if is_selected.get() {
                                                        INK_HERO
                                                    } else {
                                                        INK_SECONDARY
                                                    })
                                                }),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(when(
                                            move || {
                                                tab == WorkspaceTab::Inbox
                                                    && store.inbox_summary.with(|s| {
                                                        s.as_ref()
                                                            .map(|x| x.unread_signals)
                                                            .unwrap_or(0)
                                                    }) > 0
                                            },
                                            move || {
                                                view()
                                                    .px(px(6.0))
                                                    .py(px(1.0))
                                                    .rounded_full()
                                                    .bg(rgba(EMERALD_BG))
                                                    .border_1()
                                                    .border_color(rgba(EMERALD_BORDER))
                                                    .child(
                                                        text(move || {
                                                            let cnt =
                                                                store.inbox_summary.with(|s| {
                                                                    s.as_ref()
                                                                        .map(|x| x.unread_signals)
                                                                        .unwrap_or(0)
                                                                });
                                                            format!("{cnt}")
                                                        })
                                                        .font_family(FONT_MONO_NUM)
                                                        .text_size(px(TEXT_2XS))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(rgb(EMERALD_FG)),
                                                    )
                                            },
                                        ))
                                        .child(kbd(shortcut)),
                                )
                        }),
                ),
        )
        // Circle Summary Tree (CORE / WATCH / OUT / TOO HARD)
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .px(px(10.0))
                .py(px(10.0))
                .child(
                    view()
                        .flex()
                        .flex_col()
                        .child(
                            view().px(px(8.0)).mb(px(6.0)).child(
                                text(move || store.lang.get().t("能力圈", "CIRCLE").to_owned())
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_MUTED)),
                            ),
                        )
                        .child(dynamic(move || {
                            let l = store.lang.get();
                            let reports = store.reports.get();
                            let active_tab = store.active_tab.get();
                            let company_open = store.company_open.get();
                            let basket_filter = store.basket_filter.get();
                            let selected_ticker = store.selected_ticker.get();

                            view().flex().flex_col().gap(px(8.0)).children(
                                Bucket::display_order().into_iter().map(|b| {
                                    let members: Vec<_> =
                                        reports.iter().filter(|r| r.bucket == b).collect();
                                    let cnt = members.len();
                                    let (fg, _, _) = bucket_colors(b);
                                    let bkt_active = !company_open
                                        && active_tab == WorkspaceTab::Baskets
                                        && basket_filter == Some(b);

                                    view()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            view()
                                                .px(px(8.0))
                                                .py(px(4.0))
                                                .mb(px(2.0))
                                                .rounded_md()
                                                .cursor_pointer()
                                                .bg(if bkt_active {
                                                    rgba(RAIL_SELECTED)
                                                } else {
                                                    rgba(0x00000000)
                                                })
                                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                                .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .justify_between()
                                                .on_press(move || {
                                                    if bkt_active {
                                                        store.focus_basket(None);
                                                    } else {
                                                        store.focus_basket(Some(b));
                                                    }
                                                })
                                                .child(
                                                    view()
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .gap(px(6.0))
                                                        .child(
                                                            view()
                                                                .w(px(6.0))
                                                                .h(px(6.0))
                                                                .rounded_full()
                                                                .bg(rgb(fg)),
                                                        )
                                                        .child(
                                                            text(b.code())
                                                                .font_family(FONT_MONO_TERM)
                                                                .text_size(px(TEXT_2XS))
                                                                .font_weight(FontWeight::BOLD)
                                                                .text_color(rgb(if bkt_active {
                                                                    fg
                                                                } else {
                                                                    INK_PRIMARY
                                                                })),
                                                        )
                                                        .child(
                                                            text(b.short_label_for(l))
                                                                .font_family(FONT_UI)
                                                                .text_size(px(TEXT_XS))
                                                                .text_color(rgb(if bkt_active {
                                                                    INK_HERO
                                                                } else {
                                                                    INK_MUTED
                                                                })),
                                                        ),
                                                )
                                                .child(
                                                    text(format!("{cnt}"))
                                                        .font_family(FONT_MONO_NUM)
                                                        .text_size(px(TEXT_2XS))
                                                        .text_color(rgb(if bkt_active {
                                                            INK_HERO
                                                        } else {
                                                            INK_MUTED
                                                        })),
                                                ),
                                        )
                                        .children(members.into_iter().map(|rep| {
                                            let is_sel =
                                                company_open && selected_ticker == rep.ticker;
                                            let dot_col = status_dot_color(rep.status);
                                            let ticker = rep.ticker.clone();
                                            let ticker_ctx = rep.ticker.clone();
                                            let price_str = fmt_price(
                                                rep.price.as_ref().map(|p| p.price),
                                                &rep.currency,
                                            );
                                            view()
                                                .pl(px(14.0))
                                                .pr(px(8.0))
                                                .py(px(4.0))
                                                .mb(px(2.0))
                                                .rounded_md()
                                                .cursor_pointer()
                                                .bg(if is_sel {
                                                    rgba(RAIL_SELECTED)
                                                } else {
                                                    rgba(0x00000000)
                                                })
                                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                                .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .justify_between()
                                                .on_press(move || {
                                                    store.open_company_contextual(&ticker);
                                                })
                                                .on_context_menu(move |e: &ContextMenuEvent| {
                                                    show_company_context_menu(
                                                        store,
                                                        ticker_ctx.clone(),
                                                        e.page_position,
                                                    );
                                                })
                                                .child(
                                                    view()
                                                        .flex()
                                                        .flex_row()
                                                        .items_center()
                                                        .gap(px(8.0))
                                                        .min_w(px(0.0))
                                                        .child(
                                                            view()
                                                                .w(px(5.0))
                                                                .h(px(5.0))
                                                                .rounded_full()
                                                                .bg(rgb(dot_col)),
                                                        )
                                                        .child(
                                                            text(rep.ticker.clone())
                                                                .font_family(FONT_MONO_NUM)
                                                                .text_size(px(TEXT_SM))
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(rgb(if is_sel {
                                                                    INK_HERO
                                                                } else {
                                                                    INK_PRIMARY
                                                                })),
                                                        ),
                                                )
                                                .child(
                                                    text(price_str)
                                                        .font_family(FONT_MONO_NUM)
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(rgb(if is_sel {
                                                            INK_SECONDARY
                                                        } else {
                                                            INK_RAIL_PRICE
                                                        })),
                                                )
                                        }))
                                }),
                            )
                        })),
                ),
        )
        // Bottom Settings Row (`⌘,`)
        .child(
            view()
                .px(px(10.0))
                .py(px(6.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .child({
                    let active = computed(move || {
                        !store.company_open.get()
                            && store.active_tab.get() == WorkspaceTab::Settings
                    });
                    view()
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded_md()
                        .cursor_pointer()
                        .bg(move || {
                            if active.get() {
                                rgba(RAIL_SELECTED)
                            } else {
                                rgba(0x00000000)
                            }
                        })
                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                        .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .on_press(move || store.open_settings())
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(move || {
                                    rgb(if active.get() {
                                        INK_HERO
                                    } else {
                                        BORDER_STRONG
                                    })
                                }))
                                .child(
                                    text(move || store.lang.get().t("设置", "Settings").to_owned())
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_SM))
                                        .text_color(move || {
                                            rgb(if active.get() {
                                                INK_HERO
                                            } else {
                                                INK_SECONDARY
                                            })
                                        }),
                                ),
                        )
                        .child(kbd("⌘,"))
                }),
        )
        // Bottom Status Dock
        .child(render_sidebar_status_dock(store))
}

fn render_sidebar_status_dock(store: AppStore) -> impl IntoElement {
    view()
        .flex_shrink_0()
        .px(px(10.0))
        .pt(px(8.0))
        .pb(px(10.0))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .bg(rgb(BG_SHELL))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(dynamic(move || {
            let notice = store.notice.get();
            let u_sync = store.universe_syncing.get();
            let u_prog = store.universe_sync_progress.get();
            let i_catch = store.inbox_catching_up.get();
            let i_prog = store.inbox_catchup_progress.get();
            let dr_run = store.deep_research_running.get();
            let ind_run = store.industry_research_running.get();
            let ind_prog = store.industry_research_progress.get();
            let syncing = store.sync_running.get();
            let t_sync = store.themes_syncing.get();
            let upd_chk = store.update_checking.get();
            let upd_app = store.update_applying.get();
            let upd_prog = store.update_progress.get();
            let upd_detail = store.update_progress_detail.get();
            let l = store.lang.get();

            let Some(mut n) = resolve_sidebar_dock_notice_for(
                notice.as_ref(),
                u_sync,
                u_prog.as_deref(),
                i_catch,
                i_prog.as_deref(),
                dr_run,
                ind_run,
                ind_prog.as_deref(),
                syncing,
                t_sync,
                upd_chk,
                upd_app,
                l,
            ) else {
                return view().into_element();
            };

            if upd_app && let Some(prog_msg) = upd_prog {
                n.message = prog_msg;
            }
            let upd_frac = upd_app
                .then(|| upd_detail.as_ref().map(|d| d.fraction))
                .flatten();

            let mut hdr = view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            view()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded_full()
                                .bg(rgb(n.dot_color)),
                        )
                        .child(
                            text(n.tag)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(n.dot_color)),
                        ),
                );
            if let Some(frac) = upd_frac {
                hdr = hdr.child(
                    text(format!("{}%", (frac * 100.0).round() as u32))
                        .font_family(FONT_MONO_NUM)
                        .text_size(px(TEXT_2XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(n.dot_color)),
                );
            }
            if n.dismissible {
                hdr = hdr.child(
                    view()
                        .px(px(4.0))
                        .rounded_xs()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                        .on_press(move || store.clear_notice())
                        .child(
                            text("×")
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(INK_MUTED)),
                        ),
                );
            }

            let mut card = view()
                .rounded_md()
                .bg(rgb(BG_SURFACE))
                .border_1()
                .border_color(rgba(n.border_color))
                .p(px(10.0))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(hdr)
                .child(
                    text(n.message)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .line_height(px(16.0))
                        .text_color(rgb(INK_PRIMARY)),
                );
            if let Some(frac) = upd_frac {
                card = card.child(progress_bar(frac, n.dot_color));
            }
            card.into_element()
        }))
        .child(dynamic(move || {
            let l = store.lang.get();
            let total_companies = store
                .universe_summary
                .get()
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
            let budget_rep = store.budget_report.get();
            let (budget_text, budget_col) = budget_watermark_summary_for(budget_rep.as_ref(), l);
            let budget_compact = budget_text
                .strip_prefix("本月 AI 预算 ")
                .or_else(|| budget_text.strip_prefix("AI Budget "))
                .unwrap_or(&budget_text)
                .to_owned();

            let version_badge = if let Some(ver) = store.update_applied_version.get() {
                let short_ver = compact_sidebar_version_label(&ver);
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded_xs()
                    .cursor_pointer()
                    .bg(rgba(EMERALD_BG))
                    .border_1()
                    .border_color(rgba(EMERALD_BORDER))
                    .on_press(move || restart_after_update(store))
                    .child(
                        text(format!("{} {short_ver}", l.t("重启", "Restart")))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(EMERALD_FG))
                            .truncate()
                            .number_of_lines(1),
                    )
            } else if let Some(notice) = store.update_notice.get() {
                let short_ver = compact_sidebar_version_label(&notice.latest_version);
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded_xs()
                    .cursor_pointer()
                    .bg(rgba(SKY_BG))
                    .border_1()
                    .border_color(rgba(SKY_BORDER))
                    .on_press(move || store.start_update_apply())
                    .child(
                        text(format!("↑ {short_ver}"))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(SKY_FG))
                            .truncate()
                            .number_of_lines(1),
                    )
            } else {
                let short_ver = compact_sidebar_version_label(update::current_binary_version());
                let checking = store.update_checking.get();
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .py(px(2.0))
                    .rounded_xs()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(RAIL_HOVER)))
                    .on_press(move || store.start_update_check(true))
                    .child(
                        text(if checking {
                            l.t("检查更新中…", "Checking…").to_owned()
                        } else {
                            short_ver
                        })
                        .font_family(FONT_MONO_NUM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED))
                        .truncate()
                        .number_of_lines(1),
                    )
            };

            let channel_label = store.update_channel.get().as_str().to_owned();

            view()
                .px(px(4.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(rgb(
                                    if total_companies > 0 {
                                        EMERALD_FG
                                    } else {
                                        INK_MUTED
                                    },
                                )))
                                .child(
                                    text(lake_label)
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(INK_SECONDARY))
                                        .truncate()
                                        .number_of_lines(1),
                                ),
                        )
                        .child(
                            text(budget_compact)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(budget_col)),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(version_badge)
                        .child(
                            text(channel_label)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                )
        }))
}

fn render_topbar(store: AppStore) -> impl IntoElement {
    let breadcrumb = computed(move || {
        let l = store.lang.get();
        if store.company_open.get() {
            let ticker = store.selected_ticker.get();
            if ticker.is_empty() {
                return "Research Desk".to_owned();
            }
            let name = store.reports.with(|reps| {
                reps.iter()
                    .find(|r| r.ticker == ticker)
                    .map(|r| r.name.clone())
                    .unwrap_or_default()
            });
            return format!("{ticker} · {name} · Research Desk");
        }
        match store.active_tab.get() {
            WorkspaceTab::Inbox => l
                .t(
                    "简报收件箱 · Event-Driven L1",
                    "Briefing Inbox · Event-Driven L1",
                )
                .to_owned(),
            WorkspaceTab::Universe => l
                .t(
                    "全市场雷达 · US Lakehouse L0/L1",
                    "Universe Radar · US Lakehouse L0/L1",
                )
                .to_owned(),
            WorkspaceTab::Industry => {
                let slug = store.selected_industry_slug.get();
                let ind = slug.as_deref().and_then(|s| {
                    store
                        .industries
                        .with(|list| list.iter().find(|p| p.slug == s).cloned())
                });
                if let Some(p) = ind {
                    format!(
                        "{} · {} ({})",
                        l.t("行业研究", "Industry Research"),
                        p.title,
                        p.status.label_for(l)
                    )
                } else {
                    l.t(
                        "行业研究工作台 · Alan Deep Research",
                        "Industry Research Studio · Alan Deep Research",
                    )
                    .to_owned()
                }
            }
            WorkspaceTab::Baskets => match (store.basket_filter.get(), store.circle_filter.get()) {
                (Some(b), Some(tag)) => format!(
                    "{} / {} ({}) / #{tag}",
                    l.t("能力圈总览", "Circle"),
                    b.code(),
                    b.short_label_for(l)
                ),
                (Some(b), None) => format!(
                    "{} / {} ({})",
                    l.t("能力圈总览", "Circle"),
                    b.code(),
                    b.short_label_for(l)
                ),
                (None, Some(tag)) => format!("{} / #{tag}", l.t("能力圈总览", "Circle")),
                (None, None) => l.t("能力圈总览", "Circle Overview").to_owned(),
            },
            WorkspaceTab::Journal => l
                .t("决策账本 · Decision Journal", "Decision Journal")
                .to_owned(),
            WorkspaceTab::Settings => l
                .t(
                    "设置 · Preferences & Guardrails",
                    "Settings · Preferences & Guardrails",
                )
                .to_owned(),
        }
    });

    view()
        .h(px(TOOLBAR_HEIGHT))
        .flex_shrink_0()
        .px(px(16.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .bg(rgb(BG_TOOLBAR))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(when(
                    move || store.company_open.get(),
                    move || {
                        view()
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_SUBTLE))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .active(|s| s.bg(rgb(BG_ACTIVE)))
                            .cursor_pointer()
                            .on_press(move || store.close_company_overlay())
                            .child(
                                text("←")
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_SM))
                                    .text_color(rgb(INK_SECONDARY)),
                            )
                    },
                ))
                .child(
                    text(move || breadcrumb.get())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_PRIMARY)),
                ),
        )
        // Window drag spacer
        .child(view().flex_1().h_full().window_drag(true))
        // Right telemetry & Sync controls
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(dynamic(move || {
                    let l = store.lang.get();
                    if let Some(ver) = store.update_applied_version.get() {
                        return view()
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgba(EMERALD_BG))
                            .border_1()
                            .border_color(rgba(EMERALD_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .on_press(move || restart_after_update(store))
                            .child(
                                text(format!(
                                    "{} v{ver} · {}",
                                    l.t("已更新", "Updated"),
                                    l.t("重启", "Restart")
                                ))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(EMERALD_FG)),
                            )
                            .into_element();
                    }
                    if store.update_applying.get() {
                        return view()
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_SUBTLE))
                            .child(
                                text(l.t("更新中…", "Updating…").to_owned())
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            )
                            .into_element();
                    }
                    if let Some(notice) = store.update_notice.get() {
                        return view()
                            .px(px(10.0))
                            .py(px(4.0))
                            .rounded_sm()
                            .cursor_pointer()
                            .bg(rgba(SKY_BG))
                            .border_1()
                            .border_color(rgba(SKY_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .on_press(move || store.start_update_apply())
                            .child(
                                text(format!("↑ v{}", notice.latest_version))
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(SKY_FG)),
                            )
                            .into_element();
                    }
                    view().into_element()
                }))
                .child(
                    view()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded_sm()
                        .bg(rgb(BG_SURFACE))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            text("S&P 500")
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        )
                        .child(
                            text(move || {
                                store
                                    .benchmark
                                    .get()
                                    .map(|b| format!("{:.2}", b.price))
                                    .unwrap_or_else(|| "—".to_owned())
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_PRIMARY)),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(8.0))
                        .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(move || {
                            rgb(if store.sync_status.get().is_some() {
                                EMERALD_FG
                            } else {
                                INK_MUTED
                            })
                        }))
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                match store.sync_status.get() {
                                    Some(s) => fmt_time_cn_for(Some(s.synced_at_ms), l),
                                    None => l.t("未同步", "Unsynced").to_owned(),
                                }
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(
                    view()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .border_1()
                        .border_color(move || {
                            rgb(if store.sync_running.get() {
                                BORDER_SUBTLE
                            } else {
                                BORDER_PILL
                            })
                        })
                        .bg(rgb(BG_WELL))
                        .hover(|s| s.bg(rgb(0x21212F)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .on_press(move || store.start_sync())
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                if store.sync_running.get() {
                                    l.t("同步中…", "Syncing…").to_owned()
                                } else {
                                    l.t("同步", "Sync").to_owned()
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                rgb(if store.sync_running.get() {
                                    INK_MUTED
                                } else {
                                    INK_PRIMARY
                                })
                            }),
                        )
                        .child(kbd("⌘R")),
                ),
        )
}

fn render_statusline(store: AppStore) -> impl IntoElement {
    let status_meta = computed(move || {
        let l = store.lang.get();
        if store.palette_open.get() {
            return (
                SKY_FG,
                l.t(
                    "↑/↓ 选择 · ↵ 执行 · esc 关闭命令面板",
                    "↑/↓ select · ↵ run · esc close palette",
                ),
                "PALETTE",
            );
        }
        if store.company_open.get() {
            let is_alan = store.company_view_tab.get() == CompanyViewTab::DeepResearchAlan;
            return (
                if is_alan { VIOLET_FG } else { EMERALD_FG },
                l.t(
                    "1 财报与活研报 · 2 Deep Research & Alan · 3 XBRL · t TTM/FY · j/k 换股 · esc 返回",
                    "1 Financials & Memo · 2 Deep Research & Alan · 3 XBRL · t TTM/FY · j/k switch · esc back",
                ),
                if is_alan {
                    "EMBEDDED ALAN · PAPER ONLY"
                } else {
                    "ZERO-LLM · PAPER ONLY"
                },
            );
        }
        match store.active_tab.get() {
            WorkspaceTab::Inbox => (
                EMERALD_FG,
                l.t(
                    "j/k 移动 · ↵ 打开 · w 观察 · x TooHard · e 归档 · u 未读 · d Alan · r 追平 · ⌘K 命令",
                    "j/k move · ↵ open · w watch · x too hard · e archive · u unread · d Alan · r catch-up · ⌘K commands",
                ),
                "L0/L1 · 0 FLAGSHIP TOKEN",
            ),
            WorkspaceTab::Universe => (
                SKY_FG,
                l.t(
                    "/ 搜索 · j/k 选择 · ↵ 打开已入池研报 · r 刷新 · ⌘K 命令 · esc 返回",
                    "/ search · j/k select · ↵ open dossier · r refresh · ⌘K palette · esc back",
                ),
                "DUCKDB L0/L1 · ZERO-LLM",
            ),
            WorkspaceTab::Industry => (
                VIOLET_FG,
                l.t(
                    "新建行业 / 选中代表公司 / Alan 4 视角深研 + Web 检索 + 费曼通关进能力圈 · s 同步 SIC 聚类 · ⌘K 命令",
                    "New Study / Bound Peers / Alan 4-Lens + Web Search + Feynman Graduation · s sync SIC clusters · ⌘K commands",
                ),
                "EMBEDDED ALAN · PAPER ONLY",
            ),
            WorkspaceTab::Baskets => (
                EMERALD_FG,
                l.t(
                    "v 列表/看板 · 1-4 切分组 · 0 全部 · j/k 移动 · ↵ 打开 · ⌘K 命令",
                    "v list/board · 1-4 tier · 0 all · j/k move · ↵ open · ⌘K commands",
                ),
                "ZERO-LLM · PAPER ONLY",
            ),
            WorkspaceTab::Journal => (
                GOLD_FG,
                l.t(
                    "/ 或 ⌘K 命令面板 · esc 返回总览",
                    "/ or ⌘K palette · esc back to Circle",
                ),
                "ZERO-LLM · PAPER ONLY",
            ),
            WorkspaceTab::Settings => (
                SKY_FG,
                l.t("⌘K 命令面板 · esc 返回", "⌘K palette · esc back"),
                "ZERO-LLM · PAPER ONLY",
            ),
        }
    });

    view()
        .h(px(STATUSLINE_HEIGHT))
        .flex_shrink_0()
        .px(px(16.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .bg(rgb(BG_STATUSLINE))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .gap(px(16.0))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    view()
                        .w(px(5.0))
                        .h(px(5.0))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(move || rgb(status_meta.get().0)),
                )
                .child(
                    text(move || status_meta.get().1.to_owned())
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED))
                        .truncate()
                        .number_of_lines(1),
                ),
        )
        .child(
            text(move || status_meta.get().2.to_owned())
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED)),
        )
}

fn register_keyboard_shortcuts(store: AppStore) {
    // Global Cmd/Mod shortcuts (safe across Input focus via When::Auto; never bind reserved keys!)
    on_key("mod+1", move |_| store.open_tab(WorkspaceTab::Inbox));
    on_key("mod+2", move |_| store.open_tab(WorkspaceTab::Universe));
    on_key("mod+3", move |_| store.open_tab(WorkspaceTab::Industry));
    on_key("mod+4", move |_| store.focus_basket(None));
    on_key("mod+5", move |_| store.open_tab(WorkspaceTab::Journal));
    on_key("mod+,", move |_| store.open_settings());
    on_key("mod+k", move |_| store.toggle_palette());
    on_key("mod+r", move |_| store.start_sync());
    on_key("mod+shift+r", move |_| store.trigger_l1_catchup());
    on_key("mod+[", move |_| store.close_company_overlay());

    // Escape closes palette even when its native Input has focus (IME composition still takes priority)
    on_key_with(
        "escape",
        KeyOptions {
            when: When::Always,
            repeat: false,
            pass_through: false,
            ..Default::default()
        },
        move |_| {
            if store.palette_open.get_untracked() {
                store.close_palette();
                return;
            }
            if store.company_open.get_untracked() {
                store.close_company_overlay();
                return;
            }
            match store.active_tab.get_untracked() {
                WorkspaceTab::Inbox => {
                    let cur = store.inbox_state.get_untracked().filter;
                    if cur != crate::stores::InboxFilter::Unread {
                        store.set_inbox_filter(crate::stores::InboxFilter::Unread);
                    }
                }
                WorkspaceTab::Baskets => {
                    if store.basket_filter.get_untracked().is_some()
                        || store.circle_filter.get_untracked().is_some()
                        || store.status_filter.get_untracked()
                            != crate::stores::StatusQuickFilter::All
                    {
                        store.basket_filter.set(None);
                        store.circle_filter.set(None);
                        store
                            .status_filter
                            .set(crate::stores::StatusQuickFilter::All);
                        store.baskets_cursor_index.set(0);
                    }
                }
                WorkspaceTab::Universe => {
                    let q = store.universe_state.get_untracked().query;
                    if !q.is_empty() {
                        store.set_universe_query(String::new());
                    } else {
                        store.open_tab(WorkspaceTab::Baskets);
                    }
                }
                WorkspaceTab::Industry | WorkspaceTab::Settings => {
                    store.open_tab(WorkspaceTab::Inbox);
                }
                WorkspaceTab::Journal => {
                    store.open_tab(WorkspaceTab::Baskets);
                }
            }
        },
    );

    // Palette arrow/enter navigation while palette is open
    for (key, delta) in [("up", -1isize), ("down", 1isize)] {
        on_key_with(
            key,
            KeyOptions {
                when: When::Always,
                repeat: true,
                pass_through: false,
                ..Default::default()
            },
            move |_| {
                if store.palette_open.get_untracked() {
                    let all = build_palette_items_from_store(store);
                    let q = store.palette_query.get_untracked();
                    let len = filter_palette_items(&all, &q).len();
                    if len > 0 {
                        store.palette_cursor.update(|c| {
                            *c = (*c as isize + delta).rem_euclid(len as isize) as usize;
                        });
                    }
                    return;
                }
                if !store.company_open.get_untracked() {
                    match store.active_tab.get_untracked() {
                        WorkspaceTab::Inbox => store.step_inbox_cursor(delta),
                        WorkspaceTab::Universe => store.step_universe_cursor(delta),
                        WorkspaceTab::Baskets => {
                            let len = store.visible_linear_tickers().len();
                            if len > 0 {
                                store.baskets_cursor_index.update(|c| {
                                    *c =
                                        (*c as isize + delta).clamp(0, (len - 1) as isize) as usize;
                                });
                            }
                        }
                        _ => {}
                    }
                }
            },
        );
    }

    // Single-key triage & navigation shortcuts strictly gated with When::NotEditing (IME-safe!)
    let not_editing = KeyOptions {
        when: When::NotEditing,
        repeat: false,
        pass_through: false,
        ..Default::default()
    };

    on_key_with("j", not_editing, move |_| {
        if store.company_open.get_untracked() {
            store.step_selected_company(1);
            return;
        }
        match store.active_tab.get_untracked() {
            WorkspaceTab::Inbox => store.step_inbox_cursor(1),
            WorkspaceTab::Universe => store.step_universe_cursor(1),
            WorkspaceTab::Baskets => {
                let len = store.visible_linear_tickers().len();
                if len > 0 {
                    store
                        .baskets_cursor_index
                        .update(|c| *c = (*c + 1).min(len - 1));
                }
            }
            _ => {}
        }
    });

    on_key_with("k", not_editing, move |_| {
        if store.company_open.get_untracked() {
            store.step_selected_company(-1);
            return;
        }
        match store.active_tab.get_untracked() {
            WorkspaceTab::Inbox => store.step_inbox_cursor(-1),
            WorkspaceTab::Universe => store.step_universe_cursor(-1),
            WorkspaceTab::Baskets => {
                store
                    .baskets_cursor_index
                    .update(|c| *c = c.saturating_sub(1));
            }
            _ => {}
        }
    });

    on_key_with("w", not_editing, move |_| {
        if !store.company_open.get_untracked()
            && store.active_tab.get_untracked() == WorkspaceTab::Inbox
            && let Some(sig) = store.active_inbox_signal()
        {
            store.triage_inbox_signal(&sig.signal_id, SignalStatus::Watch, true, false);
        }
    });

    on_key_with("x", not_editing, move |_| {
        if !store.company_open.get_untracked()
            && store.active_tab.get_untracked() == WorkspaceTab::Inbox
            && let Some(sig) = store.active_inbox_signal()
        {
            store.triage_inbox_signal(&sig.signal_id, SignalStatus::TooHard, false, false);
        }
    });

    on_key_with("e", not_editing, move |_| {
        if !store.company_open.get_untracked()
            && store.active_tab.get_untracked() == WorkspaceTab::Inbox
            && let Some(sig) = store.active_inbox_signal()
        {
            store.triage_inbox_signal(&sig.signal_id, SignalStatus::Archived, false, false);
        }
    });

    on_key_with("u", not_editing, move |_| {
        if !store.company_open.get_untracked()
            && store.active_tab.get_untracked() == WorkspaceTab::Inbox
        {
            if let Some(sig) = store.active_inbox_signal() {
                store.triage_inbox_signal(&sig.signal_id, SignalStatus::Unread, false, false);
            }
        } else {
            store.open_tab(WorkspaceTab::Universe);
        }
    });

    on_key_with("d", not_editing, move |_| {
        if store.company_open.get_untracked() {
            store.company_view_tab.set(CompanyViewTab::FinancialsMemo);
            return;
        }
        if store.active_tab.get_untracked() == WorkspaceTab::Inbox
            && let Some(sig) = store.active_inbox_signal()
        {
            store.triage_inbox_signal(&sig.signal_id, SignalStatus::Researching, false, true);
        }
    });

    on_key_with("r", not_editing, move |_| {
        if store.company_open.get_untracked() {
            return;
        }
        match store.active_tab.get_untracked() {
            WorkspaceTab::Inbox => store.start_l1_catchup(false),
            WorkspaceTab::Universe => store.reload_universe_from_lake(),
            _ => {}
        }
    });

    on_key_with("enter", not_editing, move |_| {
        if store.palette_open.get_untracked() {
            let all = build_palette_items_from_store(store);
            let q = store.palette_query.get_untracked();
            let filtered = filter_palette_items(&all, &q);
            let idx = store
                .palette_cursor
                .get_untracked()
                .min(filtered.len().saturating_sub(1));
            if let Some(item) = filtered.get(idx) {
                execute_palette_action(store, item.action.clone());
            }
            return;
        }
        if store.company_open.get_untracked() {
            return;
        }
        match store.active_tab.get_untracked() {
            WorkspaceTab::Inbox => {
                if let Some(sig) = store.active_inbox_signal() {
                    store.open_inbox_signal_target(&sig.primary_ticker);
                }
            }
            WorkspaceTab::Baskets => {
                let tickers = store.visible_linear_tickers();
                let idx = store.baskets_cursor_index.get_untracked();
                if let Some(t) = tickers.get(idx) {
                    store.open_company(t, CompanyViewTab::FinancialsMemo);
                }
            }
            WorkspaceTab::Universe => {
                if let Some(row) = store.universe_selected_row.get_untracked() {
                    let exists = store.reports.with_untracked(|reps| {
                        reps.iter()
                            .any(|r| r.ticker.eq_ignore_ascii_case(&row.ticker))
                    });
                    if exists {
                        store.open_company(&row.ticker, CompanyViewTab::FinancialsMemo);
                    }
                }
            }
            _ => {}
        }
    });

    on_key_with("/", not_editing, move |_| {
        store.toggle_palette();
    });

    on_key_with("v", not_editing, move |_| {
        if !store.company_open.get_untracked()
            && store.active_tab.get_untracked() == WorkspaceTab::Baskets
        {
            store.toggle_baskets_view();
        }
    });

    on_key_with("s", not_editing, move |_| {
        if !store.company_open.get_untracked()
            && store.active_tab.get_untracked() == WorkspaceTab::Industry
        {
            store.start_themes_sync();
        }
    });

    on_key_with("t", not_editing, move |_| {
        if store.company_open.get_untracked() {
            store.toggle_basis();
        }
    });

    on_key_with("i", not_editing, move |_| {
        if store.company_open.get_untracked() {
            store.toggle_inspector();
        }
    });

    on_key_with("a", not_editing, move |_| {
        if store.company_open.get_untracked() {
            store.company_view_tab.set(CompanyViewTab::DeepResearchAlan);
        } else if store.active_tab.get_untracked() == WorkspaceTab::Baskets {
            let tickers = store.visible_linear_tickers();
            let idx = store.baskets_cursor_index.get_untracked();
            if let Some(t) = tickers.get(idx) {
                store.open_company(t, CompanyViewTab::DeepResearchAlan);
            }
        }
    });
}
