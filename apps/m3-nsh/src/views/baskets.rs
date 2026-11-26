//! Circle Overview (`CORE / WATCH / OUT / TOO HARD`) view (`⌘4`).

use crate::components::{
    action_button, badge_pill, circle_tag_button, cmd_kbd_corner, mono_right, obsidian_card,
    seg_group, seg_tab, v_divider, view_filter_strip, view_header_strip,
};
use crate::format::{
    collect_circle_tags, compute_today_pulse, filter_reports_for_basket, fmt_money_compact_opt,
    fmt_pct_clean, fmt_price, primary_val,
};
use crate::icons::{ICON_CHEVRON_DOWN, ICON_CHEVRON_RIGHT, ICON_CLOSE, icon};
use crate::platform_desktop::show_company_context_menu;
use crate::stores::{
    AppStore, BasketsViewMode, CompanyViewTab, SortCol, StatusQuickFilter, WorkspaceTab,
};
use crate::theme::*;
use crate::views::onboarding::onboarding_card;
use m3_core::i18n::UiLang;
use m3_core::model::{Bucket, MungerReport, primary_circle_tag, secondary_circle_detail};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

/// Renders the Circle Overview (`CORE` / `WATCH` / `OUT` / `TOO HARD`) view (`⌘4`).
pub fn baskets_view(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(render_pulse_strip(store))
        .child(render_filter_bar(store))
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .child(dynamic(move || render_baskets_body(store))),
        )
}

fn render_pulse_strip(store: AppStore) -> View {
    view_header_strip()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let reports = store.reports.get();
                    let pulse = compute_today_pulse(&reports);
                    let pulse_quiet = pulse.strike_tickers.is_empty()
                        && pulse.red_core_tickers.is_empty()
                        && pulse.watch_count == 0;

                    let mut row = view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .min_w(px(0.0))
                        .child(
                            text(l.t("今日信号", "TODAY'S PULSE"))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        )
                        .child(v_divider());

                    if reports.is_empty() {
                        row = row.child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(rgb(INK_MUTED)))
                                .child(
                                    text(
                                        l.t(
                                            "能力圈为空 · 从全市场雷达或收件箱挑选首批标的",
                                            "Circle is empty · pick holdings from Universe Radar or Inbox",
                                        )
                                        .to_owned(),
                                    )
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_SECONDARY)),
                                ),
                        );
                    } else if pulse_quiet {
                        row = row.child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    view()
                                        .w(px(5.0))
                                        .h(px(5.0))
                                        .rounded_full()
                                        .bg(rgb(EMERALD_FG)),
                                )
                                .child(
                                    text(
                                        l.t(
                                            "今日无球可挥 · 耐心等待",
                                            "No fat pitch today · wait patiently",
                                        )
                                        .to_owned(),
                                    )
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_SECONDARY)),
                                ),
                        );
                    } else {
                        if let Some(first_strike) = pulse.strike_tickers.first().cloned() {
                            let list_txt = pulse.strike_tickers.join(", ");
                            row = row.child(
                                view()
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded_xs()
                                    .bg(rgba(EMERALD_BG))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .on_press(move || {
                                        store.open_company(
                                            &first_strike,
                                            CompanyViewTab::FinancialsMemo,
                                        );
                                    })
                                    .child(
                                        view()
                                            .w(px(5.0))
                                            .h(px(5.0))
                                            .rounded_full()
                                            .bg(rgb(EMERALD_FG)),
                                    )
                                    .child(
                                        text(format!("{}: {list_txt}", l.t("击球区", "Strike")))
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_XS))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(EMERALD_FG)),
                                    ),
                            );
                        } else {
                            row = row.child(
                                text(l.t("0 击球", "0 Strike"))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            );
                        }

                        row = row.child(
                            text("·")
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        );

                        if let Some(first_red) = pulse.red_core_tickers.first().cloned() {
                            let list_txt = pulse.red_core_tickers.join(", ");
                            row = row.child(
                                view()
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded_xs()
                                    .bg(rgba(CORAL_BG))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .on_press(move || {
                                        store.open_company(
                                            &first_red,
                                            CompanyViewTab::FinancialsMemo,
                                        );
                                    })
                                    .child(
                                        view()
                                            .w(px(5.0))
                                            .h(px(5.0))
                                            .rounded_full()
                                            .bg(rgb(CORAL_FG)),
                                    )
                                    .child(
                                        text(format!(
                                            "{}: {list_txt}",
                                            l.t("红灯复核", "Red Review")
                                        ))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(CORAL_FG)),
                                    ),
                            );
                        } else {
                            row = row.child(
                                text(l.t("核心池无红灯", "0 Core Warnings"))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            );
                        }

                        row = row.child(
                            text("·")
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        );

                        if pulse.watch_count > 0 {
                            let cnt = pulse.watch_count;
                            row = row.child(
                                view()
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded_xs()
                                    .bg(rgba(SKY_BG))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(BG_HOVER)))
                                    .on_press(move || store.focus_basket(Some(Bucket::Watch)))
                                    .child(
                                        text(match l {
                                            UiLang::Zh => format!("○ 待分拣 {cnt} 家"),
                                            UiLang::En => format!("○ {cnt} to triage"),
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(SKY_FG)),
                                    ),
                            );
                        } else {
                            row = row.child(
                                text(l.t("待审讯已清空", "Watch cleared"))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            );
                        }
                    }

                    row
                })),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let mode = store.baskets_view_mode.get();
            seg_group()
                .relative()
                .child(seg_tab(
                    l.t("列表", "List"),
                    mode == BasketsViewMode::List,
                    None,
                    false,
                    move || {
                        store.baskets_view_mode.set(BasketsViewMode::List);
                        store.baskets_cursor_index.set(0);
                    },
                ))
                .child(seg_tab(
                    l.t("看板", "Board"),
                    mode == BasketsViewMode::Board,
                    None,
                    false,
                    move || {
                        store.baskets_view_mode.set(BasketsViewMode::Board);
                        store.baskets_cursor_index.set(0);
                    },
                ))
                .child(cmd_kbd_corner(store, "v"))
        }))
}

