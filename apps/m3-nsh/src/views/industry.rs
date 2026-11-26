//! Industry Research Studio view (`apps/m3-nsh/src/views/industry.rs`):
//! - Left rail: Topic composer, Quick Bind Company composer, status filter bar,
//!   scrollable industry cards (status badge, peer count, Feynman progress), and
//!   SIC / Archetype Theme promotion rail.
//! - Main Industry Studio: Header & Human Graduation control, Human Sovereignty
//!   panel (valuation multiple presets + low/high inputs, clickable peer chips,
//!   Feynman mastery checklist, protected `## Human Notes` editor), L0 Peer Matrix
//!   with 10-K batch fetch, Living Industry Primer with strict `SEC 10-K` vs `Web`
//!   source separation, and Alan 4-Lens streaming + Feynman Quiz tutor.

use super::agent::feynman_coach_panel;
use crate::components::{
    action_button, badge_pill, concept_hint, obsidian_card, seg_group, styled_input, v_divider,
    view_header_strip,
};
use crate::format::{fmt_pct_clean, fmt_price};
use crate::icons::{ICON_CHECK, ICON_CLOSE, ICON_LAYERS, ICON_PLUS, icon};
use crate::platform_desktop::{copy_text_with_notice, reveal_workspace_dir};
use crate::stores::{AgentMessageRole, AppStore, INDUSTRY_VALUATION_PRESETS};
use crate::theme::*;
use m3_core::i18n::UiLang;
use m3_core::industry::{self, IndustryProfile, IndustryStatus};
use m3_core::industry_research::IndustryPerspective;
use m3_core::lake::{LakeDb, UniverseRow};
use m3_core::model::Bucket;
use nsh_core::prelude::*;
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrimerSection {
    heading: String,
    body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedIndustryPrimer {
    title: String,
    metadata: Vec<(String, String)>,
    sections: Vec<PrimerSection>,
    sec_citations: Vec<String>,
    web_citations: Vec<String>,
    human_notes_readonly: Option<String>,
}

/// Primary Industry Research Studio view mounted under the `⌘3` Industry tab.
pub fn industry_view(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(render_industry_header_strip(store))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .flex()
                .flex_row()
                .overflow_hidden()
                .child(render_left_rail(store))
                .child(
                    scroll().flex_1().min_w(px(0.0)).h_full().child(
                        view()
                            .w_full()
                            .p(px(20.0))
                            .flex()
                            .flex_col()
                            .gap(px(16.0))
                            .child(dynamic(move || {
                                let slug = store.selected_industry_slug.get();
                                let profile_opt = slug.as_deref().and_then(|s| {
                                    store
                                        .industries
                                        .with(|list| list.iter().find(|p| p.slug == s).cloned())
                                });
                                match profile_opt {
                                    Some(profile) => {
                                        render_industry_workspace(store, profile).into_element()
                                    }
                                    None => render_industry_empty_guide(store).into_element(),
                                }
                            })),
                    ),
                ),
        )
}

fn render_industry_header_strip(store: AppStore) -> View {
    view_header_strip().child(dynamic(move || {
        let l = store.lang.get();
        let (total, graduated, studying, selected_meta) = store.industries.with(|list| {
            let total = list.len();
            let graduated = list
                .iter()
                .filter(|p| p.status == IndustryStatus::InsideCircle)
                .count();
            let studying = list
                .iter()
                .filter(|p| p.status == IndustryStatus::Studying)
                .count();
            let slug = store.selected_industry_slug.get();
            let selected = slug
                .as_deref()
                .and_then(|s| list.iter().find(|p| p.slug == s))
                .map(|p| (p.title.clone(), p.status));
            (total, graduated, studying, selected)
        });

        let left = view()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .min_w(px(0.0))
            .child(icon(ICON_LAYERS, 13.0).text_color(rgb(INK_MUTED)))
            .child(
                text(l.t("行业研究工作室", "Industry Studio"))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO))
                    .number_of_lines(1),
            )
            .child(v_divider())
            .child(industry_stat_inline(
                l.t("研究", "Studies"),
                total,
                INK_HERO,
            ))
            .child(v_divider())
            .child(industry_stat_inline(
                l.t("已毕业", "Graduated"),
                graduated,
                if graduated > 0 { EMERALD_FG } else { INK_MUTED },
            ))
            .child(v_divider())
            .child(industry_stat_inline(
                l.t("进行中", "Active"),
                studying,
                if studying > 0 { SKY_FG } else { INK_MUTED },
            ));

        let right = match selected_meta {
            Some((title, status)) => {
                let (fg, bg, br) = industry_status_colors(status);
                view()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .min_w(px(0.0))
                    .child(
                        text(title)
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_SECONDARY))
                            .truncate()
                            .number_of_lines(1),
                    )
                    .child(badge_pill(status.label_for(l).to_owned(), fg, bg, br))
            }
            None => view().child(
                text(l.t("选择或新建行业主题", "Select or create a study"))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED))
                    .number_of_lines(1),
            ),
        };

        view()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(left)
            .child(right)
    }))
}

fn industry_stat_inline(label: &str, value: usize, color: u32) -> View {
    view()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(5.0))
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED))
                .number_of_lines(1),
        )
        .child(
            text(format!("{value}"))
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(color))
                .number_of_lines(1),
        )
}

// ── Left Rail ────────────────────────────────────────────────────────────────

fn render_left_rail(store: AppStore) -> View {
    view()
        .w(px(320.0))
        .flex_shrink_0()
        .h_full()
        .border_r_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .bg(rgb(BG_SURFACE))
        .flex()
        .flex_col()
        .child(
            scroll().flex_1().min_h(px(0.0)).w_full().child(
                view()
                    .w_full()
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(render_topic_composer(store))
                    .child(render_company_bind_composer(store))
                    .child(render_status_filter_bar(store))
                    .child(render_industry_cards_list(store))
                    .child(render_themes_promotion_rail(store)),
            ),
        )
}

fn render_topic_composer(store: AppStore) -> View {
    obsidian_card()
        .w_full()
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .child(icon(ICON_PLUS, 12.0).text_color(rgb(SKY_FG)))
                .child(
                    text(move || {
                        store
                            .lang
                            .get()
                            .t("新建行业研究（自由主题）", "New Industry Study (Topic)")
                            .to_owned()
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO)),
                ),
        )
        .child(
            styled_input(
                move || store.industry_new_topic_input.get(),
                move || {
                    store
                        .lang
                        .get()
                        .t(
                            "输入行业/主题（如 EDA软件、血透诊所）…",
                            "Industry topic (e.g. EDA, Dialysis)…",
                        )
                        .to_owned()
                },
                false,
            )
            .w_full()
            .on_change(move |val: &str| store.industry_new_topic_input.set(val.to_owned()))
            .on_submit(move |val: &str| store.open_or_create_industry_from_topic(val)),
        )
        .child(action_button(
            move || {
                store
                    .lang
                    .get()
                    .t("自动匹配代表公司并创建", "Match Peers & Start Study")
                    .to_owned()
            },
            true,
            move || {
                let topic = store.industry_new_topic_input.get_untracked();
                store.open_or_create_industry_from_topic(&topic);
            },
        ))
}

fn render_company_bind_composer(store: AppStore) -> View {
    let company_input = signal(String::new());
    obsidian_card()
        .w_full()
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .child(icon(ICON_PLUS, 12.0).text_color(rgb(SKY_FG)))
                .child(
                    text(move || {
                        store
                            .lang
                            .get()
                            .t("按标的快速定位/建组", "Quick Bind by Company Ticker")
                            .to_owned()
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO)),
                ),
        )
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    styled_input(
                        move || company_input.get(),
                        move || {
                            store
                                .lang
                                .get()
                                .t("代码如 NVDA / MSFT…", "Ticker e.g. NVDA…")
                                .to_owned()
                        },
                        true,
                    )
                    .flex_1()
                    .min_w(px(0.0))
                    .on_change(move |val: &str| company_input.set(val.to_owned()))
                    .on_submit(move |val: &str| {
                        trigger_quick_company_industry(store, company_input, val);
                    }),
                )
                .child(action_button(
                    move || store.lang.get().t("建组", "Bind").to_owned(),
                    false,
                    move || {
                        let raw = company_input.get_untracked();
                        trigger_quick_company_industry(store, company_input, &raw);
                    },
                )),
        )
}

