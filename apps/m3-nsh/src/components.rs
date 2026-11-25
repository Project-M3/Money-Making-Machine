//! Shared Obsidian UI primitives built on native NSH 0.7.0 elements.

use crate::stores::AppStore;
use crate::theme::*;
use m3_core::i18n::UiLang;
use m3_core::model::{AnnualMetricRow, WarningFlag, WarningSeverity};
use nsh_core::element::{LayoutAnimation, LayoutCurve, LayoutTransition};
use nsh_core::prelude::*;
use rust_decimal::Decimal;

/// Pixel-art M3 monogram mark matching the dark app icon.
pub fn pixel_m3_mark(cell_px: f32) -> View {
    view()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .children(PIXEL_M3.iter().map(|row| {
            let mut runs: Vec<(bool, usize)> = Vec::new();
            for ch in row.chars() {
                let on = ch == '#';
                if let Some((last_on, count)) = runs.last_mut()
                    && *last_on == on
                {
                    *count += 1;
                } else {
                    runs.push((on, 1));
                }
            }
            view()
                .flex()
                .flex_row()
                .h(px(cell_px))
                .children(runs.into_iter().map(|(on, count)| {
                    let seg = view().w(px(count as f32 * cell_px)).h(px(cell_px));
                    if on { seg.bg(rgb(INK_HERO)) } else { seg }
                }))
        }))
}

/// Obsidian card container (`BG_SURFACE`, `BORDER_SUBTLE`, rounded 8px).
pub fn obsidian_card() -> View {
    view()
        .rounded_lg()
        .bg(rgb(BG_SURFACE))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .overflow_hidden()
}

/// Segmented control container.
pub fn seg_group() -> View {
    view()
        .p(px(2.0))
        .rounded_md()
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
}

/// Static segmented tab button.
pub fn seg_tab<M>(
    label: &str,
    active: bool,
    accent_bg: Option<(u32, u32)>,
    mono: bool,
    on_press: impl IntoListener<PressEvent, M>,
) -> View {
    let family = if mono { FONT_MONO_TERM } else { FONT_UI };
    let (bg_col, fg_col) = match (active, accent_bg) {
        (true, Some((fg, bg))) => (rgba(bg), fg),
        (true, None) => (rgb(BG_HOVER), INK_HERO),
        (false, _) => (rgba(0x00000000), INK_MUTED),
    };
    let mut el = view()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_sm()
        .cursor_pointer()
        .bg(bg_col)
        .on_press(on_press)
        .child(
            text(label.to_owned())
                .font_family(family)
                .text_size(px(TEXT_SM))
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(rgb(fg_col)),
        );
    if !active {
        el = el.hover(|s| s.bg(rgb(BG_HOVER)));
    }
    el
}

/// Configures an NSH spring layout transition for the next UI commit (sidebar, popovers, Command Box).
pub fn animate_shell_spring() {
    animate_layout(LayoutAnimation {
        create: Some(LayoutTransition::spring(
            Spring::new(220.0).damping_ratio(0.88),
        )),
        update: Some(LayoutTransition::spring(
            Spring::new(250.0).damping_ratio(0.86),
        )),
        delete: Some(LayoutTransition::new(LayoutCurve::EaseOut, 140.0)),
    });
}

/// Keyboard shortcut pill (`⌘K`, `⌘1`, etc.).
pub fn kbd(label: impl IntoReactive<SharedString>) -> View {
    view()
        .px(px(6.0))
        .py(px(2.0))
        .rounded_xs()
        .bg(rgb(0x161620))
        .border_1()
        .border_color(rgb(BORDER_PILL))
        .child(
            text(label)
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED)),
        )
}

fn floating_kbd(label: &'static str) -> View {
    view()
        .px(px(5.0))
        .py(px(1.0))
        .rounded_xs()
        .bg(rgb(0x14141E))
        .border_1()
        .border_color(rgba(BORDER_GLASS_STRONG))
        .shadow_sm()
        .child(
            text(label)
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_2XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_PRIMARY)),
        )
}

