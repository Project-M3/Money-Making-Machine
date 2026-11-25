//! Guided First-Run Onboarding card for fresh workspaces (`apps/m3-nsh`).

use crate::components::{action_button, badge_pill, obsidian_card, section_header};
use crate::stores::AppStore;
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