fn trigger_quick_company_industry(store: AppStore, input_sig: Signal<String>, raw: &str) {
    let upper = raw.trim().to_ascii_uppercase();
    let l = store.lang.get_untracked();
    if upper.is_empty() {
        store.set_info_notice(
            l.t("请输入公司代码后再关联行业", "Enter a company ticker first")
                .to_owned(),
        );
        return;
    }

    let mut company_name = store.reports.with_untracked(|reps| {
        reps.iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(&upper))
            .map(|r| r.name.clone())
            .unwrap_or_default()
    });
    let mut sic: Option<u32> = None;
    let mut sic_desc: Option<String> = None;

    if !store.universe_syncing.get_untracked() && !store.inbox_catching_up.get_untracked() {
        let paths = store.paths.get_untracked();
        if let Ok(Some(db)) = LakeDb::open_read_only_if_exists(&paths)
            && let Ok(rows) = db.load_universe_rows_by_tickers(std::slice::from_ref(&upper))
            && let Some(row) = rows.into_iter().next()
        {
            if company_name.is_empty() {
                company_name = row.name;
            }
            sic = row.sic;
            sic_desc = row.sic_description;
        }
    }

    store.open_or_create_industry_for_company(&upper, &company_name, sic, sic_desc.as_deref());
    input_sig.set(String::new());
}

fn render_status_filter_bar(store: AppStore) -> View {
    let filters: [Option<IndustryStatus>; 4] = [
        None,
        Some(IndustryStatus::Studying),
        Some(IndustryStatus::InsideCircle),
        Some(IndustryStatus::TooHard),
    ];

    view()
        .w_full()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(6.0))
        .children(filters.into_iter().map(|st_opt| {
            let active = computed(move || store.industry_status_filter.get() == st_opt);
            view()
                .px(px(8.0))
                .py(px(4.0))
                .rounded_sm()
                .cursor_pointer()
                .bg(move || rgb(if active.get() { BG_ACTIVE } else { BG_WELL }))
                .border_1()
                .border_color(move || {
                    rgb(if active.get() {
                        BORDER_PILL
                    } else {
                        BORDER_HAIRLINE
                    })
                })
                .hover(|s| s.bg(rgb(BG_HOVER)))
                .on_press(move || store.industry_status_filter.set(st_opt))
                .child(
                    text(move || {
                        let l = store.lang.get();
                        match st_opt {
                            None => l.t("全部", "All").to_owned(),
                            Some(st) => st.label_for(l).to_owned(),
                        }
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        rgb(if active.get() {
                            INK_HERO
                        } else {
                            INK_SECONDARY
                        })
                    }),
                )
        }))
}

fn render_industry_cards_list(store: AppStore) -> View {
    view()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(dynamic(move || {
            let l = store.lang.get();
            let filter = store.industry_status_filter.get();
            let selected_slug = store.selected_industry_slug.get();
            let filtered: Vec<IndustryProfile> = store.industries.with(|list| {
                list.iter()
                    .filter(|p| filter.is_none_or(|st| p.status == st))
                    .cloned()
                    .collect()
            });

            if filtered.is_empty() {
                return view()
                    .w_full()
                    .p(px(12.0))
                    .rounded_md()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        text(
                            l.t(
                                "暂无匹配行业研究。请在上方输入主题/代码，或从下方 SIC 主题聚类一键发起。",
                                "No matching industry studies. Enter a topic/ticker above or promote a cluster below.",
                            )
                            .to_owned(),
                        )
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .line_height(px(18.0))
                        .text_color(rgb(INK_MUTED)),
                    )
                    .into_element();
            }

            view()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .children(filtered.into_iter().map(|ind| {
                    let is_selected = selected_slug.as_deref() == Some(ind.slug.as_str());
                    let (st_fg, st_bg, st_br) = industry_status_colors(ind.status);
                    let slug = ind.slug.clone();
                    let anchor = ind.valuation_anchor_label();
                    let total_fq = ind.feynman_questions.len();
                    let done_fq = ind.feynman_checked.iter().filter(|b| **b).count();
                    let peer_label = match l {
                        UiLang::Zh => format!("{} 家代表公司", ind.bound_tickers.len()),
                        UiLang::En => format!("{} peers", ind.bound_tickers.len()),
                    };

                    let mut meta_row = view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(6.0))
                        .child(
                            text(peer_label)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        );

                    let mut right_meta = view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0));
                    if total_fq > 0 {
                        let fq_color = if done_fq == total_fq {
                            EMERALD_FG
                        } else if done_fq > 0 {
                            GOLD_FG
                        } else {
                            INK_MUTED
                        };
                        let fq_txt = match l {
                            UiLang::Zh => format!("费曼 {done_fq}/{total_fq}"),
                            UiLang::En => format!("Feynman {done_fq}/{total_fq}"),
                        };
                        right_meta = right_meta.child(
                            text(fq_txt)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(fq_color)),
                        );
                    }
                    if let Some(a) = anchor {
                        right_meta = right_meta.child(
                            text(a)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(EMERALD_FG)),
                        );
                    }
                    meta_row = meta_row.child(right_meta);

                    view()
                        .w_full()
                        .p(px(10.0))
                        .rounded_md()
                        .cursor_pointer()
                        .bg(rgb(if is_selected {
                            BG_SELECTED_ROW
                        } else {
                            BG_VIEWPORT
                        }))
                        .border_1()
                        .border_color(rgb(if is_selected {
                            BORDER_PILL
                        } else {
                            BORDER_HAIRLINE
                        }))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .on_press(move || store.select_industry(&slug))
                        .child(
                            view()
                                .w_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap(px(8.0))
                                .child(
                                    view().flex_1().min_w(px(0.0)).child(
                                        text(ind.title)
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_XS))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO))
                                            .truncate()
                                            .number_of_lines(1),
                                    ),
                                )
                                .child(badge_pill(
                                    ind.status.label_for(l).to_owned(),
                                    st_fg,
                                    st_bg,
                                    st_br,
                                )),
                        )
                        .child(meta_row)
                }))
                .into_element()
        }))
}

fn render_themes_promotion_rail(store: AppStore) -> View {
    view()
        .w_full()
        .pt(px(10.0))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    view().flex_1().min_w(px(0.0)).child(
                        text(move || {
                            store
                                .lang
                                .get()
                                .t("SIC / 主题聚类快速发起", "Promote SIC / Theme Clusters")
                                .to_owned()
                        })
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_SECONDARY))
                        .truncate()
                        .number_of_lines(1),
                    ),
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded_xs()
                        .cursor_pointer()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_SUBTLE))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .on_press(move || store.start_themes_sync())
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                if store.themes_syncing.get() {
                                    l.t("同步中…", "Syncing…").to_owned()
                                } else {
                                    l.t("同步聚类", "Sync").to_owned()
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(SKY_FG)),
                        ),
                ),
        )
        .child(when(
            move || store.themes_error.with(|e| e.is_some()),
            move || {
                view()
                    .w_full()
                    .p(px(8.0))
                    .rounded_sm()
                    .bg(rgba(CORAL_BG))
                    .border_1()
                    .border_color(rgba(CORAL_BORDER))
                    .child(
                        text(move || store.themes_error.get().unwrap_or_default())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(CORAL_FG)),
                    )
            },
        ))
        .child(dynamic(move || {
            let l = store.lang.get();
            let clusters = store
                .themes_file
                .with(|tf| tf.clusters.iter().take(8).cloned().collect::<Vec<_>>());
            if clusters.is_empty() {
                return view()
                    .w_full()
                    .p(px(10.0))
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        text(
                            l.t(
                                "点击「同步聚类」从 DuckDB L0 生成商业原型簇。",
                                "Click Sync to cluster US companies by SIC & archetype.",
                            )
                            .to_owned(),
                        )
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED)),
                    )
                    .into_element();
            }

            view()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .children(clusters.into_iter().map(|cluster| {
                    let count = cluster.tickers.len().max(cluster.members.len());
                    let title = cluster.title.clone();
                    let cluster_for_click = cluster.clone();
                    view()
                        .w_full()
                        .px(px(10.0))
                        .py(px(6.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .on_press(move || {
                            store.open_or_create_industry_from_theme(&cluster_for_click);
                        })
                        .child(
                            view().flex_1().min_w(px(0.0)).child(
                                text(format!("{title} ({count})"))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_PRIMARY))
                                    .truncate()
                                    .number_of_lines(1),
                            ),
                        )
                        .child(
                            text(l.t("研究 →", "Study →").to_owned())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(SKY_FG)),
                        )
                }))
                .into_element()
        }))
}

// ── Main Studio Empty Guide ──────────────────────────────────────────────────

