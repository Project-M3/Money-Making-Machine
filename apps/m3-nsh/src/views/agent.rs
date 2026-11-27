//! Unified Alan Canvas (`Closed | Split | Immersive`) and reusable Company Desk helpers.

use crate::components::{
    action_button, badge_pill, card_title, obsidian_card, prose_block, section_header, seg_group,
    seg_tab, styled_input,
};
use crate::format::{
    build_alan_interrogation_prompt_for, fmt_pct_clean, fmt_price, fmt_time_cn_for,
};
use crate::icons::{
    ICON_ALAN, ICON_CHEVRON_DOWN, ICON_CHEVRON_RIGHT, ICON_CLOSE, ICON_EXPAND, ICON_PIN,
    ICON_SHRINK, icon,
};
use crate::platform_desktop::copy_text_with_notice;
use crate::stores::{
    AgentMessageRole, AlanCanvasTab, AlanScope, AlanStageMode, AppStore, CompanyViewTab,
    DeepResearchPerspectiveUi, FeynmanRound, FeynmanScope, InlineEvidenceCard, MemoSection,
    PerspectiveUiStatus, default_deep_research_perspectives,
};
use crate::theme::*;
use m3_core::alan_interrogator::{AcmeEvidenceKind, TuringTapeKind};
use m3_core::feynman;
use m3_core::i18n::UiLang;
use m3_core::lake_l2::ResearchRunStatus;
use m3_core::model::{Bucket, MungerReport};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

fn scope_pill_label(store: AppStore, scope: &AlanScope, lang: UiLang) -> String {
    match scope {
        AlanScope::Company(ticker) => format!("Alan · {}", ticker.trim().to_ascii_uppercase()),
        AlanScope::Industry(slug) => {
            let title = store.industries.with(|list| {
                list.iter()
                    .find(|p| p.slug == *slug)
                    .map(|p| p.title.clone())
            });
            format!("Alan · {}", title.unwrap_or_else(|| slug.clone()))
        }
        AlanScope::Briefing(_) => format!("Alan · {}", lang.t("简报", "Briefing")),
        AlanScope::Portfolio => format!("Alan · {}", lang.t("全局能力圈", "Portfolio")),
    }
}

/// Renders the unified App-Shell Alan Canvas (`Split` right card or `Immersive` 760px studio).
pub fn alan_canvas_view(store: AppStore) -> impl IntoElement {
    let tape_expanded = signal(false);

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .h_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(render_alan_canvas_topbar(store))
        .child(when(
            move || store.alan_canvas_tab.get() == AlanCanvasTab::Interrogate,
            move || render_alan_interrogate_pane(store, tape_expanded),
        ))
        .child(when(
            move || store.alan_canvas_tab.get() == AlanCanvasTab::Feynman,
            move || render_alan_feynman_pane(store),
        ))
}

fn render_alan_canvas_topbar(store: AppStore) -> View {
    view()
        .h(px(42.0))
        .flex_shrink_0()
        .px(px(12.0))
        .bg(rgb(BG_SURFACE))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(dynamic(move || {
            let l = store.lang.get();
            let pinned = store.alan_context_pinned.get();
            let scope = store.alan_scope.get();
            let scope_txt = scope_pill_label(store, &scope, l);

            view()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    view()
                        .px(px(6.0))
                        .py(px(4.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(if pinned {
                            rgba(GOLD_BG)
                        } else {
                            rgba(0x00000000)
                        })
                        .border_1()
                        .border_color(if pinned {
                            rgba(GOLD_BORDER)
                        } else {
                            rgba(0x00000000)
                        })
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .tooltip(
                            view()
                                .px(px(8.0))
                                .py(px(4.0))
                                .rounded_sm()
                                .bg(rgb(0x161620))
                                .border_1()
                                .border_color(rgba(BORDER_GLASS_STRONG))
                                .child(
                                    text(if pinned {
                                        l.t(
                                            "已锁定上下文（点击恢复自动跟随）",
                                            "Context pinned (click to auto-follow)",
                                        )
                                    } else {
                                        l.t(
                                            "自动跟随当前视图（点击锁定上下文）",
                                            "Auto-following view (click to pin)",
                                        )
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_PRIMARY)),
                                )
                                .into_element(),
                        )
                        .on_press(move || store.toggle_alan_pin())
                        .child(icon(ICON_PIN, 13.0).text_color(rgb(if pinned {
                            GOLD_FG
                        } else {
                            INK_MUTED
                        }))),
                )
                .child(
                    view()
                        .min_w(px(0.0))
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded_sm()
                        .bg(rgba(VIOLET_BG))
                        .border_1()
                        .border_color(rgba(VIOLET_BORDER))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .child(icon(ICON_ALAN, 11.0).text_color(rgb(VIOLET_FG)))
                        .child(
                            text(scope_txt)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO))
                                .truncate()
                                .number_of_lines(1),
                        ),
                )
        }))
        .child(
            view()
                .flex_shrink_0()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let active_tab = store.alan_canvas_tab.get();
                    seg_group().children(AlanCanvasTab::all().into_iter().map(|tab| {
                        seg_tab(
                            tab.label_for(l),
                            active_tab == tab,
                            Some((VIOLET_FG, VIOLET_BG)),
                            false,
                            move || store.set_alan_canvas_tab(tab),
                        )
                    }))
                }))
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let immersive = store.alan_stage_mode.get() == AlanStageMode::Immersive;
                    view()
                        .px(px(7.0))
                        .py(px(4.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_SUBTLE))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .on_press(move || {
                            crate::components::animate_shell_spring();
                            store.toggle_alan_immersive();
                        })
                        .child(
                            icon(if immersive { ICON_SHRINK } else { ICON_EXPAND }, 12.0)
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(
                            text(if immersive {
                                l.t("分栏", "Split")
                            } else {
                                l.t("沉浸", "Immerse")
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_SECONDARY)),
                        )
                }))
                .child(
                    view()
                        .px(px(6.0))
                        .py(px(4.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgba(RAIL_HOVER)))
                        .on_press(move || {
                            crate::components::animate_shell_spring();
                            store.close_alan_canvas();
                        })
                        .child(icon(ICON_CLOSE, 13.0).text_color(rgb(INK_MUTED))),
                ),
        )
}

fn render_alan_interrogate_pane(store: AppStore, tape_expanded: Signal<bool>) -> View {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        // Single-line collapsible Turing Tape status capsule (collapsed by default)
        .child(render_turing_tape_capsule(store, tape_expanded))
        // Scrollable conversation stream + deterministic InlineEvidenceCards
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .child(dynamic(move || render_alan_message_stream(store))),
        )
        // Native IME-safe Composer mounted ONCE outside dynamic()
        .child(render_alan_composer_footer(store))
}

fn render_turing_tape_capsule(store: AppStore, tape_expanded: Signal<bool>) -> View {
    view().child(dynamic(move || {
        let l = store.lang.get();
        let streaming = store.agent_streaming.get();
        let hint = store.agent_status_hint.get();
        let tape = store.agent_tape.get();
        if !streaming && tape.is_empty() {
            return view();
        }
        let open = tape_expanded.get();
        let expanded_seq = store.agent_expanded_tape.get();
        let len = tape.len();
        let latest = tape.last();

        let dot_col = if streaming {
            VIOLET_FG
        } else if let Some(entry) = latest {
            match entry.kind {
                TuringTapeKind::Policy | TuringTapeKind::Error => CORAL_FG,
                TuringTapeKind::Out => EMERALD_FG,
                TuringTapeKind::Tool => SKY_FG,
                TuringTapeKind::Input => INK_MUTED,
            }
        } else {
            EMERALD_FG
        };

        let capsule_text = if streaming {
            hint.or_else(|| latest.map(|e| format!("[{}] {}", e.kind.tag(), e.summary)))
                .unwrap_or_else(|| l.t("Alan 正在思考…", "Alan thinking…").to_owned())
        } else if let Some(entry) = latest {
            format!("[{}] {}", entry.kind.tag(), entry.summary)
        } else {
            l.t("确定性工具轨迹就绪", "Deterministic trace ready")
                .to_owned()
        };

        let mut wrapper = view()
            .w_full()
            .flex_shrink_0()
            .bg(rgb(BG_ROW_ALT))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .flex_col()
            .child(
                view()
                    .h(px(30.0))
                    .px(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .on_press(move || tape_expanded.update(|v| *v = !*v))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                view()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(rgb(dot_col)),
                            )
                            .child(
                                text(capsule_text)
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_SECONDARY))
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
                            .gap(px(5.0))
                            .child(
                                text(format!("{len}"))
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                icon(
                                    if open {
                                        ICON_CHEVRON_DOWN
                                    } else {
                                        ICON_CHEVRON_RIGHT
                                    },
                                    11.0,
                                )
                                .text_color(rgb(INK_MUTED)),
                            ),
                    ),
            );

        if open && !tape.is_empty() {
            wrapper = wrapper.child(
                scroll().w_full().max_h(px(180.0)).child(
                    view()
                        .w_full()
                        .px(px(12.0))
                        .py(px(8.0))
                        .border_t_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .bg(rgb(BG_WELL))
                        .flex()
                        .flex_col()
                        .children(tape.into_iter().enumerate().map(|(idx, row)| {
                            let row_seq = row.seq;
                            let row_expanded = expanded_seq == Some(row_seq);
                            let tag_color = match row.kind {
                                TuringTapeKind::Input => INK_MUTED,
                                TuringTapeKind::Tool => SKY_FG,
                                TuringTapeKind::Out => EMERALD_FG,
                                TuringTapeKind::Policy => CORAL_FG,
                                TuringTapeKind::Error => GOLD_FG,
                            };
                            let mut entry = view()
                                .py(px(4.0))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .flex()
                                .flex_col()
                                .gap(px(3.0))
                                .on_press(move || {
                                    store.agent_expanded_tape.update(|cur| {
                                        *cur = if *cur == Some(row_seq) {
                                            None
                                        } else {
                                            Some(row_seq)
                                        };
                                    });
                                });
                            if idx + 1 < len {
                                entry = entry.border_b_1().border_color(rgb(BORDER_HAIRLINE));
                            }
                            entry = entry.child(
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(6.0))
                                    .child(
                                        view()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap(px(6.0))
                                            .child(
                                                text(format!("{:04}", row.seq))
                                                    .font_family(FONT_MONO_TERM)
                                                    .text_size(px(TEXT_2XS))
                                                    .text_color(rgb(INK_MUTED)),
                                            )
                                            .child(
                                                text(format!("[{}]", row.kind.tag()))
                                                    .font_family(FONT_MONO_TERM)
                                                    .text_size(px(TEXT_2XS))
                                                    .text_color(rgb(tag_color)),
                                            )
                                            .child(
                                                view().flex_1().min_w(px(0.0)).child(
                                                    text(row.summary)
                                                        .font_family(FONT_MONO_TERM)
                                                        .text_size(px(TEXT_2XS))
                                                        .text_color(rgb(INK_PRIMARY))
                                                        .truncate()
                                                        .number_of_lines(1),
                                                ),
                                            ),
                                    )
                                    .child(
                                        text(row.status)
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_2XS))
                                            .text_color(rgb(INK_MUTED)),
                                    ),
                            );
                            if row_expanded && let Some(detail) = row.detail {
                                entry = entry.child(
                                    view().pl(px(20.0)).child(
                                        text(detail)
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_2XS))
                                            .line_height(px(16.0))
                                            .text_color(rgb(INK_SECONDARY)),
                                    ),
                                );
                            }
                            entry
                        })),
                ),
            );
        }

        wrapper
    }))
}

