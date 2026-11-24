//! View modules for each M3 workspace tab and company overlay.
//!
//! Phase 1 scaffolds clean Obsidian skeleton views wired to [`crate::stores::AppStore`]
//! so Wave 3 parallel view workers can implement each view file independently.

pub mod agent;
pub mod baskets;
pub mod chart;
pub mod company;
pub mod inbox;
pub mod industry;
pub mod journal;
pub mod settings;
pub mod universe;

use crate::components::{action_button, badge_pill, obsidian_card};
use crate::stores::AppStore;
use crate::theme::*;
use nsh_core::prelude::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn skeleton_placeholder(
    store: AppStore,
    badge_text: &'static str,
    title_zh: &'static str,
    title_en: &'static str,
    subtitle_zh: &'static str,
    subtitle_en: &'static str,
    stat_line: impl Fn() -> String + 'static,
    primary_action_zh: &'static str,
    primary_action_en: &'static str,
    on_primary: impl Fn() + 'static,
) -> View {
    view()
        .flex_1()
        .min_h(px(0.0))
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(
            obsidian_card()
                .p(px(20.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(badge_pill(badge_text, SKY_FG, SKY_BG, SKY_BORDER))
                        .child(
                            text(stat_line)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(
                    text(move || store.lang.get().t(title_zh, title_en).to_owned())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_LG))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_HERO)),
                )
                .child(
                    text(move || store.lang.get().t(subtitle_zh, subtitle_en).to_owned())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_BASE))
                        .text_color(rgb(INK_SECONDARY)),
                )
                .child(view().flex().flex_row().gap(px(8.0)).child(action_button(
                    move || {
                        store
                            .lang
                            .get()
                            .t(primary_action_zh, primary_action_en)
                            .to_owned()
                    },
                    true,
                    on_primary,
                ))),
        )
}
