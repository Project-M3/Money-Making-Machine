//! Settings & Guardrails view (`⌘,`) organized into General, AI, and Data tabs.

use crate::components::{
    action_button, badge_pill, concept_hint, concept_hint_static, obsidian_card, progress_bar,
    seg_group, seg_tab, styled_input,
};
use crate::format::fmt_time_cn_for;
use crate::platform_desktop::{
    M3_GITHUB_URL, M3_RELEASES_URL, M3_WEBSITE_URL, copy_text_with_notice, open_external_url,
    prompt_choose_workspace, restart_after_update, reveal_workspace_dir,
};
use crate::stores::{AppStore, SettingsTab};
use crate::theme::*;
use m3_core::alan_connection::AlanConnectionProvider;
use m3_core::app_config;
use m3_core::budget::BudgetConfig;
use m3_core::i18n::UiLang;
use m3_core::storage::{self, StorageCategory};
use m3_core::update::{self, ReleaseChannel};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

const ALAN_SETUP_CMD: &str = "alan connection add openrouter --profile main && alan connection set-secret main && alan connection default set main";

/// Renders Preferences & Guardrails (`⌘,`) organized into General, AI, and Data tabs.
pub fn settings_view(store: AppStore) -> impl IntoElement {
    let build_commit = update::current_build_commit();
    let ver_text = if build_commit.is_empty() {
        format!("v{}", update::current_binary_version())
    } else {
        format!("v{} ({build_commit})", update::current_binary_version())
    };
    let ver_for_body = ver_text.clone();

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
                    .max_w(px(820.0))
                    .flex()
                    .flex_col()
                    .gap(px(20.0))
                    // Page header with category tabs
                    .child(
                        view()
                            .w_full()
                            .pb(px(14.0))
                            .border_b_1()
                            .border_color(rgb(BORDER_HAIRLINE))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .flex_wrap()
                            .gap(px(12.0))
                            .child(
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(10.0))
                                    .child(
                                        text(move || {
                                            store.lang.get().t("偏好设置", "Settings").to_owned()
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_LG))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO)),
                                    )
                                    .child(badge_pill(
                                        ver_text,
                                        INK_SECONDARY,
                                        SLATE_BG,
                                        SLATE_BORDER,
                                    )),
                            )
                            .child(dynamic(move || {
                                let l = store.lang.get();
                                let cur = store.settings_tab.get();
                                seg_group()
                                    .child(seg_tab(
                                        SettingsTab::General.label_for(l),
                                        cur == SettingsTab::General,
                                        None,
                                        false,
                                        move || store.set_settings_tab(SettingsTab::General),
                                    ))
                                    .child(seg_tab(
                                        SettingsTab::Ai.label_for(l),
                                        cur == SettingsTab::Ai,
                                        Some((VIOLET_FG, VIOLET_BG)),
                                        false,
                                        move || store.set_settings_tab(SettingsTab::Ai),
                                    ))
                                    .child(seg_tab(
                                        SettingsTab::Data.label_for(l),
                                        cur == SettingsTab::Data,
                                        Some((SKY_FG, SKY_BG)),
                                        false,
                                        move || store.set_settings_tab(SettingsTab::Data),
                                    ))
                            })),
                    )
                    // Active tab content
                    .child(dynamic(move || match store.settings_tab.get() {
                        SettingsTab::General => view()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(20.0))
                            .child(render_language_card(store))
                            .child(render_workspace_card(store))
                            .child(render_updates_card(store, &ver_for_body)),
                        SettingsTab::Ai => view()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(20.0))
                            .child(render_alan_connection_card(store))
                            .child(render_budget_card(store)),
                        SettingsTab::Data => view()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(20.0))
                            .child(render_datalake_card(store))
                            .child(render_storage_card(store)),
                    })),
            ),
    )
}

