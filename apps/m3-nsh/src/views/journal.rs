//! Theme Archetype Clusters & Append-Only Decision Journal view (`⌘5`).

use crate::components::{
    action_button, badge_pill, cmd_kbd_corner, concept_hint_static, mono_left, mono_right,
    obsidian_card, seg_group, seg_tab, styled_input, v_divider, view_header_strip,
};
use crate::format::{
    fmt_pct_clean, fmt_price, fmt_time_cn_for, format_theme_member_distance,
    format_theme_member_roic,
};
use crate::stores::{AppStore, JournalViewTab};
use crate::theme::*;
use m3_core::budget::ThemeCluster;
use m3_core::i18n::UiLang;
use m3_core::model::{DecisionJournal, JournalAction, MungerReport, PriceSnapshot};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

/// Renders Decision Ledger (primary) and Archetype Themes sub-tabs (`⌘5`).
pub fn journal_view(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(render_journal_header_strip(store))
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .child(dynamic(move || match store.journal_view_tab.get() {
                    JournalViewTab::Ledger => render_journal_section(store).into_element(),
                    JournalViewTab::Themes => render_themes_section(store).into_element(),
                })),
        )
}

fn render_journal_header_strip(store: AppStore) -> View {
    view_header_strip()
        .child(dynamic(move || {
            let l = store.lang.get();
            let active = store.journal_view_tab.get();
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(seg_group().children(JournalViewTab::all().into_iter().map(|tab| {
                    seg_tab(
                        tab.label_for(l),
                        active == tab,
                        Some((GOLD_FG, GOLD_BG)),
                        false,
                        move || store.journal_view_tab.set(tab),
                    )
                })))
                .child(concept_hint_static(match active {
                    JournalViewTab::Ledger => l.t(
                        "事前冻结、只增不改的决策账本，自动以 100% 现金与 S&P 500 检验。",
                        "Append-only pre-committed decision ledger benchmarked against 100% cash and S&P 500.",
                    ),
                    JournalViewTab::Themes => l.t(
                        "基于 SIC 与财报聚类生成的商业原型，可直接升级为行业研究。",
                        "Business archetypes clustered from SIC and 10-K filings; promote any to an Industry Study.",
                    ),
                }))
        }))
        .child(dynamic(move || {
            let l = store.lang.get();
            match store.journal_view_tab.get() {
                JournalViewTab::Ledger => {
                    let journal = store.journal.get();
                    let reports = store.reports.get();
                    let benchmark = store.benchmark.get();
                    let (nav_val, _) = compute_portfolio_totals(&journal, &reports);
                    let port_ret_pct = if journal.starting_cash > Decimal::ZERO {
                        Some(
                            ((nav_val - journal.starting_cash) / journal.starting_cash)
                                * Decimal::from(100),
                        )
                    } else {
                        None
                    };
                    let (port_ret_txt, port_pol) = fmt_pct_clean(port_ret_pct, true);
                    let bench_ret_pct = compute_benchmark_return_pct(&journal, benchmark.as_ref());
                    let (bench_ret_txt, bench_pol) = fmt_pct_clean(bench_ret_pct, true);
                    let port_col = if port_pol > 0 {
                        EMERALD_FG
                    } else if port_pol < 0 {
                        CORAL_FG
                    } else {
                        INK_PRIMARY
                    };
                    let bench_col = if bench_pol > 0 {
                        EMERALD_FG
                    } else if bench_pol < 0 {
                        CORAL_FG
                    } else {
                        INK_SECONDARY
                    };
                    let form_open = store.journal_form_open.get();

                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(stat_inline(
                            l.t("模拟现金", "Cash"),
                            format!("${:.0}", journal.cash_balance),
                            EMERALD_FG,
                        ))
                        .child(v_divider())
                        .child(stat_inline(
                            l.t("组合净值", "NAV"),
                            format!("${:.0} ({port_ret_txt})", nav_val),
                            port_col,
                        ))
                        .child(v_divider())
                        .child(stat_inline("S&P 500", bench_ret_txt, bench_col))
                        .child(v_divider())
                        .child(stat_inline(
                            l.t("记录", "Entries"),
                            format!("{}", journal.entries.len()),
                            INK_HERO,
                        ))
                        .child(action_button(
                            if form_open {
                                l.t("收起表单", "Close Form").to_owned()
                            } else {
                                l.t("+ 记录一笔决策", "+ Record Decision").to_owned()
                            },
                            !form_open,
                            move || {
                                if form_open {
                                    store.close_journal_form();
                                } else {
                                    store.open_journal_form(None);
                                }
                            },
                        ))
                }
                JournalViewTab::Themes => {
                    let tf = store.themes_file.get();
                    let syncing = store.themes_syncing.get();
                    let updated_label = if tf.updated_at_ms > 0 {
                        fmt_time_cn_for(Some(tf.updated_at_ms), l)
                    } else {
                        l.t("未同步", "Unsynced").to_owned()
                    };

                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(stat_inline(
                            l.t("原型簇", "Clusters"),
                            format!("{}", tf.clusters.len()),
                            INK_HERO,
                        ))
                        .child(v_divider())
                        .child(stat_inline(
                            l.t("索引公司", "Indexed"),
                            format!("{}", tf.total_indexed_companies),
                            SKY_FG,
                        ))
                        .child(v_divider())
                        .child(stat_inline(
                            l.t("更新时间", "Updated"),
                            updated_label,
                            INK_SECONDARY,
                        ))
                        .child(
                            view()
                                .relative()
                                .child(action_button(
                                    if syncing {
                                        l.t("同步中…", "Syncing…").to_owned()
                                    } else {
                                        l.t("同步主题图谱", "Sync Themes").to_owned()
                                    },
                                    !syncing,
                                    move || store.start_themes_sync(),
                                ))
                                .child(cmd_kbd_corner(store, "s")),
                        )
                }
            }
        }))
}

