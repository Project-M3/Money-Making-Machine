//! Event-Driven L1 Briefing Inbox view (`⌘1`).

use crate::components::{action_button, badge_pill, kbd, obsidian_card, section_header, v_divider};
use crate::format::{
    budget_watermark_summary_for, count_inbox_filter, filter_inbox_signals, fmt_money_compact,
    fmt_money_compact_opt, fmt_pct_clean, fmt_price, fmt_time_cn_for, resolve_active_inbox_signal,
    signal_kind_badge_for, signal_status_badge_for,
};
use crate::platform_desktop::{copy_text_with_notice, open_external_url};
use crate::stores::{AppStore, InboxFilter};
use crate::theme::*;
use m3_core::i18n::UiLang;
use m3_core::industry::{self, IndustryStatus};
use m3_core::lake::{UniverseDrawerDetails, UniverseRow};
use m3_core::lake_l1::{SignalEvidence, SignalInboxRow, SignalStatus};
use m3_core::model::WarningSeverity;
use nsh_core::prelude::*;
use rust_decimal::Decimal;

/// Renders the Event-Driven L1 Briefing Inbox (`⌘1`).
pub fn inbox_view(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(render_inbox_kpi_ribbon(store))
        .child(render_inbox_filter_bar(store))
        .child(when(
            move || store.inbox_error.with(|e| e.is_some()),
            move || {
                view()
                    .px(px(20.0))
                    .py(px(8.0))
                    .bg(rgba(CORAL_BG))
                    .border_b_1()
                    .border_color(rgba(CORAL_BORDER))
                    .child(
                        text(move || store.inbox_error.get().unwrap_or_default())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(CORAL_FG)),
                    )
            },
        ))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(dynamic(move || render_inbox_body(store))),
        )
}

fn render_inbox_kpi_ribbon(store: AppStore) -> View {
    view()
        .h(px(44.0))
        .flex_shrink_0()
        .px(px(20.0))
        .bg(rgb(BG_SURFACE))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let u_sum = store.universe_summary.get().unwrap_or_default();
                    let l1_sum = store.inbox_summary.get().unwrap_or_default();
                    let last_catchup = fmt_time_cn_for(l1_sum.last_catchup_ms, l);
                    let budget_rep = store.budget_report.get();
                    let (budget_txt, budget_col) =
                        budget_watermark_summary_for(budget_rep.as_ref(), l);

                    let universe_val = match l {
                        UiLang::Zh => format!("{} 家", u_sum.total_companies),
                        UiLang::En => format!("{}", u_sum.total_companies),
                    };
                    let sem_val = match l {
                        UiLang::Zh => format!(
                            "{} 片段 / {} 公司",
                            l1_sum.doc_chunks_count, l1_sum.indexed_companies_count
                        ),
                        UiLang::En => format!(
                            "{} chunks / {} cos",
                            l1_sum.doc_chunks_count, l1_sum.indexed_companies_count
                        ),
                    };
                    let unread_col = if l1_sum.unread_signals > 0 {
                        EMERALD_FG
                    } else {
                        INK_MUTED
                    };

                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(kpi_stat_inline(
                            l.t("全美股池", "US Universe"),
                            universe_val,
                            INK_HERO,
                        ))
                        .child(v_divider())
                        .child(kpi_stat_inline(
                            l.t("上次追平", "Last Catch-Up"),
                            last_catchup,
                            INK_SECONDARY,
                        ))
                        .child(v_divider())
                        .child(kpi_stat_inline(
                            l.t("语义索引", "Semantic Index"),
                            sem_val,
                            SKY_FG,
                        ))
                        .child(v_divider())
                        .child(kpi_stat_inline(
                            l.t("未读信号", "Unread"),
                            format!("{}", l1_sum.unread_signals),
                            unread_col,
                        ))
                        .child(v_divider())
                        .child(
                            text(budget_txt)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(budget_col))
                                .number_of_lines(1),
                        )
                })),
        )
        .child(
            view()
                .flex_shrink_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(when(
                    move || store.inbox_catching_up.get(),
                    move || {
                        text(move || store.inbox_catchup_progress.get().unwrap_or_default())
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(SKY_FG))
                            .truncate()
                            .number_of_lines(1)
                    },
                ))
                .child(action_button(
                    move || {
                        let l = store.lang.get();
                        if store.inbox_catching_up.get() {
                            l.t("追平中…", "Catching Up…").to_owned()
                        } else {
                            l.t("追平事件流", "Catch Up L1").to_owned()
                        }
                    },
                    true,
                    move || store.start_l1_catchup(false),
                )),
        )
}

fn kpi_stat_inline(label: &str, value: String, color: u32) -> View {
    view()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(6.0))
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED))
                .number_of_lines(1),
        )
        .child(
            text(value)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(color))
                .number_of_lines(1),
        )
}