fn render_filter_bar(store: AppStore) -> View {
    view_filter_strip()
        .child(dynamic(move || {
            let l = store.lang.get();
            let reports = store.reports.get();
            let basket_filter = store.basket_filter.get();
            let total_cnt = reports.len();
            let all_active = basket_filter.is_none();

            let mut all_btn = view()
                .px(px(8.0))
                .py(px(2.0))
                .rounded_xs()
                .cursor_pointer()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                .on_press(move || store.focus_basket(None))
                .child(
                    text(l.t("全部", "All"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(if all_active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(rgb(if all_active { INK_HERO } else { INK_MUTED })),
                )
                .child(
                    text(format!("{total_cnt}"))
                        .font_family(FONT_MONO_NUM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(if all_active { INK_PRIMARY } else { INK_MUTED })),
                );
            if all_active {
                all_btn = all_btn
                    .bg(rgb(BG_HOVER))
                    .border_1()
                    .border_color(rgb(BORDER_PILL));
            }

            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(all_btn)
                .children(Bucket::display_order().into_iter().map(|b| {
                    let (fg, bg, br) = bucket_colors(b);
                    let active = basket_filter == Some(b);
                    let cnt = reports.iter().filter(|r| r.bucket == b).count();
                    let tier_label = match b {
                        Bucket::Core => "IN · CORE",
                        Bucket::Watch => "WATCH",
                        Bucket::Out => "OUT",
                        Bucket::TooHard => "TOO HARD",
                    };
                    let mut btn = view()
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded_xs()
                        .cursor_pointer()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                        .on_press(move || {
                            if active {
                                store.focus_basket(None);
                            } else {
                                store.focus_basket(Some(b));
                            }
                        })
                        .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(rgb(fg)))
                        .child(
                            text(tier_label)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .font_weight(if active {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(rgb(if active { fg } else { INK_SECONDARY })),
                        )
                        .child(
                            text(format!("{cnt}"))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(if active { INK_HERO } else { INK_MUTED })),
                        );
                    if active {
                        btn = btn.bg(rgba(bg)).border_1().border_color(rgba(br));
                    }
                    btn
                }))
        }))
        .child(dynamic(move || {
            let l = store.lang.get();
            let status_filter = store.status_filter.get();
            let has_any = store.basket_filter.get().is_some()
                || store.circle_filter.get().is_some()
                || status_filter != StatusQuickFilter::All;

            let mut right = view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .children(
                    [StatusQuickFilter::StrikeOnly, StatusQuickFilter::WarnedOnly]
                        .into_iter()
                        .map(|sf| {
                            let active = status_filter == sf;
                            let mut btn = view()
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded_xs()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(RAIL_HOVER)))
                                .on_press(move || store.toggle_status_filter(sf))
                                .child(
                                    text(sf.label_for(l))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(if active { INK_HERO } else { INK_MUTED })),
                                );
                            if active {
                                btn = btn
                                    .bg(rgb(BG_HOVER))
                                    .border_1()
                                    .border_color(rgb(BORDER_PILL));
                            }
                            btn
                        }),
                );

            if has_any {
                right = right.child(
                    view()
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded_xs()
                        .bg(rgba(GOLD_BG))
                        .border_1()
                        .border_color(rgba(GOLD_BORDER))
                        .cursor_pointer()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .on_press(move || {
                            store.basket_filter.set(None);
                            store.circle_filter.set(None);
                            store.status_filter.set(StatusQuickFilter::All);
                            store.baskets_cursor_index.set(0);
                        })
                        .child(icon(ICON_CLOSE, 11.0).text_color(rgb(GOLD_FG)))
                        .child(
                            text(l.t("重置", "Reset"))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(GOLD_FG)),
                        ),
                );
            }

            right
        }))
}