fn render_industry_empty_guide(store: AppStore) -> View {
    obsidian_card()
        .w_full()
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(badge_pill(
                    move || store.lang.get().t("行业研究", "INDUSTRY STUDIO").to_owned(),
                    VIOLET_FG,
                    VIOLET_BG,
                    VIOLET_BORDER,
                ))
                .child(badge_pill(
                    move || store.lang.get().t("人类主权", "HUMAN SOVEREIGNTY").to_owned(),
                    GOLD_FG,
                    GOLD_BG,
                    GOLD_BORDER,
                )),
        )
        .child(
            text(move || {
                store
                    .lang
                    .get()
                    .t(
                        "行业研究工作台 · 从 Too Hard 扩展你的能力圈",
                        "Industry Research Studio · Expand Your Circle Beyond Too Hard",
                    )
                    .to_owned()
            })
            .font_family(FONT_UI)
            .text_size(px(TEXT_LG))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(INK_HERO)),
        )
        .child(
            text(move || {
                store
                    .lang
                    .get()
                    .t(
                        "未研究过的行业不应盲目给出机械击球价。你可以通过三种方式启动行业研究：(1) 在左侧输入自由主题；(2) 输入任意公司代码快速按 SIC 建组（或在收件箱/雷达中点击「发起 / 查看行业研究」）；(3) 从左下角的商业原型聚类一键发起。\n\nAlan 会自动结合代表公司 SEC 10-K 原文、本地 5 年财务矩阵与公开网页资料完成 4 视角研究并生成费曼自测题。唯有当你通过费曼自检、手动设好估值锚并亲手点击「毕业进能力圈」，且具体标的纳入 IN 核心池后，才会触发正式击球区提醒。",
                        "Unknown industries never receive blind mechanical Strike alerts. Start an Industry Study in 3 ways: (1) enter a free-form topic on the left, (2) bind a company ticker by SIC (or click 'Study Industry' in Inbox/Radar), or (3) promote an archetype cluster below.\n\nAlan combines representative SEC 10-Ks, the local 5Y Peer Matrix, and bounded public web search with strict source separation. Formal Strike Zone alerts trigger only after you graduate the industry, pass Feynman checks, and promote a company into IN (Core Circle).",
                    )
                    .to_owned()
            })
            .font_family(FONT_UI)
            .text_size(px(TEXT_BASE))
            .line_height(px(21.0))
            .text_color(rgb(INK_SECONDARY)),
        )
}

// ── Main Industry Workspace ──────────────────────────────────────────────────

fn render_industry_workspace(store: AppStore, profile: IndustryProfile) -> View {
    view()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(render_header_card(store, &profile))
        .child(feynman_coach_panel(store, true))
        .child(render_sovereignty_card(store, &profile))
        .child(render_peer_matrix_card(store, &profile))
        .child(render_primer_card(store, &profile))
        .child(render_live_research_and_tutor_card(store, &profile))
}