fn scope_starter_prompts(scope: &AlanScope, lang: UiLang) -> Vec<(&'static str, String)> {
    match scope {
        AlanScope::Company(ticker) => vec![
            (
                lang.t("护城河与财务质量", "Moat & Accounting Check"),
                match lang {
                    UiLang::Zh => format!(
                        "基于 /mnt/m3 和确定性报告，检验 {ticker} 的 ROIC、严苛自由现金流转化率与护城河脆弱点。"
                    ),
                    UiLang::En => format!(
                        "Using /mnt/m3 and deterministic reports, stress-test {ticker}'s ROIC, strict FCF conversion, and moat fragility."
                    ),
                },
            ),
            (
                lang.t("5 年逆向事前验尸", "5Y Pre-Mortem"),
                match lang {
                    UiLang::Zh => format!(
                        "站在 5 年后回看，找出 {ticker} 最可能导致永久性本金损失的 3 条失败路径与可证伪指标。"
                    ),
                    UiLang::En => format!(
                        "Looking back from 5 years ahead, what are the 3 likeliest paths to >50% permanent drawdown for {ticker} and testable falsification metrics?"
                    ),
                },
            ),
        ],
        AlanScope::Industry(slug) => vec![
            (
                lang.t("产业链利润池", "Value Chain Profit Pool"),
                match lang {
                    UiLang::Zh => format!(
                        "读取行业 {slug} 档案与同行数据，拆解该产业链的定价权归属、单位经济模型与最容易看走眼的护城河幻觉。"
                    ),
                    UiLang::En => format!(
                        "Inspect industry {slug} and peer data to break down value-chain pricing power, unit economics, and moat illusions."
                    ),
                },
            ),
            (
                lang.t("行业典型死法", "Industry Failure Modes"),
                match lang {
                    UiLang::Zh => format!(
                        "在 {slug} 行业里，历史上的赢家或扩张者通常死于哪几类周期/技术/资本配置陷阱？"
                    ),
                    UiLang::En => format!(
                        "In {slug}, which cycle, tech, or capital-allocation traps typically destroy incumbents?"
                    ),
                },
            ),
        ],
        AlanScope::Briefing(_) => vec![
            (
                lang.t("解读本期异动", "Interpret Latest Briefing"),
                lang.t(
                    "结合最新简报与能力圈信号，哪条变化最值得我今天花时间深挖，哪几条属于可以忽略的噪音？",
                    "Based on the latest briefing and Circle signals, which delta truly merits deep work today and what is noise?",
                )
                .to_owned(),
            ),
            (
                lang.t("击球区优先排序", "Prioritize Strike Radar"),
                lang.t(
                    "对当前接近或处于击球区的 WATCH/IN 标的，按确定性与安全边际排序，并指出最需要补充证伪指标的公司。",
                    "Rank WATCH/IN companies near or in the Strike Zone by certainty and margin of safety, and flag missing falsification criteria.",
                )
                .to_owned(),
            ),
        ],
        AlanScope::Portfolio => vec![
            (
                lang.t("全局能力圈体检", "Circle Health Check"),
                lang.t(
                    "审视当前 watchlist 与决策账本：核心池 (IN) 存在哪些共性风险暴露或待处理红灯？",
                    "Review my watchlist and decision journal: what shared risks or open red flags exist in Core (IN)?",
                )
                .to_owned(),
            ),
            (
                lang.t("机会成本对照", "Opportunity Cost Audit"),
                lang.t(
                    "对照 IN 与 WATCH 中的候选标的，当前哪家公司的期望回报与确定性最占优？",
                    "Compare IN and WATCH candidates: where is expected return vs certainty strongest right now?",
                )
                .to_owned(),
            ),
        ],
    }
}

fn render_inline_evidence_card(store: AppStore, card: InlineEvidenceCard, lang: UiLang) -> View {
    let jump_ticker = card.ticker.clone();
    let is_xbrl = card.source_kind == AcmeEvidenceKind::Xbrl;
    let cur = card.currency.as_deref().unwrap_or("USD");
    let price_str = fmt_price(card.latest_price, cur);
    let strike_str = fmt_price(card.strike_price, cur);
    let (gap_str, gap_pol) = fmt_pct_clean(card.strike_gap_pct, true);
    let roic_str = card
        .roic_5y
        .map(|v| format!("{:.1}%", v))
        .unwrap_or_else(|| "—".to_owned());
    let fcf_str = card
        .fcf_conversion_or_yield
        .map(|v| format!("{:.1}%", v))
        .unwrap_or_else(|| "—".to_owned());

    let (src_lbl, src_fg, src_bg, src_br) = match card.source_kind {
        AcmeEvidenceKind::Report => ("REPORT", EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        AcmeEvidenceKind::Xbrl => ("XBRL", SKY_FG, SKY_BG, SKY_BORDER),
        AcmeEvidenceKind::Dossier => ("DOSSIER", GOLD_FG, GOLD_BG, GOLD_BORDER),
    };

    let mut header_left = view()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            text(card.ticker.clone())
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(INK_HERO)),
        )
        .child(
            view().flex_1().min_w(px(0.0)).child(
                text(card.name)
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_SECONDARY))
                    .truncate()
                    .number_of_lines(1),
            ),
        );
    if let Some(b) = card.bucket {
        let (bfg, bbg, bbr) = bucket_colors(b);
        header_left = header_left.child(badge_pill(b.code(), bfg, bbg, bbr));
    }

    let mut header_right = view()
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(badge_pill(src_lbl, src_fg, src_bg, src_br));
    if card.warning_count > 0 {
        header_right = header_right.child(badge_pill(
            match lang {
                UiLang::Zh => format!("{} 警示", card.warning_count),
                UiLang::En => format!("{} warn", card.warning_count),
            },
            CORAL_FG,
            CORAL_BG,
            CORAL_BORDER,
        ));
    }

    let gap_col = if gap_pol < 0 {
        EMERALD_FG
    } else if gap_pol > 0 {
        GOLD_FG
    } else {
        INK_SECONDARY
    };

    view()
        .w_full()
        .p(px(10.0))
        .rounded_md()
        .cursor_pointer()
        .bg(rgb(BG_SURFACE))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .on_press(move || {
            let exists = store.reports.with_untracked(|reps| {
                reps.iter()
                    .any(|r| r.ticker.eq_ignore_ascii_case(&jump_ticker))
            });
            if exists {
                store.open_company(
                    &jump_ticker,
                    if is_xbrl {
                        CompanyViewTab::XbrlInspector
                    } else {
                        CompanyViewTab::FinancialsMemo
                    },
                );
            } else {
                store.open_inbox_signal_target(&jump_ticker);
            }
        })
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(header_left)
                .child(header_right),
        )
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(12.0))
                .child(evidence_kv(lang.t("现价", "Price"), price_str, INK_PRIMARY))
                .child(evidence_kv(
                    lang.t("击球价", "Strike"),
                    strike_str,
                    INK_PRIMARY,
                ))
                .child(evidence_kv(lang.t("距击球", "Gap"), gap_str, gap_col))
                .child(evidence_kv("5Y ROIC", roic_str, SKY_FG))
                .child(evidence_kv("FCF", fcf_str, EMERALD_FG)),
        )
}

fn evidence_kv(label: &str, val: String, val_col: u32) -> View {
    view()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(4.0))
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED)),
        )
        .child(
            text(val)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(val_col)),
        )
}

fn render_alan_message_stream(store: AppStore) -> View {
    let l = store.lang.get();
    let scope = store.alan_scope.get();
    let msgs = store.agent_messages.get();
    let cards_map = store.alan_evidence_cards.get();
    let expanded_ev = store.agent_expanded_evidence.get();

    let expanded_token = expanded_ev.as_ref().and_then(|eid| {
        msgs.iter()
            .flat_map(|m| m.tokens.iter())
            .find(|t| &t.id == eid)
            .cloned()
    });

    let mut stream = view().w_full().p(px(14.0)).flex().flex_col().gap(px(12.0));

    if msgs.is_empty() {
        let starters = scope_starter_prompts(&scope, l);
        stream = stream.child(
            obsidian_card()
                .p(px(14.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(6.0))
                        .child(badge_pill(
                            scope_pill_label(store, &scope, l),
                            VIOLET_FG,
                            VIOLET_BG,
                            VIOLET_BORDER,
                        ))
                        .child(badge_pill(
                            l.t(
                                "只读 /mnt/m3 · 红线保护",
                                "Read-only /mnt/m3 · Redline Guarded",
                            ),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                        )),
                )
                .child(
                    text(l.t(
                        "直接提问或从下方切入分析；Alan 基于本地确定性财务与只读挂载树推演，严禁篡改人类主权区。",
                        "Ask a question or pick a prompt below; Alan reasons over deterministic financials and read-only /mnt/m3 without mutating human judgment.",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(18.0))
                    .text_color(rgb(INK_SECONDARY)),
                )
                .child(
                    view()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .children(starters.into_iter().map(|(label, prompt)| {
                            view()
                                .w_full()
                                .px(px(10.0))
                                .py(px(7.0))
                                .rounded_sm()
                                .cursor_pointer()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_SUBTLE))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .on_press(move || store.submit_scoped_alan_question(prompt.clone()))
                                .child(
                                    text(format!("→ {label}"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(INK_PRIMARY)),
                                )
                        })),
                ),
        );
        return stream;
    }

    for msg in msgs {
        match msg.role {
            AgentMessageRole::User => {
                stream = stream.child(
                    view()
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded_md()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .child(
                            text(match l {
                                UiLang::Zh => format!("你  ·  {}", msg.text),
                                UiLang::En => format!("You  ·  {}", msg.text),
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .line_height(px(19.0))
                            .text_color(rgb(INK_HERO)),
                        ),
                );
            }
            AgentMessageRole::SystemNote => {
                stream = stream.child(
                    view()
                        .px(px(12.0))
                        .py(px(8.0))
                        .rounded_md()
                        .bg(rgba(GOLD_BG))
                        .border_1()
                        .border_color(rgba(GOLD_BORDER))
                        .child(
                            text(msg.text)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .line_height(px(18.0))
                                .text_color(rgb(GOLD_FG)),
                        ),
                );
            }
            AgentMessageRole::Assistant => {
                let body = if msg.text.is_empty() && msg.streaming {
                    l.t(
                        "Alan 正在读取 /mnt/m3 与确定性报告…",
                        "Alan is reading /mnt/m3 and deterministic reports…",
                    )
                    .to_owned()
                } else {
                    msg.text.clone()
                };
                let mut box_el = view().flex().flex_col().gap(px(8.0)).child(
                    text(body)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_BASE))
                        .line_height(px(21.0))
                        .text_color(rgb(if msg.text.is_empty() && msg.streaming {
                            INK_MUTED
                        } else {
                            INK_PRIMARY
                        }))
                        .selectable(true),
                );

                if let Some(cards) = cards_map.get(&msg.id)
                    && !cards.is_empty()
                {
                    box_el = box_el.child(
                        view().flex().flex_col().gap(px(6.0)).children(
                            cards
                                .iter()
                                .cloned()
                                .map(|card| render_inline_evidence_card(store, card, l)),
                        ),
                    );
                }

                if !msg.tokens.is_empty() {
                    let chips = view().flex().flex_row().flex_wrap().gap(px(5.0)).children(
                        msg.tokens.into_iter().map(|tok| {
                            let tok_id = tok.id.clone();
                            let selected = expanded_ev.as_deref() == Some(tok_id.as_str());
                            let (fg, bg, border) = match tok.kind {
                                AcmeEvidenceKind::Report => {
                                    (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                                }
                                AcmeEvidenceKind::Xbrl => (SKY_FG, SKY_BG, SKY_BORDER),
                                AcmeEvidenceKind::Dossier => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                            };
                            view()
                                .px(px(7.0))
                                .py(px(2.0))
                                .rounded_sm()
                                .bg(if selected { rgb(BG_ACTIVE) } else { rgba(bg) })
                                .border_1()
                                .border_color(rgba(border))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .on_press(move || {
                                    store.agent_expanded_evidence.update(|cur| {
                                        if cur.as_deref() == Some(tok_id.as_str()) {
                                            *cur = None;
                                        } else {
                                            *cur = Some(tok_id.clone());
                                        }
                                    });
                                })
                                .child(
                                    text(tok.label)
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(fg)),
                                )
                        }),
                    );
                    box_el = box_el.child(chips);
                }
                stream = stream.child(box_el);
            }
        }
    }

    if let Some(tok) = expanded_token {
        let jump_ticker = tok.ticker.clone();
        let is_xbrl = tok.kind == AcmeEvidenceKind::Xbrl;
        let cite_text = format!("[{}:{}] {}", tok.kind.prefix(), tok.ticker, tok.summary);
        let cite_label = tok.label.clone();

        let mut hdr_actions = view().flex().flex_row().items_center().gap(px(6.0));
        if is_xbrl {
            hdr_actions = hdr_actions.child(action_button(
                l.t("跳至 XBRL 底表 →", "XBRL Ledger →"),
                false,
                move || {
                    let exists = store.reports.with_untracked(|reps| {
                        reps.iter()
                            .any(|r| r.ticker.eq_ignore_ascii_case(&jump_ticker))
                    });
                    if exists {
                        store.open_company(&jump_ticker, CompanyViewTab::XbrlInspector);
                    }
                },
            ));
        }
        hdr_actions = hdr_actions
            .child(action_button(
                l.t("复制引用", "Copy"),
                false,
                move || {
                    copy_text_with_notice(store, cite_text.clone(), &cite_label);
                },
            ))
            .child(action_button(l.t("收起", "Hide"), false, move || {
                store.agent_expanded_evidence.set(None);
            }));

        stream = stream.child(
            view()
                .p(px(10.0))
                .rounded_sm()
                .bg(rgb(BG_WELL))
                .border_1()
                .border_color(rgb(BORDER_PILL))
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            text(match l {
                                UiLang::Zh => format!("证据核验 · {}", tok.label),
                                UiLang::En => format!("Evidence · {}", tok.label),
                            })
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(hdr_actions),
                )
                .child(
                    text(tok.summary)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .line_height(px(18.0))
                        .text_color(rgb(INK_SECONDARY)),
                ),
        );
    }

    stream
}

