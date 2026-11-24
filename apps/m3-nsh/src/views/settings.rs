//! Settings & Guardrails view (`⌘,`) in a 2-column Obsidian layout.

use crate::components::{
    action_button, badge_pill, obsidian_card, seg_group, seg_tab, styled_input,
};
use crate::format::fmt_time_cn_for;
use crate::platform_desktop::{
    M3_GITHUB_URL, M3_RELEASES_URL, M3_WEBSITE_URL, copy_text_with_notice, open_external_url,
    prompt_choose_workspace, restart_after_update, reveal_workspace_dir,
};
use crate::stores::AppStore;
use crate::theme::*;
use m3_core::app_config;
use m3_core::budget::BudgetConfig;
use m3_core::i18n::UiLang;
use m3_core::update::{self, ReleaseChannel};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

const ALAN_SETUP_CMD: &str = "alan connection add openrouter --profile main && alan connection set-secret main && alan connection default set main";

/// Renders Preferences & Guardrails (`⌘,`) in a balanced 2-column Obsidian card grid.
pub fn settings_view(store: AppStore) -> impl IntoElement {
    let build_commit = update::current_build_commit();
    let ver_text = if build_commit.is_empty() {
        format!("v{}", update::current_binary_version())
    } else {
        format!("v{} ({build_commit})", update::current_binary_version())
    };

    scroll().flex_1().min_h(px(0.0)).w_full().child(
        view()
            .w_full()
            .px(px(24.0))
            .py(px(24.0))
            .flex()
            .flex_col()
            .items_center()
            .child(
                view()
                    .w_full()
                    .max_w(px(1160.0))
                    .flex()
                    .flex_col()
                    .gap(px(20.0))
                    // Page header
                    .child(
                        view()
                            .w_full()
                            .pb(px(12.0))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .flex_row()
                            .items_end()
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
                                        text(move || store.lang.get().t("偏好设置", "Settings").to_owned())
                                            .font_family(FONT_UI)
                                            .text_size(px(TEXT_LG))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK_HERO)),
                                    )
                                    .child(
                                        text(move || {
                                            store
                                                .lang
                                                .get()
                                                .t(
                                                    "管理显示语言、本地工作区目录、发布通道、AI 预算护栏与 DuckDB 湖仓同步",
                                                    "Manage language, local workspace, release channel, AI budget guardrails, and DuckDB sync",
                                                )
                                                .to_owned()
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                    ),
                            )
                            .child(badge_pill(
                                ver_text.clone(),
                                INK_SECONDARY,
                                SLATE_BG,
                                SLATE_BORDER,
                            )),
                    )
                    // 2-Column Card Grid
                    .child(
                        view()
                            .w_full()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(20.0))
                            // Left Column: Language, Workspace, Software Updates
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(20.0))
                                    .child(render_language_card(store))
                                    .child(render_workspace_card(store))
                                    .child(render_updates_card(store, &ver_text)),
                            )
                            // Right Column: AI Budget Guardrails & Data Lake Maintenance
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(20.0))
                                    .child(render_budget_card(store))
                                    .child(render_datalake_card(store)),
                            ),
                    )
                    // Footer hint
                    .child(
                        view()
                            .w_full()
                            .pt(px(4.0))
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
                                            "esc 返回简报收件箱 · ⌘, 打开设置 · ⌘K 命令面板",
                                            "esc back to Inbox · ⌘, Settings · ⌘K Command Palette",
                                        )
                                        .to_owned()
                                })
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                            ),
                    ),
            ),
    )
}