fn render_themes_section(store: AppStore) -> View {
    view()
        .w_full()
        .flex()
        .flex_col()
        .child(dynamic(move || {
            let l = store.lang.get();
            let tf = store.themes_file.get();
            let reports = store.reports.get();
            let err_opt = store.themes_error.get();

            let mut body = view()
                .px(px(18.0))
                .pt(px(16.0))
                .pb(px(8.0))
                .flex()
                .flex_col()
                .gap(px(12.0));

            if let Some(err) = err_opt {
                body = body.child(
                    view()
                        .px(px(16.0))
                        .py(px(10.0))
                        .rounded_md()
                        .bg(rgba(CORAL_BG))
                        .border_1()
                        .border_color(rgba(CORAL_BORDER))
                        .child(
                            text(err)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(CORAL_FG)),
                        ),
                );
            }

            if tf.clusters.is_empty() {
                let syncing = store.themes_syncing.get();
                body = body.child(
                    obsidian_card()
                        .p(px(20.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(16.0))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .flex_col()
                                .gap(px(4.0))
                                .child(
                                    text(
                                        l.t(
                                            "尚未生成商业原型主题簇",
                                            "No cross-company archetype clusters yet",
                                        )
                                        .to_owned(),
                                    )
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_BASE))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO)),
                                )
                                .child(
                                    text(
                                        l.t(
                                            "点击右侧「立即同步主题图谱」，自动将全美股同类护城河与商业模式公司聚成主题，方便一键升级为行业研究。",
                                            "Click Sync Themes to cluster US companies with similar business models from the local lakehouse.",
                                        )
                                        .to_owned(),
                                    )
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                                ),
                        )
                        .child(action_button(
                            if syncing {
                                l.t("同步中…", "Syncing…").to_owned()
                            } else {
                                l.t("立即同步主题图谱", "Sync Themes Now").to_owned()
                            },
                            !syncing,
                            move || store.start_themes_sync(),
                        )),
                );
            } else {
                body = body.children(
                    tf.clusters
                        .iter()
                        .map(|cluster| render_theme_cluster_card(store, cluster, &reports, l)),
                );
            }

            body
        }))
}