fn render_inbox_filter_bar(store: AppStore) -> View {
    view()
        .flex_shrink_0()
        .px(px(20.0))
        .py(px(8.0))
        .bg(rgb(BG_ROW_ALT))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(dynamic(move || {
            let l = store.lang.get();
            let active_filter = store.inbox_state.with(|st| st.filter);
            let signals = store.inbox_signals.get();

            view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(6.0))
                .children(InboxFilter::all().into_iter().map(|f| {
                    let active = active_filter == f;
                    let count = count_inbox_filter(&signals, f);
                    let display_lbl = f.label_for(l);
                    let label = if count > 0 || matches!(f, InboxFilter::Unread | InboxFilter::All)
                    {
                        format!("{display_lbl} {count}")
                    } else {
                        display_lbl.to_owned()
                    };
                    let mut btn = view()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded_xs()
                        .cursor_pointer()
                        .border_1()
                        .on_press(move || store.set_inbox_filter(f))
                        .child(
                            text(label)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(if active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(rgb(if active { EMERALD_FG } else { INK_SECONDARY }))
                                .number_of_lines(1),
                        );
                    if active {
                        btn = btn.bg(rgba(EMERALD_BG)).border_color(rgba(EMERALD_BORDER));
                    } else {
                        btn = btn
                            .bg(rgb(BG_SURFACE))
                            .border_color(rgb(BORDER_SUBTLE))
                            .hover(|s| s.bg(rgb(BG_HOVER)));
                    }
                    btn
                }))
        }))
        .child(dynamic(move || {
            let l = store.lang.get();
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(kbd("j/k"))
                .child(kbd(l.t("w 观察", "w Watch")))
                .child(kbd("x TooHard"))
                .child(kbd(l.t("e 归档", "e Archive")))
                .child(kbd("d Alan"))
                .child(kbd(l.t("r 追平", "r Catch-Up")))
        }))
}

fn render_inbox_body(store: AppStore) -> View {
    let l = store.lang.get();
    let lake_present = store.universe_lake_present.get();
    let lake_summary = store.universe_summary.get();
    let lake_empty = !lake_present
        || lake_summary
            .as_ref()
            .is_some_and(|s| s.total_companies == 0);
    let signals = store.inbox_signals.get();
    let st = store.inbox_state.get();

    if lake_empty && signals.is_empty() {
        return view().flex_1().min_h(px(0.0)).flex().flex_col().child(
            scroll().flex_1().min_h(px(0.0)).child(
                view()
                    .p(px(24.0))
                    .child(render_inbox_empty_lake_state(store, l)),
            ),
        );
    }

    let visible = filter_inbox_signals(&signals, st.filter);
    let active =
        resolve_active_inbox_signal(&visible, st.selected_signal_id.as_deref(), st.cursor_index);

    let list_inner = if visible.is_empty() {
        render_inbox_empty_filter_state(store, st.filter, l)
    } else {
        let active_id = active.map(|(_, a)| a.signal_id.clone());
        view()
            .flex()
            .flex_col()
            .children(visible.iter().enumerate().map(|(idx, sig)| {
                let is_selected = active_id.as_deref() == Some(sig.signal_id.as_str());
                render_inbox_row(store, sig, idx, is_selected, l)
            }))
    };

    let detail_inner = match active {
        Some((_, sig)) => render_inbox_detail(store, sig, l),
        None => render_inbox_no_selection(l),
    };

    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_row()
        .overflow_hidden()
        .child(
            view()
                .w(px(430.0))
                .flex_shrink_0()
                .h_full()
                .border_r_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .bg(rgb(BG_VIEWPORT))
                .flex()
                .flex_col()
                .child(scroll().flex_1().min_h(px(0.0)).child(list_inner)),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .flex()
                .flex_col()
                .child(
                    scroll()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(view().p(px(20.0)).child(detail_inner)),
                ),
        )
}

fn render_inbox_row(
    store: AppStore,
    sig: &SignalInboxRow,
    idx: usize,
    is_selected: bool,
    l: UiLang,
) -> View {
    let (kind_label, kind_fg, kind_bg, kind_br) = signal_kind_badge_for(sig.kind, l);
    let (st_label, st_fg, st_bg, st_br) = signal_status_badge_for(sig.status, l);
    let sig_id = sig.signal_id.clone();
    let secondary_tickers: Vec<&str> = sig
        .tickers
        .iter()
        .filter(|t| !t.eq_ignore_ascii_case(&sig.primary_ticker))
        .take(3)
        .map(String::as_str)
        .collect();
    let date_label = fmt_time_cn_for(Some(sig.updated_at_ms), l);

    let mut row = view()
        .px(px(16.0))
        .py(px(12.0))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .cursor_pointer()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .on_press(move || store.select_inbox_signal(&sig_id, idx));

    if is_selected {
        row = row.bg(rgb(BG_SELECTED_ROW));
    } else {
        row = row
            .bg(rgb(if idx % 2 == 1 {
                BG_ROW_ALT
            } else {
                BG_VIEWPORT
            }))
            .hover(|s| s.bg(rgb(BG_HOVER)));
    }

    let mut top_left = view()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0));
    if sig.status == SignalStatus::Unread {
        top_left = top_left.child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(kind_fg)));
    }
    top_left = top_left
        .child(badge_pill(kind_label, kind_fg, kind_bg, kind_br))
        .child(
            text(sig.primary_ticker.clone())
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(INK_HERO))
                .number_of_lines(1),
        );
    if !secondary_tickers.is_empty() {
        top_left = top_left.child(
            text(format!("← {}", secondary_tickers.join(", ")))
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED))
                .truncate()
                .number_of_lines(1),
        );
    }

    let mut top_right = view()
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0));
    if sig.status != SignalStatus::Unread {
        top_right = top_right.child(badge_pill(st_label, st_fg, st_bg, st_br));
    }
    top_right = top_right.child(
        text(format!("{:.2}", sig.score))
            .font_family(FONT_MONO_NUM)
            .text_size(px(TEXT_2XS))
            .text_color(rgb(INK_MUTED)),
    );

    row.child(
        view()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .child(top_left)
            .child(top_right),
    )
    .child(
        text(sig.title.clone())
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .font_weight(if sig.status == SignalStatus::Unread {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(rgb(if is_selected { INK_HERO } else { INK_PRIMARY }))
            .truncate()
            .number_of_lines(1),
    )
    .child(
        view()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .child(
                view().flex_1().min_w(px(0.0)).child(
                    text(sig.summary.clone())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED))
                        .truncate()
                        .number_of_lines(1),
                ),
            )
            .child(
                text(format!(
                    "{} {} · {}",
                    sig.evidence.len(),
                    l.t("条证据", "ev"),
                    date_label
                ))
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED))
                .number_of_lines(1),
            ),
    )
}