/// Zero-layout-shift inline shortcut pill hidden via opacity until `⌘` (Command) is held.
pub fn cmd_kbd(store: AppStore, label: &'static str) -> View {
    kbd(label)
        .opacity(move || {
            if store.cmd_held.get() {
                1.0_f32
            } else {
                0.0_f32
            }
        })
        .pointer_events(PointerEvents::None)
}

/// Zero-layout-shift keycap badge floating on the top-right corner of a `.relative()` button.
pub fn cmd_kbd_corner(store: AppStore, label: &'static str) -> View {
    view()
        .absolute()
        .top(px(-7.0))
        .right(px(-5.0))
        .pointer_events(PointerEvents::None)
        .opacity(move || {
            if store.cmd_held.get() {
                1.0_f32
            } else {
                0.0_f32
            }
        })
        .child(floating_kbd(label))
}

/// Zero-layout-shift keycap badge floating on the bottom-right corner of topbar icon buttons.
pub fn cmd_kbd_bottom_corner(store: AppStore, label: &'static str) -> View {
    view()
        .absolute()
        .bottom(px(-7.0))
        .right(px(-4.0))
        .pointer_events(PointerEvents::None)
        .opacity(move || {
            if store.cmd_held.get() {
                1.0_f32
            } else {
                0.0_f32
            }
        })
        .child(floating_kbd(label))
}

/// Zero-layout-shift keycap badge floating vertically centered at the right edge of a `.relative()` row.
pub fn cmd_kbd_right_overlay(store: AppStore, label: &'static str) -> View {
    view()
        .absolute()
        .right(px(8.0))
        .top(px(0.0))
        .bottom(px(0.0))
        .pointer_events(PointerEvents::None)
        .flex()
        .items_center()
        .opacity(move || {
            if store.cmd_held.get() {
                1.0_f32
            } else {
                0.0_f32
            }
        })
        .child(floating_kbd(label))
}

/// Semantic pill badge.
pub fn badge_pill(label: impl IntoReactive<SharedString>, fg: u32, bg: u32, border: u32) -> View {
    view()
        .px(px(8.0))
        .py(px(2.0))
        .rounded_sm()
        .border_1()
        .border_color(rgba(border))
        .bg(rgba(bg))
        .child(
            text(label)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(fg))
                .number_of_lines(1),
        )
}

/// Subtle circular `?` concept-explanation badge with native NSH hover tooltip.
///
/// Reserve strictly for domain concepts rather than self-explanatory controls.
pub fn concept_hint(explanation: impl Fn() -> String + 'static) -> View {
    let tip = view()
        .max_w(px(280.0))
        .px(px(10.0))
        .py(px(7.0))
        .rounded_md()
        .bg(rgb(0x181822))
        .border_1()
        .border_color(rgb(BORDER_PILL))
        .child(
            text(explanation)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .line_height(px(16.0))
                .text_color(rgb(INK_PRIMARY)),
        );

    view()
        .w(px(15.0))
        .h(px(15.0))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(BG_WELL))
        .border_1()
        .border_color(rgb(BORDER_PILL))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(tip.into_element())
        .child(
            text("?")
                .font_family(FONT_MONO_TERM)
                .text_size(px(10.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_MUTED)),
        )
}

/// Static-string variant of [`concept_hint`] for use inside `dynamic(...)` closures.
pub fn concept_hint_static(explanation: &str) -> View {
    let owned = explanation.to_owned();
    concept_hint(move || owned.clone())
}

/// Horizontal Obsidian progress track (`0.0..=1.0`).
pub fn progress_bar(fraction: f32, fill_color: u32) -> View {
    let f = fraction.clamp(0.02, 1.0);
    let mut track = view()
        .w_full()
        .h(px(5.0))
        .rounded_full()
        .bg(rgb(BG_WELL))
        .border_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .overflow_hidden()
        .flex()
        .flex_row()
        .child(view().w(px(0.0)).flex_grow(f).h_full().bg(rgb(fill_color)));
    if f < 1.0 {
        track = track.child(view().w(px(0.0)).flex_grow(1.0 - f).h_full());
    }
    track
}