fn render_baskets_body(store: AppStore) -> View {
    let l = store.lang.get();
    let reports = store.reports.get();
    let tags = collect_circle_tags(&reports);
    let basket_filter = store.basket_filter.get();
    let circle_filter = store.circle_filter.get();
    let status_filter = store.status_filter.get();
    let mode = store.baskets_view_mode.get();
    let has_any_filter = basket_filter.is_some()
        || circle_filter.is_some()
        || status_filter != StatusQuickFilter::All;

    let mut root = view().w_full().flex().flex_col();

    if store.should_show_onboarding() {
        root = root.child(
            view()
                .px(px(20.0))
                .pt(px(16.0))
                .child(onboarding_card(store)),
        );
    }

    if !tags.is_empty() {
        root = root.child(
            view()
                .px(px(20.0))
                .pt(px(12.0))
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(6.0))
                .children(tags.into_iter().map(|(tag, cnt)| {
                    let active = circle_filter.as_deref() == Some(tag.as_str());
                    let tag_click = tag.clone();
                    circle_tag_button(tag, active, true, Some(cnt), move || {
                        store.toggle_circle_filter(&tag_click);
                    })
                })),
        );
    }

    if reports.is_empty() && !has_any_filter {
        return root.child(
            view()
                .px(px(20.0))
                .pt(px(16.0))
                .pb(px(24.0))
                .child(render_empty_circle_card(store, l)),
        );
    }

    match mode {
        BasketsViewMode::Board => root.child(render_baskets_board(store, &reports, l)),
        BasketsViewMode::List => {
            root.child(render_baskets_list(store, &reports, has_any_filter, l))
        }
    }
}