fn render_inbox_detail(store: AppStore, sig: &SignalInboxRow, l: UiLang) -> View {
    let (kind_label, kind_fg, kind_bg, kind_br) = signal_kind_badge_for(sig.kind, l);
    let (st_label, st_fg, st_bg, st_br) = signal_status_badge_for(sig.status, l);
    let ticker_open = sig.primary_ticker.clone();
    let id_watch = sig.signal_id.clone();
    let id_alan = sig.signal_id.clone();
    let id_toohard = sig.signal_id.clone();
    let id_archive = sig.signal_id.clone();
    let id_unread = sig.signal_id.clone();

    let reports = store.reports.get();
    let report_opt = reports
        .iter()
        .find(|r| r.ticker.eq_ignore_ascii_case(&sig.primary_ticker));
    let in_watchlist = report_opt.is_some();

    let details_snap = store.inbox_selected_details.get();
    let details_opt = details_snap
        .as_ref()
        .filter(|d| d.row.ticker.eq_ignore_ascii_case(&sig.primary_ticker));

    let industries = store.industries.get();
    let matched_ind = industry::find_industry_for_company(
        &industries,
        &sig.primary_ticker,
        details_opt.and_then(|d| d.row.sic),
    );
    let (circle_badge_label, cb_fg, cb_bg, cb_br) = if let Some(rep) = report_opt {
        let (fg, bg, br) = bucket_colors(rep.bucket);
        (
            match l {
                UiLang::Zh => format!("能力圈 · {}", rep.bucket.code()),
                UiLang::En => format!("Circle · {}", rep.bucket.code()),
            },
            fg,
            bg,
            br,
        )
    } else if let Some(ind) = matched_ind {
        let (fg, bg, br) = industry_status_colors(ind.status);
        let lbl = match (ind.status, l) {
            (IndustryStatus::InsideCircle, UiLang::Zh) => format!("能力圈行业 · {}", ind.title),
            (IndustryStatus::InsideCircle, UiLang::En) => format!("Circle Ind. · {}", ind.title),
            (IndustryStatus::Studying, UiLang::Zh) => format!("行业学习中 · {}", ind.title),
            (IndustryStatus::Studying, UiLang::En) => format!("Studying · {}", ind.title),
            (IndustryStatus::TooHard, UiLang::Zh) => format!("行业 Too Hard · {}", ind.title),
            (IndustryStatus::TooHard, UiLang::En) => format!("Ind. Too Hard · {}", ind.title),
        };
        (lbl, fg, bg, br)
    } else if let Some(det) = details_opt {
        let lbl = match (l, det.row.sic) {
            (UiLang::Zh, Some(sic)) => format!("未研究行业 · SIC {sic}"),
            (UiLang::En, Some(sic)) => format!("Unstudied · SIC {sic}"),
            (UiLang::Zh, None) => "未研究行业".into(),
            (UiLang::En, None) => "Unstudied Industry".into(),
        };
        (lbl, SLATE_FG, SLATE_BG, SLATE_BORDER)
    } else {
        (
            l.t("未入能力圈", "Outside Circle").to_owned(),
            SLATE_FG,
            SLATE_BG,
            SLATE_BORDER,
        )
    };

    let ind_ticker = sig.primary_ticker.clone();
    let ind_name = details_opt
        .map(|d| d.row.name.clone())
        .or_else(|| report_opt.map(|r| r.name.clone()))
        .unwrap_or_else(|| sig.primary_ticker.clone());
    let ind_sic = details_opt.and_then(|d| d.row.sic);
    let ind_sic_desc = details_opt.and_then(|d| d.row.sic_description.clone());

    let mut actions = view()
        .pt(px(8.0))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(px(8.0))
        .child(inbox_action_button(
            if in_watchlist {
                l.t("打开研报 Dossier", "Open Dossier")
            } else {
                l.t("在全市场雷达查看", "Open in Radar")
            },
            "↵",
            true,
            (CTA_INK, CTA_BG, CTA_BG),
            move || store.open_inbox_signal_target(&ticker_open),
        ))
        .child(inbox_action_button(
            l.t("研究所在行业", "Research Industry"),
            "⌘3",
            false,
            (SKY_FG, SKY_BG, SKY_BORDER),
            move || {
                store.open_or_create_industry_for_company(
                    &ind_ticker,
                    &ind_name,
                    ind_sic,
                    ind_sic_desc.as_deref(),
                );
            },
        ))
        .child(inbox_action_button(
            l.t("入观察池 WATCH", "Watch"),
            "w",
            false,
            (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
            move || store.triage_inbox_signal(&id_watch, SignalStatus::Watch, true, false),
        ))
        .child(inbox_action_button(
            l.t("Deep Research · Alan", "Deep Research"),
            "d",
            false,
            (VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
            move || store.triage_inbox_signal(&id_alan, SignalStatus::Researching, false, true),
        ))
        .child(inbox_action_button(
            "Too Hard",
            "x",
            false,
            (SLATE_FG, SLATE_BG, SLATE_BORDER),
            move || store.triage_inbox_signal(&id_toohard, SignalStatus::TooHard, false, false),
        ))
        .child(inbox_action_button(
            l.t("归档", "Archive"),
            "e",
            false,
            (INK_SECONDARY, BG_WELL, BORDER_SUBTLE),
            move || store.triage_inbox_signal(&id_archive, SignalStatus::Archived, false, false),
        ));

    if sig.status != SignalStatus::Unread {
        actions = actions.child(inbox_action_button(
            l.t("标为未读", "Unread"),
            "u",
            false,
            (SKY_FG, SKY_BG, SKY_BORDER),
            move || store.triage_inbox_signal(&id_unread, SignalStatus::Unread, false, false),
        ));
    }

    let header_card = obsidian_card()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            view()
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
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(badge_pill(kind_label, kind_fg, kind_bg, kind_br))
                        .child(badge_pill(st_label, st_fg, st_bg, st_br))
                        .child(badge_pill(circle_badge_label, cb_fg, cb_bg, cb_br))
                        .child(
                            text(sig.primary_ticker.clone())
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_LG))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .children(sig.tickers.iter().map(|t| {
                            let tk = t.clone();
                            view()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded_xs()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .on_press(move || store.open_inbox_signal_target(&tk))
                                .child(
                                    text(t.clone())
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                )
                        })),
                )
                .child(
                    text(format!(
                        "Score {:.2} · {}",
                        sig.score,
                        fmt_time_cn_for(Some(sig.updated_at_ms), l)
                    ))
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED))
                    .number_of_lines(1),
                ),
        )
        .child(
            text(sig.title.clone())
                .font_family(FONT_UI)
                .text_size(px(TEXT_LG))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        )
        .child(
            text(sig.summary.clone())
                .font_family(FONT_UI)
                .text_size(px(TEXT_BASE))
                .line_height(px(21.0))
                .text_color(rgb(INK_PRIMARY)),
        )
        .child(actions);

    let mut root = view()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(header_card)
        .child(render_inbox_business_profile_card(
            store,
            sig,
            details_opt,
            l,
        ));

    if let Some(det) = details_opt {
        root = root.child(render_inbox_financial_snapshot_card(
            store,
            det,
            in_watchlist,
            l,
        ));
    }

    let mut ev_section = view().flex().flex_col().gap(px(10.0)).child(section_header(
        l.t("一手原文出处证据", "PRIMARY SOURCE EVIDENCE"),
        format!(
            "{} {} · 0 Flagship Token",
            sig.evidence.len(),
            l.t("条检索片段", "retrieved chunks")
        ),
    ));

    if sig.evidence.is_empty() {
        ev_section = ev_section.child(
            obsidian_card().p(px(16.0)).child(
                text(
                    l.t(
                        "该信号暂无附带原文引用片段。",
                        "No source excerpts attached to this signal.",
                    )
                    .to_owned(),
                )
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_MUTED)),
            ),
        );
    } else {
        ev_section = ev_section.children(
            sig.evidence
                .iter()
                .map(|ev| render_evidence_card(store, ev, l)),
        );
    }

    root.child(ev_section)
}

