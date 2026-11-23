use super::{FontRoles, theme::*};
use crate::model::{AnnualMetricRow, WarningFlag, WarningSeverity};
use gpui::{MouseButton, SharedString, Window, div, prelude::*, px, rgb, rgba};
use rust_decimal::Decimal;

pub(super) fn pixel_m3_mark(cell_px: f32) -> gpui::Div {
    div()
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
            div()
                .flex()
                .flex_row()
                .h(px(cell_px))
                .children(runs.into_iter().map(|(on, count)| {
                    let seg = div().w(px(count as f32 * cell_px)).h(px(cell_px));
                    if on { seg.bg(rgb(INK_HERO)) } else { seg }
                }))
        }))
}

pub(super) fn card_shell() -> gpui::Div {
    div()
        .rounded_lg()
        .bg(rgb(BG_SURFACE))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .overflow_hidden()
}

pub(super) fn seg_group() -> gpui::Div {
    div()
        .p_0p5()
        .rounded_md()
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .flex()
        .items_center()
        .gap_0p5()
}

pub(super) fn seg_tab(
    id: impl Into<SharedString>,
    label: &str,
    active: bool,
    accent_bg: Option<(u32, u32)>,
    mono: bool,
    fonts: &FontRoles,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let family = if mono {
        fonts.mono_term.clone()
    } else {
        fonts.ui.clone()
    };
    let (bg_el, fg_col) = match (active, accent_bg) {
        (true, Some((fg, _))) => (Some(accent_bg.map(|(_, bg)| rgba(bg))), fg),
        (true, None) => (None, INK_HERO),
        (false, _) => (None, INK_MUTED),
    };
    let mut el = div()
        .id(id.into())
        .px_2p5()
        .py_1()
        .rounded_sm()
        .cursor_pointer()
        .font_family(family)
        .text_xs()
        .on_click(on_click);
    if active {
        el = el.font_weight(gpui::FontWeight::SEMIBOLD);
        if let Some(Some(bg_rgba)) = bg_el {
            el = el.bg(bg_rgba);
        } else {
            el = el.bg(rgb(BG_HOVER));
        }
    } else {
        el = el.hover(|h| h.text_color(rgb(INK_PRIMARY)));
    }
    el.text_color(rgb(fg_col)).child(label.to_owned())
}

pub(super) fn kbd(label: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .px_1p5()
        .py_0p5()
        .rounded_xs()
        .bg(rgb(0x161620))
        .border_1()
        .border_color(rgb(BORDER_PILL))
        .font_family(fonts.mono_term.clone())
        .text_size(px(10.))
        .text_color(rgb(INK_MUTED))
        .child(label.to_owned())
}

pub(super) fn badge(text: String, fg: u32, bg: u32, border: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(rgba(border))
        .bg(rgba(bg))
        .font_family(fonts.ui.clone())
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(fg))
        .whitespace_nowrap()
        .child(text)
}

pub(super) fn prop_section(title: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .pt_3()
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .font_family(fonts.mono_term.clone())
                .text_size(px(10.))
                .text_color(rgb(INK_MUTED))
                .child(title.to_owned()),
        )
}

pub(super) fn circle_tag_button(
    id: impl Into<SharedString>,
    tag: String,
    active: bool,
    quiet_when_inactive: bool,
    count: Option<usize>,
    fonts: &FontRoles,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let text = match count {
        Some(c) => format!("#{tag} {c}"),
        None => format!("#{tag}"),
    };
    let mut el = div()
        .id(id.into())
        .px_2()
        .py_0p5()
        .rounded_xs()
        .cursor_pointer()
        .font_family(fonts.mono_term.clone())
        .text_size(px(11.))
        .whitespace_nowrap()
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(move |ev, win, cx| {
            cx.stop_propagation();
            on_click(ev, win, cx);
        });
    if active {
        el = el
            .bg(rgba(SKY_BG))
            .border_1()
            .border_color(rgba(SKY_BORDER))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .text_color(rgb(SKY_FG));
    } else if quiet_when_inactive {
        el = el
            .text_color(rgb(INK_MUTED))
            .hover(|s| s.text_color(rgb(INK_PRIMARY)).bg(rgb(BG_HOVER)));
    } else {
        el = el
            .bg(rgb(0x161622))
            .border_1()
            .border_color(rgb(BORDER_SUBTLE))
            .hover(|s| s.bg(rgb(BG_HOVER)))
            .text_color(rgb(INK_SECONDARY));
    }
    el.child(text)
}

