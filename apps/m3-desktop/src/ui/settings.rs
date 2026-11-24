use super::{MungerWorkbench, components::*, format::*, theme::*};
use crate::{
    app_config,
    budget::BudgetConfig,
    i18n::UiLang,
    update::{self, ReleaseChannel},
};
use gpui::{Context, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;

const ALAN_SETUP_CMD: &str = "alan connection add openrouter --profile main && alan connection set-secret main && alan connection default set main";
const WEBSITE_URL: &str = "https://m3.thinksmart.systems";
const GITHUB_URL: &str = "https://github.com/Project-M3/Money-Making-Machine";
const RELEASES_URL: &str = "https://github.com/Project-M3/Money-Making-Machine/releases";

fn setting_chip(
    id: impl Into<gpui::SharedString>,
    label: &str,
    active: bool,
    accent: Option<(u32, u32, u32)>,
    fonts: &super::FontRoles,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let mut el = div()
        .id(id.into())
        .px_2p5()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .border_1()
        .font_family(fonts.mono_num.clone())
        .text_size(px(11.))
        .on_click(on_click);

    if active {
        let (fg, bg, br) = accent.unwrap_or((EMERALD_FG, EMERALD_BG, EMERALD_BORDER));
        el = el
            .bg(rgba(bg))
            .border_color(rgba(br))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(fg));
    } else {
        el = el
            .bg(rgb(BG_WELL))
            .border_color(rgb(BORDER_SUBTLE))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .active(|s| s.bg(rgb(BG_ACTIVE)))
            .text_color(rgb(INK_SECONDARY));
    }
    el.child(label.to_owned())
}

fn section_header(tag: &str, title: &str, subtitle: &str, fonts: &super::FontRoles) -> gpui::Div {
    div()
        .px_4()
        .py_3()
        .bg(rgb(BG_WELL))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO))
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_size(px(11.))
                        .text_color(rgb(INK_MUTED))
                        .child(subtitle.to_owned()),
                ),
        )
        .child(
            div()
                .font_family(fonts.mono_term.clone())
                .text_size(px(10.))
                .text_color(rgb(INK_MUTED))
                .child(tag.to_owned()),
        )
}

fn setting_row(label: &str, hint: Option<&str>, fonts: &super::FontRoles) -> gpui::Div {
    let mut left = div().flex().flex_col().gap_0p5().child(
        div()
            .font_family(fonts.ui.clone())
            .text_xs()
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(rgb(INK_PRIMARY))
            .child(label.to_owned()),
    );
    if let Some(h) = hint {
        left = left.child(
            div()
                .font_family(fonts.ui.clone())
                .text_size(px(11.))
                .text_color(rgb(INK_MUTED))
                .child(h.to_owned()),
        );
    }
    div()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(left)
}

impl MungerWorkbench {
    pub(super) fn render_settings_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let l = self.lang;
        let has_custom_workspace = app_config::read_configured_workspace_root().is_some();
        let workspace_path = self.paths.repo_root.display().to_string();
        let build_commit = update::current_build_commit();
        let ver_text = if build_commit.is_empty() {
            format!("v{}", update::current_binary_version())
        } else {
            format!("v{} ({build_commit})", update::current_binary_version())
        };

        let monthly_used = self
            .budget_report
            .as_ref()
            .map(|r| r.usage.total_cost_usd.round_dp(4).normalize().to_string())
            .unwrap_or_else(|| "0".into());
        let monthly_cap = self
            .budget_config
            .monthly_budget_usd
            .round_dp(2)
            .normalize();
        let tokens_used = self
            .budget_report
            .as_ref()
            .map(|r| {
                r.usage
                    .total_input_tokens
                    .saturating_add(r.usage.total_output_tokens)
            })
            .unwrap_or(0);

        let lake_companies = self
            .universe_summary
            .as_ref()
            .map(|s| s.total_companies)
            .unwrap_or(0);
        let semantic_chunks = self
            .inbox_summary
            .as_ref()
            .map(|s| s.doc_chunks_count)
            .unwrap_or(0);
        let unread_signals = self
            .inbox_summary
            .as_ref()
            .map(|s| s.unread_signals)
            .unwrap_or(0);
        let last_sync = self
            .sync_status
            .as_ref()
            .map(|s| fmt_time_cn_for(Some(s.synced_at_ms), l))
            .unwrap_or_else(|| l.t("未同步", "Never").to_owned());

