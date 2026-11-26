//! Guided First-Run Onboarding card for fresh workspaces (`apps/m3-nsh`).

use crate::components::{
    action_button, animate_shell_spring, badge_pill, obsidian_card, pixel_m3_mark, section_header,
};
use crate::stores::{AppStore, OnboardingStep};
use crate::theme::*;
use m3_core::i18n::UiLang;
use m3_core::lake::UniverseSummary;
use m3_core::model::Bucket;
use m3_core::starter::starter_pack_entries;
use nsh_core::prelude::*;

/// Returns `true` when the active workspace is fresh and onboarding has not been dismissed.
pub(crate) fn is_fresh_workspace_for_onboarding(
    watchlist_empty: bool,
    universe_summary: Option<&UniverseSummary>,
    dismissed: bool,
) -> bool {
    !dismissed && watchlist_empty && universe_summary.is_none_or(|s| s.total_companies == 0)
}

/// Formats the curated starter pack bucket summary for UI display and tests.
pub(crate) fn starter_pack_summary_line(lang: UiLang) -> String {
    let entries = starter_pack_entries();
    let bucket_list = |target: Bucket| -> String {
        entries
            .iter()
            .filter(|e| e.bucket == target)
            .map(|e| e.ticker.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let core = bucket_list(Bucket::Core);
    let watch = bucket_list(Bucket::Watch);
    let too_hard = bucket_list(Bucket::TooHard);

    match lang {
        UiLang::Zh => format!("Core: {core}  ·  Watch: {watch}  ·  Too Hard: {too_hard}"),
        UiLang::En => format!("Core: {core}  ·  Watch: {watch}  ·  Too Hard: {too_hard}"),
    }
}

/// Renders the calm, uncluttered Obsidian First-Run onboarding card at the top of
/// Inbox / Circle views when the workspace is fresh.
pub fn onboarding_card(store: AppStore) -> View {
    let l = store.lang.get();
    let applying_starter = store.onboarding_applying.get() || store.sync_running.get();
    let syncing_universe = store.universe_syncing.get();
    let summary_line = starter_pack_summary_line(l);

    obsidian_card()
        .p(px(22.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(section_header(
            l.t(
                "首次启动引导 · 搭建你的个人能力圈",
                "Welcome to M3 · Start Your Circle of Competence",
            ),
            l.t("BERKSHIRE × MUNGER 经典包", "FIRST-RUN SETUP").to_owned(),
        ))
        .child(
            text(
                l.t(
                    "当前工作区为空（无自选标的，也尚未同步全美股 L0 湖仓）。你可以一键载入 8 家伯克希尔 × 芒格非金融经典公司立即体验 K 线、ROIC、Owner Earnings 与击球价，也可以直接在后台同步 6,000+ 家全美股 L0 确定性湖仓。",
                    "This workspace is fresh (no watchlist holdings and no US L0 lakehouse synced yet). Load 8 curated Berkshire × Munger non-financial classics to populate K-lines, ROIC, Owner Earnings, and Strike Zones in ~5s, or sync the full 6,000+ US issuer lakehouse in the background.",
                )
                .to_owned(),
            )
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .line_height(px(20.0))
            .text_color(rgb(INK_SECONDARY)),
        )
        .child(
            view()
                .px(px(12.0))
                .py(px(10.0))
                .rounded_md()
                .bg(rgb(BG_WELL))
                .border_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(badge_pill(
                            l.t("8 家精选示例", "8 Curated Classics"),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                        ))
                        .child(
                            text(
                                l.t(
                                    "伯克希尔 × 芒格非金融经典能力圈（不覆盖已有用户条目）",
                                    "Berkshire × Munger Non-Financial Classics (non-destructive)",
                                )
                                .to_owned(),
                            )
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .text_color(rgb(INK_PRIMARY)),
                        ),
                )
                .child(
                    text(summary_line)
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
        .child(
            view()
                .pt(px(2.0))
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(10.0))
                .child(action_button(
                    if applying_starter {
                        l.t("正在装载并同步 8 家经典标的…", "Loading & Syncing 8 Classics…")
                            .to_owned()
                    } else {
                        l.t(
                            "快速开局：8 家经典标的 + 后台渐进 L0 (~5s 可用)",
                            "Quick Start: 8 Classics + Progressive L0 (~5s ready)",
                        )
                        .to_owned()
                    },
                    true,
                    move || store.start_starter_pack_and_progressive_l0(),
                ))
                .child(action_button(
                    if syncing_universe {
                        l.t(
                            "全美股 L0 正在后台同步中…",
                            "Syncing Full-US L0 Lakehouse…",
                        )
                        .to_owned()
                    } else {
                        l.t(
                            "仅后台同步全美股 L0 (6,000+ 家)",
                            "Sync Full-US L0 Only (6,000+ issuers)",
                        )
                        .to_owned()
                    },
                    false,
                    move || store.start_universe_sync(false),
                ))
                .child(action_button(
                    l.t("保持空白 / 暂不提示", "Start Blank / Dismiss"),
                    false,
                    move || store.dismiss_onboarding(),
                )),
        )
}

fn concept_card(
    badge: &'static str,
    fg: u32,
    bg: u32,
    border: u32,
    title: &'static str,
    body: &'static str,
) -> View {
    obsidian_card()
        .flex_1()
        .min_w(px(280.0))
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
                    text(title.to_owned())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(badge_pill(badge, fg, bg, border)),
        )
        .child(
            text(body.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .line_height(px(18.5))
                .text_color(rgb(INK_SECONDARY)),
        )
}

fn render_step_concepts(store: AppStore, l: UiLang) -> View {
    view()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap(px(12.0))
                .child(
                    view()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            text(l.t(
                                "机器读完 6,000 家财报，你只挥击甜区好球",
                                "Machines digest 6,000 filings; you only swing at fat pitches",
                            ))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_MD))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(l.t(
                                "花 1 分钟了解 M3 的核心心智模型，或随时点右上角跳过。",
                                "Take a minute to see how M3 separates deterministic math from AI critique—or skip anytime.",
                            ))
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(INK_SECONDARY)),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(action_button("中文", l == UiLang::Zh, move || {
                            store.set_language(UiLang::Zh);
                        }))
                        .child(action_button("English", l == UiLang::En, move || {
                            store.set_language(UiLang::En);
                        })),
                ),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .gap(px(12.0))
                .child(concept_card(
                    "WATCH → IN",
                    EMERALD_FG,
                    EMERALD_BG,
                    EMERALD_BORDER,
                    l.t("1. 能力圈四分桶", "1. Circle of Competence"),
                    l.t(
                        "全市场初筛后先放 WATCH 跟踪研究，读懂并确认后晋升 IN（只有 IN 才触发正式击球区提醒），不懂则果断归 OUT / TOO HARD。",
                        "Start candidates in WATCH while researching; promote to IN once understood (only IN triggers Strike Zone alerts), or mark OUT / TOO HARD.",
                    ),
                ))
                .child(concept_card(
                    "0 TOKEN MATH",
                    SKY_FG,
                    SKY_BG,
                    SKY_BORDER,
                    l.t("2. 本地确定性计算", "2. Deterministic Local Math"),
                    l.t(
                        "全美股 SEC 财报底表、5 年 ROIC、严格 Owner Earnings 与参考估值全部在本地离线算出，可溯源、无幻觉。",
                        "6,000+ US SEC filings, 5Y ROIC, Owner Earnings, and valuation math run deterministically offline in Rust.",
                    ),
                )),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .gap(px(12.0))
                .child(concept_card(
                    "PROACTIVE ALAN",
                    GOLD_FG,
                    GOLD_BG,
                    GOLD_BORDER,
                    l.t("3. 行业研究与主动简报", "3. Industry Studio & Calm Briefings"),
                    l.t(
                        "在「行业研究」吃透行业护城河；「简报收件箱」只替你盯防能力圈新财报与证伪风险，无事发生则保持静默。",
                        "Study industry economics in Industry Studio; Briefing Inbox watches Circle filings calmly and stays quiet when nothing breaks.",
                    ),
                ))
                .child(concept_card(
                    "SOVEREIGNTY",
                    VIOLET_FG,
                    VIOLET_BG,
                    VIOLET_BORDER,
                    l.t("4. 费曼答辩与决策账本", "4. Feynman Coach & Decision Ledger"),
                    l.t(
                        "3 轮苏格拉底答辩磨出反驳红线；出手前在「决策账本」事前冻结依据、证伪条件与模拟持仓，长期跑赢 S&P 500 才算数。",
                        "3 Socratic rounds sharpen your kill criteria; freeze theses and paper trades in Decision Ledger to benchmark against Cash & S&P 500.",
                    ),
                )),
        )
}