fn render_language_card(store: AppStore) -> View {
    section_block(
        move || {
            store
                .lang
                .get()
                .t("界面与语言", "Interface & Language")
                .to_owned()
        },
        None,
        None,
        obsidian_card().child(
            setting_row_reactive(
                move || {
                    store
                        .lang
                        .get()
                        .t("显示语言", "Display Language")
                        .to_owned()
                },
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
        move || store.lang.get().t("本地工作区", "Workspace").to_owned(),
        None,
        None,
        obsidian_card()
            .child(dynamic(move || {
                let l = store.lang.get();
                let workspace_path = store.paths.with(|p| p.repo_root.display().to_string());
                let has_custom = app_config::read_configured_workspace_root().is_some();
                let holdings_cnt = store.reports.with(|r| r.len());

                setting_row_static(
                    l.t("当前工作区目录", "Active Workspace Root"),
                    Some(&workspace_path),
                    None,
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
        move || {
            store
                .lang
                .get()
                .t("软件更新与通道", "Updates & Release Channel")
                .to_owned()
        },
        None,
        None,
        obsidian_card()
            .child(dynamic(move || {
                let l = store.lang.get();
                let ch = store.update_channel.get();
                let applied = store.update_applied_version.get();
                let applying = store.update_applying.get();
                let checking = store.update_checking.get();
                let notice = store.update_notice.get();
                let progress = store.update_progress.get();
                let prog_detail = store.update_progress_detail.get();

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
                } else if checking {
                    setting_chip(l.t("检查中…", "Checking…"), false, false, None, || {})
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
                        l.t("检查更新", "Check for Updates"),
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
                        None,
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
                    let frac = prog_detail.as_ref().map(|d| d.fraction).unwrap_or(0.15);
                    let pct_lbl = prog_detail
                        .as_ref()
                        .map(|d| format!("{}%", (d.fraction * 100.0).round() as u32))
                        .unwrap_or_default();
                    col = col.child(
                        view()
                            .px(px(16.0))
                            .py(px(10.0))
                            .bg(rgba(SKY_BG))
                            .border_b_1()
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
                                    .gap(px(8.0))
                                    .child(
                                        text(prog)
                                            .font_family(FONT_MONO_TERM)
                                            .text_size(px(TEXT_XS))
                                            .text_color(rgb(SKY_FG)),
                                    )
                                    .child(
                                        text(pct_lbl)
                                            .font_family(FONT_MONO_NUM)
                                            .text_size(px(TEXT_XS))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(SKY_FG)),
                                    ),
                            )
                            .child(progress_bar(frac, SKY_FG)),
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
        move || {
            store
                .lang
                .get()
                .t("AI 深研预算与护栏", "AI Budget & Guardrails")
                .to_owned()
        },
        Some(Box::new(move || {
            store
                .lang
                .get()
                .t(
                    "L0/L1 全程零 LLM；到达月度或单次硬上限后禁止启动 Alan。",
                    "L0/L1 stay Zero-LLM. Hard ceilings block Alan L2 runs before spending.",
                )
                .to_owned()
        })),
        None,
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
                    None,
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
                        view().flex_1().min_w(px(0.0)).child(
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
                    None,
                    None,
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
                    None,
                    Some(l.t(
                        "开启后仅在每周配额内于后台自动运行 L2 深研。",
                        "When enabled, background L2 runs stay capped by your weekly quota.",
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
                    None,
                    None,
                    false,
                    false,
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
            })),
    )
}

fn render_alan_connection_card(store: AppStore) -> View {
    section_block(
        move || {
            store
                .lang
                .get()
                .t("Alan 模型连接", "Alan Connection")
                .to_owned()
        },
        Some(Box::new(move || {
            store
                .lang
                .get()
                .t(
                    "凭据写入 Alan 标准连接库（与 `alan connection` CLI 互通），用于 Deep Research 与 Alan 审讯台。",
                    "Saved directly to Alan's canonical connection store (compatible with `alan connection` CLI) for Deep Research and Alan.",
                )
                .to_owned()
        })),
        None,
        obsidian_card()
            // 1. Active Status Badge Row
            .child(dynamic(move || {
                let l = store.lang.get();
                let status_opt = store.alan_conn_status.get();
                let active = status_opt.as_ref().and_then(|st| st.active_profile());

                let (hint, badge_el) = if let Some(prof) = active {
                    let prov_label = AlanConnectionProvider::parse(&prof.provider)
                        .map(|p| p.display_name())
                        .unwrap_or(prof.provider.as_str());
                    let model_label = prof.model.as_deref().unwrap_or("default");
                    let summary = format!("{} · {} · {}", prof.id, prov_label, model_label);
                    let mut badges = view()
                        .flex_shrink_0()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(badge_pill(
                            summary,
                            if prof.has_secret { EMERALD_FG } else { GOLD_FG },
                            if prof.has_secret { EMERALD_BG } else { GOLD_BG },
                            if prof.has_secret {
                                EMERALD_BORDER
                            } else {
                                GOLD_BORDER
                            },
                        ));
                    if let Some(masked) = prof.masked_hint.as_deref() {
                        badges = badges.child(badge_pill(
                            masked.to_owned(),
                            INK_SECONDARY,
                            SLATE_BG,
                            SLATE_BORDER,
                        ));
                    } else {
                        badges = badges.child(badge_pill(
                            l.t("缺少密钥", "Missing Key").to_owned(),
                            GOLD_FG,
                            GOLD_BG,
                            GOLD_BORDER,
                        ));
                    }
                    (prof.base_url.clone(), badges)
                } else {
                    (
                        None,
                        view().flex_shrink_0().child(badge_pill(
                            l.t("未配置", "Not Configured").to_owned(),
                            SLATE_FG,
                            SLATE_BG,
                            SLATE_BORDER,
                        )),
                    )
                };

                setting_row_static(
                    l.t("当前默认连接", "Active Connection"),
                    hint.as_deref(),
                    None,
                    true,
                    true,
                )
                .child(badge_el)
            }))
            // 2. Provider Segmented Toggle
            .child(dynamic(move || {
                let l = store.lang.get();
                let provider = store.alan_conn_provider.get();
                setting_row_static(l.t("模型服务商", "Provider"), None, None, false, true).child(
                    view().flex_shrink_0().child(
                        seg_group()
                            .child(seg_tab(
                                "OpenRouter",
                                provider == AlanConnectionProvider::OpenRouter,
                                Some((EMERALD_FG, EMERALD_BG)),
                                false,
                                move || {
                                    store.select_alan_conn_provider(
                                        AlanConnectionProvider::OpenRouter,
                                    );
                                },
                            ))
                            .child(seg_tab(
                                "OpenAI",
                                provider == AlanConnectionProvider::OpenAiResponses,
                                Some((SKY_FG, SKY_BG)),
                                false,
                                move || {
                                    store.select_alan_conn_provider(
                                        AlanConnectionProvider::OpenAiResponses,
                                    );
                                },
                            )),
                    ),
                )
            }))
            // 3. Profile ID & Model Inputs (static outside dynamic so IME & focus stay intact)
            .child(
                view()
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(12.0))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                text(move || {
                                    store
                                        .lang
                                        .get()
                                        .t("配置名 (Profile ID)", "Profile ID")
                                        .to_owned()
                                })
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                styled_input(
                                    move || store.alan_conn_profile_id_draft.get(),
                                    move || {
                                        store
                                            .alan_conn_provider
                                            .get()
                                            .default_profile_id()
                                            .to_owned()
                                    },
                                    true,
                                )
                                .w_full()
                                .on_change(move |val: &str| {
                                    store.alan_conn_profile_id_draft.set(val.to_owned());
                                }),
                            ),
                    )
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                text(move || {
                                    store.lang.get().t("默认模型 (Model)", "Model").to_owned()
                                })
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                styled_input(
                                    move || store.alan_conn_model_draft.get(),
                                    move || {
                                        store.alan_conn_provider.get().default_model().to_owned()
                                    },
                                    true,
                                )
                                .w_full()
                                .on_change(move |val: &str| {
                                    store.alan_conn_model_draft.set(val.to_owned());
                                }),
                            ),
                    ),
            )
            // 4. Masked API Key & Optional Base URL Inputs (static outside dynamic)
            .child(
                view()
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(12.0))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                text(move || {
                                    store
                                        .lang
                                        .get()
                                        .t("API 密钥 (掩码保护)", "API Key (Masked)")
                                        .to_owned()
                                })
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                styled_input(
                                    move || store.alan_conn_api_key_draft.get(),
                                    move || {
                                        let l = store.lang.get();
                                        let draft_id = store.alan_conn_profile_id_draft.get();
                                        let existing_hint = store.alan_conn_status.with(|st| {
                                            st.as_ref().and_then(|s| {
                                                s.profiles
                                                    .iter()
                                                    .find(|p| p.id == draft_id.trim())
                                                    .and_then(|p| p.masked_hint.clone())
                                            })
                                        });
                                        if let Some(hint) = existing_hint {
                                            match l {
                                                UiLang::Zh => format!("已保存 ({hint})，留空不变"),
                                                UiLang::En => {
                                                    format!("Saved ({hint}) — leave blank to keep")
                                                }
                                            }
                                        } else {
                                            l.t("输入 sk-... 密钥", "Enter API key (sk-...)")
                                                .to_owned()
                                        }
                                    },
                                    true,
                                )
                                .masked(true)
                                .w_full()
                                .on_change(move |val: &str| {
                                    store.alan_conn_api_key_draft.set(val.to_owned());
                                })
                                .on_submit(move |_: &str| store.save_alan_connection()),
                            ),
                    )
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                text(move || {
                                    store
                                        .lang
                                        .get()
                                        .t("自定义端点 (可选)", "Base URL (Optional)")
                                        .to_owned()
                                })
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                styled_input(
                                    move || store.alan_conn_base_url_draft.get(),
                                    move || {
                                        store.alan_conn_provider.get().default_base_url().to_owned()
                                    },
                                    true,
                                )
                                .w_full()
                                .on_change(move |val: &str| {
                                    store.alan_conn_base_url_draft.set(val.to_owned());
                                })
                                .on_submit(move |_: &str| store.save_alan_connection()),
                            ),
                    ),
            )
            // 5. Save & Test Actions + Inline Feedback
            .child(dynamic(move || {
                let l = store.lang.get();
                let saving = store.alan_conn_saving.get();
                let testing = store.alan_conn_testing.get();
                let feedback = store.alan_conn_feedback.get();

                let mut row = view()
                    .w_full()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
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
                            .gap(px(8.0))
                            .child(action_button(
                                if saving {
                                    l.t("保存中…", "Saving…").to_owned()
                                } else {
                                    l.t("保存并设为默认", "Save & Set Default").to_owned()
                                },
                                true,
                                move || store.save_alan_connection(),
                            ))
                            .child(setting_chip(
                                if testing {
                                    l.t("测试中…", "Testing…")
                                } else {
                                    l.t("测试连接", "Test Connection")
                                },
                                testing,
                                false,
                                Some((SKY_FG, SKY_BG, SKY_BORDER)),
                                move || store.test_alan_connection(),
                            )),
                    );

                if let Some((msg, is_err)) = feedback {
                    row = row.child(
                        text(msg)
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(if is_err { CORAL_FG } else { EMERALD_FG })),
                    );
                }

                row
            }))
            // 6. Subtle CLI Fallback Strip
            .child(dynamic(move || {
                let l = store.lang.get();
                action_strip()
                    .justify_between()
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                text(l.t("CLI 备选:", "CLI fallback:").to_owned())
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                text(ALAN_SETUP_CMD.to_owned())
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_2XS))
                                    .text_color(rgb(INK_MUTED))
                                    .truncate()
                                    .number_of_lines(1),
                            ),
                    )
                    .child(setting_chip(
                        l.t("复制命令", "Copy CLI"),
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
        move || {
            store
                .lang
                .get()
                .t("数据湖与日常维护", "Data Lake & Maintenance")
                .to_owned()
        },
        None,
        Some(Box::new(move || {
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
        })),
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

fn storage_category_meta(
    category: StorageCategory,
    l: UiLang,
) -> (&'static str, u32, Option<&'static str>) {
    match category {
        StorageCategory::SecBulkArchives => (
            l.t("SEC 批量归档", "SEC Bulk Archives"),
            SKY_FG,
            Some(l.t(
                "全美股 XBRL / Submissions 压缩包；清理后不影响现有 DuckDB，下次同步重新下载。",
                "Bulk SEC XBRL & submissions archives. Cleaning keeps existing DuckDB tables; re-downloaded on next sync.",
            )),
        ),
        StorageCategory::SecFilingsCache => (
            l.t("10-K / 8-K 原文缓存", "SEC Filings Cache"),
            VIOLET_FG,
            Some(l.t(
                "本地缓存的财报正文文件，可随时按需重新拉取。",
                "Cached raw 10-K/8-K filing files; re-fetched on demand.",
            )),
        ),
        StorageCategory::DuckDbLakehouse => (
            l.t("DuckDB 湖仓与索引", "DuckDB & Parquet"),
            EMERALD_FG,
            Some(l.t(
                "全市场 L0 榜单与 L1 语义索引；清理后需重新同步全美股 L0。",
                "Local L0/L1 DuckDB & Parquet index; rebuildable via US L0 sync.",
            )),
        ),
        StorageCategory::QuotesAndCache => (
            l.t("行情与计算缓存", "Quotes & Cache"),
            GOLD_FG,
            None,
        ),
        StorageCategory::PersonalWorkbench => (
            l.t("个人工作台", "Personal Workbench"),
            INK_SECONDARY,
            Some(l.t(
                "能力圈、决策账本、研报与行业笔记，受保护不会被清理。",
                "Your Circle, Decision Journal, Dossiers, and Industry notes. Protected from cleanup.",
            )),
        ),
    }
}

fn render_storage_card(store: AppStore) -> View {
    section_block(
        move || store.lang.get().t("本地存储", "Storage").to_owned(),
        None,
        None,
        obsidian_card().child(dynamic(move || {
            let l = store.lang.get();
            let scanning = store.storage_scanning.get();
            let cleaning = store.storage_cleaning.get();
            let report_opt = store.storage_report.get();
            let repo_root = store.paths.with(|p| p.repo_root.clone());

            let total_txt = report_opt
                .as_ref()
                .map(|r| storage::format_bytes(r.total_bytes))
                .unwrap_or_else(|| "—".to_owned());
            let purgeable_txt = report_opt
                .as_ref()
                .map(|r| storage::format_bytes(r.cleanable_bytes))
                .unwrap_or_else(|| "0 B".to_owned());

            let mut segs: Vec<View> = Vec::new();
            if let Some(ref rep) = report_opt
                && rep.total_bytes > 0
            {
                for cat in &rep.categories {
                    if cat.bytes == 0 {
                        continue;
                    }
                    let (_, col, _) = storage_category_meta(cat.category, l);
                    segs.push(
                        view()
                            .w(px(0.0))
                            .flex_grow((cat.bytes as f32).max(1.0))
                            .min_w(px(4.0))
                            .h_full()
                            .bg(rgb(col)),
                    );
                }
            }

            let top_block = view()
                .w_full()
                .px(px(16.0))
                .py(px(14.0))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
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
                        .gap(px(12.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_baseline()
                                .gap(px(10.0))
                                .child(
                                    text(total_txt)
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_MD))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(INK_HERO)),
                                )
                                .child(
                                    text(l.t("已占用", "used").to_owned())
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                )
                                .child(
                                    text(format!(
                                        "· {} {purgeable_txt}",
                                        l.t("可清理", "purgeable")
                                    ))
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                                ),
                        )
                        .child(setting_chip(
                            if scanning {
                                l.t("计算中…", "Scanning…")
                            } else {
                                l.t("重新计算", "Recalculate")
                            },
                            scanning,
                            false,
                            None,
                            move || store.refresh_storage_report(),
                        )),
                )
                .child(
                    view()
                        .w_full()
                        .h(px(10.0))
                        .rounded_full()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .overflow_hidden()
                        .flex()
                        .flex_row()
                        .gap(px(1.5))
                        .children(segs),
                );

            let Some(rep) = report_opt else {
                return view()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(top_block)
                    .into_element();
            };

            let len = rep.categories.len();
            let rows = rep.categories.into_iter().enumerate().map(|(idx, cat)| {
                let (title, dot_col, tip_opt) = storage_category_meta(cat.category, l);
                let has_divider = idx + 1 < len;
                let reveal_target = if cat.reveal_path.exists() {
                    cat.reveal_path.clone()
                } else {
                    repo_root.clone()
                };
                let is_cleaning_this = cleaning == Some(cat.category);

                let mut label_row = view()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(view().w(px(8.0)).h(px(8.0)).rounded_full().bg(rgb(dot_col)))
                    .child(
                        text(title.to_owned())
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(INK_PRIMARY)),
                    );
                if let Some(tip) = tip_opt {
                    label_row = label_row.child(concept_hint_static(tip));
                }

                let action_el = if cat.cleanable {
                    let cat_kind = cat.category;
                    if cat.bytes > 0 || is_cleaning_this {
                        setting_chip(
                            if is_cleaning_this {
                                l.t("清理中…", "Cleaning…")
                            } else {
                                l.t("清理", "Clean")
                            },
                            is_cleaning_this,
                            false,
                            Some((CORAL_FG, CORAL_BG, CORAL_BORDER)),
                            move || store.clean_storage_category(cat_kind),
                        )
                    } else {
                        view().w(px(48.0))
                    }
                } else {
                    badge_pill(
                        l.t("受保护", "Protected").to_owned(),
                        INK_MUTED,
                        SLATE_BG,
                        SLATE_BORDER,
                    )
                };

                let mut row = view()
                    .w_full()
                    .px(px(16.0))
                    .py(px(10.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .child(label_row)
                    .child(
                        view()
                            .flex_shrink_0()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                text(format!("{} {}", cat.file_count, l.t("文件", "files")))
                                    .font_family(FONT_MONO_TERM)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(INK_MUTED)),
                            )
                            .child(
                                text(storage::format_bytes(cat.bytes))
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_SM))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(INK_PRIMARY)),
                            )
                            .child(setting_chip("Finder ↗", false, false, None, move || {
                                reveal_workspace_dir(&reveal_target);
                            }))
                            .child(action_el),
                    );
                if has_divider {
                    row = row.border_b_1().border_color(rgb(BORDER_HAIRLINE));
                }
                row
            });

            view()
                .w_full()
                .flex()
                .flex_col()
                .child(top_block)
                .children(rows)
                .into_element()
        })),
    )
}

