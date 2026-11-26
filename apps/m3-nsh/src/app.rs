//! Root M3 shell: Antigravity-style unified topbar, Message Box, collapsible sidebar, shortcuts, and viewport.

use crate::components::{
    action_button, animate_shell_spring, badge_pill, cmd_kbd_bottom_corner, cmd_kbd_right_overlay,
    icon_button, pixel_m3_mark, progress_bar,
};
use crate::format::{
    TopbarMessageInputs, budget_watermark_summary_for, build_topbar_messages,
    compact_sidebar_version_label, fmt_price, fmt_time_cn_for,
};
use crate::icons::{
    ICON_ALERT, ICON_ARROW_CIRCLE_UP, ICON_ARROW_LEFT, ICON_BELL, ICON_CIRCLE_CHECK, ICON_CLOSE,
    ICON_INBOX, ICON_JOURNAL, ICON_LAYERS, ICON_RADAR, ICON_REFRESH, ICON_SETTINGS,
    ICON_SIDEBAR_LEFT, ICON_TARGET, icon,
};
use crate::palette::{
    build_palette_items_from_store, execute_palette_action, morphing_topbar_command_box,
    visible_palette_items_for,
};
use crate::platform_desktop::{
    bind_live_application_menu, bind_live_system_tray, install_smoked_black_glass_backdrop,
    main_window_attributes, restart_after_update, set_macos_dock_visible,
    show_company_context_menu, titlebar_left_inset,
};
use crate::stores::{
    AppStore, CompanyViewTab, TopbarMessage, TopbarMessageAction, TopbarMessageStatus, WorkspaceTab,
};
use crate::theme::*;
use crate::views::{
    baskets::baskets_view,
    company::company_view,
    inbox::inbox_view,
    industry::industry_view,
    journal::journal_view,
    settings::{settings_modal_overlay, settings_view},
    universe::universe_view,
};
use m3_core::i18n::UiLang;
use m3_core::lake_l1::SignalStatus;
use m3_core::model::Bucket;
use m3_core::update;
use nsh_core::desktop::WindowKind;
use nsh_core::element::desktop::{current_window, on_reopen, window};
use nsh_core::keymap::When;
use nsh_core::platform::Os;
use nsh_core::prelude::*;

fn shell_root_bg() -> Rgba {
    if os() == Os::MacOs {
        rgba(BG_SHELL_GLASS)
    } else {
        rgb(BG_SHELL)
    }
}

fn topbar_breadcrumb(store: AppStore) -> Memo<String> {
    computed(move || {
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
            WorkspaceTab::Settings => format!(
                "{} · {}",
                l.t("设置", "Settings"),
                store.settings_tab.get().label_for(l)
            ),
        }
    })
}