pub(super) fn v_divider() -> gpui::Div {
    div()
        .w(px(1.))
        .h(px(14.))
        .flex_shrink_0()
        .bg(rgb(BORDER_HAIRLINE))
}

pub(super) fn section_header(title: &str, subtitle: String, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .pb_2p5()
        .mb_3()
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
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
                .font_family(fonts.mono_term.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(subtitle),
        )
}


pub(super) fn ratio_bar(
    label: String,
    right_label: String,
    amount: Decimal,
    base: Decimal,
    color: u32,
    fonts: &FontRoles,
) -> gpui::Div {
    let pct = if base > Decimal::ZERO {
        let raw = (amount.max(Decimal::ZERO) / base) * Decimal::from(100);
        raw.min(Decimal::from(100))
            .to_string()
            .parse::<f32>()
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let fill_px = (pct * 1.1).clamp(2.0, 110.0);
    div()
        .mt_2()
        .flex()
        .items_center()
        .justify_between()
        .gap_2p5()
        .child(
            div()
                .w(px(76.))
                .flex_shrink_0()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_SECONDARY))
                .child(label),
        )
        .child(
            div()
                .w(px(110.))
                .flex_shrink_0()
                .h(px(6.))
                .rounded_full()
                .bg(rgb(BG_WELL))
                .child(div().w(px(fill_px)).h_full().rounded_full().bg(rgb(color))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_right()
                .font_family(fonts.mono_num.clone())
                .text_xs()
                .text_color(rgb(INK_PRIMARY))
                .child(right_label),
        )
}

pub(super) fn roic_sparkline(rows: &[AnnualMetricRow], fonts: &FontRoles) -> gpui::Div {
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

    div()
        .mt_3()
        .pt_2p5()
        .border_t_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb_2()
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_MUTED))
                        .child("历年 ROIC 走势（15% 基准）"),
                )
                .child(
                    div()
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_SECONDARY))
                        .child(format!("峰值 {:.0}%", max_roic)),
                ),
        )
        .child(
            div()
                .h(px(66.))
                .flex()
                .items_end()
                .gap_2()
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
                        None => (4.0, INK_MUTED, "净现".into(), false),
                    };
                    let fy_short = r.label.replace("FY20", "'");
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_end()
                        .gap_0p5()
                        .child(
                            div()
                                .font_family(if is_num {
                                    fonts.mono_num.clone()
                                } else {
                                    fonts.ui.clone()
                                })
                                .text_size(px(11.))
                                .text_color(rgb(if is_num { INK_SECONDARY } else { INK_MUTED }))
                                .child(val_txt),
                        )
                        .child(
                            div()
                                .w(px(14.))
                                .h(px(bar_h))
                                .rounded_xs()
                                .bg(rgb(bar_color)),
                        )
                        .child(
                            div()
                                .font_family(fonts.mono_num.clone())
                                .text_size(px(10.))
                                .text_color(rgb(INK_MUTED))
                                .child(fy_short),
                        )
                })),
        )
}