fn render_step_workspace(store: AppStore, l: UiLang) -> View {
    view()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(
            obsidian_card()
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
                        .flex_wrap()
                        .gap(px(10.0))
                        .child(
                            text(l.t("界面语言", "Interface Language"))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(action_button("中文", l == UiLang::Zh, move || {
                                    store.set_language(UiLang::Zh);
                                }))
                                .child(action_button("English", l == UiLang::En, move || {
                                    store.set_language(UiLang::En);
                                })),
                        ),
                )
                .child(
                    text(l.t(
                        "所有持仓分桶、Dossier 判断、活研备忘与决策账本均以纯本地 JSON / Markdown / DuckDB 保存，可随时直接备份或用 Git 管理。",
                        "Your Circle buckets, dossiers, memos, and decision journal live transparently on disk as JSON, Markdown, and DuckDB.",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(18.0))
                    .text_color(rgb(INK_SECONDARY)),
                ),
        )
        .child(crate::views::settings::render_workspace_card(store))
}

fn render_step_alan(store: AppStore, l: UiLang) -> View {
    view()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .child(
            obsidian_card()
                .p(px(14.0))
                .border_color(rgba(SKY_BORDER))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap(px(10.0))
                .child(
                    text(l.t(
                        "跳过连接也能 100% 免费使用全美股雷达、财报底表与估值沙盒；连接 Alan 后解锁 4 视角深研、费曼答辩与主动财报巡检。",
                        "Zero-LLM mode still gives you full SEC screener, financial tables & valuation sandbox. Connect Alan anytime for Deep Research, Feynman Coach, and L1.5 Watchdog.",
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(18.0))
                    .text_color(rgb(INK_PRIMARY)),
                ),
        )
        .child(crate::views::settings::render_alan_connection_card(store))
}

fn render_step_starter_data(store: AppStore, l: UiLang) -> View {
    let summary_line = starter_pack_summary_line(l);
    let ws_path = store.paths.with(|p| p.repo_root.display().to_string());
    let alan_ready = store
        .alan_conn_status
        .with(|opt| opt.as_ref().is_some_and(|s| !s.profiles.is_empty()));
    let alan_label = if alan_ready {
        l.t(
            "Alan 已连接 · 主动巡检已就绪",
            "Alan Connected · Proactive Ready",
        )
    } else {
        l.t(
            "纯确定性 0-Token 模式（稍后可在 设置 → AI 连接）",
            "0-Token Deterministic Mode (configure Alan later in Settings → AI)",
        )
    };
    let applying_starter = store.onboarding_applying.get() || store.sync_running.get();
    let syncing_universe = store.universe_syncing.get();

    view()
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(
            obsidian_card()
                .p(px(16.0))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    text(l.t("当前基础配置摘要", "Setup Readiness Summary"))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(
                    text(format!(
                        "{}: {}",
                        l.t("界面语言", "Language"),
                        match l {
                            UiLang::Zh => "简体中文",
                            UiLang::En => "English",
                        }
                    ))
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_SECONDARY)),
                )
                .child(
                    text(format!("{}: {ws_path}", l.t("工作区目录", "Workspace")))
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_SECONDARY))
                        .truncate()
                        .number_of_lines(1),
                )
                .child(
                    text(format!("{}: {alan_label}", l.t("Alan 模型", "Alan")))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(if alan_ready { EMERALD_FG } else { GOLD_FG })),
                ),
        )
        .child(
            obsidian_card()
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
                        .gap(px(8.0))
                        .child(
                            text(l.t("选择初始化方式", "Choose How to Initialize Data"))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(badge_pill(
                            l.t("8 家伯克希尔 × 芒格示例", "8 Curated Classics"),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                        )),
                )
                .child(
                    text(summary_line)
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
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
                            if applying_starter {
                                l.t("正在装载并同步 8 家经典标的…", "Loading 8 Classics…")
                                    .to_owned()
                            } else {
                                l.t(
                                    "快速开局：装载 8 家经典标的 + 后台渐进 L0 (~5s)",
                                    "Quick Start: 8 Classics + Progressive L0 (~5s)",
                                )
                                .to_owned()
                            },
                            true,
                            move || {
                                store.start_starter_pack_and_progressive_l0();
                                store.complete_landing_onboarding();
                            },
                        ))
                        .child(action_button(
                            if syncing_universe {
                                l.t("全美股 L0 同步中…", "Syncing Full-US L0…").to_owned()
                            } else {
                                l.t(
                                    "仅后台同步 6,000+ 全美股 L0",
                                    "Sync 6,000+ US L0 Lakehouse Only",
                                )
                                .to_owned()
                            },
                            false,
                            move || {
                                store.start_universe_sync(false);
                                store.complete_landing_onboarding();
                            },
                        ))
                        .child(action_button(
                            l.t("暂不同步，先逛一逛工作台", "Explore Blank Workbench"),
                            false,
                            move || store.complete_landing_onboarding(),
                        )),
                ),
        )
}