/// Compact clickable action button in Obsidian style.
pub fn action_button<M>(
    label: impl IntoReactive<SharedString>,
    primary: bool,
    on_press: impl IntoListener<PressEvent, M>,
) -> View {
    let (bg_col, fg_col, br_col, hover_bg, active_bg) = if primary {
        (CTA_BG, CTA_INK, CTA_BG, CTA_BG_HOVER, CTA_BG_ACTIVE)
    } else {
        (BG_WELL, INK_PRIMARY, BORDER_PILL, BG_HOVER, BG_ACTIVE)
    };
    view()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_sm()
        .cursor_pointer()
        .bg(rgb(bg_col))
        .border_1()
        .border_color(rgb(br_col))
        .hover(move |s| s.bg(rgb(hover_bg)))
        .active(move |s| s.bg(rgb(active_bg)))
        .on_press(on_press)
        .child(
            text(label)
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(fg_col))
                .number_of_lines(1),
        )
}

/// Native IME-safe Obsidian text input backed by `nsh_core::element::input()`.
pub fn styled_input(
    value: impl IntoReactive<String>,
    placeholder: impl IntoReactive<String>,
    mono: bool,
) -> Input {
    let family = if mono { FONT_MONO_TERM } else { FONT_UI };
    nsh_core::element::input()
        .value(value)
        .placeholder(placeholder)
        .placeholder_color(hsla_rgb(INK_MUTED))
        .caret_color(hsla_rgb(SKY_FG))
        .selection_color(hsla_rgba(SKY_BG))
        .font_family(family)
        .text_size(px(TEXT_SM))
        .text_color(rgb(INK_PRIMARY))
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .rounded_sm()
        .px(px(10.0))
        .py(px(6.0))
}

/// Clickable citation / evidence chip.
pub fn citation_chip<M>(
    label: impl IntoReactive<SharedString>,
    fg: u32,
    bg: u32,
    border: u32,
    on_press: impl IntoListener<PressEvent, M>,
) -> View {
    view()
        .px(px(8.0))
        .py(px(3.0))
        .rounded_xs()
        .cursor_pointer()
        .bg(rgba(bg))
        .border_1()
        .border_color(rgba(border))
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .active(|s| s.bg(rgb(BG_ACTIVE)))
        .on_press(on_press)
        .child(
            text(label)
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_XS))
                .text_color(rgb(fg))
                .number_of_lines(1),
        )
}

pub fn prop_section(title: &str) -> View {
    view()
        .pt(px(12.0))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            text(title.to_owned())
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED)),
        )
}

pub fn circle_tag_button<M>(
    tag: String,
    active: bool,
    quiet_when_inactive: bool,
    count: Option<usize>,
    on_press: impl IntoListener<PressEvent, M>,
) -> View {
    let label = match count {
        Some(c) => format!("#{tag} {c}"),
        None => format!("#{tag}"),
    };
    let mut el = view()
        .px(px(8.0))
        .py(px(2.0))
        .rounded_xs()
        .cursor_pointer()
        .on_press(on_press);
    let fg_col = if active {
        el = el
            .bg(rgba(SKY_BG))
            .border_1()
            .border_color(rgba(SKY_BORDER))
            .hover(|s| s.bg(rgb(BG_HOVER)));
        SKY_FG
    } else if quiet_when_inactive {
        el = el.hover(|s| s.bg(rgb(BG_HOVER)));
        INK_MUTED
    } else {
        el = el
            .bg(rgb(0x161622))
            .border_1()
            .border_color(rgb(BORDER_SUBTLE))
            .hover(|s| s.bg(rgb(BG_HOVER)));
        INK_SECONDARY
    };
    el.child(
        text(label)
            .font_family(FONT_MONO_TERM)
            .text_size(px(TEXT_XS))
            .text_color(rgb(fg_col))
            .number_of_lines(1),
    )
}

pub fn v_divider() -> View {
    view()
        .w(px(1.0))
        .h(px(14.0))
        .flex_shrink_0()
        .bg(rgb(BORDER_HAIRLINE))
}