fn render_alan_composer_footer(store: AppStore) -> View {
    view()
        .flex_shrink_0()
        .w_full()
        .p(px(12.0))
        .bg(rgb(BG_SURFACE))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(when(
            move || store.alan_anchor_label.with(|a| a.is_some()),
            move || {
                view()
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded_sm()
                    .bg(rgba(SKY_BG))
                    .border_1()
                    .border_color(rgba(SKY_BORDER))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(
                        text(move || {
                            let lbl = store.alan_anchor_label.get().unwrap_or_default();
                            match store.lang.get() {
                                UiLang::Zh => format!("引用锚点：{lbl}"),
                                UiLang::En => format!("Anchor: {lbl}"),
                            }
                        })
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(SKY_FG))
                        .truncate()
                        .number_of_lines(1),
                    )
                    .child(
                        view()
                            .cursor_pointer()
                            .on_press(move || store.alan_anchor_label.set(None))
                            .child(icon(ICON_CLOSE, 11.0).text_color(rgb(INK_MUTED))),
                    )
            },
        ))
        .child(
            view()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    styled_input(
                        move || store.agent_input_draft.get(),
                        move || {
                            let l = store.lang.get();
                            let scope = store.alan_scope.get();
                            let label = scope.label_for(l);
                            match l {
                                UiLang::Zh => format!("向 Alan 追问「{label}」（Enter 发送）…"),
                                UiLang::En => format!("Ask Alan about {label} (Enter to send)…"),
                            }
                        },
                        false,
                    )
                    .flex_1()
                    .min_w(px(0.0))
                    .on_change(move |val: &str| store.agent_input_draft.set(val.to_owned()))
                    .on_submit(move |val: &str| {
                        store.submit_scoped_alan_question(val.to_owned());
                    }),
                )
                .child(action_button(
                    move || store.lang.get().t("发送", "Send").to_owned(),
                    true,
                    move || {
                        let q = store.agent_input_draft.get_untracked();
                        store.submit_scoped_alan_question(q);
                    },
                )),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let streaming = store.agent_streaming.get();
            let has_history = store.agent_messages.with(|m| !m.is_empty());
            let scope = store.alan_scope.get();

            let row = view()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    text(l.t(
                        "会话已按作用域自动持久化 · 最多保留 30 轮",
                        "Session persisted per scope · bounded to 30 turns",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED)),
                );

            let mut btns = view().flex().flex_row().items_center().gap(px(6.0));
            if streaming {
                btns = btns.child(action_button(
                    l.t("中断", "Interrupt"),
                    false,
                    move || store.interrupt_alan(),
                ));
            } else if has_history {
                btns = btns.child(action_button(l.t("清空", "Reset"), false, move || {
                    store.reset_alan_session()
                }));
            }
            if let AlanScope::Company(ticker) = scope {
                let t_prompt = ticker.clone();
                btns = btns.child(action_button(
                    l.t("复制 Prompt", "Copy Prompt"),
                    false,
                    move || {
                        let cur_l = store.lang.get_untracked();
                        let (name, bucket) = store
                            .reports
                            .with_untracked(|reps| {
                                reps.iter()
                                    .find(|r| r.ticker.eq_ignore_ascii_case(&t_prompt))
                                    .map(|r| (r.name.clone(), r.bucket))
                            })
                            .unwrap_or_else(|| (t_prompt.clone(), Bucket::Watch));
                        let p =
                            build_alan_interrogation_prompt_for(&t_prompt, &name, bucket, cur_l);
                        copy_text_with_notice(store, p, &format!("Alan Prompt {t_prompt}"));
                    },
                ));
            }

            row.child(btns)
        }))
}

fn render_alan_feynman_pane(store: AppStore) -> impl IntoElement {
    scroll()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(
            view()
                .w_full()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let scope = store.alan_scope.get();
                    let feynman_active = store.feynman_mode_active.get();
                    let feynman_scope = store.feynman_scope.get();

                    match &scope {
                        AlanScope::Company(ticker) => {
                            let matches_active = feynman_active
                                && matches!(
                                    &feynman_scope,
                                    Some(FeynmanScope::Company { ticker: t }) if t.eq_ignore_ascii_case(ticker)
                                );
                            if matches_active {
                                return view().into_element();
                            }
                            let tk = ticker.clone();
                            obsidian_card()
                                .p(px(16.0))
                                .flex()
                                .flex_col()
                                .gap(px(10.0))
                                .child(
                                    text(match l {
                                        UiLang::Zh => format!("3 轮费曼答辩 · {ticker}"),
                                        UiLang::En => format!("3-Round Feynman Defense · {ticker}"),
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_MD))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(
                                    text(l.t(
                                        "通过「护城河与单位经济 → 5年死法与可证伪指标 → 估值纪律」三轮苏格拉底反刍后，由你亲手签字写入 Dossier。",
                                        "Complete 3 Socratic rounds (Moat & Economics -> 5Y Failure Modes -> Valuation Discipline), then sign off personally into your Dossier.",
                                    ))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .line_height(px(18.0))
                                    .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(
                                    view().pt(px(4.0)).flex().flex_row().child(action_button(
                                        l.t("开始 3 轮费曼答辩", "Start 3-Round Feynman Coach"),
                                        true,
                                        move || store.start_company_feynman(&tk),
                                    )),
                                )
                                .into_element()
                        }
                        AlanScope::Industry(slug) => {
                            let matches_active = feynman_active
                                && matches!(
                                    &feynman_scope,
                                    Some(FeynmanScope::Industry { slug: s }) if s == slug
                                );
                            if matches_active {
                                return view().into_element();
                            }
                            let sl = slug.clone();
                            let label = scope_pill_label(store, &scope, l);
                            obsidian_card()
                                .p(px(16.0))
                                .flex()
                                .flex_col()
                                .gap(px(10.0))
                                .child(
                                    text(match l {
                                        UiLang::Zh => format!("行业 3 轮费曼答辩 · {label}"),
                                        UiLang::En => format!("Industry Feynman Defense · {label}"),
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_MD))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(
                                    text(l.t(
                                        "用大白话讲清该行业利润池归属、失败模式与估值锚区间，完成后归档至行业研究手册。",
                                        "Articulate value-chain profit pools, failure modes, and valuation bounds in plain language to archive your defense.",
                                    ))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .line_height(px(18.0))
                                    .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(
                                    view().pt(px(4.0)).flex().flex_row().child(action_button(
                                        l.t("开始行业费曼答辩", "Start Industry Feynman Coach"),
                                        true,
                                        move || store.start_industry_feynman(&sl),
                                    )),
                                )
                                .into_element()
                        }
                        AlanScope::Briefing(_) | AlanScope::Portfolio => {
                            let sel_ticker = store.selected_ticker.get();
                            let sel_ind = store.selected_industry_slug.get();
                            let mut btns = view().flex().flex_row().flex_wrap().gap(px(8.0));
                            if !sel_ticker.trim().is_empty() {
                                let tk = sel_ticker.clone();
                                btns = btns.child(action_button(
                                    match l {
                                        UiLang::Zh => format!("答辩 {sel_ticker}"),
                                        UiLang::En => format!("Defend {sel_ticker}"),
                                    },
                                    true,
                                    move || {
                                        store.open_alan_for_scope(
                                            AlanScope::Company(tk.clone()),
                                            AlanCanvasTab::Feynman,
                                        );
                                    },
                                ));
                            }
                            if let Some(slug) = sel_ind {
                                let sl = slug.clone();
                                btns = btns.child(action_button(
                                    l.t("答辩当前行业", "Defend Selected Industry"),
                                    false,
                                    move || {
                                        store.open_alan_for_scope(
                                            AlanScope::Industry(sl.clone()),
                                            AlanCanvasTab::Feynman,
                                        );
                                    },
                                ));
                            }
                            obsidian_card()
                                .p(px(16.0))
                                .flex()
                                .flex_col()
                                .gap(px(10.0))
                                .child(
                                    text(l.t(
                                        "费曼答辩需绑定具体公司或行业",
                                        "Feynman Defense Scopes to a Company or Industry",
                                    ))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_MD))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(
                                    text(l.t(
                                        "当前上下文为全局视图。请打开一家公司研报或行业研究，或使用下方快捷入口启动 3 轮费曼答辩。",
                                        "Current scope is overview. Open a company dossier or industry study, or launch a 3-round defense below.",
                                    ))
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .line_height(px(18.0))
                                    .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(btns)
                                .into_element()
                        }
                    }
                }))
                .child(feynman_coach_panel_for_canvas(store)),
        )
}

/// Renders the legacy Company Desk view wrapper (retained while feature views migrate).
pub fn agent_view(store: AppStore) -> impl IntoElement {
    let focus_draft = signal(String::new());
    let tape_drawer_open = signal(false);

    scroll().flex_1().min_w(px(0.0)).min_h(px(0.0)).child(
        view()
            .p(px(20.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(deep_research_console(store, focus_draft))
            .child(feynman_coach_panel(store, false))
            .child(cross_company_context_strip(store))
            .child(living_research_memo_card(store, false))
            .child(research_runs_history_card(store))
            .child(alan_console(store, tape_drawer_open))
            .child(dossier_staff_draft_section(store)),
    )
}

fn current_report_untracked(store: AppStore) -> Option<MungerReport> {
    let ticker = store.selected_ticker.get_untracked();
    store
        .reports
        .with_untracked(|reps| reps.iter().find(|r| r.ticker == ticker).cloned())
}

fn perspective_title_for(id: &str, fallback_cn: &str, lang: UiLang) -> String {
    match (lang, id) {
        (UiLang::En, "moat") => "Moat".to_owned(),
        (UiLang::En, "accounting") => "Accounting".to_owned(),
        (UiLang::En, "latent_peers") => "Latent Peers".to_owned(),
        (UiLang::En, "premortem") => "5Y Pre-Mortem".to_owned(),
        _ => fallback_cn.to_owned(),
    }
}

fn deep_research_console(store: AppStore, focus_draft: Signal<String>) -> View {
    obsidian_card()
        .child(dynamic(move || {
            let l = store.lang.get();
            let ticker = store.selected_ticker.get();
            let running = store.deep_research_running.get();
            let active_for_ticker = store
                .deep_research_ticker
                .get()
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(&ticker));
            let blocked_reason = store.deep_research_budget_blocked.get();
            let last_completed = store
                .deep_research_last_result
                .with(|r| r.as_ref().is_some_and(|x| x.status == "completed"));

            let (budget_txt, budget_fg, budget_bg, budget_br) = match store.budget_report.get() {
                Some(b) => {
                    let used = b.usage.total_cost_usd.round_dp(2).normalize();
                    let limit = b.config.monthly_budget_usd.round_dp(2).normalize();
                    let exhausted = b.remaining_monthly_usd <= Decimal::ZERO
                        || b.remaining_monthly_tokens == 0;
                    let (fg, bg, br) = if exhausted {
                        (CORAL_FG, CORAL_BG, CORAL_BORDER)
                    } else if b.config.monthly_budget_usd > Decimal::ZERO
                        && used * Decimal::from(10) >= b.config.monthly_budget_usd * Decimal::from(8)
                    {
                        (GOLD_FG, GOLD_BG, GOLD_BORDER)
                    } else {
                        (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                    };
                    let lbl = match l {
                        UiLang::Zh => format!("本月 AI 预算 ${used:.2} / ${limit:.2}"),
                        UiLang::En => format!("Monthly AI Budget ${used:.2} / ${limit:.2}"),
                    };
                    (lbl, fg, bg, br)
                }
                None => (
                    l.t("本月 AI 预算 $0.00 / $10.00", "Monthly AI Budget $0.00 / $10.00")
                        .to_owned(),
                    EMERALD_FG,
                    EMERALD_BG,
                    EMERALD_BORDER,
                ),
            };

            let status_badge = if running {
                badge_pill(
                    l.t("4 视角并行分析中…", "4-perspective analysis running…"),
                    VIOLET_FG,
                    VIOLET_BG,
                    VIOLET_BORDER,
                )
            } else if active_for_ticker && blocked_reason.is_some() {
                badge_pill(
                    l.t("预算闸门已拦截", "Budget Gate Blocked"),
                    CORAL_FG,
                    CORAL_BG,
                    CORAL_BORDER,
                )
            } else if active_for_ticker && last_completed {
                badge_pill(
                    l.t("最近运行已完成", "Latest Run Complete"),
                    EMERALD_FG,
                    EMERALD_BG,
                    EMERALD_BORDER,
                )
            } else {
                badge_pill(
                    l.t(
                        "受预算硬门控 · 只读湖仓工具",
                        "Budget Hard-Gated · Read-only Lakehouse Tools",
                    ),
                    SKY_FG,
                    SKY_BG,
                    SKY_BORDER,
                )
            };

            let header_title = if ticker.is_empty() {
                "Research Desk · Deep Research".to_owned()
            } else {
                format!("Research Desk · Deep Research ({ticker})")
            };

            view()
                .px(px(16.0))
                .py(px(12.0))
                .bg(rgb(BG_WELL))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(VIOLET_FG)))
                        .child(
                            text(header_title)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(status_badge)
                        .child(badge_pill(budget_txt, budget_fg, budget_bg, budget_br)),
                )
                .child(action_button(
                    if running {
                        l.t("4 视角并行运行中…", "4-Perspective Run Active…")
                    } else {
                        l.t("启动 4 视角深研", "Run 4-Lens Deep Research")
                    },
                    !running,
                    move || {
                        let cur_ticker = store.selected_ticker.get_untracked();
                        let focus = focus_draft.get_untracked().trim().to_owned();
                        if focus.is_empty() || cur_ticker.trim().is_empty() {
                            store.run_deep_research_selected();
                        } else {
                            store.start_deep_research(&cur_ticker, Some(focus), "manual");
                        }
                    },
                ))
        }))
        // Optional Focus Input mounted once outside dynamic() so IME & focus stay intact
        .child(
            view()
                .px(px(16.0))
                .py(px(10.0))
                .bg(rgb(BG_SURFACE))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    text(move || store.lang.get().t("聚焦维度", "Focus").to_owned())
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                )
                .child(
                    styled_input(
                        move || focus_draft.get(),
                        move || {
                            store
                                .lang
                                .get()
                                .t(
                                    "可选：指定 4 视角深研重点（例如：定价权 / SBC 稀释 / 客户集中度）…",
                                    "Optional focus for 4-lens research (e.g. pricing power, SBC, churn)…",
                                )
                                .to_owned()
                        },
                        false,
                    )
                    .flex_1()
                    .on_change(move |val: &str| {
                        focus_draft.set(val.to_owned());
                    })
                    .on_submit(move |val: &str| {
                        let cur_ticker = store.selected_ticker.get_untracked();
                        let focus = val.trim().to_owned();
                        if focus.is_empty() || cur_ticker.trim().is_empty() {
                            store.run_deep_research_selected();
                        } else {
                            store.start_deep_research(&cur_ticker, Some(focus), "manual");
                        }
                    }),
                )
                .child(when(
                    move || !focus_draft.with(|f| f.trim().is_empty()),
                    move || {
                        action_button(
                            move || store.lang.get().t("清除", "Clear").to_owned(),
                            false,
                            move || focus_draft.set(String::new()),
                        )
                    },
                )),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let ticker = store.selected_ticker.get();
            let active_for_ticker = store
                .deep_research_ticker
                .get()
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(&ticker));

            let mut body = view().flex().flex_col();

            if active_for_ticker && let Some(reason) = store.deep_research_budget_blocked.get() {
                body = body.child(
                    view()
                        .px(px(16.0))
                        .py(px(10.0))
                        .bg(rgba(CORAL_BG))
                        .border_b_1()
                        .border_color(rgba(CORAL_BORDER))
                        .child(
                            text(match l {
                                UiLang::Zh => format!("预算硬闸门已拦截（0 LLM Token 消耗）：{reason}"),
                                UiLang::En => {
                                    format!("Hard budget gate blocked (0 LLM tokens used): {reason}")
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(CORAL_FG)),
                        ),
                );
            } else if active_for_ticker && let Some(note) = store.deep_research_status_note.get() {
                body = body.child(
                    view()
                        .px(px(16.0))
                        .py(px(8.0))
                        .bg(rgb(BG_ROW_ALT))
                        .border_b_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .child(
                            text(note)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                );
            }

            let perspectives: Vec<DeepResearchPerspectiveUi> = if active_for_ticker {
                let live = store.deep_research_perspectives.get();
                if live.is_empty() {
                    default_deep_research_perspectives()
                } else {
                    live
                }
            } else {
                default_deep_research_perspectives()
            };

            let top_pair: Vec<DeepResearchPerspectiveUi> = perspectives.iter().take(2).cloned().collect();
            let bot_pair: Vec<DeepResearchPerspectiveUi> =
                perspectives.iter().skip(2).take(2).collect::<Vec<_>>().into_iter().cloned().collect();

            body.child(
                view()
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        view()
                            .flex()
                            .flex_row()
                            .gap(px(10.0))
                            .children(top_pair.into_iter().map(|p| perspective_card(&p, l))),
                    )
                    .child(
                        view()
                            .flex()
                            .flex_row()
                            .gap(px(10.0))
                            .children(bot_pair.into_iter().map(|p| perspective_card(&p, l))),
                    ),
            )
        }))
}