fn render_inbox_business_profile_card(
    store: AppStore,
    sig: &SignalInboxRow,
    details_opt: Option<&UniverseDrawerDetails>,
    l: UiLang,
) -> View {
    let ticker_upper = sig.primary_ticker.trim().to_ascii_uppercase();
    let chunks = store.inbox_selected_business_chunks.get();
    let narrative_chunks: Vec<&SignalEvidence> = chunks
        .iter()
        .filter(|c| {
            c.ticker.eq_ignore_ascii_case(&ticker_upper)
                && !c.section.eq_ignore_ascii_case("company_profile")
        })
        .collect();

    let item1_chunks: Vec<&&SignalEvidence> = narrative_chunks
        .iter()
        .filter(|c| c.section == "item_1")
        .take(2)
        .collect();
    let risk_chunk = narrative_chunks
        .iter()
        .find(|c| c.section == "item_1a" || c.section == "item_7")
        .copied();

    let cik_opt = details_opt.map(|d| d.row.cik.clone());
    let sic_line = details_opt.map(|d| format_sic_line(&d.row, l));
    let fetching_ticker = store.inbox_profile_fetching_ticker.get();
    let is_fetching = fetching_ticker
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case(&ticker_upper));
    let id_alan = sig.signal_id.clone();

    let filing_tag = narrative_chunks
        .first()
        .and_then(|c| c.filing_date.as_deref())
        .map(|d| format!("SEC 10-K · {d}"))
        .unwrap_or_else(|| l.t("公司业务与行业档案", "BUSINESS & SECTOR").to_owned());

    let mut card = obsidian_card()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(section_header(
            l.t(
                "公司业务简介 · Business Profile",
                "Business Profile · 10-K Item 1",
            ),
            filing_tag,
        ));

    if let Some(sic) = sic_line {
        card = card.child(
            text(sic)
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_SECONDARY)),
        );
    }

    if !item1_chunks.is_empty() || risk_chunk.is_some() {
        for ch in item1_chunks {
            card = card.child(
                view()
                    .p(px(12.0))
                    .rounded_md()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        text(ch.title.clone())
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(SKY_FG)),
                    )
                    .child(
                        text(ch.quote.clone())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .line_height(px(20.0))
                            .text_color(rgb(INK_PRIMARY)),
                    ),
            );
        }
        if let Some(rc) = risk_chunk {
            card = card.child(
                view()
                    .p(px(12.0))
                    .rounded_md()
                    .bg(rgb(BG_SHELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        text(rc.title.clone())
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(GOLD_FG)),
                    )
                    .child(
                        text(rc.quote.clone())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .line_height(px(19.0))
                            .text_color(rgb(INK_SECONDARY)),
                    ),
            );
        }
        return card;
    }

    let mut cta_row = view()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(px(8.0));
    if let Some(cik_val) = cik_opt {
        let tk_fetch = ticker_upper;
        cta_row = cta_row.child(
            view()
                .px(px(12.0))
                .py(px(6.0))
                .rounded_sm()
                .cursor_pointer()
                .bg(rgba(SKY_BG))
                .border_1()
                .border_color(rgba(SKY_BORDER))
                .hover(|s| s.bg(rgb(BG_HOVER)))
                .on_press(move || store.trigger_profile_fetch(&tk_fetch, &cik_val))
                .child(
                    text(
                        if is_fetching {
                            l.t("正在拉取 10-K 业务原文…", "Fetching 10-K Profile…")
                        } else {
                            l.t("拉取 10-K 业务简介 (0 LLM)", "Fetch 10-K Profile (0 LLM)")
                        }
                        .to_owned(),
                    )
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(SKY_FG)),
                ),
        );
    }
    cta_row = cta_row.child(
        view()
            .px(px(12.0))
            .py(px(6.0))
            .rounded_sm()
            .cursor_pointer()
            .bg(rgba(VIOLET_BG))
            .border_1()
            .border_color(rgba(VIOLET_BORDER))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .on_press(move || {
                store.triage_inbox_signal(&id_alan, SignalStatus::Researching, false, true);
            })
            .child(
                text(l.t("派 Alan 快速解读", "Ask Alan").to_owned())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(VIOLET_FG)),
            ),
    );

    card.child(
        text(
            l.t(
                "本地仅缓存了代码与行业元数据，尚未下载最新 SEC 10-K 业务原文。可一键零 LLM 拉取 10-K Item 1 业务说明与核心风险，或直接派 Alan 快速梳理。",
                "Only registration metadata is indexed locally; latest 10-K Item 1 business text has not been fetched yet. Pull 10-K sections below (0 LLM tokens) or ask Alan.",
            )
            .to_owned(),
        )
        .font_family(FONT_UI)
        .text_size(px(TEXT_SM))
        .line_height(px(19.0))
        .text_color(rgb(INK_MUTED)),
    )
    .child(cta_row)
}