fn section_block(
    title: impl Fn() -> String + 'static,
    concept_tip: Option<Box<dyn Fn() -> String>>,
    live_meta: Option<Box<dyn Fn() -> String>>,
    card_body: View,
) -> View {
    let mut heading_row = view()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            text(title)
                .font_family(FONT_UI)
                .text_size(px(TEXT_BASE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        );
    if let Some(tip_fn) = concept_tip {
        heading_row = heading_row.child(concept_hint(tip_fn));
    }

    let mut header = view()
        .w_full()
        .px(px(4.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(heading_row);
    if let Some(meta_fn) = live_meta {
        header = header.child(
            text(meta_fn)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED)),
        );
    }

    view()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(header)
        .child(card_body.w_full().flex().flex_col())
}

fn setting_row_reactive(label: impl Fn() -> String + 'static, has_divider: bool) -> View {
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
        view().flex_1().min_w(px(0.0)).child(
            text(label)
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(INK_PRIMARY)),
        ),
    )
}

fn setting_row_static(
    label: &str,
    hint: Option<&str>,
    concept_tip: Option<&str>,
    mono_hint: bool,
    has_divider: bool,
) -> View {
    let mut title_row = view().flex().flex_row().items_center().gap(px(6.0)).child(
        text(label.to_owned())
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(INK_PRIMARY)),
    );
    if let Some(tip) = concept_tip {
        title_row = title_row.child(concept_hint_static(tip));
    }

    let mut left = view()
        .flex_1()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(title_row);
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