/// Mounts the M3 reactive shell inside NSH 0.7.0's declarative `window()`, configuring desktop
/// chrome, live menus, Dock reopen behavior, translucent dark glass, and global shortcuts.
pub fn render_app_root() -> impl IntoElement {
    // NSH 0.7.0 resets declarative `window()` appearance from `surface.root.ns().appearance()`
    // (`None`) when `Appearance::Dark` is only set in `AppOptions`; `set_color_scheme` updates
    // both `surface.root` and opened windows to `NSAppearanceNameDarkAqua`.
    set_color_scheme(Appearance::Dark);
    let store = AppStore::start();
    provide_context(store);
    bind_live_application_menu(store);
    register_keyboard_shortcuts(store);

    let main_open = signal(true);
    bind_live_system_tray(store, main_open);
    effect(move || {
        set_macos_dock_visible(main_open.get());
    });
    on_reopen(move || {
        set_macos_dock_visible(true);
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
            let main_win = current_window();
            main_win.on_focus_change(move |focused| {
                if !focused {
                    store.cmd_held.set(false);
                } else {
                    install_smoked_black_glass_backdrop();
                }
            });
            spawn(async move {
                let _ = next_frame().await;
                install_smoked_black_glass_backdrop();
            });

            let breadcrumb = topbar_breadcrumb(store);

            let root = view()
                .relative()
                .w_full()
                .h_full()
                .flex()
                .flex_col()
                .bg(shell_root_bg())
                .text_color(rgb(INK_PRIMARY))
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .on_key_down(move |e: &KeyEvent| {
                    if e.key.as_str() == "Meta" || e.modifiers.cmd {
                        store.cmd_held.set(true);
                    }
                })
                .on_key_up(move |e: &KeyEvent| {
                    if e.key.as_str() == "Meta" || !e.modifiers.cmd {
                        store.cmd_held.set(false);
                    }
                })
                .child(render_unified_topbar(store))
                .child(
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .w_full()
                        .flex()
                        .flex_row()
                        .child(when(
                            move || !store.sidebar_collapsed.get(),
                            move || render_sidebar(store),
                        ))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .h_full()
                                .pl(move || {
                                    px(if store.sidebar_collapsed.get() {
                                        8.0
                                    } else {
                                        0.0
                                    })
                                })
                                .pr(px(8.0))
                                .pb(px(8.0))
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
                                        .child(render_main_viewport(store))
                                        .child(render_statusline(store)),
                                ),
                        ),
                )
                .child(when(
                    move || store.message_box_open.get(),
                    move || render_message_box_popover(store),
                ))
                .child(when(
                    move || store.settings_open.get(),
                    move || settings_modal_overlay(store),
                ))
                .child(when(
                    move || store.pending_graduation_ticker.get().is_some(),
                    move || render_graduation_soft_gate_modal(store),
                ))
                .child(morphing_topbar_command_box(store, breadcrumb));

            if os().is_desktop() {
                root
            } else {
                root.p_safe_area()
            }
        })
}