fn render_language_card(store: AppStore) -> View {
    section_block(
        "INTERFACE · I18N",
        move || {
            store
                .lang
                .get()
                .t("界面与语言", "Interface & Language")
                .to_owned()
        },
        move || {
            store
                .lang
                .get()
                .t(
                    "即时切换界面、菜单栏与命令面板语言，重启后保持记忆",
                    "Switch all views, menus, and command palette live; persisted in config.json",
                )
                .to_owned()
        },
        obsidian_card().child(
            setting_row_reactive(
                move || {
                    store
                        .lang
                        .get()
                        .t("显示语言", "Display Language")
                        .to_owned()
                },
                move || {
                    Some(
                        store
                            .lang
                            .get()
                            .t(
                                "支持中文与 English 实时无感切换",
                                "Switches live between 中文 and English without restarting",
                            )
                            .to_owned(),
                    )
                },
                false,
                false,
            )
            .child(dynamic(move || {
                let lang = store.lang.get();
                view().flex_shrink_0().child(
                    seg_group()
                        .child(seg_tab(
                            "中文",
                            lang == UiLang::Zh,
                            Some((EMERALD_FG, EMERALD_BG)),
                            false,
                            move || store.set_language(UiLang::Zh),
                        ))
                        .child(seg_tab(
                            "English",
                            lang == UiLang::En,
                            Some((SKY_FG, SKY_BG)),
                            false,
                            move || store.set_language(UiLang::En),
                        )),
                )
            })),
        ),
    )
}

fn render_workspace_card(store: AppStore) -> View {
    section_block(
        "WORKSPACE · LOCAL-FIRST",
        move || store.lang.get().t("本地工作区", "Workspace").to_owned(),
        move || {
            store
                .lang
                .get()
                .t(
                    "全部财报缓存、能力圈分层、决策账本与 DuckDB 湖仓均本地保存",
                    "Circle watchlist, decision journal, filings, and DuckDB lakehouse stay local",
                )
                .to_owned()
        },
        obsidian_card()
            .child(dynamic(move || {
                let l = store.lang.get();
                let workspace_path = store.paths.with(|p| p.repo_root.display().to_string());
                let has_custom = app_config::read_configured_workspace_root().is_some();
                let holdings_cnt = store.reports.with(|r| r.len());

                setting_row_static(
                    l.t("当前工作区目录", "Active Workspace Root"),
                    Some(&workspace_path),
                    true,
                    true,
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(badge_pill(
                            if has_custom {
                                l.t("自定义工作区", "Custom Folder").to_owned()
                            } else {
                                l.t("独立空白工作区", "Standalone Default").to_owned()
                            },
                            if has_custom { SKY_FG } else { EMERALD_FG },
                            if has_custom { SKY_BG } else { EMERALD_BG },
                            if has_custom {
                                SKY_BORDER
                            } else {
                                EMERALD_BORDER
                            },
                        ))
                        .child(badge_pill(
                            format!("{holdings_cnt} {}", l.t("家标的", "holdings")),
                            INK_SECONDARY,
                            SLATE_BG,
                            SLATE_BORDER,
                        )),
                )
            }))
            .child(dynamic(move || {
                let l = store.lang.get();
                let has_custom = app_config::read_configured_workspace_root().is_some();
                let mut strip = action_strip()
                    .child(setting_chip(
                        l.t("在 Finder 中打开", "Reveal in Finder"),
                        false,
                        false,
                        None,
                        move || {
                            let root = store.paths.get_untracked().repo_root;
                            reveal_workspace_dir(&root);
                        },
                    ))
                    .child(setting_chip(
                        l.t("切换或新建目录…", "Choose Folder…"),
                        false,
                        false,
                        None,
                        move || prompt_choose_workspace(store),
                    ));
                if has_custom {
                    strip = strip.child(setting_chip(
                        l.t("恢复默认空白工作区", "Reset to Default"),
                        false,
                        false,
                        None,
                        move || store.reset_workspace_to_default(),
                    ));
                }
                strip.child(setting_chip(
                    l.t("重新加载磁盘数据", "Reload from Disk"),
                    false,
                    false,
                    None,
                    move || {
                        store.reload_all();
                        let cur = store.lang.get_untracked();
                        store.set_info_notice(
                            cur.t("已重新加载工作区数据", "Reloaded workspace from disk")
                                .to_owned(),
                        );
                    },
                ))
            })),
    )
}