fn render_inbox_financial_snapshot_card(
    store: AppStore,
    details: &UniverseDrawerDetails,
    in_watchlist: bool,
    l: UiLang,
) -> View {
    let row = &details.row;
    let roic_txt = row
        .roic_5y_avg_pct
        .map(|v| format!("{:.1}%", v.round_dp(1)))
        .unwrap_or_else(|| "—".into());
    let gm_txt = row
        .gross_margin_5y_avg_pct
        .map(|v| format!("{:.1}%", v.round_dp(1)))
        .unwrap_or_else(|| "—".into());
    let (fcf_txt, fcf_pol) = fmt_pct_clean(row.strict_fcf_yield_pct, false);
    let (sbc_txt, _) = fmt_pct_clean(row.sbc_to_ocf_pct, false);
    let (share_txt, share_pol) = fmt_pct_clean(row.share_change_3y_pct, true);
    let nd_txt = row
        .net_debt_to_oe
        .map(|v| format!("{v:.2}x"))
        .unwrap_or_else(|| "—".into());

    let kpi_tile = |label: &str, val: String, val_col: u32| -> View {
        view()
            .flex_1()
            .min_w(px(0.0))
            .p(px(10.0))
            .rounded_md()
            .bg(rgb(BG_VIEWPORT))
            .border_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                text(label.to_owned())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED)),
            )
            .child(
                text(val)
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(val_col))
                    .truncate()
                    .number_of_lines(1),
            )
    };

    let mut card = obsidian_card()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(section_header(
            l.t(
                "确定性 5Y 财务快照 & 同业对照",
                "Deterministic 5Y Financials & Peers",
            ),
            "DUCKDB L0 · 0 LLM TOKEN".to_owned(),
        ));

    if !in_watchlist {
        card = card.child(
            view()
                .px(px(12.0))
                .py(px(6.0))
                .rounded_sm()
                .bg(rgb(BG_SHELL))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .child(
                    text(
                        l.t(
                            "未入能力圈 · 仅展示历史确定性财务与同业分位；认可行业与商业模式后再入观察池并设定个人击球倍数。",
                            "Outside Circle · showing historical deterministic metrics and peers only; add to Watch and set personal multiples after understanding the business.",
                        )
                        .to_owned(),
                    )
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                ),
        );
    }

    card = card
        .child(
            view()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(kpi_tile(
                    l.t("最新股价", "Price"),
                    fmt_price(row.latest_price, "USD"),
                    INK_HERO,
                ))
                .child(kpi_tile("5Y ROIC", roic_txt, EMERALD_FG))
                .child(kpi_tile(
                    l.t("严苛 FCF%", "Strict FCF%"),
                    fcf_txt,
                    if fcf_pol < 0 { CORAL_FG } else { INK_PRIMARY },
                ))
                .child(kpi_tile(
                    l.t("5Y 毛利率", "5Y Gross Margin"),
                    gm_txt,
                    INK_PRIMARY,
                )),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(kpi_tile("SBC / OCF", sbc_txt, INK_SECONDARY))
                .child(kpi_tile(
                    l.t("3Y 股本变化", "3Y Shrs Δ"),
                    share_txt,
                    share_delta_color(row.share_change_3y_pct, share_pol),
                ))
                .child(kpi_tile(
                    l.t("净负债 / OE", "Net Debt / OE"),
                    nd_txt,
                    INK_SECONDARY,
                ))
                .child(kpi_tile(
                    l.t("质量综合分", "Quality Score"),
                    format!("{:.1}", row.quality_score),
                    SKY_FG,
                )),
        );

    if in_watchlist && (row.strike_price.is_some() || row.distance_to_strike_pct.is_some()) {
        let (dist_txt, _) = fmt_pct_clean(row.distance_to_strike_pct, true);
        let dist_col = if row
            .distance_to_strike_pct
            .is_some_and(|d| d <= Decimal::ZERO)
        {
            EMERALD_FG
        } else {
            GOLD_FG
        };
        card = card.child(
            view()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(kpi_tile(
                    l.t("个人击球价", "Strike Price"),
                    fmt_price(row.strike_price, "USD"),
                    INK_HERO,
                ))
                .child(kpi_tile(
                    l.t("距击球价", "Dist. to Strike"),
                    dist_txt,
                    dist_col,
                )),
        );
    }

    if !details.annuals.is_empty() {
        let recent: Vec<_> = details.annuals.iter().rev().take(5).collect();
        let recent_len = recent.len();
        let mut tbl = view()
            .rounded_md()
            .bg(rgb(BG_VIEWPORT))
            .border_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .px(px(12.0))
            .py(px(6.0))
            .flex()
            .flex_col()
            .child(
                view()
                    .flex()
                    .flex_row()
                    .items_center()
                    .py(px(4.0))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        view().w(px(82.0)).flex_shrink_0().child(
                            text(l.t("财年", "FY"))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                    )
                    .child(
                        view().flex_1().flex().flex_row().justify_end().child(
                            text(l.t("营收", "Revenue"))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                    )
                    .child(
                        view().w(px(64.0)).flex().flex_row().justify_end().child(
                            text(l.t("毛利%", "GM%"))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                    )
                    .child(
                        view().w(px(64.0)).flex().flex_row().justify_end().child(
                            text("ROIC")
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                    )
                    .child(
                        view().flex_1().flex().flex_row().justify_end().child(
                            text(l.t("严苛OE", "Strict OE"))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                    ),
            );
        for (idx, a) in recent.into_iter().enumerate() {
            let gm = a
                .gross_margin_pct
                .map(|v| format!("{:.1}%", v.round_dp(1)))
                .unwrap_or_else(|| "—".into());
            let roic = a
                .roic_pct
                .map(|v| format!("{:.1}%", v.round_dp(1)))
                .unwrap_or_else(|| "—".into());
            let oe_col = match a.strict_owner_earnings {
                Some(v) if v > Decimal::ZERO => EMERALD_FG,
                Some(_) => CORAL_FG,
                None => INK_MUTED,
            };
            let mut tr = view()
                .flex()
                .flex_row()
                .items_center()
                .py(px(4.0))
                .child(
                    view().w(px(82.0)).flex_shrink_0().child(
                        text(a.period_end.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_SECONDARY)),
                    ),
                )
                .child(
                    view().flex_1().flex().flex_row().justify_end().child(
                        text(fmt_money_compact(a.revenue, "USD"))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_PRIMARY)),
                    ),
                )
                .child(
                    view().w(px(64.0)).flex().flex_row().justify_end().child(
                        text(gm)
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_PRIMARY)),
                    ),
                )
                .child(
                    view().w(px(64.0)).flex().flex_row().justify_end().child(
                        text(roic)
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(EMERALD_FG)),
                    ),
                )
                .child(
                    view().flex_1().flex().flex_row().justify_end().child(
                        text(fmt_money_compact_opt(a.strict_owner_earnings, "USD"))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(oe_col)),
                    ),
                );
            if idx + 1 < recent_len {
                tr = tr.border_b_1().border_color(rgb(BORDER_HAIRLINE));
            }
            tbl = tbl.child(tr);
        }
        card = card.child(tbl);
    }

    if let Some(peers) = details.peers.as_ref().filter(|p| p.peer_count > 1) {
        let roic_p = peers
            .roic_percentile
            .map(|p| format!("P{p:.0}"))
            .unwrap_or_else(|| "—".into());
        let gm_p = peers
            .gross_margin_percentile
            .map(|p| format!("P{p:.0}"))
            .unwrap_or_else(|| "—".into());
        let fcf_p = peers
            .strict_fcf_yield_percentile
            .map(|p| format!("P{p:.0}"))
            .unwrap_or_else(|| "—".into());

        let mut peer_bar = view()
            .pt(px(4.0))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(8.0))
            .child(
                text(match l {
                    UiLang::Zh => format!(
                        "{} · {} 家同业分位: ROIC {roic_p} · 毛利 {gm_p} · FCF {fcf_p}",
                        peers.sic_group, peers.peer_count
                    ),
                    UiLang::En => format!(
                        "{} · {} peers: ROIC {roic_p} · GM {gm_p} · FCF {fcf_p}",
                        peers.sic_group, peers.peer_count
                    ),
                })
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_XS))
                .text_color(rgb(SKY_FG)),
            );

        if !peers.top_peers.is_empty() {
            peer_bar = peer_bar.child(
                view()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_wrap()
                    .gap(px(6.0))
                    .children(peers.top_peers.iter().take(4).map(|p| {
                        let jump = p.ticker.clone();
                        view()
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded_xs()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_SUBTLE))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .on_press(move || store.open_inbox_signal_target(&jump))
                            .child(
                                text(p.ticker.clone())
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_SECONDARY)),
                            )
                    })),
            );
        }
        card = card.child(peer_bar);
    }

    let warnings = row.parsed_warnings();
    if !warnings.is_empty() {
        card = card.child(view().flex().flex_row().flex_wrap().gap(px(6.0)).children(
            warnings.into_iter().map(|w| {
                let (fg, bg, br) = match w.severity {
                    WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                    WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                    WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
                };
                badge_pill(w.title, fg, bg, br)
            }),
        ));
    }

    card
}