fn render_header_card(store: AppStore, profile: &IndustryProfile) -> View {
    let status = profile.status;
    let title = profile.title.clone();
    let slug = profile.slug.clone();
    let slug_for_progress = slug.clone();
    let slug_for_btn = slug.clone();
    let sic_codes = profile.sic_codes.clone();
    let (st_fg, st_bg, st_br) = industry_status_colors(status);

    let status_options: [IndustryStatus; 3] = [
        IndustryStatus::Studying,
        IndustryStatus::InsideCircle,
        IndustryStatus::TooHard,
    ];

    obsidian_card()
        .w_full()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            view()
                .w_full()
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
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.0))
                        .child(badge_pill(
                            move || status.label_for(store.lang.get()).to_owned(),
                            st_fg,
                            st_bg,
                            st_br,
                        ))
                        .child(
                            text(title)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_LG))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(badge_pill(slug.clone(), SLATE_FG, SLATE_BG, SLATE_BORDER))
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                if sic_codes.is_empty() {
                                    l.t("自定义主题池", "Custom Archetype Pool").to_owned()
                                } else {
                                    format!(
                                        "SIC {}",
                                        sic_codes
                                            .iter()
                                            .map(u32::to_string)
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                }
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .px(px(14.0))
                        .py(px(8.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgba(VIOLET_BG))
                        .border_1()
                        .border_color(rgba(VIOLET_BORDER))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .on_press(move || store.start_industry_deep_research())
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                let running = store.industry_research_running.get()
                                    && store.industry_research_slug.get().as_deref()
                                        == Some(slug_for_btn.as_str());
                                if running {
                                    l.t("Alan 4 视角深研中…", "Alan 4-Lens Running…").to_owned()
                                } else {
                                    l.t(
                                        "派 Alan 4 视角深研 (10-K + Web)",
                                        "Run Alan 4-Lens Research (10-K + Web)",
                                    )
                                    .to_owned()
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(VIOLET_FG))
                            .number_of_lines(1),
                        ),
                ),
        )
        .child(when(
            move || {
                store.industry_research_running.get()
                    && store.industry_research_slug.get().as_deref()
                        == Some(slug_for_progress.as_str())
                    && store.industry_research_progress.with(|p| p.is_some())
            },
            move || {
                view()
                    .w_full()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded_sm()
                    .bg(rgba(VIOLET_BG))
                    .border_1()
                    .border_color(rgba(VIOLET_BORDER))
                    .child(
                        text(move || store.industry_research_progress.get().unwrap_or_default())
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(VIOLET_FG)),
                    )
            },
        ))
        .child(
            view()
                .w_full()
                .pt(px(10.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            text(move || {
                                store
                                    .lang
                                    .get()
                                    .t("能力圈裁决", "Circle Graduation")
                                    .to_owned()
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(concept_hint(move || {
                            store
                                .lang
                                .get()
                                .t(
                                    "仅人类可将行业毕业进能力圈；毕业后按估值锚解锁击球区信号。",
                                    "Only human judgment graduates an industry into your Circle, unlocking Circle-gated Strike alerts at your multiple anchor.",
                                )
                                .to_owned()
                        })),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .children(status_options.into_iter().map(|opt| {
                            let active = status == opt;
                            let (fg, bg, br) = industry_status_colors(opt);
                            view()
                                .flex_shrink_0()
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(if active { rgba(bg) } else { rgb(BG_WELL) })
                                .border_1()
                                .border_color(if active {
                                    rgba(br)
                                } else {
                                    rgb(BORDER_SUBTLE)
                                })
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_press(move || store.set_selected_industry_status(opt))
                                .child(
                                    text(move || {
                                        let l = store.lang.get();
                                        match opt {
                                            IndustryStatus::Studying => {
                                                l.t("学习中", "Studying").to_owned()
                                            }
                                            IndustryStatus::InsideCircle => {
                                                l.t("毕业进能力圈 ✓", "Graduate to Circle ✓")
                                                    .to_owned()
                                            }
                                            IndustryStatus::TooHard => {
                                                l.t("标为 Too Hard", "Mark Too Hard").to_owned()
                                            }
                                        }
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(if active {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(rgb(if active { fg } else { INK_SECONDARY }))
                                    .number_of_lines(1),
                                )
                        }))
                        .child({
                            let slug_confirm = slug.clone();
                            let slug_delete = slug.clone();
                            view().ml(px(6.0)).child(dynamic(move || {
                                let l = store.lang.get();
                                let confirming = store
                                    .industry_delete_confirm_slug
                                    .get()
                                    .as_deref()
                                    == Some(slug_confirm.as_str());
                                if confirming {
                                    let del_slug = slug_delete.clone();
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(
                                            view()
                                                .px(px(10.0))
                                                .py(px(5.0))
                                                .rounded_sm()
                                                .cursor_pointer()
                                                .bg(rgba(CORAL_BG))
                                                .border_1()
                                                .border_color(rgba(CORAL_BORDER))
                                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                                .on_press(move || store.delete_industry(&del_slug))
                                                .child(
                                                    text(l.t("确认删除", "Confirm Delete"))
                                                        .font_family(FONT_UI)
                                                        .text_size(px(TEXT_XS))
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(rgb(CORAL_FG))
                                                        .number_of_lines(1),
                                                ),
                                        )
                                        .child(
                                            view()
                                                .px(px(8.0))
                                                .py(px(5.0))
                                                .rounded_sm()
                                                .cursor_pointer()
                                                .bg(rgb(BG_WELL))
                                                .border_1()
                                                .border_color(rgb(BORDER_SUBTLE))
                                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                                .on_press(move || {
                                                    store.industry_delete_confirm_slug.set(None)
                                                })
                                                .child(
                                                    text(l.t("取消", "Cancel"))
                                                        .font_family(FONT_UI)
                                                        .text_size(px(TEXT_XS))
                                                        .text_color(rgb(INK_SECONDARY))
                                                        .number_of_lines(1),
                                                ),
                                        )
                                        .into_element()
                                } else {
                                    let arm_slug = slug_confirm.clone();
                                    view()
                                        .px(px(10.0))
                                        .py(px(5.0))
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgb(BORDER_SUBTLE))
                                        .hover(|s| s.bg(rgba(CORAL_BG)))
                                        .on_press(move || {
                                            store
                                                .industry_delete_confirm_slug
                                                .set(Some(arm_slug.clone()));
                                        })
                                        .child(
                                            text(l.t("删除研究", "Delete"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_MUTED))
                                                .number_of_lines(1),
                                        )
                                        .into_element()
                                }
                            }))
                        }),
                ),
        )
}

fn render_sovereignty_card(store: AppStore, profile: &IndustryProfile) -> View {
    let metric_basis = signal(0u8); // 0 = EV/EBIT, 1 = P/E, 2 = EV/FCF (informational lens)
    let anchor_label_opt = profile.valuation_anchor_label();
    let mult_low = profile.multiple_low;
    let mult_high = profile.multiple_high;
    let bound_tickers = profile.bound_tickers.clone();
    let feynman_questions = profile.feynman_questions.clone();
    let feynman_checked = profile.feynman_checked.clone();
    let checked_count = feynman_checked.iter().filter(|b| **b).count();
    let total_fq = feynman_questions.len();

    let metric_lenses: [(u8, &str); 3] = [(0, "EV/EBIT"), (1, "P/E"), (2, "EV/FCF")];

    obsidian_card()
        .w_full()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        // Card header
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    text(move || {
                        store
                            .lang
                            .get()
                            .t("人类主权区", "Human Sovereignty")
                            .to_owned()
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_MD))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO)),
                )
                .child(badge_pill(
                    move || store.lang.get().t("仅人类可编辑", "HUMAN ONLY").to_owned(),
                    GOLD_FG,
                    GOLD_BG,
                    GOLD_BORDER,
                )),
        )
        // 1. Valuation Multiple Anchor
        .child(
            view()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                match (l, anchor_label_opt.as_deref()) {
                                    (UiLang::Zh, Some(lbl)) => format!(
                                        "行业合理倍数锚：{lbl}（毕业进能力圈后，按倍数下沿折算击球价）"
                                    ),
                                    (UiLang::En, Some(lbl)) => format!(
                                        "Valuation Multiple Anchor: {lbl} (low end drives Circle Strike Price)"
                                    ),
                                    (UiLang::Zh, None) => {
                                        "行业合理倍数锚：尚未设定（默认 18x，可点选预设或输入区间）"
                                            .to_owned()
                                    }
                                    (UiLang::En, None) => {
                                        "Valuation Multiple Anchor: Unset (defaults to 18x; pick a preset or enter custom band)"
                                            .to_owned()
                                    }
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(
                            seg_group().children(metric_lenses.into_iter().map(|(id, name)| {
                                let is_active = computed(move || metric_basis.get() == id);
                                view()
                                    .px(px(8.0))
                                    .py(px(3.0))
                                    .rounded_xs()
                                    .cursor_pointer()
                                    .bg(move || {
                                        if is_active.get() {
                                            rgba(SKY_BG)
                                        } else {
                                            rgba(0x00000000)
                                        }
                                    })
                                    .on_press(move || metric_basis.set(id))
                                    .child(
                                        text(name)
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_2XS))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(move || {
                                                rgb(if is_active.get() {
                                                    SKY_FG
                                                } else {
                                                    INK_MUTED
                                                })
                                            }),
                                    )
                            })),
                        ),
                )
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.0))
                        .children(INDUSTRY_VALUATION_PRESETS.iter().map(|preset| {
                            let lo_d = Decimal::from(preset.low);
                            let hi_d = Decimal::from(preset.high);
                            let active = mult_low == Some(lo_d) && mult_high == Some(hi_d);
                            let zh = preset.label_zh;
                            let en = preset.label_en;
                            view()
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(if active {
                                    rgba(EMERALD_BG)
                                } else {
                                    rgb(BG_WELL)
                                })
                                .border_1()
                                .border_color(if active {
                                    rgba(EMERALD_BORDER)
                                } else {
                                    rgb(BORDER_SUBTLE)
                                })
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_press(move || {
                                    store.set_selected_industry_anchor(Some(lo_d), Some(hi_d));
                                })
                                .child(
                                    text(move || store.lang.get().t(zh, en).to_owned())
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(if active {
                                            EMERALD_FG
                                        } else {
                                            INK_PRIMARY
                                        })),
                                )
                        })),
                )
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                let basis = match metric_basis.get() {
                                    0 => "EV/EBIT",
                                    1 => "P/E",
                                    _ => "EV/FCF",
                                };
                                match l {
                                    UiLang::Zh => format!("自定义区间 ({basis}):"),
                                    UiLang::En => format!("Custom Band ({basis}):"),
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_MUTED)),
                        )
                        .child(
                            styled_input(
                                move || store.industry_anchor_low_draft.get(),
                                move || store.lang.get().t("下沿 x", "Low x").to_owned(),
                                true,
                            )
                            .w(px(78.0))
                            .on_change(move |val: &str| {
                                store.industry_anchor_low_draft.set(val.to_owned())
                            })
                            .on_submit(move |_: &str| apply_anchor_drafts(store)),
                        )
                        .child(
                            text("–")
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(INK_MUTED)),
                        )
                        .child(
                            styled_input(
                                move || store.industry_anchor_high_draft.get(),
                                move || store.lang.get().t("上沿 x", "High x").to_owned(),
                                true,
                            )
                            .w(px(78.0))
                            .on_change(move |val: &str| {
                                store.industry_anchor_high_draft.set(val.to_owned())
                            })
                            .on_submit(move |_: &str| apply_anchor_drafts(store)),
                        )
                        .child(action_button(
                            move || store.lang.get().t("应用区间", "Apply Band").to_owned(),
                            false,
                            move || apply_anchor_drafts(store),
                        ))
                        .child(action_button(
                            move || store.lang.get().t("清除", "Clear").to_owned(),
                            false,
                            move || store.set_selected_industry_anchor(None, None),
                        )),
                ),
        )
        // 2. Bound Peer Tickers Chips
        .child(
            view()
                .w_full()
                .pt(px(10.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    text(move || {
                        store
                            .lang
                            .get()
                            .t("代表公司池", "Bound Peers")
                            .to_owned()
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_SECONDARY)),
                )
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.0))
                        .children(bound_tickers.into_iter().map(|tk| {
                            let tk_open = tk.clone();
                            let tk_rm = tk.clone();
                            view()
                                .px(px(8.0))
                                .py(px(4.0))
                                .rounded_xs()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    view()
                                        .cursor_pointer()
                                        .hover(|s| s.opacity(0.85))
                                        .on_press(move || store.open_company_contextual(&tk_open))
                                        .child(
                                            text(tk)
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_XS))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(SKY_FG)),
                                        ),
                                )
                                .child(
                                    view()
                                        .cursor_pointer()
                                        .px(px(2.0))
                                        .hover(|s| s.opacity(0.8))
                                        .on_press(move || {
                                            store.remove_ticker_from_selected_industry(&tk_rm);
                                        })
                                        .child(icon(ICON_CLOSE, 10.0).text_color(rgb(INK_MUTED))),
                                )
                        }))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    styled_input(
                                        move || store.industry_add_ticker_input.get(),
                                        move || {
                                            store
                                                .lang
                                                .get()
                                                .t("+ 代码回车", "+ Ticker ↵")
                                                .to_owned()
                                        },
                                        true,
                                    )
                                    .w(px(120.0))
                                    .on_change(move |val: &str| {
                                        store.industry_add_ticker_input.set(val.to_owned())
                                    })
                                    .on_submit(move |val: &str| {
                                        store.add_ticker_to_selected_industry(val);
                                    }),
                                )
                                .child(action_button(
                                    move || store.lang.get().t("添加", "Add").to_owned(),
                                    false,
                                    move || {
                                        let tk = store.industry_add_ticker_input.get_untracked();
                                        store.add_ticker_to_selected_industry(&tk);
                                    },
                                )),
                        ),
                ),
        )
        // 3. Feynman 4-Criterion Checklist
        .child(
            view()
                .w_full()
                .pt(px(10.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    view()
                        .w_full()
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
                                    text(move || {
                                        store
                                            .lang
                                            .get()
                                            .t("费曼通关自检", "Feynman Check")
                                            .to_owned()
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(concept_hint(move || {
                                    store
                                        .lang
                                        .get()
                                        .t(
                                            "用自己的语言讲清商业本质与死法后，再将行业毕业进能力圈。",
                                            "Confirm you can explain the economics and failure modes simply before graduating this industry.",
                                        )
                                        .to_owned()
                                })),
                        )
                        .child(badge_pill(
                            format!("{checked_count}/{total_fq}"),
                            if total_fq > 0 && checked_count == total_fq {
                                EMERALD_FG
                            } else {
                                GOLD_FG
                            },
                            if total_fq > 0 && checked_count == total_fq {
                                EMERALD_BG
                            } else {
                                GOLD_BG
                            },
                            if total_fq > 0 && checked_count == total_fq {
                                EMERALD_BORDER
                            } else {
                                GOLD_BORDER
                            },
                        )),
                )
                .child(if feynman_questions.is_empty() {
                    view()
                        .w_full()
                        .p(px(10.0))
                        .rounded_sm()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .child(
                            text(move || {
                                store
                                    .lang
                                    .get()
                                    .t(
                                        "尚未生成费曼自检清单。点击右上角「派 Alan 4 视角深研」后将自动提炼 4 道检验题。",
                                        "No Feynman criteria yet. Run Alan 4-Lens Research to generate the 4 mastery checks.",
                                    )
                                    .to_owned()
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_MUTED)),
                        )
                } else {
                    view().w_full().flex().flex_col().gap(px(6.0)).children(
                        feynman_questions.into_iter().enumerate().map(|(idx, q)| {
                            let checked = feynman_checked.get(idx).copied().unwrap_or(false);
                            view()
                                .w_full()
                                .px(px(10.0))
                                .py(px(8.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(if checked {
                                    rgba(EMERALD_BORDER)
                                } else {
                                    rgb(BORDER_HAIRLINE)
                                })
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .on_press(move || store.toggle_selected_industry_feynman(idx))
                                .child({
                                    let mut box_el = view()
                                        .w(px(15.0))
                                        .h(px(15.0))
                                        .flex_shrink_0()
                                        .rounded_xs()
                                        .border_1()
                                        .flex()
                                        .items_center()
                                        .justify_center();
                                    if checked {
                                        box_el = box_el
                                            .bg(rgba(EMERALD_BG))
                                            .border_color(rgba(EMERALD_BORDER))
                                            .child(
                                                icon(ICON_CHECK, 10.0).text_color(rgb(EMERALD_FG)),
                                            );
                                    } else {
                                        box_el = box_el
                                            .bg(rgb(BG_SURFACE))
                                            .border_color(rgb(BORDER_PILL));
                                    }
                                    box_el
                                })
                                .child(
                                    view().flex_1().min_w(px(0.0)).child(
                                        text(q)
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_XS))
                                            .line_height(px(18.0))
                                            .text_color(rgb(INK_PRIMARY)),
                                    ),
                                )
                        }),
                    )
                }),
        )
        // 4. Protected ## Human Notes Editor
        .child(
            view()
                .w_full()
                .pt(px(10.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            text(move || {
                                store
                                    .lang
                                    .get()
                                    .t(
                                        "人类行业研判笔记（Alan 永不覆盖）：",
                                        "Human Industry Notes (## Human Notes · Protected from Alan):",
                                    )
                                    .to_owned()
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(action_button(
                            move || store.lang.get().t("保存笔记", "Save Notes").to_owned(),
                            true,
                            move || {
                                let note = store.industry_human_note_draft.get_untracked();
                                store.save_selected_industry_human_notes(&note);
                            },
                        )),
                )
                .child(
                    styled_input(
                        move || store.industry_human_note_draft.get(),
                        move || {
                            store
                                .lang
                                .get()
                                .t(
                                    "记录护城河边界、竞争格局变化或你的反驳思考…",
                                    "Write your moat boundaries, unit economics notes, or pre-mortem thoughts…",
                                )
                                .to_owned()
                        },
                        false,
                    )
                    .w_full()
                    .multiline(true)
                    .min_h(px(88.0))
                    .on_change(move |val: &str| {
                        store.industry_human_note_draft.set(val.to_owned())
                    }),
                ),
        )
}

fn apply_anchor_drafts(store: AppStore) {
    let low_raw = store.industry_anchor_low_draft.get_untracked();
    let high_raw = store.industry_anchor_high_draft.get_untracked();
    let l = store.lang.get_untracked();

    let parse_opt = |s: &str| -> Result<Option<Decimal>, String> {
        let t = s.trim();
        if t.is_empty() {
            Ok(None)
        } else {
            t.parse::<Decimal>().map(Some).map_err(|_| {
                l.t(
                    "估值倍数区间必须是有效数字（如 14 或 18.5）",
                    "Valuation multiple must be a valid number (e.g. 14 or 18.5)",
                )
                .to_owned()
            })
        }
    };

    let low = match parse_opt(&low_raw) {
        Ok(v) => v,
        Err(msg) => {
            store.set_error_notice(msg);
            return;
        }
    };
    let high = match parse_opt(&high_raw) {
        Ok(v) => v,
        Err(msg) => {
            store.set_error_notice(msg);
            return;
        }
    };
    if let (Some(lo), Some(hi)) = (low, high)
        && lo > hi
    {
        store.set_error_notice(
            l.t(
                "倍数下沿不能大于上沿",
                "Multiple low bound cannot exceed high bound",
            )
            .to_owned(),
        );
        return;
    }
    store.set_selected_industry_anchor(low, high);
}

// ── L0 Peer Matrix & 10-K Batch Fetch ────────────────────────────────────────

fn render_peer_matrix_card(store: AppStore, profile: &IndustryProfile) -> View {
    let multiple_low = profile.multiple_low;
    let slug = profile.slug.clone();
    let anchor_mult_lbl = multiple_low
        .map(|m| format!("{}x", m.round_dp(1).normalize()))
        .unwrap_or_else(|| "18x".to_owned());

    obsidian_card()
        .w_full()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            text(move || {
                                store
                                    .lang
                                    .get()
                                    .t(
                                        "代表公司 L0 确定性财务矩阵",
                                        "Representative Peer L0 Financial Matrix",
                                    )
                                    .to_owned()
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_MD))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(badge_pill("0 LLM", EMERALD_FG, EMERALD_BG, EMERALD_BORDER)),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            view()
                                .flex_shrink_0()
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgba(SKY_BG))
                                .border_1()
                                .border_color(rgba(SKY_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_press(move || store.start_industry_batch_10k_fetch())
                                .child(
                                    text(move || {
                                        let l = store.lang.get();
                                        if store.industry_batch_10k_running.get() {
                                            l.t("拉取 10-K 中…", "Fetching 10-Ks…").to_owned()
                                        } else {
                                            l.t("拉取同行 10-K", "Fetch Peer 10-Ks").to_owned()
                                        }
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(SKY_FG)),
                                ),
                        )
                        .child(
                            view()
                                .flex_shrink_0()
                                .px(px(10.0))
                                .py(px(5.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgba(VIOLET_BG))
                                .border_1()
                                .border_color(rgba(VIOLET_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_press(move || store.start_industry_deep_research())
                                .child(
                                    text(move || {
                                        let l = store.lang.get();
                                        let running = store.industry_research_running.get()
                                            && store.industry_research_slug.get().as_deref()
                                                == Some(slug.as_str());
                                        if running {
                                            l.t("4 视角运行中…", "4-Lens Running…").to_owned()
                                        } else {
                                            l.t("派 Alan 4 视角深研", "Run Alan 4-Lens")
                                                .to_owned()
                                        }
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(VIOLET_FG)),
                                ),
                        ),
                ),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let rows = store.industry_peer_rows.get();
            let buckets_by_ticker: BTreeMap<String, Bucket> = store.reports.with(|reps| {
                reps.iter()
                    .map(|r| (r.ticker.to_ascii_uppercase(), r.bucket))
                    .collect()
            });

            if rows.is_empty() {
                return view()
                    .w_full()
                    .p(px(12.0))
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        text(
                            l.t(
                                "暂无 DuckDB L0 同行财务快照（请添加美股代码或先同步全美股 L0 湖仓）。",
                                "No DuckDB L0 financial snapshot yet (add US tickers or sync US L0 lakehouse).",
                            )
                            .to_owned(),
                        )
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                    )
                    .into_element();
            }

            view()
                .w_full()
                .rounded_sm()
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(render_peer_matrix_header(l, &anchor_mult_lbl))
                .children(rows.into_iter().enumerate().map(|(idx, row)| {
                    let bucket_opt = buckets_by_ticker
                        .get(&row.ticker.to_ascii_uppercase())
                        .copied()
                        .or(row.bucket_opt);
                    render_peer_matrix_row(store, idx, row, multiple_low, bucket_opt, l)
                }))
                .into_element()
        }))
}

fn render_peer_matrix_header(l: UiLang, anchor_mult_lbl: &str) -> View {
    let hdr_cell = |txt: &str, width: f32, right: bool| {
        let mut cell = view().w(px(width)).flex_shrink_0().flex().flex_row();
        if right {
            cell = cell.justify_end();
        }
        cell.child(
            text(txt.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_2XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_MUTED))
                .number_of_lines(1),
        )
    };

    view()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .bg(rgb(BG_WELL))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
            view()
                .flex_1()
                .min_w(px(120.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(hdr_cell("Ticker", 64.0, false))
                .child(
                    view().flex_1().min_w(px(0.0)).child(
                        text(l.t("公司", "Company").to_owned())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_2XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED)),
                    ),
                )
                .child(hdr_cell(l.t("现价", "Price"), 66.0, true))
                .child(hdr_cell("5Y ROIC", 62.0, true))
                .child(hdr_cell(l.t("5Y毛利", "5Y GM"), 58.0, true))
                .child(hdr_cell(l.t("杠杆", "NetDebt"), 58.0, true))
                .child(hdr_cell("FCF%", 58.0, true))
                .child(hdr_cell(
                    &format!("{} ({anchor_mult_lbl})", l.t("击球差", "Strike")),
                    108.0,
                    true,
                )),
        )
        .child(hdr_cell(l.t("能力圈", "Circle"), 86.0, true))
}

fn render_peer_matrix_row(
    store: AppStore,
    idx: usize,
    row: UniverseRow,
    multiple_low: Option<Decimal>,
    bucket_opt: Option<Bucket>,
    l: UiLang,
) -> View {
    let (adj_strike, dist_pct) = industry::compute_industry_strike_price(&row, multiple_low);
    let ticker_open = row.ticker.clone();
    let ticker_lbl = row.ticker.clone();
    let name_lbl = row.name.clone();
    let price_txt = fmt_price(row.latest_price, "USD");
    let roic_txt = fmt_pct_clean(row.roic_5y_avg_pct, false).0;
    let margin_txt = fmt_pct_clean(row.gross_margin_5y_avg_pct, false).0;
    let lev_txt = row
        .net_debt_to_oe
        .map(|d| format!("{:.1}x", d.round_dp(1)))
        .unwrap_or_else(|| "—".to_owned());
    let fcf_txt = fmt_pct_clean(row.strict_fcf_yield_pct, false).0;

    let (strike_txt, strike_col) = match (adj_strike, dist_pct) {
        (Some(s), Some(d)) => {
            let (d_str, _) = fmt_pct_clean(Some(d), true);
            let col = if d <= Decimal::ZERO {
                EMERALD_FG
            } else if d <= Decimal::from(15) {
                GOLD_FG
            } else {
                SKY_FG
            };
            (format!("${:.2} ({d_str})", s), col)
        }
        (Some(s), None) => (format!("${:.2}", s), SKY_FG),
        (None, _) => ("—".to_owned(), INK_MUTED),
    };

    let num_cell = |txt: String, width: f32, col: u32| {
        view()
            .w(px(width))
            .flex_shrink_0()
            .flex()
            .flex_row()
            .justify_end()
            .child(
                text(txt)
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(col))
                    .number_of_lines(1),
            )
    };

    let row_bg = if idx.is_multiple_of(2) {
        BG_SURFACE
    } else {
        BG_ROW_ALT
    };

    let circle_cell = match bucket_opt {
        Some(bucket) => {
            let (fg, bg, br) = bucket_colors(bucket);
            let ticker_open_badge = row.ticker.clone();
            view()
                .w(px(86.0))
                .flex_shrink_0()
                .flex()
                .flex_row()
                .justify_end()
                .child(
                    view()
                        .cursor_pointer()
                        .on_press(move || store.open_company_contextual(&ticker_open_badge))
                        .child(badge_pill(bucket.code(), fg, bg, br)),
                )
        }
        None => {
            let row_watch = row.clone();
            view()
                .w(px(86.0))
                .flex_shrink_0()
                .flex()
                .flex_row()
                .justify_end()
                .child(
                    view()
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded_xs()
                        .cursor_pointer()
                        .bg(rgba(EMERALD_BG))
                        .border_1()
                        .border_color(rgba(EMERALD_BORDER))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .on_press(move || store.add_universe_company_to_watch(&row_watch))
                        .child(
                            text(l.t("+观察池", "+WATCH").to_owned())
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(EMERALD_FG)),
                        ),
                )
        }
    };

    view()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .bg(rgb(row_bg))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
            view()
                .flex_1()
                .min_w(px(120.0))
                .cursor_pointer()
                .hover(|s| s.opacity(0.88))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .on_press(move || store.open_company_contextual(&ticker_open))
                .child(
                    view().w(px(64.0)).flex_shrink_0().child(
                        text(ticker_lbl)
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_HERO))
                            .number_of_lines(1),
                    ),
                )
                .child(
                    view().flex_1().min_w(px(0.0)).child(
                        text(name_lbl)
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_SECONDARY))
                            .truncate()
                            .number_of_lines(1),
                    ),
                )
                .child(num_cell(price_txt, 66.0, INK_HERO))
                .child(num_cell(roic_txt, 62.0, EMERALD_FG))
                .child(num_cell(margin_txt, 58.0, INK_PRIMARY))
                .child(num_cell(lev_txt, 58.0, INK_SECONDARY))
                .child(num_cell(fcf_txt, 58.0, INK_PRIMARY))
                .child(num_cell(strike_txt, 108.0, strike_col)),
        )
        .child(circle_cell)
}