pub fn section_header(title: &str, subtitle: String) -> View {
    view()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .pb(px(10.0))
        .mb(px(12.0))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            text(title.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_MD))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        )
        .child(
            text(subtitle)
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_MUTED)),
        )
}

pub fn ratio_bar(
    label: String,
    right_label: String,
    amount: Option<Decimal>,
    base: Decimal,
    color: u32,
) -> View {
    let fill_px = amount.map(|amt| {
        let pct = if base > Decimal::ZERO {
            let raw = (amt.max(Decimal::ZERO) / base) * Decimal::from(100);
            raw.min(Decimal::from(100))
                .to_string()
                .parse::<f32>()
                .unwrap_or(0.0)
        } else {
            0.0
        };
        (pct * 1.1).clamp(2.0, 110.0)
    });
    let value_color = if amount.is_some() {
        INK_PRIMARY
    } else {
        INK_MUTED
    };
    let mut track = view()
        .w(px(110.0))
        .flex_shrink_0()
        .h(px(6.0))
        .rounded_full()
        .bg(rgb(BG_WELL));
    if let Some(w) = fill_px {
        track = track.child(view().w(px(w)).h_full().rounded_full().bg(rgb(color)));
    }
    view()
        .mt(px(8.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .child(
            view().w(px(76.0)).flex_shrink_0().child(
                text(label)
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(INK_SECONDARY)),
            ),
        )
        .child(track)
        .child(
            view().flex_1().min_w(px(0.0)).child(
                text(right_label)
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(value_color)),
            ),
        )
}

pub fn roic_sparkline_for(rows: &[AnnualMetricRow], lang: UiLang) -> View {
    let slice: Vec<&AnnualMetricRow> = rows
        .iter()
        .rev()
        .take(7)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let max_roic = slice
        .iter()
        .filter_map(|r| r.roic_pct)
        .fold(Decimal::from(25), |acc, v| acc.max(v))
        .max(Decimal::from(20));

    view()
        .mt(px(12.0))
        .pt(px(10.0))
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .mb(px(8.0))
                .child(
                    text(
                        lang.t(
                            "历年 ROIC 走势（15% 基准）",
                            "Annual ROIC Trend (15% Hurdle)",
                        )
                        .to_owned(),
                    )
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(INK_MUTED)),
                )
                .child(
                    text(format!("{} {:.0}%", lang.t("峰值", "Peak"), max_roic))
                        .font_family(FONT_MONO_NUM)
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(INK_SECONDARY)),
                ),
        )
        .child(
            view()
                .h(px(66.0))
                .flex()
                .flex_row()
                .items_end()
                .gap(px(8.0))
                .children(slice.into_iter().map(|r| {
                    let (bar_h, bar_color, val_txt, is_num) = match r.roic_pct {
                        Some(v) if v > Decimal::ZERO => {
                            let h = ((v / max_roic) * Decimal::from(40))
                                .to_string()
                                .parse::<f32>()
                                .unwrap_or(4.0)
                                .clamp(4.0, 40.0);
                            let col = if v >= Decimal::from(15) {
                                EMERALD_FG
                            } else {
                                GOLD_FG
                            };
                            (h, col, format!("{v:.0}%"), true)
                        }
                        Some(v) => (4.0, CORAL_FG, format!("{v:.0}%"), true),
                        None => (4.0, INK_MUTED, lang.t("净现", "NetCash").into(), false),
                    };
                    let fy_short = r.label.replace("FY20", "'");
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_end()
                        .gap(px(2.0))
                        .child(
                            text(val_txt)
                                .font_family(if is_num { FONT_MONO_NUM } else { FONT_UI })
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(if is_num { INK_SECONDARY } else { INK_MUTED })),
                        )
                        .child(
                            view()
                                .w(px(14.0))
                                .h(px(bar_h))
                                .rounded_xs()
                                .bg(rgb(bar_color)),
                        )
                        .child(
                            text(fy_short)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_MUTED)),
                        )
                })),
        )
}