fn render_empty_circle_card(store: AppStore, l: UiLang) -> View {
    obsidian_card()
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(view().w(px(7.0)).h(px(7.0)).rounded_full().bg(rgb(EMERALD_FG)))
                .child(
                    text(l.t("尚未建立个人能力圈", "Your Circle is Empty"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                ),
        )
        .child(
            text(
                l.t(
                    "M3 不会预置任何持仓或观察名单。你可以先同步全美股 L0 湖仓，从全市场雷达或简报收件箱中挑选真正读懂的公司加入能力圈（CORE / WATCH / OUT / TOO HARD），也可以在终端通过 CLI 自行维护。",
                    "M3 ships with zero pre-loaded holdings or watchlists. Sync the US L0 lakehouse, then promote businesses you understand from Universe Radar or Briefing Inbox into your Circle (CORE / WATCH / OUT / TOO HARD).",
                )
                .to_owned(),
            )
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .line_height(px(19.0))
            .text_color(rgb(INK_SECONDARY)),
        )
        .child(
            view()
                .pt(px(4.0))
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(10.0))
                .child(action_button(
                    l.t("前往全市场雷达 (⌘2)", "Open Universe Radar (⌘2)"),
                    true,
                    move || store.open_tab(WorkspaceTab::Universe),
                ))
                .child(action_button(
                    l.t("打开简报收件箱 (⌘1)", "Open Briefing Inbox (⌘1)"),
                    false,
                    move || store.open_tab(WorkspaceTab::Inbox),
                )),
        )
        .child(
            view()
                .px(px(12.0))
                .py(px(8.0))
                .rounded_md()
                .bg(rgb(BG_WELL))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .child(
                    text(
                        l.t(
                            "CLI: m3 add AAPL --bucket watch --circle \"消费生态\"",
                            "CLI: m3 add AAPL --bucket watch --circle \"Ecosystem\"",
                        )
                        .to_owned(),
                    )
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                ),
        )
}