fn render_theme_cluster_card(
    store: AppStore,
    cluster: &ThemeCluster,
    reports: &[MungerReport],
    l: UiLang,
) -> View {
    let median_roic = cluster
        .median_roic_5y_pct
        .map(|r| format!("{} {r:.1}%", l.t("中位 ROIC", "Med ROIC")))
        .unwrap_or_else(|| format!("{} —", l.t("中位 ROIC", "Med ROIC")));
    let sic_txt = if cluster.sic_codes.is_empty() {
        "SIC —".to_owned()
    } else {
        let codes: Vec<String> = cluster
            .sic_codes
            .iter()
            .take(4)
            .map(|c| c.to_string())
            .collect();
        format!("SIC {}", codes.join(","))
    };

    let mut title_row = view()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(px(8.0))
        .child(
            text(cluster.title.clone())
                .font_family(FONT_UI)
                .text_size(px(TEXT_BASE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        )
        .child(badge_pill(
            cluster.archetype.clone(),
            VIOLET_FG,
            VIOLET_BG,
            VIOLET_BORDER,
        ));
    if cluster.strike_zone_count > 0 {
        title_row = title_row.child(badge_pill(
            format!(
                "{} {}",
                cluster.strike_zone_count,
                l.t("家击球区", "in strike")
            ),
            EMERALD_FG,
            EMERALD_BG,
            EMERALD_BORDER,
        ));
    }

    let cluster_for_promote = cluster.clone();
    let header_right = view()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(
            text(format!(
                "{} {} · {median_roic} · sim {:.2} · {sic_txt}",
                cluster.members.len(),
                l.t("家", "cos"),
                cluster.avg_similarity
            ))
            .font_family(FONT_MONO_NUM)
            .text_size(px(TEXT_XS))
            .text_color(rgb(INK_SECONDARY)),
        )
        .child(
            view()
                .px(px(8.0))
                .py(px(3.0))
                .rounded_xs()
                .bg(rgba(SKY_BG))
                .border_1()
                .border_color(rgba(SKY_BORDER))
                .hover(|s| s.bg(rgb(BG_HOVER)))
                .cursor_pointer()
                .on_press(move || {
                    store.open_or_create_industry_from_theme(&cluster_for_promote);
                })
                .child(
                    text(l.t("转行业研究", "Study Industry"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_2XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(SKY_FG)),
                ),
        );

    let mut card = obsidian_card()
        .p(px(16.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(title_row)
                .child(header_right),
        );

    if !cluster.summary.trim().is_empty() {
        card = card.child(
            text(cluster.summary.clone())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .line_height(px(19.0))
                .text_color(rgb(INK_SECONDARY)),
        );
    }

    if !cluster.keywords.is_empty() {
        card = card.child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(4.0))
                .children(cluster.keywords.iter().take(6).map(|kw| {
                    view()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded_xs()
                        .bg(rgb(BG_WELL))
                        .child(
                            text(format!("#{kw}"))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        )
                })),
        );
    }

    if !cluster.user_notes.trim().is_empty() {
        card = card.child(
            view()
                .px(px(12.0))
                .py(px(8.0))
                .rounded_sm()
                .bg(rgba(GOLD_BG))
                .border_1()
                .border_color(rgba(GOLD_BORDER))
                .child(
                    text(format!(
                        "{}: {}",
                        l.t("人类批注 (user_notes)", "Human Notes (user_notes)"),
                        cluster.user_notes
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(18.0))
                    .text_color(rgb(GOLD_FG)),
                ),
        );
    }

    card.child(
        view()
            .pt(px(4.0))
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .gap(px(6.0))
            .children(cluster.members.iter().map(|member| {
                let ticker = member.ticker.clone();
                let in_watch = member.in_watchlist
                    || reports
                        .iter()
                        .any(|r| r.ticker.eq_ignore_ascii_case(&member.ticker));
                let in_strike = member
                    .distance_to_strike_pct
                    .is_some_and(|d| d <= Decimal::ZERO);
                let dot_col = if in_strike {
                    EMERALD_FG
                } else if in_watch {
                    SKY_FG
                } else {
                    INK_MUTED
                };
                let mut pill = view()
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded_sm()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(if in_watch {
                        BORDER_STRONG
                    } else {
                        BORDER_SUBTLE
                    }))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .active(|s| s.bg(rgb(BG_ACTIVE)))
                    .cursor_pointer()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .on_press(move || store.open_theme_member(&ticker))
                    .child(view().w(px(5.0)).h(px(5.0)).rounded_full().bg(rgb(dot_col)))
                    .child(
                        text(member.ticker.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_HERO)),
                    )
                    .child(
                        text(format_theme_member_roic(member))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_SECONDARY)),
                    );
                if member.distance_to_strike_pct.is_some() {
                    pill = pill.child(
                        text(format_theme_member_distance(member))
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(if in_strike { EMERALD_FG } else { INK_MUTED })),
                    );
                }
                pill
            })),
    )
}

fn journal_action_display(action: JournalAction, lang: UiLang) -> &'static str {
    match (action, lang) {
        (JournalAction::Wait, UiLang::Zh) => "继续等待",
        (JournalAction::Wait, UiLang::En) => "WAIT",
        (JournalAction::PaperBuy, UiLang::Zh) => "模拟买入",
        (JournalAction::PaperBuy, UiLang::En) => "PAPER BUY",
        (JournalAction::PaperSell, UiLang::Zh) => "模拟卖出",
        (JournalAction::PaperSell, UiLang::En) => "PAPER SELL",
        (JournalAction::Out, UiLang::Zh) => "看懂否决",
        (JournalAction::Out, UiLang::En) => "OUT",
        (JournalAction::TooHard, UiLang::Zh) => "能力圈外",
        (JournalAction::TooHard, UiLang::En) => "TOO HARD",
    }
}