/// Renders the centered Obsidian First-Run Landing & Setup Wizard overlay (`onboarding_completed`).
pub fn landing_onboarding_overlay(store: AppStore) -> impl IntoElement {
    overlay()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(px(28.0))
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(rgba(SCRIM_BACKDROP))
                .on_press(move || {
                    animate_shell_spring();
                    store.complete_landing_onboarding();
                }),
        )
        .child(dynamic(move || {
            let l = store.lang.get();
            let step = store.landing_onboarding_step.get();
            let step_idx = step.index();

            let header = view()
                .flex_shrink_0()
                .px(px(22.0))
                .py(px(14.0))
                .bg(rgb(BG_TOOLBAR))
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
                        .gap(px(10.0))
                        .child(pixel_m3_mark(1.4))
                        .child(
                            text("Project M3")
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_LG))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child(badge_pill(
                            l.t("首次使用向导", "GETTING STARTED"),
                            EMERALD_FG,
                            EMERALD_BG,
                            EMERALD_BORDER,
                        )),
                )
                .child(action_button(
                    l.t("跳过，稍后在设置中配置", "Skip for now"),
                    false,
                    move || {
                        animate_shell_spring();
                        store.complete_landing_onboarding();
                    },
                ));

            let stepper = view()
                .flex_shrink_0()
                .px(px(22.0))
                .py(px(10.0))
                .bg(rgb(BG_ROW_ALT))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(8.0))
                .children(OnboardingStep::ALL.into_iter().map(|s| {
                    let active = s == step;
                    let completed = s.index() < step_idx;
                    let mut pill = view()
                        .px(px(10.0))
                        .py(px(5.0))
                        .rounded_md()
                        .border_1()
                        .cursor_pointer()
                        .on_press(move || store.set_landing_onboarding_step(s))
                        .child(
                            text(format!("{}. {}", s.index() + 1, s.label_for(l)))
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .font_weight(if active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(rgb(if active {
                                    EMERALD_FG
                                } else if completed {
                                    INK_PRIMARY
                                } else {
                                    INK_MUTED
                                })),
                        );
                    if active {
                        pill = pill.bg(rgba(EMERALD_BG)).border_color(rgba(EMERALD_BORDER));
                    } else if completed {
                        pill = pill.bg(rgb(BG_SURFACE)).border_color(rgb(BORDER_SUBTLE));
                    } else {
                        pill = pill.border_color(rgb(BORDER_HAIRLINE));
                    }
                    pill
                }));

            let body = match step {
                OnboardingStep::WelcomeConcepts => render_step_concepts(store, l),
                OnboardingStep::LanguageWorkspace => render_step_workspace(store, l),
                OnboardingStep::ConnectAlan => render_step_alan(store, l),
                OnboardingStep::StarterData => render_step_starter_data(store, l),
            };

            let mut right_actions = view().flex().flex_row().items_center().gap(px(8.0));
            if let Some(next_step) = step.next() {
                if step == OnboardingStep::ConnectAlan {
                    right_actions = right_actions.child(action_button(
                        l.t("暂不配置，继续 →", "Skip Alan, Continue →"),
                        false,
                        move || store.set_landing_onboarding_step(next_step),
                    ));
                }
                right_actions = right_actions.child(action_button(
                    l.t("下一步 →", "Next →"),
                    true,
                    move || store.set_landing_onboarding_step(next_step),
                ));
            } else {
                right_actions = right_actions.child(action_button(
                    l.t("完成并进入工作台", "Enter Workbench"),
                    true,
                    move || {
                        animate_shell_spring();
                        store.complete_landing_onboarding();
                    },
                ));
            }

            let mut left_actions = view().flex().flex_row().items_center().gap(px(10.0));
            if let Some(prev_step) = step.prev() {
                left_actions = left_actions.child(action_button(
                    l.t("← 上一步", "← Back"),
                    false,
                    move || store.set_landing_onboarding_step(prev_step),
                ));
            }
            left_actions = left_actions.child(
                text(format!(
                    "{} {} / {}",
                    l.t("步骤", "Step"),
                    step_idx + 1,
                    OnboardingStep::ALL.len()
                ))
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED)),
            );

            let footer = view()
                .flex_shrink_0()
                .px(px(22.0))
                .py(px(12.0))
                .bg(rgb(BG_TOOLBAR))
                .border_t_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(left_actions)
                .child(right_actions);

            view()
                .w(px(860.0))
                .max_w(percentage(92.0))
                .h(px(640.0))
                .max_h(percentage(88.0))
                .rounded_xl()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgba(BORDER_GLASS_STRONG))
                .shadow_xl()
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(header)
                .child(stepper)
                .child(
                    scroll()
                        .flex_1()
                        .min_h(px(0.0))
                        .w_full()
                        .child(view().w_full().p(px(22.0)).child(body)),
                )
                .child(footer)
        }))
}