fn render_baskets_list(
    store: AppStore,
    reports: &[MungerReport],
    has_any_filter: bool,
    l: UiLang,
) -> View {
    let basket_filter = store.basket_filter.get();
    let collapsed_set = store.collapsed_baskets.get();
    let circle_filter = store.circle_filter.get();
    let status_filter = store.status_filter.get();
    let sort_col = store.sort_col.get();
    let sort_asc = store.sort_asc.get();
    let cursor_index = store.baskets_cursor_index.get();

    let order: Vec<Bucket> = match basket_filter {
        Some(b) => vec![b],
        None => Bucket::display_order().to_vec(),
    };

    let nd_label = l.t("净债/OE", "NetDebt/OE");
    let status_label = l.t("状态", "Status");
    let headers: &[(&str, f32, bool, Option<SortCol>)] = &[
        (
            l.t("标的 / 能力圈", "Holding / Circle"),
            0.0,
            false,
            Some(SortCol::Ticker),
        ),
        (l.t("现价", "Price"), 82.0, true, Some(SortCol::Price)),
        (l.t("击球价", "Strike"), 82.0, true, None),
        (
            l.t("距击球价", "vs Strike"),
            82.0,
            true,
            Some(SortCol::Margin),
        ),
        ("5Y ROIC", 76.0, true, Some(SortCol::Roic)),
        (
            l.t("严苛 FCF%", "Strict FCF%"),
            80.0,
            true,
            Some(SortCol::FcfYield),
        ),
        (
            l.t("3Y 股本Δ", "3Y ShareΔ"),
            76.0,
            true,
            Some(SortCol::ShareDelta),
        ),
        (nd_label, 74.0, true, Some(SortCol::NetDebt)),
        (status_label, 96.0, false, None),
        (l.t("分桶", "Move"), 138.0, false, None),
    ];

    let mut linear_idx: usize = 0;
    let mut cards = view()
        .px(px(20.0))
        .pt(px(12.0))
        .pb(px(20.0))
        .flex()
        .flex_col()
        .gap(px(12.0));

    for bucket in order {
        let (fg, _, _) = bucket_colors(bucket);
        let is_collapsed = basket_filter.is_none() && collapsed_set.contains(&bucket);
        let rows = filter_reports_for_basket(
            reports,
            bucket,
            circle_filter.as_deref(),
            status_filter,
            sort_col,
            sort_asc,
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

        let mut hdr_left = view().flex().flex_row().items_center().gap(px(10.0));
        if basket_filter.is_none() {
            hdr_left = hdr_left.child(
                icon(
                    if is_collapsed {
                        ICON_CHEVRON_RIGHT
                    } else {
                        ICON_CHEVRON_DOWN
                    },
                    12.0,
                )
                .text_color(rgb(INK_MUTED)),
            );
        }
        hdr_left = hdr_left
            .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(fg)))
            .child(
                text(bucket.code())
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(fg)),
            )
            .child(
                text(bucket.short_label_for(l))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_BASE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO)),
            )
            .child(
                text(format!("{row_cnt}"))
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(INK_MUTED)),
            );

        let mut hdr = view()
            .px(px(16.0))
            .py(px(10.0))
            .bg(rgb(BG_WELL))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .cursor_pointer()
            .on_press(move || {
                if store.basket_filter.get_untracked().is_none() {
                    store.toggle_basket_collapsed(bucket);
                }
            })
            .child(hdr_left);
        if !is_collapsed {
            hdr = hdr.border_b_1().border_color(rgb(BORDER_HAIRLINE));
        }
        if let Some(prev) = collapsed_preview {
            hdr = hdr.child(
                text(prev)
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED))
                    .truncate()
                    .number_of_lines(1),
            );
        }

        let mut block = obsidian_card().child(hdr);

        if !is_collapsed {
            if rows.is_empty() {
                let empty_msg = if has_any_filter {
                    match l {
                        UiLang::Zh => format!(
                            "当前筛选下 [{} · {}] 暂无匹配标的。",
                            bucket.code(),
                            bucket.short_label_for(l)
                        ),
                        UiLang::En => format!(
                            "No matching holdings in [{} · {}] for active filters.",
                            bucket.code(),
                            bucket.short_label_for(l)
                        ),
                    }
                } else {
                    match l {
                        UiLang::Zh => format!(
                            "当前 [{} · {}] 暂无标的。",
                            bucket.code(),
                            bucket.short_label_for(l)
                        ),
                        UiLang::En => format!(
                            "No holdings in [{} · {}] yet.",
                            bucket.code(),
                            bucket.short_label_for(l)
                        ),
                    }
                };
                block = block.child(
                    view().px(px(16.0)).py(px(14.0)).child(
                        text(empty_msg)
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(INK_MUTED)),
                    ),
                );
            } else if matches!(bucket, Bucket::Core | Bucket::Watch) {
                let hdr_row = view()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(16.0))
                    .py(px(8.0))
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .children(headers.iter().map(|(label, w, right, sort_opt)| {
                        let is_sorted = sort_opt.is_some_and(|c| c == sort_col);
                        let arrow = if is_sorted {
                            if sort_asc { " ↑" } else { " ↓" }
                        } else {
                            ""
                        };
                        let mut cell = view()
                            .px(px(4.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0));
                        if *right {
                            cell = cell.justify_end();
                        }
                        if *w > 0.0 {
                            cell = cell.w(px(*w)).flex_shrink_0();
                        } else {
                            cell = cell.flex_1().min_w(px(0.0));
                        }
                        if let Some(col) = *sort_opt {
                            cell = cell
                                .cursor_pointer()
                                .on_press(move || store.toggle_sort(col));
                        }
                        cell = cell.child(
                            text(label.to_owned())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(if is_sorted { INK_PRIMARY } else { INK_MUTED }))
                                .number_of_lines(1),
                        );
                        if !arrow.is_empty() {
                            cell = cell.child(
                                text(arrow)
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_HERO)),
                            );
                        }
                        cell
                    }));
                block = block.child(hdr_row);

                for (local_idx, rep) in rows.into_iter().enumerate() {
                    let global_idx = linear_idx;
                    linear_idx += 1;
                    let is_cursor = global_idx == cursor_index;
                    block = block.child(render_core_watch_row(
                        store,
                        rep,
                        local_idx,
                        global_idx,
                        is_cursor,
                        circle_filter.as_deref(),
                        l,
                    ));
                }
            } else {
                for (local_idx, rep) in rows.into_iter().enumerate() {
                    let global_idx = linear_idx;
                    linear_idx += 1;
                    let is_cursor = global_idx == cursor_index;
                    block = block.child(render_specimen_row(
                        store,
                        rep,
                        local_idx,
                        global_idx,
                        is_cursor,
                        circle_filter.as_deref(),
                        l,
                    ));
                }
            }
        }

        cards = cards.child(block);
    }

    cards
}