fn render_journal_section(store: AppStore) -> View {
    view().w_full().flex().flex_col().child(dynamic(move || {
        let l = store.lang.get();
        let journal = store.journal.get();
        let benchmark = store.benchmark.get();
        let reports = store.reports.get();
        let form_open = store.journal_form_open.get();

        let (nav_val, positions_val) = compute_portfolio_totals(&journal, &reports);
        let cash_pct = if nav_val > Decimal::ZERO {
            ((journal.cash_balance / nav_val) * Decimal::from(100)).round_dp(1)
        } else {
            Decimal::from(100)
        };
        let port_ret_pct = if journal.starting_cash > Decimal::ZERO {
            Some(((nav_val - journal.starting_cash) / journal.starting_cash) * Decimal::from(100))
        } else {
            None
        };
        let bench_ret_pct = compute_benchmark_return_pct(&journal, benchmark.as_ref());

        let bench_now = benchmark
            .as_ref()
            .map(|b| format!("{:.2}", b.price))
            .unwrap_or_else(|| "—".into());
        let bench_start = journal
            .benchmark_start_price
            .map(|p| format!("{p:.2}"))
            .unwrap_or_else(|| l.t("首笔冻结时锁定", "Locks on 1st entry").to_owned());

        let (port_ret_txt, port_pol) = fmt_pct_clean(port_ret_pct, true);
        let (bench_ret_txt, bench_pol) = fmt_pct_clean(bench_ret_pct, true);
        let port_col = if port_pol > 0 {
            EMERALD_FG
        } else if port_pol < 0 {
            CORAL_FG
        } else {
            INK_PRIMARY
        };
        let bench_col = if bench_pol > 0 {
            EMERALD_FG
        } else if bench_pol < 0 {
            CORAL_FG
        } else {
            INK_SECONDARY
        };

        let header = view()
            .px(px(18.0))
            .py(px(10.0))
            .bg(rgb(BG_SURFACE))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap(px(16.0))
            .child(
                view()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(badge_pill(
                        journal.account_note.clone(),
                        EMERALD_FG,
                        EMERALD_BG,
                        EMERALD_BORDER,
                    ))
                    .child(concept_hint_static(l.t(
                        "每笔重大决策前先立字据：冻结当时价格、FCF% 与证伪红线，拒绝事后合理化。",
                        "Freeze price, FCF%, thesis, and kill criteria before acting to prevent hindsight bias.",
                    ))),
            )
            .child(
                view()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_wrap()
                    .gap(px(14.0))
                    .child(stat_inline(
                        l.t("模拟现金", "Paper Cash"),
                        format!("${:.2} ({cash_pct}%)", journal.cash_balance),
                        EMERALD_FG,
                    ))
                    .child(v_divider())
                    .child(stat_inline(
                        l.t("总净值 / 起点", "NAV / Start"),
                        format!(
                            "${:.2} / ${:.2} ({port_ret_txt})",
                            nav_val, journal.starting_cash
                        ),
                        port_col,
                    ))
                    .child(v_divider())
                    .child(stat_inline(
                        "S&P 500",
                        format!(
                            "{bench_now} ({} {bench_start} · {bench_ret_txt})",
                            l.t("起点", "Base")
                        ),
                        bench_col,
                    ))
                    .child(v_divider())
                    .child(stat_inline(
                        l.t("记录数", "Entries"),
                        format!("{}", journal.entries.len()),
                        SLATE_FG,
                    )),
            );

        let positions_card = render_positions_card(
            store,
            &journal,
            &reports,
            positions_val,
            port_ret_txt,
            bench_ret_txt,
            l,
        );
        let entries_card = render_entries_card(store, &journal, l);

        let mut content = view()
            .px(px(18.0))
            .py(px(18.0))
            .flex()
            .flex_col()
            .gap(px(16.0));
        if form_open {
            content = content.child(render_journal_composer_card(store, &reports, l));
        } else if journal.entries.is_empty() {
            content = content.child(render_journal_guide_card(store, l));
        }
        content = content.child(positions_card).child(entries_card);

        view().flex().flex_col().child(header).child(content)
    }))
}