fn render_updates_card(store: AppStore, ver_text: &str) -> View {
    let ver_owned = ver_text.to_owned();
    section_block(
        "UPDATER · SIGNED & VERIFIED",
        move || {
            store
                .lang
                .get()
                .t("软件更新与通道", "Updates & Release Channel")
                .to_owned()
        },
        move || {
            store
                .lang
                .get()
                .t(
                    "支持 stable 正式版与 beta 滚动预览通道，校验 SHA-256 原子替换",
                    "Switch between stable tags and rolling beta builds with SHA-256 verification",
                )
                .to_owned()
        },
        obsidian_card()
            .child(dynamic(move || {
                let l = store.lang.get();
                let ch = store.update_channel.get();
                let applied = store.update_applied_version.get();
                let applying = store.update_applying.get();
                let checking = store.update_checking.get();
                let notice = store.update_notice.get();
                let progress = store.update_progress.get();

                let action_el = if let Some(ver) = applied {
                    setting_chip(
                        &format!("{} v{ver}", l.t("重启以生效", "Restart to Apply")),
                        true,
                        false,
                        Some((EMERALD_FG, EMERALD_BG, EMERALD_BORDER)),
                        move || restart_after_update(store),
                    )
                } else if applying {
                    setting_chip(
                        l.t("正在安装更新…", "Installing…"),
                        false,
                        false,
                        None,
                        || {},
                    )
                } else if let Some(n) = notice {
                    setting_chip(
                        &format!("{} v{}", l.t("安装更新", "Install"), n.latest_version),
                        true,
                        false,
                        Some((SKY_FG, SKY_BG, SKY_BORDER)),
                        move || store.start_update_apply(),
                    )
                } else {
                    setting_chip(
                        if checking {
                            l.t("检查中…", "Checking…")
                        } else {
                            l.t("检查更新", "Check for Updates")
                        },
                        false,
                        false,
                        None,
                        move || store.start_update_check(true),
                    )
                };

                let mut col = view().flex().flex_col().child(
                    setting_row_static(
                        l.t("当前版本", "Installed Version"),
                        Some(&ver_owned),
                        true,
                        true,
                    )
                    .child(
                        view()
                            .flex_shrink_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                seg_group()
                                    .child(seg_tab(
                                        "stable",
                                        ch == ReleaseChannel::Stable,
                                        Some((EMERALD_FG, EMERALD_BG)),
                                        true,
                                        move || store.switch_update_channel(ReleaseChannel::Stable),
                                    ))
                                    .child(seg_tab(
                                        "beta",
                                        ch == ReleaseChannel::Beta,
                                        Some((SKY_FG, SKY_BG)),
                                        true,
                                        move || store.switch_update_channel(ReleaseChannel::Beta),
                                    )),
                            )
                            .child(action_el),
                    ),
                );

                if let Some(prog) = progress {
                    col = col.child(
                        view()
                            .px(px(16.0))
                            .py(px(8.0))
                            .bg(rgba(SKY_BG))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .child(
                                text(prog)
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(SKY_FG)),
                            ),
                    );
                }

                col
            }))
            .child(dynamic(move || {
                let l = store.lang.get();
                action_strip()
                    .child(setting_chip(
                        l.t("Project M3 官网 ↗", "Website ↗"),
                        false,
                        false,
                        None,
                        || open_external_url(M3_WEBSITE_URL),
                    ))
                    .child(setting_chip("GitHub ↗", false, false, None, || {
                        open_external_url(M3_GITHUB_URL)
                    }))
                    .child(setting_chip("Releases ↗", false, false, None, || {
                        open_external_url(M3_RELEASES_URL)
                    }))
            })),
    )
}