fn perspective_card(p: &DeepResearchPerspectiveUi, lang: UiLang) -> View {
    let (st_fg, st_bg, st_br) = match p.status {
        PerspectiveUiStatus::Pending => (INK_MUTED, BG_WELL, BORDER_SUBTLE),
        PerspectiveUiStatus::Running => (VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
        PerspectiveUiStatus::Completed => (EMERALD_FG, EMERALD_BG, EMERALD_BORDER),
        PerspectiveUiStatus::Failed => (CORAL_FG, CORAL_BG, CORAL_BORDER),
    };
    let trimmed = p.stream_text.trim();
    let excerpt: String = if trimmed.is_empty() {
        match p.status {
            PerspectiveUiStatus::Pending => lang
                .t(
                    "点击「启动 4 视角深研」后，由独立只读子会话并行核验一手财报与公告证据。",
                    "Click \"Run 4-Lens Deep Research\" to launch parallel read-only sub-sessions verifying primary filings.",
                )
                .to_owned(),
            PerspectiveUiStatus::Running => lang
                .t(
                    "正在检索 Lakehouse 工具与生成分析结论…",
                    "Querying Lakehouse tools and synthesizing findings…",
                )
                .to_owned(),
            PerspectiveUiStatus::Completed => match lang {
                UiLang::Zh => format!("已完成（{} 字符，已合并至 Living Research Memo）", p.summary_chars),
                UiLang::En => format!("Completed ({} chars, merged into Living Research Memo)", p.summary_chars),
            },
            PerspectiveUiStatus::Failed => lang
                .t(
                    "未产生输出（已保留既有工作区状态）",
                    "No output produced (existing workspace preserved)",
                )
                .to_owned(),
        }
    } else {
        let chars: Vec<char> = trimmed.chars().collect();
        if chars.len() > 240 {
            let head: String = chars[..240].iter().collect();
            format!("{head}…")
        } else {
            trimmed.to_owned()
        }
    };

    let tools_line = if p.tools_used.is_empty() {
        "tools: —".to_owned()
    } else {
        format!("tools: {}", p.tools_used.join(", "))
    };
    let tok_line = if p.input_tokens > 0 || p.output_tokens > 0 {
        format!("{}in / {}out", p.input_tokens, p.output_tokens)
    } else if p.summary_chars > 0 {
        format!("{} chars", p.summary_chars)
    } else {
        p.id.clone()
    };

    view()
        .flex_1()
        .min_w(px(0.0))
        .p(px(12.0))
        .rounded_md()
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    text(perspective_title_for(&p.id, &p.title_cn, lang))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(badge_pill(p.status.label_for(lang), st_fg, st_bg, st_br)),
        )
        .child(
            text(excerpt)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .line_height(px(18.0))
                .text_color(rgb(if trimmed.is_empty() {
                    INK_MUTED
                } else {
                    INK_PRIMARY
                })),
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
                        text(tools_line)
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_MUTED))
                            .truncate()
                            .number_of_lines(1),
                    ),
                )
                .child(
                    text(tok_line)
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
}

/// Shared SIC Peer Percentiles & Lookalike Neighbors strip (used by both Tab 1 & Tab 2).
pub(super) fn cross_company_context_strip(store: AppStore) -> View {
    obsidian_card()
        .p(px(16.0))
        .child(dynamic(move || {
            let l = store.lang.get();
            let ticker = store.selected_ticker.get();
            let peers_opt = store
                .selected_peers
                .get()
                .filter(|p| p.target.ticker.eq_ignore_ascii_case(&ticker) && p.peer_count > 1);
            let sim_hits = store.selected_similar_hits.get();
            let has_data = peers_opt.is_some() || !sim_hits.is_empty();

            let mut card = view().flex().flex_col().gap(px(12.0)).child(section_header(
                l.t(
                    "跨公司对照 · 同业分位数与语义相似",
                    "Cross-Company Comparison · Peer Percentiles & Latent Lookalikes",
                ),
                if has_data {
                    l.t("本地 L0/L1 · 0 LLM Token", "DUCKDB L0/L1 · 0 LLM TOKEN")
                        .to_owned()
                } else {
                    l.t("等待 L0/L1 湖仓数据", "Awaiting L0/L1 Lakehouse Data")
                        .to_owned()
                },
            ));

            if !has_data {
                return card.child(
                    text(l.t(
                        "同步全美股 L0/L1 湖仓后，此处展示 SIC 同业 ROIC/毛利/FCF 分位数与语义相似近邻公司。",
                        "After syncing US L0/L1 lakehouse, SIC peer ROIC/gross margin/strict FCF percentiles and semantic lookalikes appear here.",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                );
            }

            if let Some(peers) = peers_opt {
                let roic_p = peers
                    .roic_percentile
                    .map(|v| format!("P{v:.0}"))
                    .unwrap_or_else(|| "—".to_owned());
                let gm_p = peers
                    .gross_margin_percentile
                    .map(|v| format!("P{v:.0}"))
                    .unwrap_or_else(|| "—".to_owned());
                let fcf_p = peers
                    .strict_fcf_yield_percentile
                    .map(|v| format!("P{v:.0}"))
                    .unwrap_or_else(|| "—".to_owned());

                card = card.child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .flex_wrap()
                                .gap(px(8.0))
                                .child(badge_pill(
                                    match l {
                                        UiLang::Zh => {
                                            format!("{} · {} 家同行", peers.sic_group, peers.peer_count)
                                        }
                                        UiLang::En => {
                                            format!("{} · {} peers", peers.sic_group, peers.peer_count)
                                        }
                                    },
                                    SKY_FG,
                                    SKY_BG,
                                    SKY_BORDER,
                                ))
                                .child(
                                    text(match l {
                                        UiLang::Zh => format!(
                                            "ROIC {roic_p} · 毛利率 {gm_p} · 严苛 FCF {fcf_p}"
                                        ),
                                        UiLang::En => format!(
                                            "ROIC {roic_p} · Gross Margin {gm_p} · Strict FCF {fcf_p}"
                                        ),
                                    })
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_PRIMARY)),
                                ),
                        )
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .flex_wrap()
                                .gap(px(6.0))
                                .children(peers.top_peers.into_iter().take(5).map(|p| {
                                    let jump = p.ticker.clone();
                                    let roic_s = p
                                        .roic_5y_avg_pct
                                        .map(|v| format!("{v:.0}%"))
                                        .unwrap_or_else(|| "—".to_owned());
                                    view()
                                        .px(px(8.0))
                                        .py(px(2.0))
                                        .rounded_xs()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgb(BORDER_SUBTLE))
                                        .hover(|s| s.bg(rgb(BG_HOVER)))
                                        .cursor_pointer()
                                        .on_press(move || store.open_theme_member(&jump))
                                        .child(
                                            text(format!("{} {roic_s}", p.ticker))
                                                .font_family(FONT_MONO_NUM)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_SECONDARY)),
                                        )
                                })),
                        ),
                );
            }

            if !sim_hits.is_empty() {
                card = card.child(
                    view()
                        .pt(px(8.0))
                        .border_t_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(6.0))
                        .child(
                            text(l.t("语义相似公司:", "Latent Lookalike:"))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        )
                        .children(sim_hits.into_iter().map(|hit| {
                            let jump = hit.company.ticker.clone();
                            let sim_pct = (hit.score * 100.0).round() as i64;
                            let doc_type = hit
                                .evidence
                                .first()
                                .map_or("10-K", |e| e.doc_type.as_str())
                                .to_owned();
                            view()
                                .px(px(8.0))
                                .py(px(2.0))
                                .rounded_xs()
                                .bg(rgba(VIOLET_BG))
                                .border_1()
                                .border_color(rgba(VIOLET_BORDER))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .on_press(move || store.open_theme_member(&jump))
                                .child(
                                    text(format!("{} · {}% · {}", hit.company.ticker, sim_pct, doc_type))
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(VIOLET_FG)),
                                )
                        })),
                );
            }

            card
        }))
}