// ── Living Industry Primer + Source Separation ───────────────────────────────

fn render_primer_card(store: AppStore, profile: &IndustryProfile) -> View {
    let slug = profile.slug.clone();
    let slug_for_live = slug.clone();

    obsidian_card()
        .w_full()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            text(move || {
                                store
                                    .lang
                                    .get()
                                    .t("行业活手册", "Living Industry Primer")
                                    .to_owned()
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_MD))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(badge_pill("SEC 10-K", EMERALD_FG, EMERALD_BG, EMERALD_BORDER))
                        .child(badge_pill("Web", SKY_FG, SKY_BG, SKY_BORDER)),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(action_button(
                            move || {
                                store
                                    .lang
                                    .get()
                                    .t("复制 Markdown", "Copy Markdown")
                                    .to_owned()
                            },
                            false,
                            move || {
                                let md = store
                                    .industry_primer_md
                                    .get_untracked()
                                    .unwrap_or_default();
                                if md.trim().is_empty() {
                                    let l = store.lang.get_untracked();
                                    store.set_info_notice(
                                        l.t("尚无活手册内容可复制", "No primer markdown to copy")
                                            .to_owned(),
                                    );
                                } else {
                                    copy_text_with_notice(store, md, "Industry Primer");
                                }
                            },
                        ))
                        .child(action_button(
                            move || store.lang.get().t("在 Finder 显示", "Reveal File").to_owned(),
                            false,
                            move || {
                                let path = store
                                    .paths
                                    .get_untracked()
                                    .industry_primer_path(&slug);
                                if path.exists() {
                                    reveal_workspace_dir(&path);
                                } else {
                                    let dir = store.paths.get_untracked().industries_dir();
                                    reveal_workspace_dir(&dir);
                                }
                            },
                        )),
                ),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let is_streaming_this = store.industry_research_running.get()
                && store.industry_research_slug.get().as_deref() == Some(slug_for_live.as_str())
                && store.industry_live_perspectives.with(|m| !m.is_empty());

            if is_streaming_this {
                let live_md = store.industry_live_perspectives.with(|m| {
                    m.iter()
                        .map(|(k, v)| format!("## {k}\n{v}"))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                });
                return view()
                    .w_full()
                    .p(px(14.0))
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgba(VIOLET_BORDER))
                    .child(
                        text(live_md)
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_XS))
                            .line_height(px(20.0))
                            .text_color(rgb(INK_PRIMARY))
                            .selectable(true),
                    )
                    .into_element();
            }

            let primer_opt = store.industry_primer_md.get();
            let Some(md) = primer_opt.filter(|s| !s.trim().is_empty()) else {
                return view()
                    .w_full()
                    .p(px(14.0))
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .child(
                        text(
                            l.t(
                                "尚未生成行业活手册。点击上方「派 Alan 4 视角深研」，系统将自动拉取代表公司 SEC 10-K、结合 DuckDB 财务矩阵与公开网页搜索，写出产业链、护城河、历史死法与关键 KPI 活手册（严格分离 SEC 10-K 与 Web 出处，并永久保留 ## Human Notes）。",
                                "No primer yet. Click 'Run Alan 4-Lens Research' above to synthesize Value Chain, Moats, Failure Modes & Key KPIs from SEC 10-K, DuckDB & bounded Web Search with strict SEC vs Web citation separation and protected ## Human Notes.",
                            )
                            .to_owned(),
                        )
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .line_height(px(19.0))
                        .text_color(rgb(INK_MUTED)),
                    )
                    .into_element();
            };

            let parsed = parse_industry_primer(&md);
            render_parsed_primer(parsed, l).into_element()
        }))
}