fn render_budget_card(store: AppStore) -> View {
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

    section_block(
        "L2 · ALAN GUARDRAILS",
        move || {
            store
                .lang
                .get()
                .t("AI 深研预算与护栏", "AI Budget & Guardrails")
                .to_owned()
        },
        move || {
            store
                .lang
                .get()
                .t(
                    "写入 workbench/budget.json · 到达硬上限前禁止启动 Embedded Alan",
                    "Saved to workbench/budget.json · Hard gate blocks Deep Research before spending",
                )
                .to_owned()
        },
        obsidian_card()
            // Monthly Usage / Cap + Presets
            .child(dynamic(move || {
                let l = store.lang.get();
                let cfg = store.budget_config.get();
                let rep = store.budget_report.get();
                let monthly_used = rep
                    .as_ref()
                    .map(|r| r.usage.total_cost_usd.round_dp(4).normalize().to_string())
                    .unwrap_or_else(|| "0".into());
                let monthly_cap = cfg.monthly_budget_usd.round_dp(2).normalize();
                let tokens_used = rep
                    .as_ref()
                    .map(|r| {
                        r.usage
                            .total_input_tokens
                            .saturating_add(r.usage.total_output_tokens)
                    })
                    .unwrap_or(0);
                let hint = format!(
                    "${monthly_used} / ${monthly_cap} · {tokens_used} / {} tokens",
                    cfg.monthly_token_limit
                );

                setting_row_static(
                    l.t("本月已用 / 月度上限", "Monthly Usage / Cap"),
                    Some(&hint),
                    true,
                    true,
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .children(monthly_presets.into_iter().map(|(label, val)| {
                            let active = cfg.monthly_budget_usd == val;
                            setting_chip(label, active, true, None, move || {
                                store.update_budget_config(|c| c.monthly_budget_usd = val);
                            })
                        })),
                )
            }))
            // Custom Monthly USD & Token Gate inputs (kept static outside dynamic so IME & focus stay intact!)
            .child(
                view()
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
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
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                text(move || {
                                    store
                                        .lang
                                        .get()
                                        .t("自定义月度硬上限", "Custom Monthly Gate")
                                        .to_owned()
                                })
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(INK_PRIMARY)),
                            )
                            .child(
                                text(move || {
                                    store
                                        .lang
                                        .get()
                                        .t(
                                            "美元 (USD) 与 Token 双重硬上限",
                                            "USD & Token hard ceilings",
                                        )
                                        .to_owned()
                                })
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                            ),
                    )
                    .child(
                        view()
                            .flex_shrink_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(4.0))
                                    .child(
                                        text("$")
                                            .font_family(FONT_MONO_NUM)
                                            .text_size(px(TEXT_SM))
                                            .text_color(rgb(INK_MUTED)),
                                    )
                                    .child(
                                        styled_input(
                                            move || store.settings_monthly_budget_draft.get(),
                                            move || {
                                                store.lang.get().t("美元上限", "USD cap").to_owned()
                                            },
                                            true,
                                        )
                                        .w(px(84.0))
                                        .on_change(move |val: &str| {
                                            store.settings_monthly_budget_draft.set(val.to_owned());
                                        })
                                        .on_submit(move |_: &str| store.apply_budget_drafts()),
                                    ),
                            )
                            .child(
                                styled_input(
                                    move || store.settings_monthly_tokens_draft.get(),
                                    move || {
                                        store.lang.get().t("Token 上限", "Token limit").to_owned()
                                    },
                                    true,
                                )
                                .w(px(116.0))
                                .on_change(move |val: &str| {
                                    store.settings_monthly_tokens_draft.set(val.to_owned());
                                })
                                .on_submit(move |_: &str| store.apply_budget_drafts()),
                            )
                            .child(action_button(
                                move || store.lang.get().t("保存", "Save").to_owned(),
                                true,
                                move || store.apply_budget_drafts(),
                            )),
                    ),
            )
            // Per-Run Cost Ceiling
            .child(dynamic(move || {
                let l = store.lang.get();
                let cfg = store.budget_config.get();
                setting_row_static(
                    l.t("单次 Deep Research 预算上限", "Per-Run Cost Ceiling"),
                    Some(l.t(
                        "4 视角并行深研总上限",
                        "USD ceiling across 4 parallel perspectives",
                    )),
                    false,
                    true,
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .children(per_run_presets.into_iter().map(|(label, val)| {
                            let active = cfg.per_run_budget_usd == val;
                            setting_chip(
                                label,
                                active,
                                true,
                                Some((SKY_FG, SKY_BG, SKY_BORDER)),
                                move || {
                                    store.update_budget_config(|c| c.per_run_budget_usd = val);
                                },
                            )
                        })),
                )
            }))
            // Auto-Trigger Deep Research
            .child(dynamic(move || {
                let l = store.lang.get();
                let cfg = store.budget_config.get();
                setting_row_static(
                    l.t("自动触发深研", "Auto-Trigger Deep Research"),
                    Some(l.t(
                        "L0/L1 零 LLM；开启后仅在每周配额内自动运行 L2",
                        "L0/L1 stay Zero-LLM; optional L2 runs stay capped weekly",
                    )),
                    false,
                    true,
                )
                .child(
                    view().flex_shrink_0().child(
                        seg_group()
                            .child(seg_tab(
                                l.t("关闭", "Off"),
                                !cfg.auto_trigger_enabled,
                                None,
                                false,
                                move || {
                                    store.update_budget_config(|c| c.auto_trigger_enabled = false);
                                },
                            ))
                            .child(seg_tab(
                                l.t("开启", "On"),
                                cfg.auto_trigger_enabled,
                                Some((EMERALD_FG, EMERALD_BG)),
                                false,
                                move || {
                                    store.update_budget_config(|c| c.auto_trigger_enabled = true);
                                },
                            )),
                    ),
                )
            }))
            // Weekly Auto Quota
            .child(dynamic(move || {
                let l = store.lang.get();
                let cfg = store.budget_config.get();
                setting_row_static(
                    l.t("每周自动深研配额", "Weekly Auto Quota"),
                    Some(l.t(
                        "限制每周后台自动派发 Deep Research 的最大次数",
                        "Maximum background Deep Research runs per week",
                    )),
                    false,
                    true,
                )
                .child(
                    view()
                        .flex_shrink_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .children(weekly_quotas.into_iter().map(|q| {
                            let active = cfg.weekly_auto_quota == q;
                            setting_chip(&format!("{q}/wk"), active, true, None, move || {
                                store.update_budget_config(|c| c.weekly_auto_quota = q);
                            })
                        }))
                        .child(setting_chip(
                            l.t("恢复默认", "Reset"),
                            false,
                            false,
                            None,
                            move || store.update_budget_config(|c| *c = BudgetConfig::default()),
                        )),
                )
            }))
            // Embedded Alan Connection Profile
            .child(dynamic(move || {
                let l = store.lang.get();
                setting_row_static(
                    l.t(
                        "Embedded Alan 模型连接配置",
                        "Embedded Alan Connection Profile",
                    ),
                    Some(ALAN_SETUP_CMD),
                    true,
                    false,
                )
                .child(setting_chip(
                    l.t("复制配置命令", "Copy Setup Command"),
                    false,
                    false,
                    None,
                    move || {
                        let lbl = store.lang.get_untracked().t("Alan 连接命令", "Alan setup");
                        copy_text_with_notice(store, ALAN_SETUP_CMD.to_owned(), lbl);
                    },
                ))
            })),
    )
}