fn render_core_watch_row(
    store: AppStore,
    rep: &MungerReport,
    local_idx: usize,
    global_idx: usize,
    is_cursor: bool,
    circle_filter: Option<&str>,
    l: UiLang,
) -> View {
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
    let row_bg = if is_cursor {
        BG_SELECTED_ROW
    } else if local_idx.is_multiple_of(2) {
        BG_VIEWPORT
    } else {
        BG_ROW_ALT
    };

    let ticker_open = rep.ticker.clone();
    let ticker_ctx = rep.ticker.clone();
    let tag = primary_circle_tag(&rep.circle);
    let tag_active = circle_filter == Some(tag.as_str());
    let tag_click = tag.clone();
    let sub_detail = secondary_circle_detail(&rep.circle);

    let mut tag_row = view()
        .mt(px(4.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(circle_tag_button(tag, tag_active, false, None, move || {
            store.toggle_circle_filter(&tag_click);
        }));
    if let Some(det) = sub_detail {
        tag_row = tag_row.child(
            text(det)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED))
                .truncate()
                .number_of_lines(1),
        );
    }

    let clickable_metrics = view()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .flex_row()
        .items_center()
        .cursor_pointer()
        .on_press(move || {
            store.baskets_cursor_index.set(global_idx);
            store.open_company(&ticker_open, CompanyViewTab::FinancialsMemo);
        })
        .on_context_menu(move |e: &ContextMenuEvent| {
            show_company_context_menu(store, ticker_ctx.clone(), e.page_position);
        })
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .px(px(4.0))
                .flex()
                .flex_col()
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            text(rep.ticker.clone())
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(rep.name.clone())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(INK_PRIMARY))
                                .truncate()
                                .number_of_lines(1),
                        ),
                )
                .child(tag_row),
        )
        .child(mono_right(
            fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency),
            82.0,
            INK_HERO,
        ))
        .child(mono_right(
            fmt_price(val.and_then(|v| v.strike_price), &rep.currency),
            82.0,
            if val.and_then(|v| v.strike_price).is_some() {
                INK_SECONDARY
            } else {
                INK_MUTED
            },
        ))
        .child(mono_right(dist_str, 82.0, dist_col))
        .child(mono_right(roic_str, 76.0, roic_col))
        .child(mono_right(yield_str, 80.0, yield_col))
        .child(mono_right(share_str, 76.0, share_col))
        .child(mono_right(nd_oe, 74.0, nd_col))
        .child(
            view()
                .w(px(96.0))
                .flex_shrink_0()
                .px(px(6.0))
                .flex()
                .flex_row()
                .items_center()
                .child(badge_pill(
                    rep.status.label_for(l).to_owned(),
                    s_fg,
                    s_bg,
                    s_br,
                )),
        );

    view()
        .flex()
        .flex_row()
        .items_center()
        .px(px(16.0))
        .py(px(10.0))
        .bg(rgb(row_bg))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .child(clickable_metrics)
        .child(quick_move_strip(store, &rep.ticker, rep.bucket))
}

fn render_specimen_row(
    store: AppStore,
    rep: &MungerReport,
    local_idx: usize,
    global_idx: usize,
    is_cursor: bool,
    circle_filter: Option<&str>,
    l: UiLang,
) -> View {
    let row_bg = if is_cursor {
        BG_SELECTED_ROW
    } else if local_idx.is_multiple_of(2) {
        BG_VIEWPORT
    } else {
        BG_ROW_ALT
    };
    let ticker_open = rep.ticker.clone();
    let ticker_ctx = rep.ticker.clone();
    let tag = primary_circle_tag(&rep.circle);
    let tag_active = circle_filter == Some(tag.as_str());
    let tag_click = tag.clone();
    let latest_row = rep.ttm_row.as_ref().or_else(|| rep.annual_rows.last());
    let price_txt = fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency);
    let oe_txt = fmt_money_compact_opt(
        latest_row.and_then(|r| r.strict_owner_earnings),
        &rep.currency,
    );

    view()
        .px(px(16.0))
        .py(px(12.0))
        .bg(rgb(row_bg))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .cursor_pointer()
                        .on_press(move || {
                            store.baskets_cursor_index.set(global_idx);
                            store.open_company(&ticker_open, CompanyViewTab::FinancialsMemo);
                        })
                        .on_context_menu(move |e: &ContextMenuEvent| {
                            show_company_context_menu(store, ticker_ctx.clone(), e.page_position);
                        })
                        .child(
                            text(rep.ticker.clone())
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_BASE))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(rep.name.clone())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_PRIMARY))
                                .truncate()
                                .number_of_lines(1),
                        )
                        .child(circle_tag_button(tag, tag_active, false, None, move || {
                            store.toggle_circle_filter(&tag_click)
                        })),
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            text(format!(
                                "{} {price_txt} · {} {oe_txt}",
                                l.t("现价", "Price"),
                                l.t("严苛 OE", "Strict OE"),
                            ))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(quick_move_strip(store, &rep.ticker, rep.bucket)),
                ),
        )
        .child(
            text(rep.too_hard_reason.clone().unwrap_or_else(|| {
                l.t("已拒绝或能力圈外。", "Rejected or Too Hard.")
                    .to_owned()
            }))
            .font_family(FONT_UI)
            .text_size(px(TEXT_BASE))
            .line_height(px(20.0))
            .text_color(rgb(INK_SECONDARY)),
        )
}