/// Shared Living Research Memo card (compact summary for Tab 1, full 4-lens + citations for Tab 2).
pub(super) fn living_research_memo_card(store: AppStore, compact: bool) -> View {
    obsidian_card().child(dynamic(move || {
        let l = store.lang.get();
        let ticker = store.selected_ticker.get();
        let memo_rel = format!("workbench/memos/{ticker}.md");
        let raw_opt = store.selected_memo_raw.get();
        let parsed_opt = store.selected_memo_parsed.get();

        let mut actions = view().flex().flex_row().items_center().gap(px(8.0));
        if let Some(md) = raw_opt {
            let t_copy = ticker.clone();
            actions = actions.child(action_button(
                l.t("复制 Markdown", "Copy Markdown"),
                false,
                move || {
                    copy_text_with_notice(store, md.clone(), &format!("Memo {t_copy}"));
                },
            ));
        }
        if compact {
            actions = actions.child(action_button(
                l.t("展开 4 视角 & Alan →", "Open Deep Research & Alan →"),
                false,
                move || store.company_view_tab.set(CompanyViewTab::DeepResearchAlan),
            ));
        }
        let t_refresh = ticker.clone();
        actions = actions.child(action_button(l.t("刷新", "Refresh"), false, move || {
            store.reload_research_desk_context(&t_refresh);
            let cur_l = store.lang.get_untracked();
            store.set_info_notice(match cur_l {
                UiLang::Zh => format!("已从磁盘刷新 {t_refresh} Memo"),
                UiLang::En => format!("Reloaded {t_refresh} Memo from disk"),
            });
        }));

        let header = view()
            .px(px(16.0))
            .py(px(10.0))
            .bg(rgb(BG_WELL))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                view()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(EMERALD_FG)))
                    .child(
                        text(format!("Living Research Memo · {memo_rel}"))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                    ),
            )
            .child(actions);

        let mut card = view().flex().flex_col().child(header);

        let Some(memo) = parsed_opt else {
            return card.child(
                view().p(px(16.0)).child(
                    text(match l {
                        UiLang::Zh => format!(
                            "尚未生成 {memo_rel}。点击「启动 4 视角深研」或运行 `m3 research run {ticker}` 后将在此展示结构化活研报与一手引用。"
                        ),
                        UiLang::En => format!(
                            "{memo_rel} not generated yet. Click \"Run 4-Lens Deep Research\" or run `m3 research run {ticker}` to render the structured living memo and primary citations."
                        ),
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(19.0))
                    .text_color(rgb(INK_MUTED)),
                ),
            );
        };

        if !memo.metadata.is_empty() {
            card = card.child(
                view()
                    .px(px(16.0))
                    .py(px(8.0))
                    .bg(rgb(BG_ROW_ALT))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_wrap()
                    .gap(px(12.0))
                    .children(memo.metadata.iter().map(|(k, v)| {
                        view()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                text(format!("{k}:"))
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                text(v.clone())
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_PRIMARY)),
                            )
                    })),
            );
        }

        let perspective_sections: Vec<&MemoSection> = memo
            .sections
            .iter()
            .filter(|s| matches!(s.id.as_str(), "moat" | "accounting" | "latent_peers" | "premortem"))
            .collect();

        let mut body = view().p(px(16.0)).flex().flex_col().gap(px(12.0));
        if compact {
            for sec in perspective_sections {
                let first_line = sec
                    .body
                    .lines()
                    .map(str::trim)
                    .find(|ln| !ln.is_empty())
                    .unwrap_or("—");
                let chars: Vec<char> = first_line.chars().collect();
                let summary_line = if chars.len() > 160 {
                    let head: String = chars[..160].iter().collect();
                    format!("{head}…")
                } else {
                    first_line.to_owned()
                };
                body = body.child(
                    view()
                        .p(px(10.0))
                        .rounded_sm()
                        .bg(rgb(BG_VIEWPORT))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            text(sec.heading.clone())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(summary_line)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .line_height(px(18.0))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                );
            }
            if !memo.citations.is_empty() {
                body = body.child(
                    text(match l {
                        UiLang::Zh => format!("已挂载一手引用 {} 条（完整列表见 Tab 2）", memo.citations.len()),
                        UiLang::En => format!("{} primary citations attached (full list in Tab 2)", memo.citations.len()),
                    })
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(SKY_FG)),
                );
            }
        } else {
            for sec in perspective_sections {
                body = body.child(
                    view()
                        .p(px(12.0))
                        .rounded_sm()
                        .bg(rgb(BG_VIEWPORT))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            text(sec.heading.clone())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(sec.body.clone())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .line_height(px(20.0))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                );
            }

            if !memo.citations.is_empty() {
                body = body.child(
                    view()
                        .p(px(12.0))
                        .rounded_sm()
                        .bg(rgb(BG_VIEWPORT))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            text(format!("EVIDENCE & CITATIONS ({})", memo.citations.len()))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(SKY_FG)),
                        )
                        .children(memo.citations.iter().take(10).map(|cite| {
                            text(cite.clone())
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .line_height(px(17.0))
                                .text_color(rgb(INK_SECONDARY))
                        })),
                );
            }
        }

        if let Some(notes) = &memo.human_notes {
            body = body.child(
                view()
                    .p(px(12.0))
                    .rounded_sm()
                    .bg(rgba(GOLD_BG))
                    .border_1()
                    .border_color(rgba(GOLD_BORDER))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        text(l.t(
                            "人类主权区（深度研究永不覆盖）",
                            "## Human Notes (Human-Owned · Never Overwritten by Deep Research)",
                        ))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(GOLD_FG)),
                    )
                    .child(
                        text(notes.clone())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .line_height(px(19.0))
                            .text_color(rgb(INK_PRIMARY)),
                    ),
            );
        }

        card.child(body)
    }))
}

fn research_runs_history_card(store: AppStore) -> View {
    obsidian_card().p(px(16.0)).child(dynamic(move || {
        let l = store.lang.get();
        let ticker = store.selected_ticker.get();
        let runs = store.selected_research_runs.get();

        let mut card = view().flex().flex_col().gap(px(10.0)).child(section_header(
            &match l {
                UiLang::Zh => format!("Deep Research 审计记录 · {ticker}"),
                UiLang::En => format!("Deep Research Audit Log · {ticker}"),
            },
            match l {
                UiLang::Zh => format!("{} 条最近运行（m3.duckdb · research_runs）", runs.len()),
                UiLang::En => format!("{} recent runs (m3.duckdb · research_runs)", runs.len()),
            },
        ));

        if runs.is_empty() {
            return card.child(
                text(l.t(
                    "暂无该标的的 Deep Research 运行记录。",
                    "No Deep Research runs recorded for this ticker yet.",
                ))
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED)),
            );
        }

        let len = runs.len();
        for (idx, run) in runs.into_iter().enumerate() {
            let (st_label, st_fg, st_bg, st_br) = match run.status {
                ResearchRunStatus::Completed => {
                    ("completed", EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                }
                ResearchRunStatus::BlockedBudget => {
                    ("blocked_budget", CORAL_FG, CORAL_BG, CORAL_BORDER)
                }
                ResearchRunStatus::Failed => ("failed", GOLD_FG, GOLD_BG, GOLD_BORDER),
                ResearchRunStatus::Running => ("running", VIOLET_FG, VIOLET_BG, VIOLET_BORDER),
            };
            let detail_txt = run
                .error
                .as_deref()
                .or(run.memo_path.as_deref())
                .unwrap_or("—")
                .to_owned();
            let mut row = view()
                .py(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0));
            if idx + 1 < len {
                row = row.border_b_1().border_color(rgb(BORDER_HAIRLINE));
            }
            card = card.child(
                row.child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(badge_pill(st_label, st_fg, st_bg, st_br))
                        .child(
                            text(run.run_id)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_PRIMARY)),
                        )
                        .child(
                            text(format!("[{}]", run.trigger))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        )
                        .child(
                            view().flex_1().min_w(px(0.0)).child(
                                text(detail_txt)
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_SECONDARY))
                                    .truncate()
                                    .number_of_lines(1),
                            ),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            text(format!("{}in/{}out", run.input_tokens, run.output_tokens))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(
                            text(format!("${:.4}", run.cost_usd.round_dp(4).normalize()))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(
                            text(fmt_time_cn_for(Some(run.started_at_ms), l))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                ),
            );
        }
        card
    }))
}