fn render_datalake_card(store: AppStore) -> View {
    section_block(
        "DUCKDB · L0 / L1",
        move || {
            store
                .lang
                .get()
                .t("数据湖与日常维护", "Data Lake & Maintenance")
                .to_owned()
        },
        move || {
            let l = store.lang.get();
            let lake_companies = store
                .universe_summary
                .with(|s| s.as_ref().map(|x| x.total_companies).unwrap_or(0));
            let (unread_signals, semantic_chunks) = store.inbox_summary.with(|s| {
                s.as_ref()
                    .map(|x| (x.unread_signals, x.doc_chunks_count))
                    .unwrap_or((0, 0))
            });
            let last_sync = store
                .sync_status
                .with(|s| s.as_ref().map(|x| fmt_time_cn_for(Some(x.synced_at_ms), l)))
                .unwrap_or_else(|| l.t("未同步", "Never").to_owned());

            format!(
                "L0: {lake_companies} {} · L1: {unread_signals} {} / {semantic_chunks} {} · {} {last_sync}",
                l.t("家公司", "issuers"),
                l.t("未读信号", "unread"),
                l.t("语义片段", "chunks"),
                l.t("上次自选同步", "Last Circle sync"),
            )
        },
        obsidian_card().child(dynamic(move || {
            let l = store.lang.get();
            let syncing = store.sync_running.get();
            let catching = store.inbox_catching_up.get();
            let u_syncing = store.universe_syncing.get();
            let t_syncing = store.themes_syncing.get();

            action_strip()
                .child(setting_chip(
                    if syncing {
                        l.t("能力圈同步中…", "Syncing Circle…")
                    } else {
                        l.t("同步能力圈财报与股价 (⌘R)", "Sync Circle Financials (⌘R)")
                    },
                    syncing,
                    false,
                    None,
                    move || store.start_sync(),
                ))
                .child(setting_chip(
                    if catching {
                        l.t("追平事件流中…", "Catching Up L1…")
                    } else {
                        l.t("追平 L1 SEC 与新闻流", "Catch Up L1 Events")
                    },
                    catching,
                    false,
                    Some((SKY_FG, SKY_BG, SKY_BORDER)),
                    move || store.trigger_l1_catchup(),
                ))
                .child(setting_chip(
                    if u_syncing {
                        l.t("全美股 L0 同步中…", "Syncing US L0…")
                    } else {
                        l.t("同步全美股 L0 湖仓", "Sync US L0 Lakehouse")
                    },
                    u_syncing,
                    false,
                    Some((SKY_FG, SKY_BG, SKY_BORDER)),
                    move || store.trigger_universe_sync(),
                ))
                .child(setting_chip(
                    if t_syncing {
                        l.t("主题图谱生成中…", "Syncing Themes…")
                    } else {
                        l.t("同步商业原型主题图谱", "Sync Theme Graph")
                    },
                    t_syncing,
                    false,
                    None,
                    move || store.trigger_themes_sync(),
                ))
        })),
    )
}