fn render_journal_guide_card(store: AppStore, l: UiLang) -> View {
    let step_box = |tag: &'static str, title: &'static str, desc: &'static str, col: u32| {
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
            .gap(px(4.0))
            .child(
                text(tag)
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_2XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(col)),
            )
            .child(
                text(title)
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(INK_HERO)),
            )
            .child(
                text(desc)
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(17.0))
                    .text_color(rgb(INK_SECONDARY)),
            )
    };

    obsidian_card()
        .p(px(18.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
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
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            text(l.t("决策账本怎么用？", "How to Use the Decision Ledger"))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_MD))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(l.t(
                                "这不是实盘券商账户，而是防止自己“涨了说早看好、跌了找借口”的事前决策记录本。",
                                "Not a live brokerage—a pre-committed decision ledger that guards against hindsight bias.",
                            ))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(INK_SECONDARY)),
                        ),
                )
                .child(action_button(
                    l.t("+ 记录第一笔决策", "+ Record First Decision").to_owned(),
                    true,
                    move || store.open_journal_form(None),
                )),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .gap(px(10.0))
                .child(step_box(
                    "STEP 1",
                    l.t("事前冻结当下依据", "Freeze Thesis Upfront"),
                    l.t(
                        "无论是决定继续等待、模拟买入，还是否决淘汰，下单/拍板前先把判断理由和「证伪红线」写下来，系统自动锁死当时股价和 FCF%。",
                        "Before acting—Wait, Paper Buy/Sell, Out, or Too Hard—record your thesis and kill criteria. Price and FCF% snapshot automatically.",
                    ),
                    GOLD_FG,
                ))
                .child(step_box(
                    "STEP 2",
                    l.t("模拟盘检验耐心与眼光", "Test Against Cash & S&P 500"),
                    l.t(
                        "默认 100% 持有模拟现金。当 IN 核心标的进入击球区时记录「模拟买入」，即可与空仓等待及标普 500 实时比较收益率。",
                        "Starts at 100% Paper Cash. Log a Paper Buy when an IN company enters Strike Zone to benchmark your timing vs Cash & S&P 500.",
                    ),
                    EMERALD_FG,
                ))
                .child(step_box(
                    "STEP 3",
                    l.t("只增不改，客观复盘", "Append-Only Post-Mortem"),
                    l.t(
                        "所有记录提交后不可篡改或删除，哪怕当年看走眼也要留档。半年或一年后回头对照，快速看出哪类判断最稳。",
                        "Entries cannot be edited or deleted after saving—giving you an honest track record to review months or years later.",
                    ),
                    SKY_FG,
                )),
        )
}