fn alan_console(store: AppStore, tape_drawer_open: Signal<bool>) -> View {
    obsidian_card()
        // Header bar
        .child(dynamic(move || {
            let l = store.lang.get();
            let ticker = store.selected_ticker.get();
            let streaming = store.agent_streaming.get();
            let hint = store.agent_status_hint.get();
            let has_history =
                store.agent_messages.with(|m| !m.is_empty()) || store.agent_tape.with(|t| !t.is_empty());

            let status_pill = if streaming {
                badge_pill(
                    hint.unwrap_or_else(|| {
                        l.t("Alan 审讯运行中…", "Alan interrogation running…")
                            .to_owned()
                    }),
                    VIOLET_FG,
                    VIOLET_BG,
                    VIOLET_BORDER,
                )
            } else {
                badge_pill(
                    l.t(
                        "只读 /mnt/m3 · 确定性工具",
                        "Read-only /mnt/m3 · Deterministic Tools",
                    ),
                    EMERALD_FG,
                    EMERALD_BG,
                    EMERALD_BORDER,
                )
            };

            let mut actions = view().flex().flex_row().items_center().gap(px(8.0));
            if streaming {
                actions = actions.child(action_button(
                    l.t("中断生成", "Interrupt"),
                    false,
                    move || store.interrupt_alan(),
                ));
            } else {
                let feynman_tk = ticker.clone();
                actions = actions
                    .child(action_button(
                        l.t("3 轮费曼答辩", "3-Round Feynman Coach"),
                        true,
                        move || {
                            if !feynman_tk.is_empty() {
                                store.start_company_feynman(&feynman_tk);
                            }
                        },
                    ))
                    .child(action_button(
                        l.t("费曼审讯", "Feynman Interrogation"),
                        false,
                        move || {
                            if let Some(rep) = current_report_untracked(store) {
                                store.start_feynman_interrogation(
                                    &rep.ticker,
                                    &rep.name,
                                    rep.bucket,
                                );
                            } else {
                                let t = store.selected_ticker.get_untracked();
                                if !t.is_empty() {
                                    store.start_feynman_interrogation(&t, &t, Bucket::Watch);
                                }
                            }
                        },
                    ));
            }
            if has_history {
                actions = actions.child(action_button(l.t("清空", "Reset"), false, move || {
                    store.reset_alan_session();
                }));
            }
            actions = actions.child(action_button(
                l.t("复制审讯 Prompt", "Copy Prompt"),
                false,
                move || {
                    let cur_l = store.lang.get_untracked();
                    if let Some(rep) = current_report_untracked(store) {
                        let p = build_alan_interrogation_prompt_for(
                            &rep.ticker,
                            &rep.name,
                            rep.bucket,
                            cur_l,
                        );
                        copy_text_with_notice(store, p, &format!("Alan Prompt {}", rep.ticker));
                    }
                },
            ));

            view()
                .px(px(16.0))
                .py(px(10.0))
                .bg(rgb(BG_WELL))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(VIOLET_FG)))
                        .child(
                            text(match l {
                                UiLang::Zh => format!("Alan 费曼审讯台 · {ticker}"),
                                UiLang::En => format!("Alan Feynman Desk · {ticker}"),
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(status_pill),
                )
                .child(actions)
        }))
        // Native IME-safe Composer mounted ONCE outside dynamic()
        .child(
            view()
                .p(px(12.0))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    styled_input(
                        move || store.agent_input_draft.get(),
                        move || {
                            let t = store.selected_ticker.get();
                            match store.lang.get() {
                                UiLang::Zh => {
                                    format!("输入对 {t} 的费曼追问（Enter 发送，支持中文输入法）…")
                                }
                                UiLang::En => {
                                    format!("Ask {t} a Feynman follow-up (Enter to send)…")
                                }
                            }
                        },
                        false,
                    )
                    .flex_1()
                    .on_change(move |val: &str| {
                        store.agent_input_draft.set(val.to_owned());
                    })
                    .on_submit(move |val: &str| {
                        let q = val.trim().to_owned();
                        if q.is_empty() {
                            return;
                        }
                        if let Some(rep) = current_report_untracked(store) {
                            store.submit_alan_question(&rep.ticker, &rep.name, rep.bucket, q);
                        } else {
                            let t = store.selected_ticker.get_untracked();
                            if !t.is_empty() {
                                store.submit_alan_question(&t, &t, Bucket::Watch, q);
                            }
                        }
                    }),
                )
                .child(action_button(
                    move || store.lang.get().t("发送", "Send").to_owned(),
                    true,
                    move || {
                        let q = store.agent_input_draft.get_untracked();
                        if q.trim().is_empty() {
                            return;
                        }
                        if let Some(rep) = current_report_untracked(store) {
                            store.submit_alan_question(&rep.ticker, &rep.name, rep.bucket, q);
                        } else {
                            let t = store.selected_ticker.get_untracked();
                            if !t.is_empty() {
                                store.submit_alan_question(&t, &t, Bucket::Watch, q);
                            }
                        }
                    },
                )),
        )
        // Message stream + clickable Acme evidence tokens
        .child(dynamic(move || {
            let l = store.lang.get();
            let msgs = store.agent_messages.get();
            let expanded_ev = store.agent_expanded_evidence.get();

            let expanded_token = expanded_ev.as_ref().and_then(|eid| {
                msgs.iter()
                    .flat_map(|m| m.tokens.iter())
                    .find(|t| &t.id == eid)
                    .cloned()
            });

            let mut stream = view().p(px(16.0)).flex().flex_col().gap(px(12.0));
            if msgs.is_empty() {
                stream = stream.child(
                    text(l.t(
                        "点击「费曼审讯」或在上方输入追问，直接与内置 Alan 对话；工具受红线策略钩子硬保护（严禁触碰 human_judgment 与 computed/*）。",
                        "Click \"Feynman Interrogation\" or enter a follow-up above to talk directly with Alan; tools are enforced by redline policy hooks (blocking human_judgment and computed/*).",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(20.0))
                    .text_color(rgb(INK_MUTED)),
                );
            } else {
                for msg in msgs {
                    match msg.role {
                        AgentMessageRole::User => {
                            stream = stream.child(
                                view()
                                    .px(px(12.0))
                                    .py(px(8.0))
                                    .rounded_sm()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_HAIRLINE))
                                    .child(
                                        text(match l {
                                            UiLang::Zh => format!("你  ·  {}", msg.text),
                                            UiLang::En => format!("You  ·  {}", msg.text),
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .line_height(px(19.0))
                                        .text_color(rgb(INK_HERO)),
                                    ),
                            );
                        }
                        AgentMessageRole::SystemNote => {
                            stream = stream.child(
                                view()
                                    .px(px(12.0))
                                    .py(px(8.0))
                                    .rounded_sm()
                                    .bg(rgba(GOLD_BG))
                                    .border_1()
                                    .border_color(rgba(GOLD_BORDER))
                                    .child(
                                        text(msg.text)
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_XS))
                                            .line_height(px(19.0))
                                            .text_color(rgb(GOLD_FG)),
                                    ),
                            );
                        }
                        AgentMessageRole::Assistant => {
                            let body = if msg.text.is_empty() && msg.streaming {
                                l.t(
                                    "Alan 正在读取 /mnt/m3 与确定性报告…",
                                    "Alan is reading /mnt/m3 and deterministic reports…",
                                )
                                .to_owned()
                            } else {
                                msg.text.clone()
                            };
                            let mut box_el = view().flex().flex_col().gap(px(8.0)).child(
                                text(body)
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_BASE))
                                    .line_height(px(22.0))
                                    .text_color(rgb(if msg.text.is_empty() && msg.streaming {
                                        INK_MUTED
                                    } else {
                                        INK_PRIMARY
                                    })),
                            );
                            if !msg.tokens.is_empty() {
                                let chips = view().flex().flex_row().flex_wrap().gap(px(6.0)).children(
                                    msg.tokens.into_iter().map(|tok| {
                                        let tok_id = tok.id.clone();
                                        let selected = expanded_ev.as_deref() == Some(tok_id.as_str());
                                        let (fg, bg, border) = match tok.kind {
                                            AcmeEvidenceKind::Report => {
                                                (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                                            }
                                            AcmeEvidenceKind::Xbrl => (SKY_FG, SKY_BG, SKY_BORDER),
                                            AcmeEvidenceKind::Dossier => {
                                                (GOLD_FG, GOLD_BG, GOLD_BORDER)
                                            }
                                        };
                                        view()
                                            .px(px(8.0))
                                            .py(px(2.0))
                                            .rounded_sm()
                                            .bg(if selected { rgb(BG_ACTIVE) } else { rgba(bg) })
                                            .border_1()
                                            .border_color(rgba(border))
                                            .hover(|s| s.bg(rgb(BG_HOVER)))
                                            .cursor_pointer()
                                            .on_press(move || {
                                                store.agent_expanded_evidence.update(|cur| {
                                                    if cur.as_deref() == Some(tok_id.as_str()) {
                                                        *cur = None;
                                                    } else {
                                                        *cur = Some(tok_id.clone());
                                                    }
                                                });
                                            })
                                            .child(
                                                text(tok.label)
                                                    .font_family(FONT_MONO_TERM)
                                                    .text_size(px(TEXT_XS))
                                                    .text_color(rgb(fg)),
                                            )
                                    }),
                                );
                                box_el = box_el.child(chips);
                            }
                            stream = stream.child(box_el);
                        }
                    }
                }
            }

            if let Some(tok) = expanded_token {
                let jump_ticker = tok.ticker.clone();
                let is_xbrl = tok.kind == AcmeEvidenceKind::Xbrl;
                let cite_text = format!("[{}:{}] {}", tok.kind.prefix(), tok.ticker, tok.summary);
                let cite_label = tok.label.clone();

                let mut hdr_actions = view().flex().flex_row().items_center().gap(px(8.0));
                if is_xbrl {
                    hdr_actions = hdr_actions.child(action_button(
                        l.t("跳至 3 XBRL 底表 →", "Jump to 3 XBRL Ledger →"),
                        false,
                        move || {
                            let exists = store.reports.with_untracked(|reps| {
                                reps.iter().any(|r| r.ticker.eq_ignore_ascii_case(&jump_ticker))
                            });
                            if exists {
                                store.load_selection(&jump_ticker);
                            }
                            store.company_view_tab.set(CompanyViewTab::XbrlInspector);
                        },
                    ));
                }
                hdr_actions = hdr_actions
                    .child(action_button(
                        l.t("复制引用", "Copy Citation"),
                        false,
                        move || {
                            copy_text_with_notice(store, cite_text.clone(), &cite_label);
                        },
                    ))
                    .child(action_button(l.t("收起", "Collapse"), false, move || {
                        store.agent_expanded_evidence.set(None);
                    }));

                stream = stream.child(
                    view()
                        .p(px(12.0))
                        .rounded_sm()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_PILL))
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
                                    text(match l {
                                        UiLang::Zh => format!("Acme 证据核验 · {}", tok.label),
                                        UiLang::En => {
                                            format!("Acme Evidence Verification · {}", tok.label)
                                        }
                                    })
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(hdr_actions),
                        )
                        .child(
                            text(tok.summary)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .line_height(px(19.0))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                );
            }

            stream
        }))
        // Expandable Turing Tape event drawer
        .child(dynamic(move || {
            let l = store.lang.get();
            let tape = store.agent_tape.get();
            if tape.is_empty() {
                return view();
            }
            let open = tape_drawer_open.get();
            let expanded_seq = store.agent_expanded_tape.get();
            let len = tape.len();

            let mut drawer = view()
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .bg(rgb(BG_WELL))
                .px(px(16.0))
                .py(px(10.0))
                .flex()
                .flex_col()
                .child(
                    view()
                        .pb(px(if open { 6.0 } else { 0.0 }))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .on_press(move || tape_drawer_open.update(|v| *v = !*v))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(5.0))
                                .child(
                                    icon(
                                        if open {
                                            ICON_CHEVRON_DOWN
                                        } else {
                                            ICON_CHEVRON_RIGHT
                                        },
                                        12.0,
                                    )
                                    .text_color(rgb(INK_MUTED)),
                                )
                                .child(
                                    text(format!("TURING TAPE · DETERMINISTIC TRACE ({len})"))
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(INK_MUTED)),
                                ),
                        )
                        .child(
                            text(if open {
                                l.t("点击折叠", "Collapse")
                            } else {
                                l.t("点击展开", "Expand")
                            })
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_MUTED)),
                        ),
                );

            if open {
                for (idx, row) in tape.into_iter().enumerate() {
                    let row_seq = row.seq;
                    let row_expanded = expanded_seq == Some(row_seq);
                    let tag_color = match row.kind {
                        TuringTapeKind::Input => INK_MUTED,
                        TuringTapeKind::Tool => SKY_FG,
                        TuringTapeKind::Out => EMERALD_FG,
                        TuringTapeKind::Policy => CORAL_FG,
                        TuringTapeKind::Error => GOLD_FG,
                    };
                    let mut entry = view()
                        .py(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .on_press(move || {
                            store.agent_expanded_tape.update(|cur| {
                                if *cur == Some(row_seq) {
                                    *cur = None;
                                } else {
                                    *cur = Some(row_seq);
                                }
                            });
                        });
                    if idx + 1 < len {
                        entry = entry.border_b_1().border_color(rgb(BORDER_HAIRLINE));
                    }
                    entry = entry.child(
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
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        text(format!("{:04}", row.seq))
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(INK_MUTED)),
                                    )
                                    .child(
                                        text(format!("[{}]", row.kind.tag()))
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(tag_color)),
                                    )
                                    .child(
                                        view().flex_1().min_w(px(0.0)).child(
                                            text(row.summary)
                                                .font_family(FONT_MONO_TERM)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_PRIMARY))
                                                .truncate()
                                                .number_of_lines(1),
                                        ),
                                    ),
                            )
                            .child(
                                text(row.status)
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED)),
                            ),
                    );
                    if row_expanded && let Some(detail) = row.detail {
                        entry = entry.child(
                            view().pl(px(24.0)).child(
                                text(detail)
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .line_height(px(17.0))
                                    .text_color(rgb(INK_SECONDARY)),
                            ),
                        );
                    }
                    drawer = drawer.child(entry);
                }
            }
            drawer
        }))
}