        let monthly_presets: [(&str, Decimal); 5] = [
            ("$5", Decimal::from(5)),
            ("$10", Decimal::from(10)),
            ("$25", Decimal::from(25)),
            ("$50", Decimal::from(50)),
            ("$100", Decimal::from(100)),
        ];
        let per_run_presets: [(&str, Decimal); 4] = [
            ("$0.50", Decimal::new(50, 2)),
            ("$0.75", Decimal::new(75, 2)),
            ("$1.50", Decimal::new(150, 2)),
            ("$3.00", Decimal::new(300, 2)),
        ];
        let weekly_quotas: [usize; 5] = [0, 1, 3, 5, 10];

        div()
            .w_full()
            .max_w(px(900.))
            .px_6()
            .py_5()
            .flex()
            .flex_col()
            .gap_4()
            // 1. Interface & Language
            .child(
                card_shell()
                    .child(section_header(
                        "INTERFACE · I18N",
                        l.t("界面与语言", "Interface & Language"),
                        l.t(
                            "即时切换界面、菜单栏与命令面板语言，重启后保持记忆",
                            "Switch all views, menus, and command palette live; persisted in config.json",
                        ),
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(INK_PRIMARY))
                                            .child(l.t("显示语言", "Display Language")),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_size(px(11.))
                                            .text_color(rgb(INK_MUTED))
                                            .child(l.t(
                                                "支持中文与 English 实时无感切换",
                                                "Switches live between 中文 and English without restarting",
                                            )),
                                    ),
                            )
                            .child(
                                seg_group()
                                    .child(seg_tab(
                                        "settings-lang-zh",
                                        "中文",
                                        self.lang == UiLang::Zh,
                                        Some((EMERALD_FG, EMERALD_BG)),
                                        false,
                                        &self.fonts,
                                        cx.listener(|this, _, _, cx| {
                                            this.set_language(UiLang::Zh, cx);
                                        }),
                                    ))
                                    .child(seg_tab(
                                        "settings-lang-en",
                                        "English",
                                        self.lang == UiLang::En,
                                        Some((SKY_FG, SKY_BG)),
                                        false,
                                        &self.fonts,
                                        cx.listener(|this, _, _, cx| {
                                            this.set_language(UiLang::En, cx);
                                        }),
                                    )),
                            ),
                    ),
            )
            // 2. Workspace
            .child(
                card_shell()
                    .child(section_header(
                        "WORKSPACE · LOCAL-FIRST",
                        l.t("本地工作区", "Workspace"),
                        l.t(
                            "全部财报缓存、能力圈分层、决策账本与 DuckDB 湖仓均本地保存",
                            "Circle watchlist, decision journal, filings, and DuckDB lakehouse stay local",
                        ),
                        &self.fonts,
                    ))
                    .child(
                        setting_row(
                            l.t("当前工作区目录", "Active Workspace Root"),
                            Some(&workspace_path),
                            &self.fonts,
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(badge(
                                    if has_custom_workspace {
                                        l.t("自定义工作区", "Custom Folder").to_owned()
                                    } else {
                                        l.t("独立空白工作区", "Standalone Default").to_owned()
                                    },
                                    if has_custom_workspace { SKY_FG } else { EMERALD_FG },
                                    if has_custom_workspace { SKY_BG } else { EMERALD_BG },
                                    if has_custom_workspace {
                                        SKY_BORDER
                                    } else {
                                        EMERALD_BORDER
                                    },
                                    &self.fonts,
                                ))
                                .child(badge(
                                    format!(
                                        "{} {}",
                                        self.reports.len(),
                                        l.t("家标的", "holdings")
                                    ),
                                    INK_SECONDARY,
                                    EMERALD_BG,
                                    BORDER_SUBTLE,
                                    &self.fonts,
                                )),
                        ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(setting_chip(
                                "settings-ws-reveal",
                                l.t("在 Finder 中打开", "Reveal in Finder"),
                                false,
                                None,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    let root = this.paths.repo_root.clone();
                                    cx.reveal_path(&root);
                                }),
                            ))
                            .child(setting_chip(
                                "settings-ws-choose",
                                l.t("切换或新建目录…", "Choose Folder…"),
                                false,
                                None,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.prompt_choose_workspace(cx);
                                }),
                            ))
                            .when(has_custom_workspace, |d| {
                                d.child(setting_chip(
                                    "settings-ws-reset",
                                    l.t("恢复默认空白工作区", "Reset to Default Workspace"),
                                    false,
                                    None,
                                    &self.fonts,
                                    cx.listener(|this, _, _, cx| {
                                        this.reset_workspace_to_default(cx);
                                    }),
                                ))
                            })
                            .child(setting_chip(
                                "settings-ws-reload",
                                l.t("重新加载磁盘数据", "Reload from Disk"),
                                false,
                                None,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.reload_from_disk();
                                    this.reload_themes_from_disk();
                                    this.reload_universe_from_lake();
                                    this.reload_inbox_from_lake();
                                    this.reload_budget_from_lake();
                                    this.banner = Some((
                                        this.lang
                                            .t("已重新加载工作区数据", "Reloaded workspace from disk")
                                            .to_owned(),
                                        false,
                                    ));
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            // 3. Updates & Release Channel
            .child(
                card_shell()
                    .child(section_header(
                        "UPDATER · SIGNED & VERIFIED",
                        l.t("软件更新与通道", "Updates & Release Channel"),
                        l.t(
                            "支持 stable 正式版与 beta 滚动预览通道，校验 SHA-256 原子替换",
                            "Switch between stable tags and rolling beta builds with SHA-256 verification",
                        ),
                        &self.fonts,
                    ))
                    .child(
                        setting_row(
                            l.t("当前版本", "Installed Version"),
                            Some(&ver_text),
                            &self.fonts,
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    seg_group()
                                        .child(seg_tab(
                                            "settings-ch-stable",
                                            "stable",
                                            self.update_channel == ReleaseChannel::Stable,
                                            Some((EMERALD_FG, EMERALD_BG)),
                                            true,
                                            &self.fonts,
                                            cx.listener(|this, _, _, cx| {
                                                this.switch_update_channel(
                                                    ReleaseChannel::Stable,
                                                    cx,
                                                );
                                            }),
                                        ))
                                        .child(seg_tab(
                                            "settings-ch-beta",
                                            "beta",
                                            self.update_channel == ReleaseChannel::Beta,
                                            Some((SKY_FG, SKY_BG)),
                                            true,
                                            &self.fonts,
                                            cx.listener(|this, _, _, cx| {
                                                this.switch_update_channel(ReleaseChannel::Beta, cx);
                                            }),
                                        )),
                                )
                                .child(if let Some(ver) = &self.update_applied_version {
                                    let restart_lbl = format!(
                                        "{} v{ver}",
                                        l.t("重启以生效", "Restart to Apply")
                                    );
                                    setting_chip(
                                        "settings-update-restart",
                                        &restart_lbl,
                                        true,
                                        Some((EMERALD_FG, EMERALD_BG, EMERALD_BORDER)),
                                        &self.fonts,
                                        cx.listener(|this, _, _, cx| {
                                            this.restart_after_update(cx);
                                        }),
                                    )
                                } else if self.update_applying {
                                    setting_chip(
                                        "settings-update-applying",
                                        l.t("正在安装更新…", "Installing…"),
                                        false,
                                        None,
                                        &self.fonts,
                                        cx.listener(|_, _, _, _| {}),
                                    )
                                } else if let Some(notice) = &self.update_notice {
                                    let apply_lbl = format!(
                                        "{} v{}",
                                        l.t("安装更新", "Install"),
                                        notice.latest_version
                                    );
                                    setting_chip(
                                        "settings-update-apply",
                                        &apply_lbl,
                                        true,
                                        Some((SKY_FG, SKY_BG, SKY_BORDER)),
                                        &self.fonts,
                                        cx.listener(|this, _, _, cx| {
                                            this.start_update_apply(cx);
                                        }),
                                    )
                                } else {
                                    setting_chip(
                                        "settings-update-check",
                                        if self.update_checking {
                                            l.t("检查中…", "Checking…")
                                        } else {
                                            l.t("检查更新", "Check for Updates")
                                        },
                                        false,
                                        None,
                                        &self.fonts,
                                        cx.listener(|this, _, _, cx| {
                                            this.start_update_check(true, cx);
                                        }),
                                    )
                                }),
                        ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_2p5()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(setting_chip(
                                "settings-link-web",
                                l.t("Project M3 官网 ↗", "Website ↗"),
                                false,
                                None,
                                &self.fonts,
                                cx.listener(|_, _, _, cx| {
                                    cx.open_url(WEBSITE_URL);
                                }),
                            ))
                            .child(setting_chip(
                                "settings-link-gh",
                                "GitHub ↗",
                                false,
                                None,
                                &self.fonts,
                                cx.listener(|_, _, _, cx| {
                                    cx.open_url(GITHUB_URL);
                                }),
                            ))
                            .child(setting_chip(
                                "settings-link-rel",
                                "Releases ↗",
                                false,
                                None,
                                &self.fonts,
                                cx.listener(|_, _, _, cx| {
                                    cx.open_url(RELEASES_URL);
                                }),
                            )),
                    ),
            )
            // 4. AI Budget & Embedded Alan Provider
            .child(
                card_shell()
                    .child(section_header(
                        "L2 · ALAN GUARDRAILS",
                        l.t("AI 深研预算与护栏", "AI Budget & Guardrails"),
                        l.t(
                            "写入 workbench/budget.json · 到达硬上限前禁止启动 Embedded Alan",
                            "Saved to workbench/budget.json · Hard gate blocks Deep Research before spending",
                        ),
                        &self.fonts,
                    ))
                    .child(
                        setting_row(
                            l.t("本月已用 / 月度上限", "Monthly Usage / Cap"),
                            Some(&format!(
                                "${monthly_used} / ${monthly_cap} · {tokens_used} / {} tokens",
                                self.budget_config.monthly_token_limit
                            )),
                            &self.fonts,
                        )
                        .child(
                            div().flex().items_center().gap_1p5().children(
                                monthly_presets.into_iter().map(|(label, val)| {
                                    let active = self.budget_config.monthly_budget_usd == val;
                                    setting_chip(
                                        gpui::SharedString::from(format!("settings-mb-{val}")),
                                        label,
                                        active,
                                        None,
                                        &self.fonts,
                                        cx.listener(move |this, _, _, cx| {
                                            this.update_budget_config(
                                                |cfg| cfg.monthly_budget_usd = val,
                                                cx,
                                            );
                                        }),
                                    )
                                }),
                            ),
                        ),
                    )
                    .child(
                        setting_row(
                            l.t("单次 Deep Research 预算上限", "Per-Run Cost Ceiling"),
                            Some(l.t(
                                "4 视角并行深研（护城河 / 会计 / 同行 / 逆向验尸）总上限",
                                "Total USD ceiling across all 4 parallel Deep Research perspectives",
                            )),
                            &self.fonts,
                        )
                        .child(
                            div().flex().items_center().gap_1p5().children(
                                per_run_presets.into_iter().map(|(label, val)| {
                                    let active = self.budget_config.per_run_budget_usd == val;
                                    setting_chip(
                                        gpui::SharedString::from(format!("settings-prb-{label}")),
                                        label,
                                        active,
                                        Some((SKY_FG, SKY_BG, SKY_BORDER)),
                                        &self.fonts,
                                        cx.listener(move |this, _, _, cx| {
                                            this.update_budget_config(
                                                |cfg| cfg.per_run_budget_usd = val,
                                                cx,
                                            );
                                        }),
                                    )
                                }),
                            ),
                        ),
                    )
                    .child(
                        setting_row(
                            l.t("自动触发深研与每周配额", "Auto-Trigger & Weekly Quota"),
                            Some(l.t(
                                "L0/L1 完全零 LLM；仅在开启后于每周配额内自动运行 L2",
                                "L0/L1 stay 100% Zero-LLM; optional L2 auto-runs stay capped weekly",
                            )),
                            &self.fonts,
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    seg_group()
                                        .child(seg_tab(
                                            "settings-auto-off",
                                            l.t("关闭", "Off"),
                                            !self.budget_config.auto_trigger_enabled,
                                            None,
                                            false,
                                            &self.fonts,
                                            cx.listener(|this, _, _, cx| {
                                                this.update_budget_config(
                                                    |cfg| cfg.auto_trigger_enabled = false,
                                                    cx,
                                                );
                                            }),
                                        ))
                                        .child(seg_tab(
                                            "settings-auto-on",
                                            l.t("开启", "On"),
                                            self.budget_config.auto_trigger_enabled,
                                            Some((EMERALD_FG, EMERALD_BG)),
                                            false,
                                            &self.fonts,
                                            cx.listener(|this, _, _, cx| {
                                                this.update_budget_config(
                                                    |cfg| cfg.auto_trigger_enabled = true,
                                                    cx,
                                                );
                                            }),
                                        )),
                                )
                                .child(
                                    div().flex().items_center().gap_1().children(
                                        weekly_quotas.into_iter().map(|q| {
                                            let active = self.budget_config.weekly_auto_quota == q;
                                            let q_lbl = format!("{q}/wk");
                                            setting_chip(
                                                gpui::SharedString::from(format!(
                                                    "settings-wq-{q}"
                                                )),
                                                &q_lbl,
                                                active,
                                                None,
                                                &self.fonts,
                                                cx.listener(move |this, _, _, cx| {
                                                    this.update_budget_config(
                                                        |cfg| cfg.weekly_auto_quota = q,
                                                        cx,
                                                    );
                                                }),
                                            )
                                        }),
                                    ),
                                )
                                .child(setting_chip(
                                    "settings-budget-reset",
                                    l.t("恢复默认", "Reset"),
                                    false,
                                    None,
                                    &self.fonts,
                                    cx.listener(|this, _, _, cx| {
                                        this.update_budget_config(
                                            |cfg| *cfg = BudgetConfig::default(),
                                            cx,
                                        );
                                    }),
                                )),
                        ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .font_family(self.fonts.ui.clone())
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .text_color(rgb(INK_PRIMARY))
                                            .child(l.t(
                                                "Embedded Alan 模型连接配置",
                                                "Embedded Alan Connection Profile",
                                            )),
                                    )
                                    .child(
                                        div()
                                            .font_family(self.fonts.mono_term.clone())
                                            .text_size(px(11.))
                                            .text_color(rgb(INK_MUTED))
                                            .truncate()
                                            .child(ALAN_SETUP_CMD),
                                    ),
                            )
                            .child(setting_chip(
                                "settings-copy-alan-cmd",
                                l.t("复制配置命令", "Copy Setup Command"),
                                false,
                                None,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    let lbl = this.lang.t("Alan 连接命令", "Alan setup");
                                    this.copy_cli_command(ALAN_SETUP_CMD.to_owned(), lbl, cx);
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            // 5. Data Lake & Maintenance
            .child(
                card_shell()
                    .child(section_header(
                        "DUCKDB · L0 / L1",
                        l.t("数据湖与日常维护", "Data Lake & Maintenance"),
                        &format!(
                            "L0: {lake_companies} {} · L1: {unread_signals} {} / {semantic_chunks} {} · {} {last_sync}",
                            l.t("家公司", "issuers"),
                            l.t("未读信号", "unread"),
                            l.t("语义片段", "chunks"),
                            l.t("上次自选同步", "Last Circle sync"),
                        ),
                        &self.fonts,
                    ))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .child(setting_chip(
                                "settings-sync-circle",
                                if self.syncing {
                                    l.t("能力圈同步中…", "Syncing Circle…")
                                } else {
                                    l.t("同步能力圈财报与股价 (⌘R)", "Sync Circle Financials (⌘R)")
                                },
                                self.syncing,
                                None,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.start_sync(cx);
                                }),
                            ))
                            .child(setting_chip(
                                "settings-sync-l1",
                                if self.inbox_catching_up {
                                    l.t("追平事件流中…", "Catching Up L1…")
                                } else {
                                    l.t("追平 L1 SEC 与新闻流", "Catch Up L1 Events")
                                },
                                self.inbox_catching_up,
                                Some((SKY_FG, SKY_BG, SKY_BORDER)),
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.trigger_l1_catchup(cx);
                                }),
                            ))
                            .child(setting_chip(
                                "settings-sync-l0",
                                if self.universe_syncing {
                                    l.t("全美股 L0 同步中…", "Syncing US L0…")
                                } else {
                                    l.t("同步全美股 L0 湖仓", "Sync US L0 Lakehouse")
                                },
                                self.universe_syncing,
                                Some((SKY_FG, SKY_BG, SKY_BORDER)),
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.trigger_universe_sync(cx);
                                }),
                            ))
                            .child(setting_chip(
                                "settings-sync-themes",
                                if self.themes_syncing {
                                    l.t("主题图谱生成中…", "Syncing Themes…")
                                } else {
                                    l.t("同步商业原型主题图谱", "Sync Theme Graph")
                                },
                                self.themes_syncing,
                                None,
                                &self.fonts,
                                cx.listener(|this, _, _, cx| {
                                    this.trigger_themes_sync(cx);
                                }),
                            )),
                    ),
            )
            .child(
                div().px_1().flex().items_center().justify_between().child(
                    div()
                        .font_family(self.fonts.mono_term.clone())
                        .text_size(px(10.))
                        .text_color(rgb(INK_MUTED))
                        .child(l.t(
                            "esc 返回简报收件箱 · ⌘, 打开设置 · ⌘K 命令面板",
                            "esc back to Inbox · ⌘, Settings · ⌘K Command Palette",
                        )),
                ),
            )
    }
}