fn render_parsed_primer(parsed: ParsedIndustryPrimer, l: UiLang) -> View {
    let mut container = view().w_full().flex().flex_col().gap(px(10.0));

    if !parsed.metadata.is_empty() {
        container = container.child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(6.0))
                .children(parsed.metadata.into_iter().map(|(k, v)| {
                    badge_pill(format!("{k}: {v}"), INK_SECONDARY, BG_WELL, BORDER_SUBTLE)
                })),
        );
    }

    for sec in parsed.sections {
        if sec.body.trim().is_empty() {
            continue;
        }
        container = container.child(
            view()
                .w_full()
                .p(px(12.0))
                .rounded_sm()
                .bg(rgb(BG_WELL))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    text(sec.heading)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(
                    text(sec.body)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .line_height(px(19.0))
                        .text_color(rgb(INK_PRIMARY))
                        .selectable(true),
                ),
        );
    }

    // Strict Source Separation panel
    container = container.child(
        view()
            .w_full()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(10.0))
            .child(
                view()
                    .flex_1()
                    .min_w(px(260.0))
                    .p(px(12.0))
                    .rounded_sm()
                    .bg(rgba(EMERALD_BG))
                    .border_1()
                    .border_color(rgba(EMERALD_BORDER))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        view()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(badge_pill(
                                l.t("一手 SEC 10-K 出处", "SEC 10-K Primary Citations")
                                    .to_owned(),
                                EMERALD_FG,
                                EMERALD_BG,
                                EMERALD_BORDER,
                            ))
                            .child(
                                text(format!("{}", parsed.sec_citations.len()))
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(EMERALD_FG)),
                            ),
                    )
                    .child(if parsed.sec_citations.is_empty() {
                        text(
                            l.t(
                                "本地 DuckDB / SEC 10-K 切片（无额外行内标签）",
                                "Local DuckDB / SEC 10-K chunks",
                            )
                            .to_owned(),
                        )
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED))
                    } else {
                        text(parsed.sec_citations.join("\n"))
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .line_height(px(17.0))
                            .text_color(rgb(INK_PRIMARY))
                            .selectable(true)
                    }),
            )
            .child(
                view()
                    .flex_1()
                    .min_w(px(260.0))
                    .p(px(12.0))
                    .rounded_sm()
                    .bg(rgba(SKY_BG))
                    .border_1()
                    .border_color(rgba(SKY_BORDER))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        view()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(badge_pill(
                                l.t("公开网页来源 (External Web)", "Public Web Citations")
                                    .to_owned(),
                                SKY_FG,
                                SKY_BG,
                                SKY_BORDER,
                            ))
                            .child(
                                text(format!("{}", parsed.web_citations.len()))
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(SKY_FG)),
                            ),
                    )
                    .child(if parsed.web_citations.is_empty() {
                        text(
                            l.t("本次未引用外部网页来源", "No external web citations")
                                .to_owned(),
                        )
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED))
                    } else {
                        text(parsed.web_citations.join("\n"))
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .line_height(px(17.0))
                            .text_color(rgb(INK_PRIMARY))
                            .selectable(true)
                    }),
            ),
    );

    if let Some(notes) = parsed.human_notes_readonly {
        container = container.child(
            view()
                .w_full()
                .p(px(12.0))
                .rounded_sm()
                .bg(rgba(GOLD_BG))
                .border_1()
                .border_color(rgba(GOLD_BORDER))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(badge_pill(
                    l.t(
                        "## Human Notes（受保护 · 只读展示）",
                        "## Human Notes (Protected · Read-Only)",
                    )
                    .to_owned(),
                    GOLD_FG,
                    GOLD_BG,
                    GOLD_BORDER,
                ))
                .child(
                    text(notes)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .line_height(px(19.0))
                        .text_color(rgb(INK_PRIMARY))
                        .selectable(true),
                ),
        );
    }

    container
}