pub(super) fn stepper_inline<F1, F2>(
    label: &str,
    value: String,
    id_prefix: &str,
    fonts: &FontRoles,
    on_dec: F1,
    on_inc: F2,
) -> gpui::Div
where
    F1: Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    F2: Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(label.to_owned()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .rounded_sm()
                .bg(rgb(BG_VIEWPORT))
                .border_1()
                .border_color(rgb(BORDER_SUBTLE))
                .overflow_hidden()
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("{id_prefix}-dec")))
                        .w(px(20.))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_SECONDARY))
                        .cursor_pointer()
                        .on_click(on_dec)
                        .child("−"),
                )
                .child(
                    div()
                        .w(px(40.))
                        .text_center()
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(INK_HERO))
                        .child(value),
                )
                .child(
                    div()
                        .id(gpui::SharedString::from(format!("{id_prefix}-inc")))
                        .w(px(20.))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .active(|s| s.bg(rgb(BG_ACTIVE)))
                        .font_family(fonts.mono_num.clone())
                        .text_xs()
                        .text_color(rgb(INK_SECONDARY))
                        .cursor_pointer()
                        .on_click(on_inc)
                        .child("+"),
                ),
        )
}

pub(super) fn journal_stat(label: &str, value: String, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_baseline()
        .gap_1p5()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(label.to_owned()),
        )
        .child(
            div()
                .font_family(fonts.mono_num.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(color))
                .child(value),
        )
}

pub(super) fn mono_left(text: String, width: f32, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .px_1()
        .font_family(fonts.mono_num.clone())
        .text_xs()
        .text_color(rgb(color))
        .truncate()
        .child(text)
}

pub(super) fn mono_right(text: String, width: f32, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .px_1p5()
        .text_right()
        .font_family(fonts.mono_num.clone())
        .text_xs()
        .text_color(rgb(color))
        .truncate()
        .child(text)
}

pub(super) fn card_title(title: &str, tag: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .pb_2()
        .mb_2()
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO))
                .child(title.to_owned()),
        )
        .child(
            div()
                .font_family(fonts.mono_term.clone())
                .text_xs()
                .text_color(rgb(INK_MUTED))
                .child(tag.to_owned()),
        )
}

pub(super) fn kv_line(key: &str, value: String, color: u32, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .mt_1p5()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .text_color(rgb(INK_SECONDARY))
                .child(key.to_owned()),
        )
        .child(
            div()
                .font_family(fonts.mono_num.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(color))
                .child(value),
        )
}

pub(super) fn inspector_kv(key: &str, value: String, fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .w(px(120.))
                .flex_shrink_0()
                .font_family(fonts.mono_term.clone())
                .text_size(px(11.))
                .text_color(rgb(INK_MUTED))
                .child(key.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(fonts.mono_term.clone())
                .text_xs()
                .text_color(rgb(INK_PRIMARY))
                .truncate()
                .child(value),
        )
}

pub(super) fn prose_block(title: &str, body: &str, fonts: &FontRoles) -> gpui::Div {
    div()
        .child(
            div()
                .font_family(fonts.ui.clone())
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(INK_SECONDARY))
                .child(title.to_owned()),
        )
        .child(
            div()
                .mt_1()
                .font_family(fonts.ui.clone())
                .text_size(px(13.))
                .line_height(px(21.))
                .text_color(rgb(INK_PRIMARY))
                .child(body.to_owned()),
        )
}

pub(super) fn render_warning_panel(warnings: &[WarningFlag], fonts: &FontRoles) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(warnings.iter().map(|w| {
            let (fg, bg, br) = match w.severity {
                WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
                WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
                WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
            };
            let sev_label = match w.severity {
                WarningSeverity::Red => "红灯预警",
                WarningSeverity::Amber => "黄灯关注",
                WarningSeverity::Info => "提示",
            };
            div()
                .rounded_md()
                .bg(rgba(bg))
                .border_1()
                .border_color(rgba(br))
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_2p5()
                .child(badge(sev_label.to_owned(), fg, bg, br, fonts))
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(fg))
                        .child(w.title.clone()),
                )
                .child(
                    div()
                        .font_family(fonts.ui.clone())
                        .text_xs()
                        .text_color(rgb(INK_PRIMARY))
                        .child(format!("— {}", w.detail)),
                )
        }))
}