fn render_journal_composer_card(store: AppStore, reports: &[MungerReport], l: UiLang) -> View {
    let cur_action = store.journal_draft_action.get();
    let needs_shares = matches!(
        cur_action,
        JournalAction::PaperBuy | JournalAction::PaperSell
    );
    let err_opt = store.journal_form_error.get();
    let actions = [
        JournalAction::Wait,
        JournalAction::PaperBuy,
        JournalAction::PaperSell,
        JournalAction::Out,
        JournalAction::TooHard,
    ];
    let quick_tickers: Vec<String> = reports.iter().take(8).map(|r| r.ticker.clone()).collect();

    let mut card = obsidian_card()
        .p(px(18.0))
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
                    text(l.t("记一笔事前冻结决策", "Record Pre-Committed Decision"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_MD))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(
                    text(l.t(
                        "提交后将锁定最新股价、严苛 FCF% 与 S&P 500 基准点位（只增不改）",
                        "Locks latest price, strict FCF%, and S&P 500 benchmark (append-only)",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
                ),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(6.0))
                .children(actions.into_iter().map(|act| {
                    let active = cur_action == act;
                    let (fg, bg, br) = journal_action_colors(act);
                    view()
                        .px(px(10.0))
                        .py(px(5.0))
                        .rounded_sm()
                        .cursor_pointer()
                        .bg(if active { rgba(bg) } else { rgb(BG_WELL) })
                        .border_1()
                        .border_color(if active { rgba(br) } else { rgb(BORDER_SUBTLE) })
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .on_press(move || {
                            store.journal_draft_action.set(act);
                            store.journal_form_error.set(None);
                        })
                        .child(
                            text(journal_action_display(act, l))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(if active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(rgb(if active { fg } else { INK_SECONDARY })),
                        )
                })),
        )
        .child({
            let mut row = view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(8.0))
                .child(
                    styled_input(
                        move || store.journal_draft_ticker.get(),
                        move || l.t("标的代码（如 COST / AAPL）", "Ticker (e.g. COST / AAPL)").to_owned(),
                        true,
                    )
                    .w(px(190.0))
                    .on_change(move |val: &str| {
                        store.journal_draft_ticker.set(val.to_owned());
                        store.journal_form_error.set(None);
                    }),
                );
            if needs_shares {
                row = row.child(
                    styled_input(
                        move || store.journal_draft_shares.get(),
                        move || l.t("股数（如 10）", "Shares (e.g. 10)").to_owned(),
                        true,
                    )
                    .w(px(140.0))
                    .on_change(move |val: &str| {
                        store.journal_draft_shares.set(val.to_owned());
                        store.journal_form_error.set(None);
                    }),
                );
            }
            if !quick_tickers.is_empty() {
                row = row.children(quick_tickers.into_iter().map(|tk| {
                    let tk_click = tk.clone();
                    view()
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded_xs()
                        .cursor_pointer()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .on_press(move || {
                            store.journal_draft_ticker.set(tk_click.clone());
                            store.journal_form_error.set(None);
                        })
                        .child(
                            text(tk)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                }));
            }
            row
        })
        .child(
            styled_input(
                move || store.journal_draft_note.get(),
                move || {
                    l.t(
                        "判断依据（必填）：护城河、估值安全边际，以及为什么现在做这个决定？",
                        "Rationale (required): moat, margin of safety, and why this decision now?",
                    )
                    .to_owned()
                },
                false,
            )
            .w_full()
            .on_change(move |val: &str| {
                store.journal_draft_note.set(val.to_owned());
                store.journal_form_error.set(None);
            }),
        )
        .child(
            styled_input(
                move || store.journal_draft_falsification.get(),
                move || {
                    l.t(
                        "事前证伪条件（选填）：未来出现什么事实说明这次判断错了？",
                        "Pre-falsification trigger (optional): what future fact proves this thesis wrong?",
                    )
                    .to_owned()
                },
                false,
            )
            .w_full()
            .on_change(move |val: &str| store.journal_draft_falsification.set(val.to_owned())),
        );

    if let Some(err) = err_opt {
        card = card.child(
            view()
                .px(px(12.0))
                .py(px(8.0))
                .rounded_sm()
                .bg(rgba(CORAL_BG))
                .border_1()
                .border_color(rgba(CORAL_BORDER))
                .child(
                    text(err)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(CORAL_FG)),
                ),
        );
    }

    card.child(
        view()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(action_button(
                l.t("冻结并写入账本", "Freeze & Save Entry").to_owned(),
                true,
                move || store.submit_journal_entry(),
            ))
            .child(action_button(
                l.t("取消", "Cancel").to_owned(),
                false,
                move || store.close_journal_form(),
            )),
    )
}