fn dossier_staff_draft_section(store: AppStore) -> View {
    view().child(dynamic(move || {
        let l = store.lang.get();
        let ticker = store.selected_ticker.get();
        let dossier_opt = store.selected_dossier.get();
        let draft_source = dossier_opt
            .as_ref()
            .map(|d| d.draft_source.clone())
            .unwrap_or_else(|| "agent_draft".to_owned());
        let updated_at = dossier_opt
            .as_ref()
            .map(|d| d.updated_at.clone())
            .unwrap_or_else(|| "—".to_owned());
        let dossier_file = format!("workbench/dossiers/{ticker}.json");
        let dossier_cmd = format!("m3 dossier {ticker}");
        let ticker_cli = ticker.clone();

        let mut col = view().flex().flex_col().gap(px(16.0)).child(
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
                        .gap(px(8.0))
                        .child(view().w(px(6.0)).h(px(6.0)).rounded_full().bg(rgb(VIOLET_FG)))
                        .child(
                            text(match l {
                                UiLang::Zh => format!("AI 幕僚草稿 · {draft_source}"),
                                UiLang::En => format!("AI Staff Draft · {draft_source}"),
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(VIOLET_FG)),
                        ),
                )
                .child(
                    text(match l {
                        UiLang::Zh => format!("更新于 {updated_at}"),
                        UiLang::En => format!("Updated {updated_at}"),
                    })
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                ),
        );

        if let Some(dossier) = dossier_opt {
            let pending = dossier.human_judgment.pending_human_review;
            let quad = |title: &str, body: &str| {
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        text(title.to_owned())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                    )
                    .child(
                        text(body.to_owned())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_BASE))
                            .line_height(px(21.0))
                            .text_color(rgb(INK_SECONDARY)),
                    )
            };

            let pm_len = dossier.premortem.len();
            let q_len = dossier.open_questions.len();

            col = col
                .child(
                    obsidian_card()
                        .p(px(16.0))
                        .child(prose_block(
                            l.t("能力圈三句话过滤", "3-Sentence Business Filter"),
                            &dossier.three_sentence_business,
                        )),
                )
                .child(
                    obsidian_card()
                        .child(
                            view()
                                .px(px(16.0))
                                .py(px(10.0))
                                .bg(rgb(BG_WELL))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .child(
                                    text(l.t("跨学科思维格栅", "Interdisciplinary Latticework"))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO)),
                                ),
                        )
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .child(
                                    quad(
                                        l.t("微观经济与护城河", "Microeconomics & Moat"),
                                        &dossier.latticework.microeconomics,
                                    )
                                    .border_r_1()
                                    .border_color(rgb(BORDER_HAIRLINE)),
                                )
                                .child(quad(
                                    l.t("生态位与技术演化", "Ecosystem & Tech Evolution"),
                                    &dossier.latticework.ecology,
                                )),
                        )
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .child(
                                    quad(
                                        l.t("心理学与管理层激励", "Psychology & Incentives"),
                                        &dossier.latticework.psychology_incentives,
                                    )
                                    .border_r_1()
                                    .border_color(rgb(BORDER_HAIRLINE)),
                                )
                                .child(quad(
                                    l.t("资本配置历史", "Capital Allocation History"),
                                    &dossier.latticework.capital_allocation,
                                )),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .gap(px(16.0))
                        .child(
                            obsidian_card()
                                .flex_1()
                                .min_w(px(0.0))
                                .p(px(16.0))
                                .flex()
                                .flex_col()
                                .child(
                                    view()
                                        .pb(px(10.0))
                                        .border_b_1()
                                        .border_color(rgb(BORDER_HAIRLINE))
                                        .child(
                                            text(l.t(
                                                "5 年逆向事前验尸（亏损 >50% 路径）",
                                                "5-Year Pre-Mortem (>50% Drawdown Paths)",
                                            ))
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_SM))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(CORAL_FG)),
                                        ),
                                )
                                .child(if dossier.premortem.is_empty() {
                                    view().pt(px(12.0)).child(
                                        text(l.t(
                                            "（尚无事前验尸草稿）",
                                            "(No pre-mortem draft yet)",
                                        ))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                    )
                                } else {
                                    view().flex().flex_col().children(
                                        dossier.premortem.into_iter().enumerate().map(|(i, item)| {
                                            let mut row = view().py(px(10.0)).child(
                                                text(format!("{}. {item}", i + 1))
                                                    .font_family(FONT_UI)
                                                    .text_size(px(TEXT_BASE))
                                                    .line_height(px(21.0))
                                                    .text_color(rgb(INK_PRIMARY)),
                                            );
                                            if i + 1 < pm_len {
                                                row = row
                                                    .border_b_1()
                                                    .border_color(rgb(BORDER_HAIRLINE));
                                            }
                                            row
                                        }),
                                    )
                                }),
                        )
                        .child(
                            obsidian_card()
                                .w(px(320.0))
                                .flex_shrink_0()
                                .p(px(16.0))
                                .flex()
                                .flex_col()
                                .child(
                                    view()
                                        .pb(px(10.0))
                                        .border_b_1()
                                        .border_color(rgb(BORDER_HAIRLINE))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            text(l.t(
                                                "费曼待答辩关键质疑",
                                                "Open Feynman Interrogation Questions",
                                            ))
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_SM))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO)),
                                        )
                                        .child(if pending {
                                            badge_pill(
                                                l.t("待作答", "Open"),
                                                GOLD_FG,
                                                GOLD_BG,
                                                GOLD_BORDER,
                                            )
                                        } else {
                                            badge_pill(
                                                l.t("已通过", "Cleared"),
                                                EMERALD_FG,
                                                EMERALD_BG,
                                                EMERALD_BORDER,
                                            )
                                        }),
                                )
                                .child(
                                    view().flex().flex_col().children(
                                        dossier.open_questions.into_iter().enumerate().map(|(i, q)| {
                                            let mut row = view().py(px(10.0)).child(
                                                text(format!("? {q}"))
                                                    .font_family(FONT_UI)
                                                    .text_size(px(TEXT_BASE))
                                                    .line_height(px(20.0))
                                                    .text_color(rgb(INK_SECONDARY)),
                                            );
                                            if i + 1 < q_len {
                                                row = row
                                                    .border_b_1()
                                                    .border_color(rgb(BORDER_HAIRLINE));
                                            }
                                            row
                                        }),
                                    ),
                                ),
                        ),
                );
        } else {
            col = col.child(
                obsidian_card().p(px(16.0)).child(
                    text(match l {
                        UiLang::Zh => format!(
                            "尚无 {ticker} 审讯档案（{dossier_file}），可点击上方「费曼审讯」或「启动 4 视角深研」生成。"
                        ),
                        UiLang::En => format!(
                            "No interrogation dossier for {ticker} ({dossier_file}). Start Feynman Interrogation or Run 4-Lens Deep Research above."
                        ),
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                ),
            );
        }

        // Keep card_title import used cleanly if needed, or expose footer
        let _ = card_title;

        col.child(
            view()
                .pt(px(8.0))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text(dossier_file)
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                )
                .child(action_button("Copy dossier CLI", false, move || {
                    copy_text_with_notice(store, dossier_cmd.clone(), &format!("dossier {ticker_cli}"));
                })),
        )
    }))
}

fn is_feynman_panel_visible_for_canvas(store: AppStore) -> bool {
    if !store.feynman_mode_active.get() {
        return false;
    }
    match (store.alan_scope.get(), store.feynman_scope.get()) {
        (AlanScope::Company(a), Some(FeynmanScope::Company { ticker: b })) => {
            a.eq_ignore_ascii_case(&b)
        }
        (AlanScope::Industry(a), Some(FeynmanScope::Industry { slug: b })) => a == b,
        _ => false,
    }
}

fn feynman_coach_panel_for_canvas(store: AppStore) -> View {
    feynman_coach_panel_with_visibility(store, move || is_feynman_panel_visible_for_canvas(store))
}

fn is_feynman_panel_visible(store: AppStore, is_industry: bool) -> bool {
    if !store.feynman_mode_active.get() {
        return false;
    }
    match (is_industry, store.feynman_scope.get()) {
        (false, Some(FeynmanScope::Company { ticker })) => {
            store.selected_ticker.get().eq_ignore_ascii_case(&ticker)
        }
        (true, Some(FeynmanScope::Industry { slug })) => {
            store.selected_industry_slug.get().as_deref() == Some(slug.as_str())
        }
        _ => false,
    }
}

/// Interactive 3-Round Socratic Feynman Coach & Human Sign-Off panel shared between
/// the unified Alan Canvas and Industry Studio.
pub(crate) fn feynman_coach_panel(store: AppStore, is_industry: bool) -> View {
    feynman_coach_panel_with_visibility(store, move || is_feynman_panel_visible(store, is_industry))
}