fn section_block(
    tag: &'static str,
    title: impl Fn() -> String + 'static,
    subtitle: impl Fn() -> String + 'static,
    card_body: View,
) -> View {
    view()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            view()
                .w_full()
                .px(px(4.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    view()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(
                            text(title)
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_BASE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(tag)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED))
                                .number_of_lines(1),
                        ),
                )
                .child(
                    text(subtitle)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
        .child(card_body.w_full().flex().flex_col())
}

fn setting_row_reactive(
    label: impl Fn() -> String + 'static,
    hint: impl Fn() -> Option<String> + 'static,
    mono_hint: bool,
    has_divider: bool,
) -> View {
    let mut row = view()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(20.0));
    if has_divider {
        row = row.border_b_1().border_color(rgb(BORDER_HAIRLINE));
    }
    row.child(
        view()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                text(label)
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(INK_PRIMARY)),
            )
            .child(
                text(move || hint().unwrap_or_default())
                    .font_family(if mono_hint { FONT_MONO_TERM } else { FONT_UI })
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
            ),
    )
}

fn setting_row_static(label: &str, hint: Option<&str>, mono_hint: bool, has_divider: bool) -> View {
    let mut left = view()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(INK_PRIMARY)),
        );
    if let Some(h) = hint {
        let mut h_txt = text(h.to_owned())
            .font_family(if mono_hint { FONT_MONO_TERM } else { FONT_UI })
            .text_size(px(TEXT_XS))
            .text_color(rgb(INK_MUTED));
        if mono_hint {
            h_txt = h_txt.truncate().number_of_lines(1);
        }
        left = left.child(h_txt);
    }
    let mut row = view()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(left);
    if has_divider {
        row = row.border_b_1().border_color(rgb(BORDER_HAIRLINE));
    }
    row
}

fn action_strip() -> View {
    view()
        .w_full()
        .px(px(16.0))
        .py(px(12.0))
        .bg(rgb(BG_WELL))
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(px(8.0))
}

fn setting_chip(
    label: &str,
    active: bool,
    mono: bool,
    accent: Option<(u32, u32, u32)>,
    on_press: impl Fn() + 'static,
) -> View {
    let family = if mono { FONT_MONO_NUM } else { FONT_UI };
    let mut el = view()
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_sm()
        .cursor_pointer()
        .border_1()
        .on_press(on_press);

    let fg_col = if active {
        let (fg, bg, br) = accent.unwrap_or((EMERALD_FG, EMERALD_BG, EMERALD_BORDER));
        el = el.bg(rgba(bg)).border_color(rgba(br));
        fg
    } else {
        el = el
            .bg(rgb(BG_WELL))
            .border_color(rgb(BORDER_SUBTLE))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .active(|s| s.bg(rgb(BG_ACTIVE)));
        INK_SECONDARY
    };

    el.child(
        text(label.to_owned())
            .font_family(family)
            .text_size(px(TEXT_XS))
            .font_weight(if active {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(rgb(fg_col))
            .number_of_lines(1),
    )
}