fn render_positions_card(
    store: AppStore,
    journal: &DecisionJournal,
    reports: &[MungerReport],
    positions_val: Decimal,
    port_ret_txt: String,
    bench_ret_txt: String,
    l: UiLang,
) -> View {
    let mut card = obsidian_card().child(
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
            .child(
                text(l.t(
                    "模拟持仓 & 100% 现金基准对比",
                    "Paper Positions & Cash vs S&P 500 Benchmark",
                ))
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
            )
            .child(
                text(format!(
                    "{}: ${:.2} · {} {port_ret_txt} vs S&P 500 {bench_ret_txt}",
                    l.t("持仓市值", "Holdings"),
                    positions_val,
                    l.t("组合", "Portfolio")
                ))
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_SECONDARY)),
            ),
    );

    if journal.positions.is_empty() {
        return card.child(
            view()
                .px(px(16.0))
                .py(px(14.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    text(
                        l.t(
                            "当前 100% 持有模拟现金。当 IN 核心标的落入击球区时，点击「+ 记录一笔决策」选择「模拟买入」，即可记账并与 S&P 500 实时对比。",
                            "Currently 100% Paper Cash — when an IN Core holding hits Strike Zone, record a Paper Buy to track performance vs S&P 500.",
                        )
                        .to_owned(),
                    )
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(INK_SECONDARY)),
                )
                .child(badge_pill(
                    format!("CASH ${:.2}", journal.cash_balance),
                    EMERALD_FG,
                    EMERALD_BG,
                    EMERALD_BORDER,
                )),
        );
    }

    let pos_cols: &[(&str, f32, bool)] = &[
        (l.t("标的", "Ticker"), 92.0, false),
        (l.t("股数", "Shares"), 92.0, true),
        (l.t("成本均价", "Avg Cost"), 100.0, true),
        (l.t("最新价", "Mark"), 100.0, true),
        (l.t("当前市值", "Market Val"), 120.0, true),
        (l.t("未实现盈亏", "Unrealized P&L"), 140.0, true),
        (l.t("建仓日期", "Opened"), 0.0, false),
    ];

    card = card.child(
        view()
            .flex()
            .flex_row()
            .px(px(12.0))
            .py(px(8.0))
            .bg(rgb(BG_ROW_ALT))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .children(pos_cols.iter().map(|(lbl, w, right)| {
                let mut cell = view().px(px(4.0)).flex().flex_row().items_center();
                if *right {
                    cell = cell.justify_end();
                }
                if *w > 0.0 {
                    cell = cell.w(px(*w)).flex_shrink_0();
                } else {
                    cell = cell.flex_1().min_w(px(0.0));
                }
                cell.child(
                    text(lbl.to_owned())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_MUTED)),
                )
            })),
    );

    card.children(journal.positions.iter().enumerate().map(|(idx, pos)| {
        let mark_opt = reports
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(&pos.ticker))
            .and_then(|r| r.price.as_ref().map(|p| p.price));
        let mark_val = mark_opt.unwrap_or(pos.avg_cost);
        let mkt_val = (pos.shares * mark_val).round_dp(2);
        let cost_val = (pos.shares * pos.avg_cost).round_dp(2);
        let pnl = mkt_val - cost_val;
        let pnl_pct = if cost_val > Decimal::ZERO {
            Some(((pnl / cost_val) * Decimal::from(100)).round_dp(2))
        } else {
            None
        };
        let pnl_col = if pnl > Decimal::ZERO {
            EMERALD_FG
        } else if pnl < Decimal::ZERO {
            CORAL_FG
        } else {
            INK_SECONDARY
        };
        let pnl_str = match pnl_pct {
            Some(pct) => format!("{pnl:+.2} ({pct:+.2}%)"),
            None => format!("{pnl:+.2}"),
        };
        let tk = pos.ticker.clone();

        view()
            .flex()
            .flex_row()
            .items_center()
            .px(px(12.0))
            .py(px(10.0))
            .bg(rgb(if idx.is_multiple_of(2) {
                BG_SURFACE
            } else {
                BG_ROW_ALT
            }))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(
                view()
                    .w(px(92.0))
                    .flex_shrink_0()
                    .px(px(4.0))
                    .cursor_pointer()
                    .on_press(move || store.open_inbox_signal_target(&tk))
                    .child(
                        text(pos.ticker.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(SKY_FG)),
                    ),
            )
            .child(mono_right(
                format!("{}", pos.shares.normalize()),
                92.0,
                INK_PRIMARY,
            ))
            .child(mono_right(
                fmt_price(Some(pos.avg_cost), &journal.currency),
                100.0,
                INK_SECONDARY,
            ))
            .child(mono_right(
                fmt_price(mark_opt, &journal.currency),
                100.0,
                if mark_opt.is_some() {
                    INK_HERO
                } else {
                    INK_MUTED
                },
            ))
            .child(mono_right(format!("${:.2}", mkt_val), 120.0, INK_PRIMARY))
            .child(mono_right(pnl_str, 140.0, pnl_col))
            .child(
                view().flex_1().min_w(px(0.0)).px(px(6.0)).child(
                    text(pos.opened_at.clone())
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                ),
            )
    }))
}