fn feynman_coach_panel_with_visibility(
    store: AppStore,
    is_visible: impl Fn() -> bool + 'static,
) -> View {
    let draft_metric_input = signal(String::new());
    let show_archive = signal(false);

    view().child(when(
        is_visible,
        move || {
            obsidian_card()
                .p(px(18.0))
                .border_color(rgba(VIOLET_BORDER))
                .flex()
                .flex_col()
                .gap(px(14.0))
                // 1. Header, Scope, Progress Stepper, Prior Turns & Active Question
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let Some(scope) = store.feynman_scope.get() else {
                        return view();
                    };
                    let active_round_num = store.feynman_round.get().clamp(1, 3);
                    let turns = store.feynman_turns.get();
                    let busy = store.feynman_busy.get();

                    let (scope_badge, subject_label) = match &scope {
                        FeynmanScope::Company { ticker } => {
                            let name = store.reports.with(|reps| {
                                reps.iter()
                                    .find(|r| r.ticker.eq_ignore_ascii_case(ticker))
                                    .map(|r| format!("{} ({})", r.ticker, r.name))
                            });
                            (
                                format!("COMPANY · {ticker}"),
                                name.unwrap_or_else(|| ticker.clone()),
                            )
                        }
                        FeynmanScope::Industry { slug } => {
                            let title = store.industries.with(|list| {
                                list.iter()
                                    .find(|p| p.slug == *slug)
                                    .map(|p| p.title.clone())
                            });
                            (
                                format!("INDUSTRY · {slug}"),
                                title.unwrap_or_else(|| slug.clone()),
                            )
                        }
                    };

                    let mut col = view()
                        .flex()
                        .flex_col()
                        .gap(px(12.0))
                        .child(
                            view()
                                .pb(px(10.0))
                                .border_b_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .flex_wrap()
                                .gap(px(8.0))
                                .child(
                                    view()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .flex_wrap()
                                        .gap(px(8.0))
                                        .child(
                                            view()
                                                .w(px(7.0))
                                                .h(px(7.0))
                                                .rounded_full()
                                                .bg(rgb(VIOLET_FG)),
                                        )
                                        .child(
                                            text(l.t(
                                                "3 轮费曼答辩教练 · Alan",
                                                "3-Round Feynman Coach · Alan",
                                            ))
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_MD))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO)),
                                        )
                                        .child(badge_pill(
                                            scope_badge,
                                            VIOLET_FG,
                                            VIOLET_BG,
                                            VIOLET_BORDER,
                                        ))
                                        .child(if busy {
                                            badge_pill(
                                                l.t("Alan 正在推敲点评…", "Alan critiquing…")
                                                    .to_owned(),
                                                SKY_FG,
                                                SKY_BG,
                                                SKY_BORDER,
                                            )
                                        } else {
                                            badge_pill(
                                                match l {
                                                    UiLang::Zh => {
                                                        format!("进度 {}/3 轮", turns.len().min(3))
                                                    }
                                                    UiLang::En => {
                                                        format!("Round {}/3 done", turns.len().min(3))
                                                    }
                                                },
                                                if turns.len() >= 3 {
                                                    EMERALD_FG
                                                } else {
                                                    GOLD_FG
                                                },
                                                if turns.len() >= 3 {
                                                    EMERALD_BG
                                                } else {
                                                    GOLD_BG
                                                },
                                                if turns.len() >= 3 {
                                                    EMERALD_BORDER
                                                } else {
                                                    GOLD_BORDER
                                                },
                                            )
                                        }),
                                )
                                .child(action_button(
                                    l.t("退出答辩", "Exit Coach"),
                                    false,
                                    move || store.exit_feynman_mode(),
                                )),
                        )
                        // 3-Round Stepper
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap(px(8.0))
                                .children(FeynmanRound::ALL.into_iter().map(|r| {
                                    let r_num = r.as_u8();
                                    let is_done = turns.iter().any(|t| t.round == r_num);
                                    let is_curr = r_num == active_round_num && turns.len() < 3;
                                    let (fg, bg, br) = if is_done {
                                        (EMERALD_FG, EMERALD_BG, EMERALD_BORDER)
                                    } else if is_curr {
                                        (VIOLET_FG, VIOLET_BG, VIOLET_BORDER)
                                    } else {
                                        (INK_MUTED, BG_WELL, BORDER_SUBTLE)
                                    };
                                    let prefix = if is_done { "✓ " } else { "" };
                                    let r_target = r_num;
                                    view()
                                        .flex_1()
                                        .min_w(px(180.0))
                                        .px(px(10.0))
                                        .py(px(8.0))
                                        .rounded_sm()
                                        .bg(rgba(bg))
                                        .border_1()
                                        .border_color(rgba(br))
                                        .cursor_pointer()
                                        .on_press(move || store.feynman_round.set(r_target))
                                        .child(
                                            text(format!("{prefix}{}", r.title_for(l)))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .font_weight(if is_curr || is_done {
                                                    FontWeight::SEMIBOLD
                                                } else {
                                                    FontWeight::NORMAL
                                                })
                                                .text_color(rgb(fg)),
                                        )
                                })),
                        );

                    // Prior turns with Alan critiques, blind spots & suggested metrics
                    if !turns.is_empty() {
                        let mut turns_col = view().flex().flex_col().gap(px(10.0));
                        for turn in &turns {
                            let round_lbl = feynman::FeynmanRound::from_u8(turn.round)
                                .map(|r| r.title_for(l).to_owned())
                                .unwrap_or_else(|| format!("Round {}", turn.round));
                            let mut turn_card = view()
                                .p(px(12.0))
                                .rounded_md()
                                .bg(rgb(BG_VIEWPORT))
                                .border_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .flex()
                                .flex_col()
                                .gap(px(8.0))
                                .child(
                                    text(round_lbl)
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(EMERALD_FG)),
                                )
                                .child(
                                    text(match l {
                                        UiLang::Zh => format!("你的回答：{}", turn.human_answer),
                                        UiLang::En => format!("Your answer: {}", turn.human_answer),
                                    })
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_SM))
                                    .line_height(px(19.0))
                                    .text_color(rgb(INK_PRIMARY)),
                                )
                                .child(
                                    view()
                                        .p(px(10.0))
                                        .rounded_sm()
                                        .bg(rgb(BG_WELL))
                                        .border_1()
                                        .border_color(rgba(VIOLET_BORDER))
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.0))
                                        .child(
                                            text("ALAN CRITIQUE")
                                                .font_family(FONT_MONO_TERM)
                                                .text_size(px(TEXT_2XS))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(rgb(VIOLET_FG)),
                                        )
                                        .child(
                                            text(turn.alan_critique.clone())
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_SM))
                                                .line_height(px(19.0))
                                                .text_color(rgb(INK_PRIMARY)),
                                        ),
                                );

                            if !turn.blind_spots.is_empty() {
                                turn_card = turn_card.child(
                                    view()
                                        .flex()
                                        .flex_col()
                                        .gap(px(3.0))
                                        .child(
                                            text(l.t("潜在盲区：", "Blind Spots:"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_2XS))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(GOLD_FG)),
                                        )
                                        .children(turn.blind_spots.iter().map(|b| {
                                            text(format!("• {b}"))
                                                .font_family(FONT_UI)
                                                .text_size(px(TEXT_XS))
                                                .text_color(rgb(INK_SECONDARY))
                                        })),
                                );
                            }

                            if !turn.suggested_metrics.is_empty() {
                                turn_card = turn_card.child(
                                    view()
                                        .flex()
                                        .flex_col()
                                        .gap(px(6.0))
                                        .child(
                                            text(l.t(
                                                "Alan 建议的事前证伪指标：",
                                                "Suggested Falsification Metrics:",
                                            ))
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_2XS))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(SKY_FG)),
                                        )
                                        .children(turn.suggested_metrics.iter().map(|m| {
                                            let m_adopt = m.clone();
                                            view()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .justify_between()
                                                .gap(px(8.0))
                                                .child(
                                                    view().flex_1().min_w(px(0.0)).child(
                                                        text(format!("+ {m}"))
                                                            .font_family(FONT_UI)
                                                            .text_size(px(TEXT_XS))
                                                            .text_color(rgb(INK_PRIMARY)),
                                                    ),
                                                )
                                                .child(action_button(
                                                    l.t("采纳", "Adopt"),
                                                    false,
                                                    move || {
                                                        store.adopt_suggested_falsification_metric(
                                                            &m_adopt,
                                                        );
                                                    },
                                                ))
                                        })),
                                );
                            }

                            turns_col = turns_col.child(turn_card);
                        }
                        col = col.child(turns_col);
                    }

                    // Active Socratic Question Prompt
                    if turns.len() < 3
                        && let Some(round) = FeynmanRound::from_u8(active_round_num)
                    {
                        let prompt =
                            feynman::round_question_prompt(&scope, round, &subject_label, l);
                        col = col.child(
                            view()
                                .p(px(14.0))
                                .rounded_md()
                                .bg(rgba(VIOLET_BG))
                                .border_1()
                                .border_color(rgba(VIOLET_BORDER))
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .child(
                                    text(round.title_for(l))
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(VIOLET_FG)),
                                )
                                .child(
                                    text(prompt)
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_BASE))
                                        .line_height(px(21.0))
                                        .text_color(rgb(INK_HERO)),
                                ),
                        );
                    }

                    col
                }))
                // 2. Multiline Answer Input (compatible with macOS Dictation) mounted outside dynamic()
                .child(when(
                    move || store.feynman_turns.with(|t| t.len() < 3),
                    move || {
                        view()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(
                                styled_input(
                                    move || store.feynman_input.get(),
                                    move || {
                                        store
                                            .lang
                                            .get()
                                            .t(
                                                "用大白话写下你的推理与边界（支持 macOS 原生语音听写 Dictation）…",
                                                "Write your plain-language reasoning (supports macOS native Dictation)…",
                                            )
                                            .to_owned()
                                    },
                                    false,
                                )
                                .multiline(true)
                                .min_h(px(96.0))
                                .w_full()
                                .on_change(move |val: &str| store.feynman_input.set(val.to_owned())),
                            )
                            .child(
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(10.0))
                                    .child(
                                        text(move || {
                                            store
                                                .lang
                                                .get()
                                                .t(
                                                    "提示：可按 macOS 听写快捷键口述，再由 Alan 逐轮追问盲区。",
                                                    "Tip: Use macOS Dictation to speak your thesis; Alan will challenge blind spots each round.",
                                                )
                                                .to_owned()
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                    )
                                    .child(action_button(
                                        move || {
                                            let l = store.lang.get();
                                            if store.feynman_busy.get() {
                                                l.t("Alan 评估中…", "Evaluating…").to_owned()
                                            } else {
                                                l.t("提交本轮回答", "Submit Round").to_owned()
                                            }
                                        },
                                        true,
                                        move || store.submit_feynman_round(),
                                    )),
                            )
                    },
                ))
                // 3. Final Sign-Off Card when 3 rounds are complete
                .child(when(
                    move || store.feynman_turns.with(|t| t.len() >= 3),
                    move || {
                        view()
                            .p(px(14.0))
                            .rounded_md()
                            .bg(rgba(EMERALD_BG))
                            .border_1()
                            .border_color(rgba(EMERALD_BORDER))
                            .flex()
                            .flex_col()
                            .gap(px(12.0))
                            .child(
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        text(move || {
                                            store
                                                .lang
                                                .get()
                                                .t(
                                                    "3 轮答辩已完成 · 人类最终签字归档",
                                                    "3 Rounds Complete · Human Final Sign-Off",
                                                )
                                                .to_owned()
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_SM))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(EMERALD_FG)),
                                    )
                                    .child(badge_pill(
                                        move || {
                                            store
                                                .lang
                                                .get()
                                                .t("仅本人签字生效", "HUMAN SIGN-OFF")
                                                .to_owned()
                                        },
                                        EMERALD_FG,
                                        EMERALD_BG,
                                        EMERALD_BORDER,
                                    )),
                            )
                            .child(
                                view()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .child(
                                        text(move || {
                                            store
                                                .lang
                                                .get()
                                                .t("我的核心判词 (my_stance)", "My Stance (my_stance)")
                                                .to_owned()
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_PRIMARY)),
                                    )
                                    .child(
                                        styled_input(
                                            move || store.feynman_draft_stance.get(),
                                            move || {
                                                store
                                                    .lang
                                                    .get()
                                                    .t(
                                                        "总结你的最终能力圈判断与估值纪律…",
                                                        "Summarize your final circle conviction and valuation discipline…",
                                                    )
                                                    .to_owned()
                                            },
                                            false,
                                        )
                                        .multiline(true)
                                        .min_h(px(72.0))
                                        .w_full()
                                        .on_change(move |val: &str| {
                                            store.feynman_draft_stance.set(val.to_owned());
                                        }),
                                    ),
                            )
                            .child(dynamic(move || {
                                let l = store.lang.get();
                                let metrics = store.feynman_draft_metrics.get();
                                let mut suggested_pool: Vec<String> = Vec::new();
                                store.feynman_turns.with(|turns| {
                                    for t in turns {
                                        for m in &t.suggested_metrics {
                                            let trimmed = m.trim();
                                            if !trimmed.is_empty()
                                                && !metrics.iter().any(|e| e.trim() == trimmed)
                                                && !suggested_pool.iter().any(|s| s == trimmed)
                                            {
                                                suggested_pool.push(trimmed.to_owned());
                                            }
                                        }
                                    }
                                });

                                let mut col = view()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.0))
                                    .child(
                                        text(l.t(
                                            "签字写入的事前证伪指标 (falsification_metrics)",
                                            "Falsification Metrics to Commit",
                                        ))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_PRIMARY)),
                                    );

                                if metrics.is_empty() {
                                    col = col.child(
                                        text(l.t(
                                            "（尚未添加；可在下方输入或从 Alan 建议中采纳）",
                                            "(None yet; add below or adopt from Alan's suggestions)",
                                        ))
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                    );
                                } else {
                                    for (idx, m) in metrics.into_iter().enumerate() {
                                        col = col.child(
                                            view()
                                                .px(px(10.0))
                                                .py(px(5.0))
                                                .rounded_sm()
                                                .bg(rgb(BG_VIEWPORT))
                                                .border_1()
                                                .border_color(rgb(BORDER_HAIRLINE))
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .justify_between()
                                                .gap(px(8.0))
                                                .child(
                                                    view().flex_1().min_w(px(0.0)).child(
                                                        text(format!("• {m}"))
                                                            .font_family(FONT_UI)
                                                            .text_size(px(TEXT_XS))
                                                            .text_color(rgb(INK_PRIMARY)),
                                                    ),
                                                )
                                                .child(action_button(
                                                    l.t("移除", "Remove"),
                                                    false,
                                                    move || store.remove_feynman_draft_metric(idx),
                                                )),
                                        );
                                    }
                                }

                                if !suggested_pool.is_empty() {
                                    col = col.child(
                                        view()
                                            .pt(px(4.0))
                                            .flex()
                                            .flex_row()
                                            .flex_wrap()
                                            .gap(px(6.0))
                                            .children(suggested_pool.into_iter().map(|m| {
                                                let m_adopt = m.clone();
                                                action_button(
                                                    format!("+ {}", m),
                                                    false,
                                                    move || store.add_feynman_draft_metric(&m_adopt),
                                                )
                                            })),
                                    );
                                }
                                col
                            }))
                            .child(
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        styled_input(
                                            move || draft_metric_input.get(),
                                            move || {
                                                store
                                                    .lang
                                                    .get()
                                                    .t(
                                                        "补充签字证伪指标…",
                                                        "Add falsification metric for sign-off…",
                                                    )
                                                    .to_owned()
                                            },
                                            false,
                                        )
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .on_change(move |val: &str| {
                                            draft_metric_input.set(val.to_owned());
                                        })
                                        .on_submit(move |val: &str| {
                                            let trimmed = val.trim().to_owned();
                                            if !trimmed.is_empty() {
                                                store.add_feynman_draft_metric(&trimmed);
                                                draft_metric_input.set(String::new());
                                            }
                                        }),
                                    )
                                    .child(action_button(
                                        move || store.lang.get().t("添加", "Add").to_owned(),
                                        false,
                                        move || {
                                            let trimmed =
                                                draft_metric_input.get_untracked().trim().to_owned();
                                            if !trimmed.is_empty() {
                                                store.add_feynman_draft_metric(&trimmed);
                                                draft_metric_input.set(String::new());
                                            }
                                        },
                                    )),
                            )
                            .child(
                                view()
                                    .pt(px(4.0))
                                    .flex()
                                    .flex_row()
                                    .justify_end()
                                    .child(action_button(
                                        move || {
                                            store
                                                .lang
                                                .get()
                                                .t(
                                                    "确认签字 · 写入判断与证伪指标",
                                                    "Sign Off · Save Judgment & Metrics",
                                                )
                                                .to_owned()
                                        },
                                        true,
                                        move || store.sign_off_feynman_defense(),
                                    )),
                            )
                    },
                ))
                // 4. Archive History Preview
                .child(dynamic(move || {
                    let l = store.lang.get();
                    let archive = store.feynman_archive_markdown.get();
                    if archive.trim().is_empty() {
                        return view();
                    }
                    let expanded = show_archive.get();
                    let mut arch = view()
                        .pt(px(8.0))
                        .border_t_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .cursor_pointer()
                                .on_press(move || show_archive.update(|v| *v = !*v))
                                .child(
                                    text(l.t(
                                        "历史费曼答辩归档 (Markdown)",
                                        "Feynman Defense Archive (Markdown)",
                                    ))
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_SECONDARY)),
                                )
                                .child(
                                    text(if expanded {
                                        l.t("收起", "Collapse")
                                    } else {
                                        l.t("展开查看", "Expand")
                                    })
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED)),
                                ),
                        );
                    if expanded {
                        arch = arch.child(
                            view()
                                .p(px(12.0))
                                .rounded_sm()
                                .bg(rgb(BG_VIEWPORT))
                                .border_1()
                                .border_color(rgb(BORDER_HAIRLINE))
                                .child(
                                    text(archive)
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .line_height(px(17.0))
                                        .text_color(rgb(INK_SECONDARY))
                                        .selectable(true),
                                ),
                        );
                    }
                    arch
                }))
        },
    ))
}
