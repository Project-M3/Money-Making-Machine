//! Theme Archetype Clusters & Append-Only Decision Journal view (`⌘5`).

use crate::components::{
    action_button, badge_pill, cmd_kbd_corner, concept_hint_static, mono_left, mono_right,
    obsidian_card, v_divider, view_header_strip,
};
use crate::format::{
    fmt_pct_clean, fmt_price, fmt_time_cn_for, format_theme_member_distance,
    format_theme_member_roic,
};
use crate::platform_desktop::copy_text_with_notice;
use crate::stores::AppStore;
use crate::theme::*;
use m3_core::budget::ThemeCluster;
use m3_core::i18n::UiLang;
use m3_core::model::{DecisionJournal, MungerReport, PriceSnapshot};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

/// Renders Theme Archetype Clusters above the Append-Only Decision Journal (`⌘5`).
pub fn journal_view(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(render_themes_header_strip(store))
        .child(
            scroll().flex_1().min_h(px(0.0)).w_full().child(
                view()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(render_themes_section(store))
                    .child(render_journal_section(store)),
            ),
        )
}

fn render_themes_header_strip(store: AppStore) -> View {
    view_header_strip()
        .child(dynamic(move || {
            let l = store.lang.get();
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(badge_pill(
                    l.t("THEMES · 商业原型图谱", "THEMES · Business Archetypes"),
                    SKY_FG,
                    SKY_BG,
                    SKY_BORDER,
                ))
                .child(concept_hint_static(l.t(
                    "基于 SIC 与财报隐空间聚类生成的商业原型，保留人类批注。",
                    "Business archetypes clustered from SIC and 10-K disclosures; human notes are preserved.",
                )))
        }))
        .child(dynamic(move || {
            let l = store.lang.get();
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
                let cli_cmd = "m3 themes sync";
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
                                            "尚无跨公司商业原型主题簇",
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
                                            "点击右上角「同步主题图谱」或运行 CLI，从 DuckDB 湖仓生成主题聚类；已手写的 user_notes 将自动保留。",
                                            "Click Sync Themes or run the CLI to generate archetype clusters from DuckDB; existing user_notes are preserved.",
                                        )
                                        .to_owned(),
                                    )
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                                ),
                        )
                        .child(
                            view()
                                .px(px(12.0))
                                .py(px(6.0))
                                .rounded_sm()
                                .bg(rgb(BG_WELL))
                                .border_1()
                                .border_color(rgb(BORDER_PILL))
                                .hover(|s| s.bg(rgb(BG_HOVER)))
                                .cursor_pointer()
                                .on_press(move || {
                                    copy_text_with_notice(
                                        store,
                                        cli_cmd.to_owned(),
                                        "themes sync",
                                    );
                                })
                                .child(
                                    text(cli_cmd)
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_PRIMARY)),
                                ),
                        ),
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

fn render_journal_section(store: AppStore) -> View {
    view().w_full().flex().flex_col().child(dynamic(move || {
        let l = store.lang.get();
        let journal = store.journal.get();
        let benchmark = store.benchmark.get();
        let reports = store.reports.get();

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
            .unwrap_or_else(|| l.t("待锁定", "Unset").to_owned());

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
            .mt(px(8.0))
            .px(px(18.0))
            .py(px(10.0))
            .bg(rgb(BG_SURFACE))
            .border_t_1()
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
                        "事前冻结、只增不改的决策账本，以 100% 现金与 S&P 500 为基准。",
                        "Append-only pre-committed decision ledger benchmarked against 100% cash and S&P 500.",
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

        view().flex().flex_col().child(header).child(
            view()
                .px(px(18.0))
                .py(px(18.0))
                .flex()
                .flex_col()
                .gap(px(16.0))
                .child(positions_card)
                .child(entries_card),
        )
    }))
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
                            "当前 100% 持有模拟现金（等待 CORE 标的落入击球区后通过 CLI 记录 paper-buy）。",
                            "Currently 100% Paper Cash — waiting patiently for CORE fat pitches (record via CLI paper-buy).",
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
    let journal_cmd = "m3 journal record --ticker SYM --action wait --note \"...\"";

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
                .child(
                    view()
                        .relative()
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded_sm()
                        .bg(rgb(BG_SURFACE))
                        .border_1()
                        .border_color(rgb(BORDER_PILL))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .cursor_pointer()
                        .flex()
                        .flex_row()
                        .items_center()
                        .on_press(move || {
                            copy_text_with_notice(store, journal_cmd.to_owned(), "journal record");
                        })
                        .child(
                            text("Copy CLI")
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(cmd_kbd_corner(store, "c")),
                ),
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
                        "决策账本尚无冻结记录。每次做出 Wait / Out / Too Hard / Paper Buy / Paper Sell 决策前，请先记录判断依据与事前证伪条件。",
                        "No append-only entries yet. Record your thesis and pre-falsification trigger before every Wait / Out / Too Hard / Paper decision.",
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
                    .child(badge_pill(e.action.label(), a_fg, a_bg, a_br)),
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