fn parse_industry_primer(raw: &str) -> ParsedIndustryPrimer {
    let mut parsed = ParsedIndustryPrimer::default();
    let mut current_heading: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();
    let mut in_preamble = true;

    let mut sec_seen = HashSet::new();
    let mut web_seen = HashSet::new();

    for line in raw.lines() {
        let trimmed = line.trim_end();
        if in_preamble && parsed.title.is_empty() && trimmed.starts_with("# ") {
            parsed.title = trimmed.trim_start_matches("# ").trim().to_owned();
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            in_preamble = false;
            if let Some(prev) = current_heading.replace(heading.trim().to_owned()) {
                flush_primer_section(
                    &mut parsed,
                    prev,
                    std::mem::take(&mut current_lines),
                    &mut sec_seen,
                    &mut web_seen,
                );
            }
            continue;
        }
        if in_preamble {
            if let Some(rest) = trimmed.trim().strip_prefix("- **")
                && let Some((k, v)) = rest.split_once("**:")
            {
                parsed
                    .metadata
                    .push((k.trim().to_owned(), v.trim().trim_matches('`').to_owned()));
            }
        } else {
            current_lines.push(trimmed.to_owned());
        }
    }

    if let Some(last) = current_heading {
        flush_primer_section(
            &mut parsed,
            last,
            current_lines,
            &mut sec_seen,
            &mut web_seen,
        );
    }

    extract_inline_citations(
        raw,
        &mut parsed.sec_citations,
        &mut parsed.web_citations,
        &mut sec_seen,
        &mut web_seen,
    );
    parsed
}

fn flush_primer_section(
    parsed: &mut ParsedIndustryPrimer,
    heading: String,
    lines: Vec<String>,
    sec_seen: &mut HashSet<String>,
    web_seen: &mut HashSet<String>,
) {
    let body = lines.join("\n").trim().to_owned();
    let lower = heading.to_ascii_lowercase();

    if lower.starts_with("human notes") {
        let cleaned: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("<!--"))
            .collect();
        if !cleaned.is_empty() {
            parsed.human_notes_readonly = Some(cleaned.join("\n"));
        }
        return;
    }

    if lower.contains("evidence sources") {
        enum BucketKind {
            None,
            Sec,
            Web,
        }
        let mut kind = BucketKind::None;
        for line in body.lines() {
            let t = line.trim();
            let tl = t.to_ascii_lowercase();
            if t.starts_with("### ") {
                if tl.contains("sec") || tl.contains("10-k") {
                    kind = BucketKind::Sec;
                } else if tl.contains("web") || tl.contains("外部") {
                    kind = BucketKind::Web;
                } else {
                    kind = BucketKind::None;
                }
                continue;
            }
            if let Some(item) = t.strip_prefix("- ") {
                let clean = item.trim().trim_matches('`').to_owned();
                if clean.is_empty() || (clean.starts_with('_') && clean.ends_with('_')) {
                    continue;
                }
                match kind {
                    BucketKind::Sec => {
                        if sec_seen.insert(clean.clone()) {
                            parsed.sec_citations.push(clean);
                        }
                    }
                    BucketKind::Web => {
                        if web_seen.insert(clean.clone()) {
                            parsed.web_citations.push(clean);
                        }
                    }
                    BucketKind::None => {}
                }
            }
        }
        return;
    }

    parsed.sections.push(PrimerSection { heading, body });
}

fn extract_inline_citations(
    raw: &str,
    sec_out: &mut Vec<String>,
    web_out: &mut Vec<String>,
    sec_seen: &mut HashSet<String>,
    web_seen: &mut HashSet<String>,
) {
    let mut idx = 0usize;
    while let Some(open) = raw[idx..].find('[') {
        let abs_open = idx + open;
        let rest = &raw[abs_open + 1..];
        if let Some(close) = rest.find(']') {
            let inner = rest[..close].trim();
            let lower = inner.to_ascii_lowercase();
            if lower.starts_with("sec 10-k:") || lower.starts_with("sec:") {
                let formatted = format!("[{inner}]");
                if sec_seen.insert(formatted.clone()) {
                    sec_out.push(formatted);
                }
            } else if lower.starts_with("web:") {
                let formatted = format!("[{inner}]");
                if web_seen.insert(formatted.clone()) {
                    web_out.push(formatted);
                }
            }
            idx = abs_open + 1 + close + 1;
        } else {
            break;
        }
    }
}

// ── Live 4-Lens Streaming + Interactive Alan Tutor & Feynman Quiz ────────────