fn render_entries_card(store: AppStore, journal: &DecisionJournal, l: UiLang) -> View {
    let cols: &[(&str, f32, bool)] = &[
        (l.t("日期", "Date"), 98.0, false),
        (l.t("标的", "Ticker"), 84.0, false),
        (l.t("决策动作", "Action"), 112.0, false),
        (l.t("冻结价格", "Price"), 92.0, true),
        (l.t("严苛 FCF%", "Strict FCF%"), 88.0, true),
        ("S&P 500", 88.0, true),
        (
            l.t("判断依据与事前证伪条件", "Rationale & Pre-Falsification"),
            0.0,
            false,
        ),
    ];

    let mut card = obsidian_card()
        .child(
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
                .child(
                    text(l.t("决策记录", "Decision Entries"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_BASE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(action_button(
                    l.t("+ 记录一笔决策", "+ Record Decision").to_owned(),
                    false,
                    move || store.open_journal_form(None),
                )),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .px(px(12.0))
                .py(px(8.0))
                .bg(rgb(BG_ROW_ALT))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .children(cols.iter().map(|(lbl, w, right)| {
                    let mut cell = view().px(px(4.0)).flex().flex_row().items_center();
                    if *right {
                        cell = cell.justify_end();
                    }
                    if *w > 0.0 {
                        cell = cell.w(px(*w)).flex_shrink_0();
                    } else {
                        cell = cell.flex_1().min_w(px(0.0));
                    }
                    cell.child(
                        text(lbl.to_owned())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_MUTED)),
                    )
                })),
        );

    if journal.entries.is_empty() {
        return card.child(
            view().px(px(16.0)).py(px(16.0)).child(
                text(
                    l.t(
                        "决策账本尚无记录。点击上方「+ 记录一笔决策」，把判断依据和事前证伪条件固定下来。",
                        "No append-only entries yet. Click '+ Record Decision' above to lock your thesis and kill criteria.",
                    )
                    .to_owned(),
                )
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_MUTED)),
            ),
        );
    }

    card = card.children(journal.entries.iter().rev().enumerate().map(|(i, e)| {
        let (fy_str, _) = fmt_pct_clean(e.strict_fcf_yield_pct, false);
        let (a_fg, a_bg, a_br) = journal_action_colors(e.action);
        let tk = e.ticker.clone();

        view()
            .flex()
            .flex_row()
            .px(px(12.0))
            .py(px(12.0))
            .bg(rgb(if i.is_multiple_of(2) {
                BG_SURFACE
            } else {
                BG_ROW_ALT
            }))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(mono_left(e.recorded_at.clone(), 98.0, INK_SECONDARY))
            .child(
                view()
                    .w(px(84.0))
                    .flex_shrink_0()
                    .px(px(4.0))
                    .cursor_pointer()
                    .on_press(move || store.open_inbox_signal_target(&tk))
                    .child(
                        text(e.ticker.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_HERO)),
                    ),
            )
            .child(
                view()
                    .w(px(112.0))
                    .flex_shrink_0()
                    .px(px(4.0))
                    .flex()
                    .flex_row()
                    .items_start()
                    .child(badge_pill(
                        journal_action_display(e.action, l),
                        a_fg,
                        a_bg,
                        a_br,
                    )),
            )
            .child(mono_right(
                e.price
                    .map(|p| format!("{p:.2}"))
                    .unwrap_or_else(|| "—".into()),
                92.0,
                if e.price.is_some() {
                    INK_SECONDARY
                } else {
                    INK_MUTED
                },
            ))
            .child(mono_right(
                fy_str,
                88.0,
                if e.strict_fcf_yield_pct.is_some() {
                    INK_SECONDARY
                } else {
                    INK_MUTED
                },
            ))
            .child(mono_right(
                e.benchmark_price
                    .map(|p| format!("{p:.2}"))
                    .unwrap_or_else(|| "—".into()),
                88.0,
                INK_MUTED,
            ))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .px(px(8.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        text(e.rationale.clone())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_BASE))
                            .line_height(px(20.0))
                            .text_color(rgb(INK_PRIMARY)),
                    )
                    .child(
                        text(format!(
                            "{}: {}",
                            l.t("证伪条件", "Falsification Trigger"),
                            e.falsification_trigger
                        ))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                    ),
            )
    }));

    card
}

fn stat_inline(label: &str, value: String, color: u32) -> View {
    view()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(6.0))
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED)),
        )
        .child(
            text(value)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(color)),
        )
}

fn compute_portfolio_totals(
    journal: &DecisionJournal,
    reports: &[MungerReport],
) -> (Decimal, Decimal) {
    let mut pos_total = Decimal::ZERO;
    for pos in &journal.positions {
        let mark = reports
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(&pos.ticker))
            .and_then(|r| r.price.as_ref().map(|p| p.price))
            .unwrap_or(pos.avg_cost);
        pos_total += pos.shares * mark;
    }
    let pos_rounded = pos_total.round_dp(2);
    let nav = (journal.cash_balance + pos_rounded).round_dp(2);
    (nav, pos_rounded)
}

fn compute_benchmark_return_pct(
    journal: &DecisionJournal,
    benchmark: Option<&PriceSnapshot>,
) -> Option<Decimal> {
    let start = journal.benchmark_start_price?;
    let now = benchmark?.price;
    if start <= Decimal::ZERO {
        return None;
    }
    Some(((now - start) / start) * Decimal::from(100))
}