fn render_baskets_board(store: AppStore, reports: &[MungerReport], l: UiLang) -> View {
    let board_order = [Bucket::Watch, Bucket::Core, Bucket::Out, Bucket::TooHard];
    let basket_filter = store.basket_filter.get();
    let circle_filter = store.circle_filter.get();
    let status_filter = store.status_filter.get();
    let sort_col = store.sort_col.get();
    let sort_asc = store.sort_asc.get();
    let cursor_index = store.baskets_cursor_index.get();
    let mut linear_idx: usize = 0;

    view()
        .px(px(20.0))
        .pt(px(12.0))
        .pb(px(20.0))
        .flex()
        .flex_row()
        .gap(px(12.0))
        .children(board_order.into_iter().map(|bucket| {
            let (fg, bg, br) = bucket_colors(bucket);
            let is_focused = basket_filter == Some(bucket);
            let rows = filter_reports_for_basket(
                reports,
                bucket,
                circle_filter.as_deref(),
                status_filter,
                sort_col,
                sort_asc,
            );
            let cnt = rows.len();

            let mut body = view().p(px(10.0)).flex().flex_col().gap(px(8.0));
            if rows.is_empty() {
                body = body.child(
                    view()
                        .p(px(12.0))
                        .rounded_md()
                        .bg(rgb(BG_VIEWPORT))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .child(
                            text(l.t("暂无标的", "Empty").to_owned())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                );
            } else {
                for rep in rows {
                    let global_idx = linear_idx;
                    linear_idx += 1;
                    let is_cursor = global_idx == cursor_index;
                    let val = primary_val(rep, true);
                    let (s_fg, s_bg, s_br) = status_colors(rep.status);
                    let price_txt = fmt_price(rep.price.as_ref().map(|p| p.price), &rep.currency);
                    let dist_txt =
                        fmt_pct_clean(val.and_then(|v| v.distance_to_strike_pct), true).0;
                    let roic_txt = fmt_pct_clean(rep.roic_5y_avg_pct, false).0;
                    let tag = primary_circle_tag(&rep.circle);
                    let tag_active = circle_filter.as_deref() == Some(tag.as_str());
                    let tag_click = tag.clone();
                    let ticker_open = rep.ticker.clone();
                    let ticker_ctx = rep.ticker.clone();

                    let mid = match bucket {
                        Bucket::Core | Bucket::Watch => view()
                            .mt(px(8.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(6.0))
                            .child(
                                view().flex_1().min_w(px(0.0)).child(
                                    text(format!(
                                        "{} {dist_txt} · ROIC {roic_txt}",
                                        l.t("边际", "Diff")
                                    ))
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED))
                                    .truncate()
                                    .number_of_lines(1),
                                ),
                            )
                            .child(badge_pill(
                                rep.status.label_for(l).to_owned(),
                                s_fg,
                                s_bg,
                                s_br,
                            )),
                        Bucket::Out | Bucket::TooHard => view().mt(px(8.0)).child(
                            text(rep.too_hard_reason.clone().unwrap_or_else(|| {
                                l.t("已拒绝或能力圈外", "Rejected or Too Hard").to_owned()
                            }))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .line_height(px(16.0))
                            .text_color(rgb(INK_MUTED)),
                        ),
                    };

                    let card_top = view()
                        .cursor_pointer()
                        .on_press(move || {
                            store.baskets_cursor_index.set(global_idx);
                            store.open_company(&ticker_open, CompanyViewTab::FinancialsMemo);
                        })
                        .on_context_menu(move |e: &ContextMenuEvent| {
                            show_company_context_menu(store, ticker_ctx.clone(), e.page_position);
                        })
                        .child(
                            view()
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
                                            view()
                                                .w(px(6.0))
                                                .h(px(6.0))
                                                .rounded_full()
                                                .bg(rgb(status_dot_color(rep.status))),
                                        )
                                        .child(
                                            text(rep.ticker.clone())
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_SM))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(rgb(INK_HERO)),
                                        ),
                                )
                                .child(
                                    text(price_txt)
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_SM))
                                        .text_color(rgb(INK_PRIMARY)),
                                ),
                        )
                        .child(
                            view().mt(px(2.0)).child(
                                text(rep.name.clone())
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_SM))
                                    .text_color(rgb(INK_SECONDARY))
                                    .truncate()
                                    .number_of_lines(1),
                            ),
                        )
                        .child(mid);

                    let card_footer = view()
                        .mt(px(8.0))
                        .pt(px(6.0))
                        .border_t_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .child(circle_tag_button(tag, tag_active, false, None, move || {
                            store.toggle_circle_filter(&tag_click)
                        }))
                        .child(quick_move_strip(store, &rep.ticker, rep.bucket));

                    body = body.child(
                        view()
                            .p(px(10.0))
                            .rounded_md()
                            .bg(rgb(if is_cursor {
                                BG_SELECTED_ROW
                            } else {
                                BG_SURFACE
                            }))
                            .border_1()
                            .border_color(rgb(if is_cursor { INK_HERO } else { BORDER_SUBTLE }))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .flex()
                            .flex_col()
                            .child(card_top)
                            .child(card_footer),
                    );
                }
            }

            view()
                .flex_1()
                .min_w(px(0.0))
                .rounded_lg()
                .bg(rgb(BG_ROW_ALT))
                .border_1()
                .border_color(rgba(if is_focused {
                    br
                } else {
                    (BORDER_SUBTLE << 8) | 0xFF
                }))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    view()
                        .px(px(12.0))
                        .py(px(10.0))
                        .bg(rgba(if is_focused { bg } else { 0x161620FF }))
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
                                .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(fg)))
                                .child(
                                    text(bucket.code())
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(fg)),
                                )
                                .child(
                                    text(bucket.short_label_for(l))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO)),
                                ),
                        )
                        .child(
                            text(format!("{cnt}"))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_PRIMARY)),
                        ),
                )
                .child(body)
        }))
}

fn quick_move_strip(store: AppStore, ticker: &str, current: Bucket) -> View {
    let targets = [
        (Bucket::Core, "IN"),
        (Bucket::Watch, "W"),
        (Bucket::Out, "OUT"),
        (Bucket::TooHard, "TH"),
    ];
    view()
        .w(px(138.0))
        .flex_shrink_0()
        .px(px(4.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(px(4.0))
        .children(targets.into_iter().map(|(target, short)| {
            let active = target == current;
            let (fg, bg, br) = bucket_colors(target);
            let tk = ticker.to_owned();
            let mut pill = view()
                .px(px(6.0))
                .py(px(2.0))
                .rounded_xs()
                .border_1()
                .cursor_pointer()
                .on_press(move || {
                    if !active {
                        store.move_company_bucket(&tk, target);
                    }
                })
                .child(
                    text(short)
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .font_weight(if active {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(rgb(if active { fg } else { INK_MUTED })),
                );
            if active {
                pill = pill.bg(rgba(bg)).border_color(rgba(br));
            } else {
                pill = pill
                    .bg(rgb(BG_WELL))
                    .border_color(rgb(BORDER_SUBTLE))
                    .hover(|s| s.bg(rgb(BG_HOVER)));
            }
            pill
        }))
}