/// Renders the calm WATCH -> IN Feynman Graduation Soft Gate modal overlay.
fn render_graduation_soft_gate_modal(store: AppStore) -> impl IntoElement {
    overlay()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.0))
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(rgba(SCRIM_BACKDROP))
                .on_press(move || {
                    animate_shell_spring();
                    store.cancel_graduation_gate();
                }),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let ticker = store
                .pending_graduation_ticker
                .get()
                .unwrap_or_default();
            let t_feynman = ticker.clone();
            let t_direct = ticker.clone();

            view()
                .w(px(520.0))
                .max_w(percentage(92.0))
                .rounded_xl()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgba(BORDER_GLASS_STRONG))
                .shadow_xl()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    view()
                        .px(px(20.0))
                        .py(px(14.0))
                        .bg(rgb(BG_TOOLBAR))
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(10.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(icon(ICON_ALERT, 16.0).text_color(rgb(GOLD_FG)))
                                .child(
                                    text(match l {
                                        UiLang::Zh => format!("毕业前置检查 · {ticker}"),
                                        UiLang::En => format!("Graduation Check · {ticker}"),
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_MD))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(badge_pill("WATCH → IN", GOLD_FG, GOLD_BG, GOLD_BORDER)),
                        )
                        .child(icon_button(ICON_CLOSE, 14.0, move || {
                            animate_shell_spring();
                            store.cancel_graduation_gate();
                        })),
                )
                .child(
                    view()
                        .px(px(20.0))
                        .py(px(18.0))
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .child(
                            text(match l {
                                UiLang::Zh => format!(
                                    "{ticker} 尚未完成 3 轮费曼答辩签字。Munger 纪律建议先通过「护城河、五年死法与可证伪指标、安全边际」三轮反刍，再移入核心击球区 (IN / CORE)。"
                                ),
                                UiLang::En => format!(
                                    "{ticker} has not completed a 3-round Feynman Defense sign-off yet. Before moving it into Core Circle (IN / CORE), a 3-round Socratic defense and testable falsification criteria are recommended."
                                ),
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .line_height(px(19.0))
                            .text_color(rgb(INK_PRIMARY)),
                        )
                        .child(
                            view()
                                .px(px(12.0))
                                .py(px(10.0))
                                .rounded_md()
                                .bg(rgb(BG_SURFACE))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .child(
                                    text(l.t(
                                        "主权说明：这是冷静的软提醒而非硬锁定，最终能力圈归属始终由你决定。",
                                        "Human Sovereignty: this is a calm soft gate, never a hard lock — your Circle tier remains solely your decision.",
                                    ))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .line_height(px(16.0))
                                    .text_color(rgb(INK_MUTED)),
                                ),
                        ),
                )
                .child(
                    view()
                        .px(px(20.0))
                        .py(px(14.0))
                        .bg(rgb(BG_TOOLBAR))
                        .border_t_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(action_button(
                            l.t("取消", "Cancel"),
                            false,
                            move || {
                                animate_shell_spring();
                                store.cancel_graduation_gate();
                            },
                        ))
                        .child(action_button(
                            l.t("已知风险，仍直接移入 IN", "Move to IN Anyway"),
                            false,
                            move || {
                                animate_shell_spring();
                                store.confirm_direct_graduation(&t_direct);
                            },
                        ))
                        .child(action_button(
                            l.t("开始 3 轮费曼答辩", "Start 3-Round Feynman Defense"),
                            true,
                            move || {
                                animate_shell_spring();
                                store.start_graduation_feynman(&t_feynman);
                            },
                        )),
                )
        }))
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
            ICON_INBOX,
        ),
        (
            WorkspaceTab::Universe,
            "全市场雷达",
            "Universe Radar",
            "⌘2",
            SKY_FG,
            ICON_RADAR,
        ),
        (
            WorkspaceTab::Industry,
            "行业研究",
            "Industry Research",
            "⌘3",
            VIOLET_FG,
            ICON_LAYERS,
        ),
        (
            WorkspaceTab::Baskets,
            "能力圈总览",
            "Circle",
            "⌘4",
            INK_HERO,
            ICON_TARGET,
        ),
        (
            WorkspaceTab::Journal,
            "决策账本",
            "Decision Journal",
            "⌘5",
            GOLD_FG,
            ICON_JOURNAL,
        ),
    ];

    view()
        .w(px(SIDEBAR_WIDTH))
        .flex_shrink_0()
        .h_full()
        .pt(px(4.0))
        .pb(px(8.0))
        .flex()
        .flex_col()
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
                .children(nav_items.into_iter().map(
                    |(tab, zh, en, shortcut, rail_col, icon_svg)| {
                        let is_selected = computed(move || {
                            !store.company_open.get()
                                && store.active_tab.get() == tab
                                && (tab != WorkspaceTab::Baskets
                                    || store.basket_filter.get().is_none())
                        });
                        view()
                            .relative()
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
                                    .child(icon(icon_svg, 14.0).text_color(move || {
                                        rgb(if is_selected.get() {
                                            rail_col
                                        } else {
                                            INK_SECONDARY
                                        })
                                    }))
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
                            .child(when(
                                move || {
                                    tab == WorkspaceTab::Inbox
                                        && store.inbox_summary.with(|s| {
                                            s.as_ref().map(|x| x.unread_signals).unwrap_or(0)
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
                                        .opacity(move || {
                                            if store.cmd_held.get() {
                                                0.0_f32
                                            } else {
                                                1.0_f32
                                            }
                                        })
                                        .child(
                                            text(move || {
                                                let cnt = store.inbox_summary.with(|s| {
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
                            .child(cmd_kbd_right_overlay(store, shortcut))
                    },
                )),
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
}

fn collect_topbar_messages(store: AppStore) -> Vec<TopbarMessage> {
    let l = store.lang.get();
    let applied = store.update_applied_version.get();
    let applying = store.update_applying.get();
    let upd_prog = store.update_progress.get();
    let upd_detail = store.update_progress_detail.get();
    let upd_chk = store.update_checking.get();
    let upd_notice = store.update_notice.get();
    let u_sync = store.universe_syncing.get();
    let u_prog = store.universe_sync_progress.get();
    let u_detail = store.universe_sync_progress_detail.get();
    let i_catch = store.inbox_catching_up.get();
    let i_prog = store.inbox_catchup_progress.get();
    let dr_run = store.deep_research_running.get();
    let dr_ticker = store.deep_research_ticker.get();
    let ind_run = store.industry_research_running.get();
    let ind_prog = store.industry_research_progress.get();
    let syncing = store.sync_running.get();
    let sync_stage = store.sync_stage.get();
    let sync_progress = store.sync_progress.get();
    let themes_syncing = store.themes_syncing.get();
    let storage_scanning = store.storage_scanning.get();
    let messages = store.messages.get();

    build_topbar_messages(&TopbarMessageInputs {
        lang: l,
        update_applied_version: applied.as_deref(),
        update_applying: applying,
        update_progress: upd_prog.as_deref(),
        update_progress_fraction: upd_detail.as_ref().map(|d| d.fraction),
        update_checking: upd_chk,
        update_notice: upd_notice.as_ref(),
        universe_syncing: u_sync,
        universe_progress: u_prog.as_deref(),
        universe_progress_fraction: u_detail.as_ref().map(|d| d.fraction),
        inbox_catching_up: i_catch,
        inbox_progress: i_prog.as_deref(),
        deep_research_running: dr_run,
        deep_research_ticker: dr_ticker.as_deref(),
        industry_research_running: ind_run,
        industry_progress: ind_prog.as_deref(),
        syncing,
        sync_stage: sync_stage.as_deref(),
        sync_progress,
        themes_syncing,
        storage_scanning,
        messages: &messages,
    })
}

fn execute_topbar_message_action(store: AppStore, action: TopbarMessageAction) {
    store.close_message_box();
    match action {
        TopbarMessageAction::ApplyUpdate => store.start_update_apply(),
        TopbarMessageAction::RestartAfterUpdate => restart_after_update(store),
        TopbarMessageAction::CheckUpdate => store.start_update_check(true),
        TopbarMessageAction::OpenSettings => store.open_settings(),
    }
}

fn topbar_action_label(action: TopbarMessageAction, l: UiLang) -> &'static str {
    match action {
        TopbarMessageAction::ApplyUpdate => l.t("安装更新", "Install"),
        TopbarMessageAction::RestartAfterUpdate => l.t("重启生效", "Restart"),
        TopbarMessageAction::CheckUpdate => l.t("检查更新", "Check"),
        TopbarMessageAction::OpenSettings => l.t("前往设置", "Settings"),
    }
}

fn topbar_tooltip(tip: impl Fn() -> String + 'static) -> AnyElement {
    view()
        .px(px(8.0))
        .py(px(4.0))
        .rounded_sm()
        .bg(rgb(0x161620))
        .border_1()
        .border_color(rgba(BORDER_GLASS_STRONG))
        .child(
            text(tip)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_PRIMARY)),
        )
        .into_element()
}

fn render_sidebar_toggle_button(store: AppStore) -> View {
    view()
        .relative()
        .window_drag(false)
        .w(px(28.0))
        .h(px(26.0))
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(rgba(RAIL_HOVER)))
        .active(|s| s.bg(rgba(RAIL_ACTIVE)))
        .flex()
        .items_center()
        .justify_center()
        .tooltip(topbar_tooltip(move || {
            store
                .lang
                .get()
                .t("切换侧边栏 (⌘B)", "Toggle Sidebar (⌘B)")
                .to_owned()
        }))
        .on_press(move || {
            animate_shell_spring();
            store.toggle_sidebar();
        })
        .child(icon(ICON_SIDEBAR_LEFT, 15.0).text_color(move || {
            rgb(if store.sidebar_collapsed.get() {
                INK_MUTED
            } else {
                INK_SECONDARY
            })
        }))
        .child(cmd_kbd_bottom_corner(store, "⌘B"))
}

fn render_unified_topbar(store: AppStore) -> impl IntoElement {
    let mark_clicks = stored((0u8, None::<std::time::Instant>));
    let settings_active = computed(move || store.settings_open.get());

    view()
        .h(px(TOOLBAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .px(px(10.0))
        .flex()
        .flex_row()
        .items_center()
        .window_drag(true)
        // Left symmetric wing: sidebar toggle, optional dossier back button, M³ mark
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .pl(move || px((titlebar_left_inset() - 10.0).max(0.0)))
                .flex()
                .flex_row()
                .items_center()
                .justify_start()
                .window_drag(true)
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .window_drag(false)
                        .child(render_sidebar_toggle_button(store))
                        .child(when(
                            move || store.company_open.get(),
                            move || {
                                view()
                                    .w(px(28.0))
                                    .h(px(26.0))
                                    .rounded_md()
                                    .cursor_pointer()
                                    .bg(rgba(RAIL_HOVER))
                                    .border_1()
                                    .border_color(rgba(BORDER_GLASS_SUBTLE))
                                    .hover(|s| s.bg(rgba(RAIL_ACTIVE)))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .tooltip(topbar_tooltip(move || {
                                        store.lang.get().t("返回", "Back").to_owned()
                                    }))
                                    .on_press(move || store.close_company_overlay())
                                    .child(icon(ICON_ARROW_LEFT, 14.0).text_color(rgb(INK_PRIMARY)))
                            },
                        ))
                        .child(
                            view()
                                .cursor_pointer()
                                .px(px(6.0))
                                .py(px(4.0))
                                .rounded_sm()
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
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
                                .child(pixel_m3_mark(1.3)),
                        ),
                ),
        )
        // Center reservation matching the dead-center floating morphing Command Box (`400px`)
        .child(view().w(px(404.0)).h_full().flex_shrink_0())
        // Right symmetric wing: compact IconMCP Message Box + Sync + Settings buttons
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .pr(px(2.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_end()
                .window_drag(true)
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .window_drag(false)
                        .child(render_topbar_message_box_button(store))
                        .child(
                            view()
                                .relative()
                                .w(px(28.0))
                                .h(px(26.0))
                                .rounded_md()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .tooltip(topbar_tooltip(move || {
                                    let l = store.lang.get();
                                    if store.sync_running.get() {
                                        store.sync_stage.get().unwrap_or_else(|| {
                                            l.t("同步中…", "Syncing…").to_owned()
                                        })
                                    } else {
                                        l.t(
                                            "同步核心池财报与价格 (⌘R)",
                                            "Sync Circle Financials & Prices (⌘R)",
                                        )
                                        .to_owned()
                                    }
                                }))
                                .on_press(move || store.start_sync())
                                .child(icon(ICON_REFRESH, 15.0).text_color(move || {
                                    rgb(if store.sync_running.get() {
                                        SKY_FG
                                    } else {
                                        INK_SECONDARY
                                    })
                                }))
                                .child(cmd_kbd_bottom_corner(store, "⌘R")),
                        )
                        .child(
                            view()
                                .relative()
                                .w(px(28.0))
                                .h(px(26.0))
                                .rounded_md()
                                .cursor_pointer()
                                .bg(move || {
                                    if settings_active.get() {
                                        rgba(RAIL_SELECTED)
                                    } else {
                                        rgba(0x00000000)
                                    }
                                })
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .active(|s| s.bg(rgba(RAIL_ACTIVE)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .tooltip(topbar_tooltip(move || {
                                    store.lang.get().t("设置 (⌘,)", "Settings (⌘,)").to_owned()
                                }))
                                .on_press(move || {
                                    animate_shell_spring();
                                    store.toggle_settings();
                                })
                                .child(icon(ICON_SETTINGS, 15.0).text_color(move || {
                                    rgb(if settings_active.get() {
                                        INK_HERO
                                    } else {
                                        INK_SECONDARY
                                    })
                                }))
                                .child(cmd_kbd_bottom_corner(store, "⌘,")),
                        ),
                ),
        )
}

fn render_topbar_message_box_button(store: AppStore) -> View {
    view().child(dynamic(move || {
        let l = store.lang.get();
        let open = store.message_box_open.get();
        let msgs = collect_topbar_messages(store);
        let head = msgs.first();
        let dot_color = head.map(|m| m.dot_color);
        let progress = head.and_then(|m| m.progress).map(|p| p.clamp(0.05, 1.0));
        let tip_text = match head {
            Some(m) => {
                if let Some(p) = m.progress {
                    format!(
                        "{} ({}%)",
                        m.title,
                        (p.clamp(0.0, 1.0) * 100.0).round() as u32
                    )
                } else {
                    m.title.clone()
                }
            }
            None => l
                .t("消息箱 · 暂无新消息", "Messages · No activity")
                .to_owned(),
        };

        let mut btn = view()
            .relative()
            .w(px(28.0))
            .h(px(26.0))
            .rounded_md()
            .cursor_pointer()
            .overflow_hidden()
            .bg(if open {
                rgba(RAIL_SELECTED)
            } else {
                rgba(0x00000000)
            })
            .hover(|s| s.bg(rgba(RAIL_HOVER)))
            .active(|s| s.bg(rgba(RAIL_ACTIVE)))
            .flex()
            .items_center()
            .justify_center()
            .tooltip(topbar_tooltip(move || tip_text.clone()))
            .on_press(move || {
                animate_shell_spring();
                store.toggle_message_box();
            })
            .child(
                icon(ICON_BELL, 15.0).text_color(rgb(if open || head.is_some() {
                    INK_HERO
                } else {
                    INK_SECONDARY
                })),
            );

        if let Some(col) = dot_color {
            btn = btn.child(
                view()
                    .absolute()
                    .top(px(4.0))
                    .right(px(4.0))
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(rgb(col)),
            );
        }

        if let (Some(f), Some(col)) = (progress, dot_color) {
            let mut bar = view()
                .absolute()
                .bottom(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .h(px(2.0))
                .bg(rgba(0xFFFFFF18))
                .flex()
                .flex_row()
                .child(view().w(px(0.0)).flex_grow(f).h_full().bg(rgb(col)));
            if f < 1.0 {
                bar = bar.child(view().w(px(0.0)).flex_grow(1.0 - f).h_full());
            }
            btn = btn.child(bar);
        }

        btn
    }))
}

fn message_status_icon(status: TopbarMessageStatus) -> &'static str {
    match status {
        TopbarMessageStatus::Running => ICON_REFRESH,
        TopbarMessageStatus::ActionRequired => ICON_ARROW_CIRCLE_UP,
        TopbarMessageStatus::Success => ICON_CIRCLE_CHECK,
        TopbarMessageStatus::Error => ICON_ALERT,
        TopbarMessageStatus::Info => ICON_BELL,
    }
}

fn render_message_box_popover(store: AppStore) -> impl IntoElement {
    overlay()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_end()
        .pt(px(48.0))
        .pr(px(12.0))
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(rgba(0x00000028))
                .on_press(move || {
                    animate_shell_spring();
                    store.close_message_box();
                }),
        )
        .child(
            view()
                .w(px(396.0))
                .max_h(px(460.0))
                .rounded_xl()
                .bg(rgba(BG_COMMAND_CARD))
                .border_1()
                .border_color(rgba(BORDER_GLASS_STRONG))
                .shadow_xl()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let msgs = collect_topbar_messages(store);
                    let has_dismissible = msgs.iter().any(|m| m.dismissible);
                    let checking = store.update_checking.get();
                    let applying = store.update_applying.get();

                    let mut right_actions =
                        view().flex().flex_row().items_center().gap(px(6.0)).child(
                            view()
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_press(move || {
                                    if !checking && !applying {
                                        store.start_update_check(true);
                                    }
                                })
                                .child(
                                    text(if checking {
                                        l.t("检查中…", "Checking…").to_owned()
                                    } else {
                                        l.t("检查更新", "Check Updates").to_owned()
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_SECONDARY)),
                                ),
                        );
                    if has_dismissible {
                        right_actions = right_actions.child(
                            view()
                                .px(px(8.0))
                                .py(px(3.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_press(move || store.clear_messages())
                                .child(
                                    text(l.t("清空", "Clear").to_owned())
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                ),
                        );
                    }

                    let mut panel = view().w_full().flex().flex_col().child(
                        view()
                            .px(px(14.0))
                            .py(px(10.0))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
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
                                    .gap(px(7.0))
                                    .child(icon(ICON_BELL, 14.0).text_color(rgb(INK_SECONDARY)))
                                    .child(
                                        text(
                                            l.t("消息与后台任务", "Messages & Activity").to_owned(),
                                        )
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO)),
                                    ),
                            )
                            .child(right_actions),
                    );

                    if msgs.is_empty() {
                        let ver = compact_sidebar_version_label(update::current_binary_version());
                        let ch = store.update_channel.get().as_str().to_owned();
                        panel = panel.child(
                            view()
                                .px(px(16.0))
                                .py(px(26.0))
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    text(l.t("暂无活动消息", "No active messages").to_owned())
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_SM))
                                        .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(
                                    text(format!("{ver} · {ch}"))
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                ),
                        );
                    } else {
                        panel = panel.child(
                            scroll().w_full().max_h(px(390.0)).child(
                                view()
                                    .w_full()
                                    .p(px(10.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.0))
                                    .children(msgs.into_iter().map(|m| {
                                        let mut top_row = view()
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
                                                        icon(message_status_icon(m.status), 13.0)
                                                            .text_color(rgb(m.dot_color)),
                                                    )
                                                    .child(
                                                        text(m.tag)
                                                            .font_family(FONT_MONO_TERM)
                                                            .text_size(px(TEXT_2XS))
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(rgb(m.dot_color)),
                                                    ),
                                            );

                                        let mut right_meta =
                                            view().flex().flex_row().items_center().gap(px(6.0));
                                        if let Some(frac) = m.progress {
                                            right_meta = right_meta.child(
                                                text(format!(
                                                    "{}%",
                                                    (frac.clamp(0.0, 1.0) * 100.0).round() as u32
                                                ))
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_2XS))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(m.dot_color)),
                                            );
                                        }
                                        if let Some(action) = m.action {
                                            let lbl = topbar_action_label(action, l);
                                            right_meta = right_meta.child(
                                                view()
                                                    .px(px(7.0))
                                                    .py(px(2.0))
                                                    .rounded_xs()
                                                    .cursor_pointer()
                                                    .bg(rgb(BG_WELL))
                                                    .border_1()
                                                    .border_color(rgba(m.border_color))
                                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                                    .on_press(move || {
                                                        execute_topbar_message_action(store, action)
                                                    })
                                                    .child(
                                                        text(lbl)
                                                            .font_family(FONT_UI)
                                                            .text_size(px(TEXT_2XS))
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(rgb(INK_HERO)),
                                                    ),
                                            );
                                        }
                                        if let Some(msg_id) = m.id.filter(|_| m.dismissible) {
                                            right_meta = right_meta.child(
                                                view()
                                                    .p(px(3.0))
                                                    .rounded_xs()
                                                    .cursor_pointer()
                                                    .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                                    .on_press(move || store.dismiss_message(msg_id))
                                                    .child(
                                                        icon(ICON_CLOSE, 12.0)
                                                            .text_color(rgb(INK_MUTED)),
                                                    ),
                                            );
                                        }
                                        top_row = top_row.child(right_meta);

                                        let mut card = view()
                                            .w_full()
                                            .rounded_md()
                                            .bg(rgb(BG_SURFACE))
                                            .border_1()
                                            .border_color(rgba(m.border_color))
                                            .p(px(10.0))
                                            .flex()
                                            .flex_col()
                                            .gap(px(6.0))
                                            .child(top_row)
                                            .child(
                                                text(m.title)
                                                    .font_family(FONT_UI)
                                                    .text_size(px(TEXT_XS))
                                                    .line_height(px(16.0))
                                                    .text_color(rgb(INK_PRIMARY)),
                                            );
                                        if let Some(detail) = m.detail {
                                            card = card.child(
                                                text(detail)
                                                    .font_family(FONT_UI)
                                                    .text_size(px(TEXT_2XS))
                                                    .text_color(rgb(INK_MUTED)),
                                            );
                                        }
                                        if let Some(frac) = m.progress {
                                            card = card.child(progress_bar(frac, m.dot_color));
                                        }
                                        card
                                    })),
                            ),
                        );
                    }
                    panel
                })),
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
                    "ALAN · PAPER ONLY"
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
                "ALAN · PAPER ONLY",
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
        .gap(px(14.0))
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
            view()
                .flex_shrink_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    text(move || {
                        let price = store
                            .benchmark
                            .get()
                            .map(|b| format!("{:.2}", b.price))
                            .unwrap_or_else(|| "—".to_owned());
                        format!("S&P {price}")
                    })
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_SECONDARY)),
                )
                .child(
                    text(move || {
                        let l = store.lang.get();
                        match store.sync_status.get() {
                            Some(s) => fmt_time_cn_for(Some(s.synced_at_ms), l),
                            None => l.t("未同步", "Unsynced").to_owned(),
                        }
                    })
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED)),
                )
                .child(
                    text(move || {
                        let l = store.lang.get();
                        let total = store
                            .universe_summary
                            .get()
                            .map(|s| s.total_companies)
                            .unwrap_or(0);
                        if total > 0 {
                            match l {
                                UiLang::Zh => format!("L0 {total} 家"),
                                UiLang::En => format!("L0 {total}"),
                            }
                        } else {
                            l.t("L0 未同步", "L0 —").to_owned()
                        }
                    })
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_SECONDARY)),
                )
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let budget_rep = store.budget_report.get();
                    let (budget_text, budget_col) =
                        budget_watermark_summary_for(budget_rep.as_ref(), l);
                    let budget_compact = budget_text
                        .strip_prefix("本月 AI 预算 ")
                        .or_else(|| budget_text.strip_prefix("AI Budget "))
                        .unwrap_or(&budget_text)
                        .to_owned();
                    view()
                        .cursor_pointer()
                        .on_press(move || store.show_budget_status_notice())
                        .child(
                            text(budget_compact)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(budget_col)),
                        )
                }))
                .child(
                    text(move || {
                        let ver = compact_sidebar_version_label(update::current_binary_version());
                        let ch = store.update_channel.get().as_str();
                        format!("{ver} {ch}")
                    })
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED)),
                )
                .child(
                    text(move || status_meta.get().2.to_owned())
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
}

