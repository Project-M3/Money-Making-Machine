use super::{
    MungerWorkbench, NavView, components::*, format::*, theme::*, universe::UniverseSearchMode,
};
use crate::{
    budget::{self, ThemeCluster, ThemeClusterMember},
    lake::LakeDb,
    store::WorkspacePaths,
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use std::sync::mpsc;

/// Formats a `ThemeClusterMember` ROIC percentage cleanly for pill display.
pub fn format_theme_member_roic(member: &ThemeClusterMember) -> String {
    match member.roic_5y_avg_pct {
        Some(v) => format!("ROIC {v:.0}%"),
        None => "ROIC —".to_owned(),
    }
}

/// Formats a `ThemeClusterMember` strike-distance percentage cleanly for pill display.
pub fn format_theme_member_distance(member: &ThemeClusterMember) -> String {
    fmt_pct_clean(member.distance_to_strike_pct, true).0
}

impl MungerWorkbench {
    pub(super) fn reload_themes_from_disk(&mut self) {
        match budget::load_themes(&self.paths) {
            Ok(themes) => {
                self.themes_file = themes;
                self.themes_error = None;
            }
            Err(err) => {
                self.themes_error = Some(format!(
                    "{}: {err:#}",
                    self.lang.t(
                        "读取 workbench/themes.json 失败",
                        "Failed to read workbench/themes.json"
                    )
                ));
            }
        }
    }

    pub(super) fn start_themes_sync(&mut self, cx: &mut Context<Self>) {
        if self.themes_syncing || self.universe_syncing {
            if self.universe_syncing {
                self.banner = Some((
                    self.lang
                        .t(
                            "全美股 L0 湖仓正在同步，请完成后再同步主题图谱",
                            "US L0 lakehouse is syncing; wait until it finishes to sync themes",
                        )
                        .into(),
                    false,
                ));
                cx.notify();
            }
            return;
        }
        self.themes_syncing = true;
        self.themes_error = None;
        self.banner = Some((
            self.lang
                .t(
                    "正在从 DuckDB 湖仓同步跨公司商业原型主题图谱…",
                    "Syncing cross-company business archetype clusters from DuckDB…",
                )
                .into(),
            false,
        ));

        let root = self.paths.repo_root.clone();
        let l = self.lang;
        let (tx, rx) = mpsc::channel();
        self.themes_sync_rx = Some(rx);

        std::thread::spawn(move || {
            let paths = WorkspacePaths::from_root(root);
            let res = match LakeDb::open_read_only_if_exists(&paths) {
                Ok(Some(db)) => db
                    .sync_and_load_themes(&paths)
                    .map_err(|e| format!("{e:#}")),
                Ok(None) => Err(l
                    .t(
                        "全美股湖仓尚未同步，请先运行 `m3 universe sync`",
                        "US lakehouse not synced yet — run `m3 universe sync` first",
                    )
                    .to_owned()),
                Err(e) => Err(format!(
                    "{}: {e:#}",
                    l.t("打开 m3.duckdb 失败", "Failed to open m3.duckdb")
                )),
            };
            let _ = tx.send(res);
        });
        cx.notify();
    }

    pub(super) fn poll_themes_sync(&mut self) -> bool {
        let Some(rx) = self.themes_sync_rx.take() else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(themes)) => {
                self.themes_syncing = false;
                self.themes_file = themes;
                self.themes_error = None;
                self.banner = Some((
                    match self.lang {
                        crate::i18n::UiLang::Zh => format!(
                            "主题图谱已同步 · {} 个商业原型簇 / 覆盖 {} 家公司（保留人类批注）",
                            self.themes_file.clusters.len(),
                            self.themes_file.total_indexed_companies
                        ),
                        crate::i18n::UiLang::En => format!(
                            "Themes synced · {} archetype clusters across {} companies (human notes preserved)",
                            self.themes_file.clusters.len(),
                            self.themes_file.total_indexed_companies
                        ),
                    },
                    false,
                ));
                true
            }
            Ok(Err(err)) => {
                self.themes_syncing = false;
                self.themes_error = Some(err.clone());
                self.banner = Some((
                    format!(
                        "{}: {err}",
                        self.lang.t("同步主题图谱失败", "Theme sync failed")
                    ),
                    true,
                ));
                true
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.themes_sync_rx = Some(rx);
                false
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.themes_syncing = false;
                true
            }
        }
    }

    pub(super) fn open_theme_member(&mut self, ticker: &str, cx: &mut Context<Self>) {
        let trimmed = ticker.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(canonical) = self
            .reports
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(trimmed))
            .map(|r| r.ticker.clone())
        {
            self.select_company(&canonical, Some(NavView::Dossier));
            cx.notify();
            return;
        }

        self.universe_state.search_mode = UniverseSearchMode::Keyword;
        self.universe_state.query = trimmed.to_owned();
        self.universe_state.exclude_financials = false;
        self.universe_state.require_5y_history = false;
        self.switch_view(NavView::Universe);
        if let Some(exact) = self
            .universe_rows
            .iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(trimmed))
            .cloned()
        {
            self.select_universe_row(&exact);
        }
        cx.notify();
    }

    pub(super) fn render_themes_and_journal_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .child(self.render_themes_section(cx))
            .child(self.render_journal_view(cx))
    }

    pub(super) fn render_themes_section(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let updated_label = if self.themes_file.updated_at_ms > 0 {
            fmt_time_cn_for(Some(self.themes_file.updated_at_ms), l)
        } else {
            l.t("未同步", "Unsynced").into()
        };
        let total_clusters = self.themes_file.clusters.len();
        let indexed = self.themes_file.total_indexed_companies;

        let header = div()
            .px_5()
            .py_3()
            .bg(rgb(BG_SURFACE))
            .border_b_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(badge(
                        l.t("THEMES · 商业原型图谱", "THEMES · Business Archetypes")
                            .into(),
                        SKY_FG,
                        SKY_BG,
                        SKY_BORDER,
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .font_family(self.fonts.ui.clone())
                            .text_xs()
                            .text_color(rgb(INK_MUTED))
                            .child(l.t(
                                "workbench/themes.json · SIC + 隐空间聚类（保留人类 user_notes）",
                                "workbench/themes.json · SIC + Latent Clustering (preserves user_notes)",
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(journal_stat(
                        l.t("原型簇", "Clusters"),
                        format!("{total_clusters}"),
                        INK_HERO,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        l.t("索引公司", "Indexed"),
                        format!("{indexed}"),
                        SKY_FG,
                        &self.fonts,
                    ))
                    .child(v_divider())
                    .child(journal_stat(
                        l.t("更新时间", "Updated"),
                        updated_label,
                        INK_SECONDARY,
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .id("themes-sync-btn")
                            .px_3()
                            .py_1p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .border_1()
                            .when(self.themes_syncing, |d| {
                                d.bg(rgb(BG_WELL))
                                    .border_color(rgb(BORDER_SUBTLE))
                                    .text_color(rgb(INK_MUTED))
                            })
                            .when(!self.themes_syncing, |d| {
                                d.bg(rgb(CTA_BG))
                                    .border_color(rgb(CTA_BG))
                                    .hover(|s| s.bg(rgb(CTA_BG_HOVER)))
                                    .active(|s| s.bg(rgb(CTA_BG_ACTIVE)))
                                    .text_color(rgb(CTA_INK))
                            })
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.start_themes_sync(cx);
                            }))
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(if self.themes_syncing {
                                        l.t("同步中…", "Syncing…")
                                    } else {
                                        l.t("同步主题图谱", "Sync Themes")
                                    }),
                            )
                            .child(kbd("s", &self.fonts)),
                    ),
            );

        let mut body = div().px_5().pt_5().pb_2().flex().flex_col().gap_3();

        if let Some(err) = &self.themes_error {
            body = body.child(
                div()
                    .px_4()
                    .py_2p5()
                    .rounded_md()
                    .bg(rgba(CORAL_BG))
                    .border_1()
                    .border_color(rgba(CORAL_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .text_color(rgb(CORAL_FG))
                    .child(err.clone()),
            );
        }

        if self.themes_file.clusters.is_empty() {
            let cli_cmd = "./apps/m3-desktop/m3 themes sync";
            body = body.child(
                card_shell()
                    .p_5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(INK_HERO))
                                    .child(l.t(
                                        "尚无跨公司商业原型主题簇",
                                        "No cross-company archetype clusters yet",
                                    )),
                            )
                            .child(
                                div()
                                    .font_family(self.fonts.ui.clone())
                                    .text_xs()
                                    .text_color(rgb(INK_MUTED))
                                    .child(l.t(
                                        "点击右上角「同步主题图谱」或运行 CLI，从 DuckDB 湖仓生成主题聚类；已手写的 user_notes 将自动保留。",
                                        "Click Sync Themes or run the CLI to generate archetype clusters from DuckDB; existing user_notes are preserved.",
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id("themes-empty-copy-cli")
                            .px_3()
                            .py_1p5()
                            .rounded_sm()
                            .bg(rgb(BG_WELL))
                            .border_1()
                            .border_color(rgb(BORDER_PILL))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .font_family(self.fonts.mono_term.clone())
                            .text_xs()
                            .text_color(rgb(INK_PRIMARY))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_cli_command(cli_cmd.into(), "themes sync", cx);
                                cx.notify();
                            }))
                            .child(cli_cmd),
                    ),
            );
        } else {
            body = body.children(
                self.themes_file
                    .clusters
                    .iter()
                    .enumerate()
                    .map(|(idx, cluster)| self.render_theme_cluster_card(cluster, idx, cx)),
            );
        }

        div().flex().flex_col().child(header).child(body)
    }

    fn render_theme_cluster_card(
        &self,
        cluster: &ThemeCluster,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let l = self.lang;
        let median_roic = cluster
            .median_roic_5y_pct
            .map(|r| format!("{} {r:.1}%", l.t("中位 ROIC", "Med ROIC")))
            .unwrap_or_else(|| format!("{} —", l.t("中位 ROIC", "Med ROIC")));
        let sic_txt = if cluster.sic_codes.is_empty() {
            "SIC —".into()
        } else {
            let codes: Vec<String> = cluster
                .sic_codes
                .iter()
                .take(4)
                .map(|c| c.to_string())
                .collect();
            format!("SIC {}", codes.join(","))
        };

        let mut card = card_shell().p_4().flex().flex_col().gap_2p5().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            div()
                                .font_family(self.fonts.ui.clone())
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO))
                                .child(cluster.title.clone()),
                        )
                        .child(badge(
                            cluster.archetype.clone(),
                            VIOLET_FG,
                            VIOLET_BG,
                            VIOLET_BORDER,
                            &self.fonts,
                        ))
                        .when(cluster.strike_zone_count > 0, |d| {
                            d.child(badge(
                                format!(
                                    "{} {}",
                                    cluster.strike_zone_count,
                                    l.t("家击球区", "in strike")
                                ),
                                EMERALD_FG,
                                EMERALD_BG,
                                EMERALD_BORDER,
                                &self.fonts,
                            ))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .font_family(self.fonts.mono_num.clone())
                        .text_size(px(11.))
                        .text_color(rgb(INK_SECONDARY))
                        .child(format!("{} {}", cluster.members.len(), l.t("家", "cos")))
                        .child("·")
                        .child(median_roic)
                        .child("·")
                        .child(format!("sim {:.2}", cluster.avg_similarity))
                        .child("·")
                        .child(sic_txt),
                ),
        );

        if !cluster.summary.trim().is_empty() {
            card = card.child(
                div()
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(19.))
                    .text_color(rgb(INK_SECONDARY))
                    .child(cluster.summary.clone()),
            );
        }

        if !cluster.keywords.is_empty() {
            card = card.child(div().flex().items_center().flex_wrap().gap_1().children(
                cluster.keywords.iter().take(6).map(|kw| {
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded_xs()
                        .bg(rgb(BG_WELL))
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .child(format!("#{kw}"))
                }),
            ));
        }

        if !cluster.user_notes.trim().is_empty() {
            card = card.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_sm()
                    .bg(rgba(GOLD_BG))
                    .border_1()
                    .border_color(rgba(GOLD_BORDER))
                    .font_family(self.fonts.ui.clone())
                    .text_xs()
                    .line_height(px(18.))
                    .text_color(rgb(GOLD_FG))
                    .child(format!(
                        "{}: {}",
                        l.t("人类批注 (user_notes)", "Human Notes (user_notes)"),
                        cluster.user_notes
                    )),
            );
        }

        card.child(
            div()
                .pt_1()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_1p5()
                .children(cluster.members.iter().enumerate().map(|(m_idx, member)| {
                    let ticker = member.ticker.clone();
                    let in_watch = member.in_watchlist
                        || self
                            .reports
                            .iter()
                            .any(|r| r.ticker.eq_ignore_ascii_case(&member.ticker));
                    let in_strike = member
                        .distance_to_strike_pct
                        .is_some_and(|d| d <= rust_decimal::Decimal::ZERO);
                    let dot_col = if in_strike {
                        EMERALD_FG
                    } else if in_watch {
                        SKY_FG
                    } else {
                        INK_MUTED
                    };
                    div()
                        .id(("theme-member", idx * 128 + m_idx))
                        .px_2p5()
                        .py_1()
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
                        .items_center()
                        .gap_1p5()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_theme_member(&ticker, cx);
                        }))
                        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(rgb(dot_col)))
                        .child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_xs()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(INK_HERO))
                                .child(member.ticker.clone()),
                        )
                        .child(
                            div()
                                .font_family(self.fonts.mono_num.clone())
                                .text_size(px(10.))
                                .text_color(rgb(INK_SECONDARY))
                                .child(format_theme_member_roic(member)),
                        )
                        .when(member.distance_to_strike_pct.is_some(), |d| {
                            d.child(
                                div()
                                    .font_family(self.fonts.mono_num.clone())
                                    .text_size(px(10.))
                                    .text_color(rgb(if in_strike { EMERALD_FG } else { INK_MUTED }))
                                    .child(format_theme_member_distance(member)),
                            )
                        })
                })),
        )
    }
}