fn render_evidence_card(store: AppStore, ev: &SignalEvidence, l: UiLang) -> View {
    let copy_url = ev.source_url.clone();
    let open_url_str = ev.source_url.clone();
    let copy_label = format!("{} {}", ev.ticker, ev.doc_type);
    let date_str = ev.filing_date.as_deref().unwrap_or("—");
    let has_url = !ev.source_url.trim().is_empty();

    let mut right_meta = view().flex().flex_row().items_center().gap(px(8.0)).child(
        text(format!(
            "{} {:.0}%",
            l.t("相似度", "Sim"),
            ev.similarity * 100.0
        ))
        .font_family(FONT_MONO_NUM)
        .text_size(px(TEXT_XS))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(EMERALD_FG)),
    );

    if has_url {
        right_meta = right_meta
            .child(
                view()
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded_xs()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_PILL))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .cursor_pointer()
                    .on_press(move || {
                        copy_text_with_notice(store, copy_url.clone(), &copy_label);
                    })
                    .child(
                        text(l.t("复制 URL", "Copy URL"))
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_SECONDARY)),
                    ),
            )
            .child(
                view()
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded_xs()
                    .bg(rgba(SKY_BG))
                    .border_1()
                    .border_color(rgba(SKY_BORDER))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .cursor_pointer()
                    .on_press(move || open_external_url(&open_url_str))
                    .child(
                        text(l.t("打开原文 ↗", "Open SEC ↗"))
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(SKY_FG)),
                    ),
            );
    }

    let mut card = obsidian_card()
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            view()
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
                        .flex_wrap()
                        .gap(px(6.0))
                        .child(badge_pill(ev.doc_type.clone(), SKY_FG, SKY_BG, SKY_BORDER))
                        .child(
                            view()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded_xs()
                                .bg(rgb(BG_WELL))
                                .child(
                                    text(ev.section.clone())
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                ),
                        )
                        .child(
                            text(format!("{} · {}", ev.ticker, date_str))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(right_meta),
        );

    if !ev.title.trim().is_empty() {
        card = card.child(
            text(ev.title.clone())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        );
    }

    card = card.child(
        view()
            .px(px(14.0))
            .py(px(10.0))
            .rounded_md()
            .bg(rgb(BG_SHELL))
            .border_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(
                text(ev.quote.clone())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .line_height(px(19.0))
                    .text_color(rgb(INK_PRIMARY)),
            ),
    );

    if has_url {
        card = card.child(
            text(ev.source_url.clone())
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED))
                .truncate()
                .number_of_lines(1),
        );
    }

    card
}