fn render_live_research_and_tutor_card(store: AppStore, profile: &IndustryProfile) -> View {
    let slug = profile.slug.clone();

    obsidian_card()
        .w_full()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        // 4-Lens live progress grid
        .child(render_4lens_live_section(store, slug))
        // Alan Industry Tutor & Feynman Quiz
        .child(
            view()
                .w_full()
                .pt(px(12.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    text(move || {
                                        store
                                            .lang
                                            .get()
                                            .t(
                                                "Alan 行业导师与费曼考官",
                                                "Alan Industry Tutor & Feynman Examiner",
                                            )
                                            .to_owned()
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_MD))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(when(
                                    move || store.industry_alan_status_hint.with(|h| h.is_some()),
                                    move || {
                                        text(move || {
                                            store.industry_alan_status_hint.get().unwrap_or_default()
                                        })
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(VIOLET_FG))
                                    },
                                )),
                        )
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(when(
                                    move || store.industry_alan_streaming.get(),
                                    move || {
                                        action_button(
                                            move || store.lang.get().t("停止", "Stop").to_owned(),
                                            false,
                                            move || store.interrupt_alan(),
                                        )
                                    },
                                ))
                                .child(
                                    view()
                                        .flex_shrink_0()
                                        .px(px(12.0))
                                        .py(px(6.0))
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .bg(rgba(VIOLET_BG))
                                        .border_1()
                                        .border_color(rgba(VIOLET_BORDER))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .on_press(move || {
                                            if let Some(slug) =
                                                store.selected_industry_slug.get_untracked()
                                            {
                                                store.start_industry_feynman(&slug);
                                            }
                                        })
                                        .child(
                                            text(move || {
                                                store
                                                    .lang
                                                    .get()
                                                    .t("3 轮费曼答辩", "3-Round Feynman Coach")
                                                    .to_owned()
                                            })
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_XS))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(VIOLET_FG)),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex_shrink_0()
                                        .px(px(12.0))
                                        .py(px(6.0))
                                        .rounded_sm()
                                        .cursor_pointer()
                                        .bg(rgba(GOLD_BG))
                                        .border_1()
                                        .border_color(rgba(GOLD_BORDER))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .on_press(move || store.trigger_industry_feynman_quiz())
                                        .child(
                                            text(move || {
                                                store
                                                    .lang
                                                    .get()
                                                    .t(
                                                        "开始费曼考官测验 (4问)",
                                                        "Start Feynman Quiz (4 Qs)",
                                                    )
                                                    .to_owned()
                                            })
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_XS))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(GOLD_FG)),
                                        ),
                                ),
                        ),
                )
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let msgs = store.industry_alan_messages.get();
                    if msgs.is_empty() {
                        return view()
                            .w_full()
                            .p(px(12.0))
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .child(
                                text(
                                    l.t(
                                        "点击右上角「开始费曼考官测验」，或在下方直接向 Alan 追问产业链定价权、护城河持续性与会计排雷细节。",
                                        "Click 'Start Feynman Quiz' above or ask Alan directly below about value-chain pricing power, moat durability, or accounting red flags.",
                                    )
                                    .to_owned(),
                                )
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                            )
                            .into_element();
                    }

                    view()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .children(msgs.into_iter().map(|msg| {
                            let (role_label, role_col, bg_col, br_col) = match msg.role {
                                AgentMessageRole::User => {
                                    ("YOU", INK_MUTED, BG_SURFACE, rgb(BORDER_SUBTLE))
                                }
                                AgentMessageRole::Assistant => (
                                    "ALAN · INDUSTRY TUTOR",
                                    VIOLET_FG,
                                    BG_WELL,
                                    rgba(VIOLET_BORDER),
                                ),
                                AgentMessageRole::SystemNote => {
                                    ("SYSTEM NOTE", CORAL_FG, BG_WELL, rgba(CORAL_BORDER))
                                }
                            };
                            let body = if msg.text.is_empty() && msg.streaming {
                                l.t("思考与检索工具中…", "Thinking & querying tools…")
                                    .to_owned()
                            } else {
                                msg.text
                            };
                            view()
                                .w_full()
                                .p(px(12.0))
                                .rounded_sm()
                                .bg(rgb(bg_col))
                                .border_1()
                                .border_color(br_col)
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(
                                    text(role_label)
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(role_col)),
                                )
                                .child(
                                    text(body)
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_BASE))
                                        .line_height(px(21.0))
                                        .text_color(rgb(INK_PRIMARY))
                                        .selectable(true),
                                )
                        }))
                        .into_element()
                }))
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            styled_input(
                                move || store.industry_alan_input.get(),
                                move || {
                                    store
                                        .lang
                                        .get()
                                        .t(
                                            "向 Alan 行业导师提问或作答费曼考题…",
                                            "Ask Alan or submit your Feynman quiz answer…",
                                        )
                                        .to_owned()
                                },
                                false,
                            )
                            .flex_1()
                            .min_w(px(0.0))
                            .on_change(move |val: &str| {
                                store.industry_alan_input.set(val.to_owned())
                            })
                            .on_submit(move |val: &str| {
                                store.submit_industry_alan_question(val);
                            }),
                        )
                        .child(action_button(
                            move || store.lang.get().t("发送 ↵", "Ask ↵").to_owned(),
                            true,
                            move || {
                                let q = store.industry_alan_input.get_untracked();
                                store.submit_industry_alan_question(&q);
                            },
                        )),
                ),
        )
}

fn render_4lens_live_section(store: AppStore, slug: String) -> View {
    let perspectives = IndustryPerspective::all();

    view()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    text(move || {
                        store
                            .lang
                            .get()
                            .t(
                                "Alan 4 视角行业深研进度",
                                "Alan 4-Lens Industry Deep Research Streams",
                            )
                            .to_owned()
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO)),
                )
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let running = store.industry_research_running.get()
                        && store.industry_research_slug.get().as_deref() == Some(slug.as_str());
                    let budget_blocked = store.deep_research_budget_blocked.get().or_else(|| {
                        store.notice.with(|n| {
                            n.as_ref()
                                .filter(|(msg, is_err)| {
                                    *is_err && (msg.contains("预算") || msg.contains("budget"))
                                })
                                .map(|(msg, _)| msg.clone())
                        })
                    });
                    if let Some(reason) = budget_blocked {
                        badge_pill(
                            format!("{}: {reason}", l.t("预算拦截", "Budget Blocked")),
                            CORAL_FG,
                            CORAL_BG,
                            CORAL_BORDER,
                        )
                        .into_element()
                    } else if running {
                        badge_pill(
                            l.t("4 视角并行运行中", "4 Lenses Streaming").to_owned(),
                            VIOLET_FG,
                            VIOLET_BG,
                            VIOLET_BORDER,
                        )
                        .into_element()
                    } else {
                        badge_pill(
                            l.t("就绪", "Ready").to_owned(),
                            SLATE_FG,
                            SLATE_BG,
                            SLATE_BORDER,
                        )
                        .into_element()
                    }
                })),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let live_map = store.industry_live_perspectives.get();
            let running = store.industry_research_running.get();
            let has_primer = store
                .industry_primer_md
                .with(|m| m.as_ref().is_some_and(|s| !s.trim().is_empty()));

            view()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .children(perspectives.into_iter().map(|p| {
                    let pid = p.id();
                    let title = match (l, p) {
                        (UiLang::En, IndustryPerspective::ValueChain) => {
                            "Value Chain & Profit Pool"
                        }
                        (UiLang::En, IndustryPerspective::BusinessModelMoat) => {
                            "Business Model & Moats"
                        }
                        (UiLang::En, IndustryPerspective::FailureModes) => "Failure Modes & Traps",
                        (UiLang::En, IndustryPerspective::ValuationKpis) => {
                            "Valuation Anchor & KPIs"
                        }
                        (UiLang::Zh, _) => p.title_cn(),
                    };
                    let stream_opt = live_map.get(pid).cloned();
                    let (st_label, fg, bg, br) = match stream_opt.as_deref() {
                        Some(txt) if running && !txt.is_empty() => (
                            l.t("流式输出中", "Streaming"),
                            VIOLET_FG,
                            VIOLET_BG,
                            VIOLET_BORDER,
                        ),
                        Some(_) if running => {
                            (l.t("启动中…", "Starting…"), SKY_FG, SKY_BG, SKY_BORDER)
                        }
                        Some(_) => (
                            l.t("已完成", "Completed"),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                        ),
                        None if has_primer => (
                            l.t("已写入活手册", "In Primer"),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                        ),
                        None => (l.t("待命", "Pending"), SLATE_FG, SLATE_BG, SLATE_BORDER),
                    };

                    let preview =
                        stream_opt
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| {
                                if has_primer {
                                    l.t(
                                        "详见上方行业活手册对应章节",
                                        "See Living Industry Primer above",
                                    )
                                    .to_owned()
                                } else {
                                    l.t("等待启动 4 视角深研…", "Awaiting 4-lens research run…")
                                        .to_owned()
                                }
                            });

                    view()
                        .flex_1()
                        .min_w(px(260.0))
                        .p(px(10.0))
                        .rounded_sm()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            view()
                                .w_full()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap(px(6.0))
                                .child(
                                    text(title)
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO)),
                                )
                                .child(badge_pill(st_label, fg, bg, br)),
                        )
                        .child(
                            text(preview)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_2XS))
                                .line_height(px(16.0))
                                .text_color(rgb(INK_SECONDARY))
                                .number_of_lines(4),
                        )
                }))
                .into_element()
        }))
}