pub fn stepper_inline<M1, M2>(
    label: &str,
    value: impl IntoReactive<SharedString>,
    on_dec: impl IntoListener<PressEvent, M1>,
    on_inc: impl IntoListener<PressEvent, M2>,
) -> View {
    view()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_MUTED)),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .rounded_sm()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgb(BORDER_SUBTLE))
                .overflow_hidden()
                .child(
                    view()
                        .w(px(20.0))
                        .h(px(22.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .cursor_pointer()
                        .on_press(on_dec)
                        .child(
                            text("−")
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                )
                .child(
                    view()
                        .w(px(40.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            text(value)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_SM))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_HERO)),
                        ),
                )
                .child(
                    view()
                        .w(px(20.0))
                        .h(px(22.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .cursor_pointer()
                        .on_press(on_inc)
                        .child(
                            text("+")
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                ),
        )
}

pub fn journal_stat(label: &str, value: impl IntoReactive<SharedString>, color: u32) -> View {
    view()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(6.0))
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_MUTED)),
        )
        .child(
            text(value)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(color)),
        )
}

pub fn mono_left(content: impl IntoReactive<SharedString>, width: f32, color: u32) -> View {
    view().w(px(width)).flex_shrink_0().px(px(4.0)).child(
        text(content)
            .font_family(FONT_MONO_NUM)
            .text_size(px(TEXT_SM))
            .text_color(rgb(color))
            .truncate()
            .number_of_lines(1),
    )
}

pub fn mono_right(content: impl IntoReactive<SharedString>, width: f32, color: u32) -> View {
    view()
        .w(px(width))
        .flex_shrink_0()
        .px(px(6.0))
        .flex()
        .flex_row()
        .justify_end()
        .child(
            text(content)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_SM))
                .text_color(rgb(color))
                .truncate()
                .number_of_lines(1),
        )
}

pub fn card_title(title: &str, tag: &str) -> View {
    view()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .pb(px(8.0))
        .mb(px(8.0))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            text(title.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        )
        .child(
            text(tag.to_owned())
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_MUTED)),
        )
}

pub fn kv_line(key: &str, value: impl IntoReactive<SharedString>, color: u32) -> View {
    view()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .mt(px(6.0))
        .child(
            text(key.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_SECONDARY)),
        )
        .child(
            text(value)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(color)),
        )
}

pub fn inspector_kv(key: &str, value: impl IntoReactive<SharedString>) -> View {
    view()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.0))
        .child(
            view().w(px(120.0)).flex_shrink_0().child(
                text(key.to_owned())
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_MUTED)),
            ),
        )
        .child(
            view().flex_1().min_w(px(0.0)).child(
                text(value)
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(INK_PRIMARY))
                    .truncate()
                    .number_of_lines(1),
            ),
        )
}

pub fn prose_block(title: &str, body: &str) -> View {
    view()
        .flex()
        .flex_col()
        .child(
            text(title.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_SECONDARY)),
        )
        .child(
            view().mt(px(4.0)).child(
                text(body.to_owned())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_BASE))
                    .line_height(px(21.0))
                    .text_color(rgb(INK_PRIMARY)),
            ),
        )
}

pub fn render_warning_panel_for(warnings: &[WarningFlag], lang: UiLang) -> View {
    view()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .children(warnings.iter().map(|w| {
            let (fg, bg, br) = match w.severity {
                WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
            };
            let sev_label = match w.severity {
                WarningSeverity::Red => lang.t("红灯预警", "Red Flag"),
                WarningSeverity::Amber => lang.t("黄灯关注", "Watch Flag"),
                WarningSeverity::Info => lang.t("提示", "Info"),
            };
            view()
                .rounded_md()
                .bg(rgba(bg))
                .border_1()
                .border_color(rgba(br))
                .px(px(12.0))
                .py(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(badge_pill(sev_label.to_owned(), fg, bg, br))
                .child(
                    text(w.title.clone())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(fg)),
                )
                .child(
                    text(format!("— {}", w.detail))
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(INK_PRIMARY)),
                )
        }))
}