fn inbox_action_button(
    label: &'static str,
    key_hint: &'static str,
    primary: bool,
    colors: (u32, u32, u32),
    on_press: impl Fn() + 'static,
) -> View {
    let (fg, bg, border) = colors;
    let mut el = view()
        .px(px(10.0))
        .py(px(6.0))
        .rounded_sm()
        .cursor_pointer()
        .border_1()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .on_press(on_press);
    if primary {
        el = el
            .bg(rgb(bg))
            .border_color(rgb(border))
            .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
            .active(|s| s.bg(rgb(CTA_BG_ACTIVE)));
    } else {
        el = el
            .bg(rgba(bg))
            .border_color(rgba(border))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .active(|s| s.bg(rgb(BG_ACTIVE)));
    }
    el.child(
        text(label)
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(fg))
            .number_of_lines(1),
    )
    .child(kbd(key_hint))
}

fn render_inbox_empty_lake_state(store: AppStore, l: UiLang) -> View {
    let cli = "m3 events catchup --bootstrap";
    obsidian_card()
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(section_header(
            l.t(
                "L1 事件流与简报收件箱尚未初始化",
                "L1 Event Stream & Briefing Inbox Not Initialized",
            ),
            "MILESTONE B · ZERO-FLAGSHIP-LLM".to_owned(),
        ))
        .child(
            text(
                l.t(
                    "先同步全美股 L0 湖仓，或点击「立即追平事件流」拉取 SEC EDGAR submissions / 8-K / 10-K 与公开新闻片段，自动生成「量化候选」「隐空间传导」「击球与红灯」待审讯简报。",
                    "Sync the US L0 lakehouse or click Catch Up Now to ingest SEC EDGAR submissions / 8-K / 10-K and public news snippets into Quant Screen, Ripple, Strike & Red-Flag briefings.",
                )
                .to_owned(),
            )
            .font_family(FONT_UI)
            .text_size(px(TEXT_BASE))
            .line_height(px(22.0))
            .text_color(rgb(INK_SECONDARY)),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(12.0))
                .child(action_button(
                    l.t("立即追平事件流", "Catch Up Now"),
                    true,
                    move || store.start_l1_catchup(true),
                ))
                .child(
                    view()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded_md()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_PILL))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .cursor_pointer()
                        .on_press(move || {
                            copy_text_with_notice(store, cli.to_owned(), "events catchup");
                        })
                        .child(
                            text(cli)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(INK_PRIMARY)),
                        ),
                ),
        )
}