fn register_keyboard_shortcuts(store: AppStore) {
    // Global Cmd/Mod shortcuts (safe across Input focus via When::Auto; never bind reserved keys!)
    on_key("mod+1", move |_| store.open_tab(WorkspaceTab::Inbox));
    on_key("mod+2", move |_| store.open_tab(WorkspaceTab::Universe));
    on_key("mod+3", move |_| store.open_tab(WorkspaceTab::Industry));
    on_key("mod+4", move |_| store.focus_basket(None));
    on_key("mod+5", move |_| store.open_tab(WorkspaceTab::Journal));
    on_key("mod+,", move |_| {
        animate_shell_spring();
        store.toggle_settings();
    });
    on_key("mod+b", move |_| {
        animate_shell_spring();
        store.toggle_sidebar();
    });
    on_key("mod+\\", move |_| {
        animate_shell_spring();
        store.toggle_sidebar();
    });
    on_key("mod+k", move |_| {
        animate_shell_spring();
        store.toggle_palette();
    });
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
            store.cmd_held.set(false);
            if store.palette_open.get_untracked() {
                animate_shell_spring();
                store.close_palette();
                return;
            }
            if store.message_box_open.get_untracked() {
                animate_shell_spring();
                store.close_message_box();
                return;
            }
            if store.settings_open.get_untracked() {
                animate_shell_spring();
                store.close_settings();
                return;
            }
            if store.pending_graduation_ticker.get_untracked().is_some() {
                animate_shell_spring();
                store.cancel_graduation_gate();
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
                    let exp = store.palette_expanded_groups.get_untracked();
                    let len = visible_palette_items_for(&all, &q, &exp).len();
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
            let exp = store.palette_expanded_groups.get_untracked();
            let visible = visible_palette_items_for(&all, &q, &exp);
            let idx = store
                .palette_cursor
                .get_untracked()
                .min(visible.len().saturating_sub(1));
            if let Some(item) = visible.get(idx) {
                animate_shell_spring();
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
        animate_shell_spring();
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