fn render_inbox_empty_filter_state(store: AppStore, filter: InboxFilter, l: UiLang) -> View {
    view()
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            text(match l {
                UiLang::Zh => format!("「{}」下暂无信号", filter.label_for(l)),
                UiLang::En => format!("No signals under \"{}\"", filter.label_for(l)),
            })
            .font_family(FONT_UI)
            .text_size(px(TEXT_MD))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(INK_HERO)),
        )
        .child(
            text(
                l.t(
                    "可点击右上角「追平事件流」拉取最新 SEC 公告与候选新闻，或切换至「全部」查看历史已分拣简报。",
                    "Click Catch Up above to fetch new SEC filings and candidate news, or switch to All to inspect triaged briefings.",
                )
                .to_owned(),
            )
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .line_height(px(18.0))
            .text_color(rgb(INK_MUTED)),
        )
        .child(
            view().flex().flex_row().gap(px(8.0)).child(action_button(
                l.t("查看全部信号", "View All Signals"),
                false,
                move || store.set_inbox_filter(InboxFilter::All),
            )),
        )
}

fn render_inbox_no_selection(l: UiLang) -> View {
    obsidian_card().p(px(24.0)).child(
        text(
            l.t(
                "在左侧列表选择一条信号，或按 r 追平 L1 事件流。",
                "Select a signal on the left, or press r to catch up L1 events.",
            )
            .to_owned(),
        )
        .font_family(FONT_UI)
        .text_size(px(TEXT_SM))
        .text_color(rgb(INK_MUTED)),
    )
}

fn format_sic_line(row: &UniverseRow, lang: UiLang) -> String {
    match (
        row.sic,
        row.sic_description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (Some(sic), Some(desc)) => format!("{} · SIC {sic} ({desc})", row.exchange),
        (Some(sic), None) => format!("{} · SIC {sic}", row.exchange),
        (None, Some(desc)) => format!(
            "{} · {desc} ({})",
            row.exchange,
            lang.t("未分类SIC", "Unclassified SIC")
        ),
        (None, None) => format!(
            "{} · {}",
            row.exchange,
            lang.t("未分类SIC", "Unclassified SIC")
        ),
    }
}
