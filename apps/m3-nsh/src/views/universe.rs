//! US Universe Radar L0/L1 view (`apps/m3-nsh`), featuring:
//! - Top Lakehouse L0/L1 telemetry strip and incremental / Full-SEC sync triggers
//! - Munger screening presets, boolean filter chips, and IME-safe dual-mode search (`input()`)
//! - Compact `roic_scatter_slot` (`canvas` + `Painter`) keeping table dominance
//! - Virtualized 6,000-row screener table (`virtual_list`) with dynamic `pos.index()` bindings
//! - Right inspection drawer with Circle-gated Strike Zone status, 5Y annuals, SIC peers & actions

use crate::components::{
    action_button, badge_pill, kbd, kv_line, obsidian_card, prop_section, section_header,
    seg_group, styled_input, v_divider,
};
use crate::format::{dec_to_f64, fmt_money_compact, fmt_pct_clean, fmt_price, fmt_time_cn_for};
use crate::platform_desktop::{copy_text_with_notice, open_external_url};
use crate::stores::{
    AppStore, CompanyViewTab, ScatterPlotModel, ScatterPoint, UniversePreset, UniverseSearchMode,
    WorkspaceTab,
};
use crate::theme::*;
use m3_core::edgar;
use m3_core::i18n::UiLang;
use m3_core::lake::{LakeAnnualRow, UniverseRow, UniverseSortBy, UniverseSummary};
use m3_core::lake_l1::{SignalEvidence, SimilarCompanyHit};
use m3_core::model::{Bucket, WarningFlag, WarningSeverity};
use nsh_core::FontVariant;
use nsh_core::canvas::{CanvasTextAlign, CanvasTextBaseline};
use nsh_core::prelude::*;
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::f32::consts::TAU;

const TABLE_ROW_HEIGHT: f32 = 44.0;
const DRAWER_WIDTH: f32 = 312.0;
const SCATTER_CANVAS_H: f32 = 116.0;
const SCATTER_PAD_X: f32 = 10.0;
const SCATTER_PAD_Y: f32 = 8.0;
const COL_ROIC_W: f32 = 54.0;
const COL_FCF_W: f32 = 58.0;
const COL_SHRS_W: f32 = 56.0;
const COL_SBC_W: f32 = 52.0;
const COL_NDOE_W: f32 = 48.0;
const COL_PRICE_W: f32 = 56.0;
const COL_STRIKE_W: f32 = 56.0;
const COL_GAP_W: f32 = 62.0;
const COL_WARN_W: f32 = 38.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrikeBadgeKind {
    CircleStrike,
    NonCircleDiscount,
}

/// Renders the US Universe Radar L0/L1 workspace tab.
pub fn universe_view(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(render_universe_summary_strip(store))
        .child(render_universe_filter_strip(store))
        .child(when(
            move || store.universe_error.with(|e| e.is_some()),
            move || render_universe_error_banner(store),
        ))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(dynamic(move || {
                    let lake_empty = is_lake_empty(
                        store.universe_lake_present.get(),
                        store.universe_summary.with(|s| s.clone()).as_ref(),
                    );
                    if lake_empty {
                        return scroll()
                            .flex_1()
                            .min_h(px(0.0))
                            .p(px(24.0))
                            .child(render_universe_empty_state(store))
                            .into_element();
                    }
                    render_universe_workspace(store).into_element()
                })),
        )
}

pub(crate) fn is_lake_empty(lake_present: bool, summary: Option<&UniverseSummary>) -> bool {
    !lake_present || summary.is_none_or(|s| s.total_companies == 0)
}

fn render_universe_workspace(store: AppStore) -> View {
    let left_pane = view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .overflow_hidden()
        .child(dynamic(move || {
            let mode = store.universe_state.with(|s| s.search_mode);
            if mode == UniverseSearchMode::Similar {
                return render_universe_similar_pane(store).into_element();
            }
            view()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .overflow_hidden()
                .child(roic_scatter_slot(store))
                .child(render_universe_table_card(store))
                .into_element()
        }));

    view()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_row()
        .overflow_hidden()
        .child(left_pane)
        .child(when(
            move || store.universe_selected_row.with(|r| r.is_some()),
            move || render_universe_drawer_shell(store),
        ))
}

// ── Top Telemetry Header & Sync Controls ─────────────────────────────────────

fn render_universe_summary_strip(store: AppStore) -> View {
    view()
        .h(px(40.0))
        .flex_shrink_0()
        .px(px(16.0))
        .bg(rgb(BG_SURFACE))
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
                .min_w(px(0.0))
                .overflow_hidden()
                .child(summary_stat(
                    move || store.lang.get().t("覆盖美股", "US Universe").to_owned(),
                    move || {
                        let l = store.lang.get();
                        store.universe_summary.with(|s| {
                            let sum = s.as_ref();
                            let total = sum.map(|x| x.total_companies).unwrap_or(0);
                            let parsed = sum.map(|x| x.parsed_financial_companies).unwrap_or(0);
                            match l {
                                UiLang::Zh => format!("{total} (解析 {parsed})"),
                                UiLang::En => format!("{total} ({parsed} parsed)"),
                            }
                        })
                    },
                    move || INK_HERO,
                ))
                .child(v_divider())
                .child(summary_stat(
                    move || store.lang.get().t("质量池", "Quality").to_owned(),
                    move || {
                        store.universe_summary.with(|s| {
                            format!("{}", s.as_ref().map(|x| x.quality_pool_count).unwrap_or(0))
                        })
                    },
                    move || SKY_FG,
                ))
                .child(v_divider())
                .child(summary_stat(
                    move || store.lang.get().t("已估值", "Priced").to_owned(),
                    move || {
                        store.universe_summary.with(|s| {
                            format!("{}", s.as_ref().map(|x| x.priced_count).unwrap_or(0))
                        })
                    },
                    move || INK_PRIMARY,
                ))
                .child(v_divider())
                .child(summary_stat(
                    move || store.lang.get().t("击球区", "Strike Zone").to_owned(),
                    move || {
                        store.universe_summary.with(|s| {
                            format!("{}", s.as_ref().map(|x| x.strike_zone_count).unwrap_or(0))
                        })
                    },
                    move || {
                        let n = store
                            .universe_summary
                            .with(|s| s.as_ref().map(|x| x.strike_zone_count).unwrap_or(0));
                        if n > 0 { EMERALD_FG } else { INK_MUTED }
                    },
                ))
                .child(v_divider())
                .child(summary_stat(
                    move || store.lang.get().t("上次同步", "Last Sync").to_owned(),
                    move || {
                        let l = store.lang.get();
                        store.universe_summary.with(|s| {
                            let ts = s
                                .as_ref()
                                .and_then(|x| x.last_sec_sync_ms.or(x.last_price_sync_ms));
                            fmt_time_cn_for(ts, l)
                        })
                    },
                    move || INK_SECONDARY,
                )),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .flex_shrink_0()
                .child(when(
                    move || store.universe_sync_progress.with(|p| p.is_some()),
                    move || {
                        view()
                            .max_w(px(220.0))
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded_xs()
                            .bg(rgba(SKY_BG))
                            .border_1()
                            .border_color(rgba(SKY_BORDER))
                            .child(
                                text(move || {
                                    store.universe_sync_progress.get().unwrap_or_default()
                                })
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(SKY_FG))
                                .truncate()
                                .number_of_lines(1),
                            )
                    },
                ))
                .child(action_button(
                    move || store.lang.get().t("刷新", "Refresh").to_owned(),
                    false,
                    move || store.reload_universe_from_lake(),
                ))
                .child(sync_action_pill(
                    move || {
                        let l = store.lang.get();
                        if store.universe_syncing.get() {
                            l.t("同步中…", "Syncing…").to_owned()
                        } else {
                            l.t("全量 SEC", "Full SEC").to_owned()
                        }
                    },
                    false,
                    move || store.universe_syncing.get(),
                    move || store.start_universe_sync(true),
                ))
                .child(sync_action_pill(
                    move || {
                        let l = store.lang.get();
                        if store.universe_syncing.get() {
                            l.t("L0 同步中…", "Syncing L0…").to_owned()
                        } else {
                            l.t("同步 L0", "Sync L0").to_owned()
                        }
                    },
                    true,
                    move || store.universe_syncing.get(),
                    move || store.start_universe_sync(false),
                )),
        )
}

fn summary_stat(
    label: impl Fn() -> String + 'static,
    value: impl Fn() -> String + 'static,
    color: impl Fn() -> u32 + 'static,
) -> View {
    view()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(5.0))
        .child(
            text(label)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED))
                .number_of_lines(1),
        )
        .child(
            text(value)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::BOLD)
                .text_color(move || rgb(color()))
                .number_of_lines(1),
        )
}

fn sync_action_pill(
    label: impl Fn() -> String + 'static,
    primary: bool,
    disabled: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_sm()
        .cursor_pointer()
        .border_1()
        .bg(move || {
            if disabled() || !primary {
                rgb(BG_WELL)
            } else {
                rgb(CTA_BG)
            }
        })
        .border_color(move || {
            if disabled() {
                rgb(BORDER_SUBTLE)
            } else if primary {
                rgb(CTA_BG)
            } else {
                rgb(BORDER_PILL)
            }
        })
        .hover(move |s| {
            if disabled() {
                s.bg(rgb(BG_WELL))
            } else if primary {
                s.bg(rgb(CTA_BG_HOVER))
            } else {
                s.bg(rgb(BG_HOVER))
            }
        })
        .on_press(move || {
            if !disabled() {
                on_press();
            }
        })
        .child(
            text(label)
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    rgb(if disabled() {
                        INK_MUTED
                    } else if primary {
                        CTA_INK
                    } else {
                        INK_PRIMARY
                    })
                })
                .number_of_lines(1),
        )
}

fn render_universe_error_banner(store: AppStore) -> View {
    view()
        .px(px(16.0))
        .py(px(8.0))
        .bg(rgba(CORAL_BG))
        .border_b_1()
        .border_color(rgba(CORAL_BORDER))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
            text(move || store.universe_error.get().unwrap_or_default())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(CORAL_FG))
                .flex_1()
                .min_w(px(0.0)),
        )
        .child(action_button(
            move || store.lang.get().t("重试", "Retry").to_owned(),
            false,
            move || store.reload_universe_from_lake(),
        ))
}

// ── Screening Presets, Boolean Filters & IME-Safe Dual-Mode Search ───────────

fn render_universe_filter_strip(store: AppStore) -> View {
    view()
        .flex_shrink_0()
        .px(px(16.0))
        .py(px(8.0))
        .bg(rgb(BG_ROW_ALT))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(6.0))
                .flex_1()
                .min_w(px(0.0))
                .children(
                    UniversePreset::all()
                        .into_iter()
                        .map(|preset| preset_chip(store, preset)),
                )
                .child(v_divider())
                .child(filter_toggle_chip(
                    move || {
                        store
                            .lang
                            .get()
                            .t("剔除金融业", "Excl. Financials")
                            .to_owned()
                    },
                    move || store.universe_state.with(|s| s.exclude_financials),
                    true,
                    move || {
                        store
                            .universe_state
                            .update(|s| s.toggle_exclude_financials());
                        store.reload_universe_from_lake();
                    },
                ))
                .child(filter_toggle_chip(
                    move || store.lang.get().t("5Y+ 完整财报", "5Y+ Filings").to_owned(),
                    move || store.universe_state.with(|s| s.require_5y_history),
                    true,
                    move || {
                        store
                            .universe_state
                            .update(|s| s.toggle_require_5y_history());
                        store.reload_universe_from_lake();
                    },
                ))
                .child(filter_toggle_chip(
                    move || store.lang.get().t("仅看报价池", "Priced Only").to_owned(),
                    move || store.universe_state.with(|s| s.in_price_pool_only),
                    false,
                    move || {
                        store
                            .universe_state
                            .update(|s| s.toggle_in_price_pool_only());
                        store.reload_universe_from_lake();
                    },
                )),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .flex_shrink_0()
                .child(
                    seg_group()
                        .child(reactive_mode_tab(
                            store,
                            UniverseSearchMode::Keyword,
                            (SKY_FG, SKY_BG),
                        ))
                        .child(reactive_mode_tab(
                            store,
                            UniverseSearchMode::Similar,
                            (VIOLET_FG, VIOLET_BG),
                        )),
                )
                .child(render_universe_search_box(store))
                .child(when(
                    move || store.universe_state.with(|s| !s.is_default()),
                    move || {
                        view()
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded_xs()
                            .bg(rgba(GOLD_BG))
                            .border_1()
                            .border_color(rgba(GOLD_BORDER))
                            .hover(|s| s.bg(rgb(BG_HOVER)))
                            .cursor_pointer()
                            .on_press(move || {
                                store.universe_state.update(|s| s.reset());
                                store.reload_universe_from_lake();
                            })
                            .child(
                                text(move || store.lang.get().t("× 重置", "× Reset").to_owned())
                                    .font_family(FONT_UI)
                                    .text_size(px(TEXT_XS))
                                    .text_color(rgb(GOLD_FG))
                                    .number_of_lines(1),
                            )
                    },
                )),
        )
}

fn preset_chip(store: AppStore, preset: UniversePreset) -> View {
    let active = move || store.universe_state.with(|s| s.preset == preset);
    view()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_xs()
        .cursor_pointer()
        .border_1()
        .bg(move || {
            if active() {
                rgba(SKY_BG)
            } else {
                rgb(BG_SURFACE)
            }
        })
        .border_color(move || {
            if active() {
                rgba(SKY_BORDER)
            } else {
                rgb(BORDER_SUBTLE)
            }
        })
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .on_press(move || store.apply_universe_preset(preset))
        .child(
            text(move || preset.label_for(store.lang.get()).to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || rgb(if active() { SKY_FG } else { INK_SECONDARY }))
                .number_of_lines(1),
        )
}

fn filter_toggle_chip(
    label: impl Fn() -> String + 'static,
    active: impl Fn() -> bool + Copy + 'static,
    emerald_accent: bool,
    on_press: impl Fn() + 'static,
) -> View {
    let (fg_on, bg_on, br_on) = if emerald_accent {
        (EMERALD_FG, rgba(EMERALD_BG), rgba(EMERALD_BORDER))
    } else {
        (INK_HERO, rgb(BG_HOVER), rgb(BORDER_PILL))
    };
    let mut row = view()
        .px(px(8.0))
        .py(px(4.0))
        .rounded_xs()
        .cursor_pointer()
        .border_1()
        .bg(move || if active() { bg_on } else { rgb(BG_SURFACE) })
        .border_color(move || if active() { br_on } else { rgb(BORDER_SUBTLE) })
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.0))
        .on_press(on_press);
    if emerald_accent {
        row = row.child(
            view()
                .w(px(5.0))
                .h(px(5.0))
                .rounded_full()
                .bg(move || rgb(if active() { fg_on } else { INK_MUTED })),
        );
    }
    row.child(
        text(label)
            .font_family(FONT_UI)
            .text_size(px(TEXT_XS))
            .text_color(move || rgb(if active() { fg_on } else { INK_MUTED }))
            .number_of_lines(1),
    )
}

fn reactive_mode_tab(store: AppStore, mode: UniverseSearchMode, accent: (u32, u32)) -> View {
    let active = move || store.universe_state.with(|s| s.search_mode == mode);
    view()
        .px(px(10.0))
        .py(px(4.0))
        .rounded_sm()
        .cursor_pointer()
        .bg(move || {
            if active() {
                rgba(accent.1)
            } else {
                rgba(0x00000000)
            }
        })
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .on_press(move || store.set_universe_search_mode(mode))
        .child(
            text(move || mode.label_for(store.lang.get()).to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || rgb(if active() { accent.0 } else { INK_MUTED }))
                .number_of_lines(1),
        )
}

fn render_universe_search_box(store: AppStore) -> View {
    view()
        .w(px(270.0))
        .rounded_sm()
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(move || {
            let mode = store.universe_state.with(|s| s.search_mode);
            if mode == UniverseSearchMode::Similar {
                rgba(VIOLET_BORDER)
            } else {
                rgb(BORDER_PILL)
            }
        })
        .px(px(8.0))
        .py(px(3.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(
            styled_input(
                move || store.universe_state.with(|s| s.query.clone()),
                move || {
                    let l = store.lang.get();
                    match store.universe_state.with(|s| s.search_mode) {
                        UniverseSearchMode::Keyword => l
                            .t(
                                "搜索 Ticker / 公司名 / SIC…",
                                "Search ticker / company / SIC…",
                            )
                            .to_owned(),
                        UniverseSearchMode::Similar => l
                            .t(
                                "输入业务描述或 Ticker 找相似…",
                                "Describe business or ticker…",
                            )
                            .to_owned(),
                    }
                },
                true,
            )
            .flex_1()
            .min_w(px(0.0))
            .border_none()
            .bg(rgba(0x00000000))
            .px(px(2.0))
            .py(px(2.0))
            .text_size(px(TEXT_XS))
            .on_change(move |val: &str| {
                let changed = store.universe_state.with_untracked(|s| s.query != val);
                if changed {
                    store.set_universe_query(val.to_owned());
                }
            })
            .on_submit(move |val: &str| {
                store.set_universe_query(val.to_owned());
            }),
        )
        .child(when(
            move || store.universe_search_running.get(),
            move || {
                view()
                    .px(px(5.0))
                    .py(px(1.0))
                    .rounded_xs()
                    .bg(rgba(VIOLET_BG))
                    .child(
                        text(move || store.lang.get().t("检索中", "Busy").to_owned())
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(VIOLET_FG)),
                    )
            },
        ))
        .child(when(
            move || {
                !store.universe_search_running.get()
                    && store
                        .universe_state
                        .with(|s| s.search_mode == UniverseSearchMode::Similar)
                    && store.universe_similar_query_ms.get().is_some()
            },
            move || {
                text(move || {
                    store
                        .universe_similar_query_ms
                        .get()
                        .map(|ms| format!("{ms}ms"))
                        .unwrap_or_default()
                })
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(VIOLET_FG))
            },
        ))
        .child(when(
            move || store.universe_state.with(|s| !s.query.is_empty()),
            move || {
                view()
                    .px(px(4.0))
                    .py(px(1.0))
                    .rounded_xs()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .on_press(move || store.set_universe_query(String::new()))
                    .child(
                        text("×")
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(INK_MUTED)),
                    )
            },
        ))
}

// ── Compact ROIC × Strict FCF Yield Scatter Slot (Wave 4 Interactive Canvas) ─

fn percentile_sorted(sorted: &[f64], frac: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() - 1) as f64 * frac.clamp(0.0, 1.0)).round() as usize;
    sorted.get(idx).copied()
}

/// Computes normalized 2D scatter coordinates for priced rows with valid 5Y ROIC and FCF yield.
/// Threshold reference lines mark `15% ROIC` and `4% FCF Yield`.
pub fn build_scatter_model(
    rows: &[UniverseRow],
    strike_zone_ciks: &HashSet<String>,
    selected_cik: Option<&str>,
) -> Option<ScatterPlotModel> {
    struct RawPt<'a> {
        row: &'a UniverseRow,
        roic: f64,
        fcf: f64,
    }

    let mut raw: Vec<RawPt<'_>> = Vec::new();
    for row in rows {
        if let (Some(roic_d), Some(fcf_d)) = (row.roic_5y_avg_pct, row.strict_fcf_yield_pct)
            && let (Some(roic), Some(fcf)) = (dec_to_f64(roic_d), dec_to_f64(fcf_d))
        {
            raw.push(RawPt { row, roic, fcf });
        }
    }
    if raw.is_empty() {
        return None;
    }

    let mut xs: Vec<f64> = raw.iter().map(|p| p.roic).collect();
    let mut ys: Vec<f64> = raw.iter().map(|p| p.fcf).collect();
    xs.sort_by(|a, b| a.total_cmp(b));
    ys.sort_by(|a, b| a.total_cmp(b));

    let x_min = if xs.first().is_some_and(|v| *v < 0.0) {
        xs.first().copied().unwrap_or(0.0).clamp(-20.0, -5.0)
    } else {
        0.0
    };
    let p90_x = percentile_sorted(&xs, 0.90).unwrap_or(45.0);
    let x_max = p90_x.clamp(35.0, 80.0);

    let y_min = if ys.first().is_some_and(|v| *v < 0.0) {
        ys.first().copied().unwrap_or(0.0).clamp(-15.0, -2.0)
    } else {
        0.0
    };
    let p90_y = percentile_sorted(&ys, 0.90).unwrap_or(12.0);
    let y_max = (p90_y * 1.15).clamp(12.0, 25.0);

    let x_span = (x_max - x_min).max(1.0);
    let y_span = (y_max - y_min).max(1.0);

    let norm_x = |v: f64| -> f32 { ((v.clamp(x_min, x_max) - x_min) / x_span) as f32 };
    let norm_y = |v: f64| -> f32 { ((v.clamp(y_min, y_max) - y_min) / y_span) as f32 };

    let mut clipped_count = 0usize;
    let mut strike_count = 0usize;
    let mut watch_count = 0usize;
    let mut points: Vec<ScatterPoint> = raw
        .into_iter()
        .map(|p| {
            let clipped = p.roic < x_min || p.roic > x_max || p.fcf < y_min || p.fcf > y_max;
            if clipped {
                clipped_count += 1;
            }
            let in_strike_zone = p.row.in_price_pool
                && strike_zone_ciks.contains(&p.row.cik)
                && p.row
                    .distance_to_strike_pct
                    .is_some_and(|d| d <= Decimal::ZERO);
            if in_strike_zone {
                strike_count += 1;
            }
            if p.row.in_watchlist {
                watch_count += 1;
            }
            let is_selected = selected_cik.is_some_and(|sel| p.row.cik == sel);
            ScatterPoint {
                cik: p.row.cik.clone(),
                ticker: p.row.ticker.clone(),
                roic_pct: p.roic,
                fcf_yield_pct: p.fcf,
                x_norm: norm_x(p.roic),
                y_norm: norm_y(p.fcf),
                in_strike_zone,
                in_watchlist: p.row.in_watchlist,
                is_selected,
                clipped,
            }
        })
        .collect();

    // Paint screened dots first, Circle/watchlist above, Strike Zone above, and selected on top.
    points.sort_by_key(|p| {
        if p.is_selected {
            3u8
        } else if p.in_strike_zone {
            2
        } else if p.in_watchlist {
            1
        } else {
            0
        }
    });

    let ref_roic_norm = (15.0 >= x_min && 15.0 <= x_max).then(|| norm_x(15.0));
    let ref_fcf_norm = (4.0 >= y_min && 4.0 <= y_max).then(|| norm_y(4.0));
    let zero_fcf_norm = (0.0 > y_min && 0.0 < y_max).then(|| norm_y(0.0));

    Some(ScatterPlotModel {
        points,
        x_min,
        x_max,
        y_min,
        y_max,
        ref_roic_norm,
        ref_fcf_norm,
        zero_fcf_norm,
        clipped_count,
        strike_count,
        watch_count,
    })
}

/// Projects normalized `[0, 1]` scatter coordinates into local canvas pixel space.
pub fn scatter_point_px(x_norm: f32, y_norm: f32, width: f32, height: f32) -> (f32, f32) {
    let pw = (width - SCATTER_PAD_X * 2.0).max(8.0);
    let ph = (height - SCATTER_PAD_Y * 2.0).max(8.0);
    (
        SCATTER_PAD_X + x_norm * pw,
        SCATTER_PAD_Y + (1.0 - y_norm) * ph,
    )
}

/// Finds the nearest scatter point within `max_radius_px`, preferring higher-z-order dots.
pub fn hit_test_scatter_point(
    points: &[ScatterPoint],
    pos_x: f32,
    pos_y: f32,
    width: f32,
    height: f32,
    max_radius_px: f32,
) -> Option<usize> {
    if points.is_empty()
        || width <= 24.0
        || height <= 24.0
        || pos_x < 0.0
        || pos_x > width
        || pos_y < 0.0
        || pos_y > height
    {
        return None;
    }
    let limit_sq = max_radius_px * max_radius_px;
    let mut best: Option<(usize, f32)> = None;
    for (idx, pt) in points.iter().enumerate().rev() {
        let (px_x, px_y) = scatter_point_px(pt.x_norm, pt.y_norm, width, height);
        let dx = pos_x - px_x;
        let dy = pos_y - px_y;
        let d2 = dx * dx + dy * dy;
        if d2 <= limit_sq && best.is_none_or(|(_, best_d2)| d2 < best_d2) {
            best = Some((idx, d2));
        }
    }
    best.map(|(idx, _)| idx)
}

/// Interactive ROIC × Strict FCF Yield scatter summary & canvas slot (`roic_scatter_slot`).
/// Kept compact so the virtualized 6,000-row table dominates the viewport.
pub fn roic_scatter_slot(store: AppStore) -> View {
    let scatter_data = computed(move || {
        let sel = store.universe_selected_cik.get();
        store.universe_rows.with(|rows| {
            store
                .universe_strike_zone_ciks
                .with(|ciks| build_scatter_model(rows, ciks, sel.as_deref()))
        })
    });
    let has_data = computed(move || scatter_data.with(|d| d.is_some()));
    let canvas_size = signal(size(px(0.0), px(0.0)));
    let hover_pt: Signal<Option<Point<Points>>> = signal(None);

    obsidian_card()
        .flex_shrink_0()
        .p(px(10.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            text(move || {
                                store
                                    .lang
                                    .get()
                                    .t(
                                        "ROIC × 严苛 FCF 收益率散点矩阵",
                                        "ROIC × Strict FCF Yield Scatter Matrix",
                                    )
                                    .to_owned()
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                scatter_data.with(|opt| match opt {
                                    Some(m) => {
                                        let clip = if m.clipped_count > 0 {
                                            match l {
                                                UiLang::Zh => format!(" · {} 贴边", m.clipped_count),
                                                UiLang::En => {
                                                    format!(" · {} clipped", m.clipped_count)
                                                }
                                            }
                                        } else {
                                            String::new()
                                        };
                                        match l {
                                            UiLang::Zh => format!(
                                                "{} 家含报价 · {} 击球 · {} 能力圈{clip}",
                                                m.points.len(),
                                                m.strike_count,
                                                m.watch_count
                                            ),
                                            UiLang::En => format!(
                                                "{} priced · {} strike · {} circle{clip}",
                                                m.points.len(),
                                                m.strike_count,
                                                m.watch_count
                                            ),
                                        }
                                    }
                                    None => l
                                        .t("0 家同时具备 ROIC 与报价", "0 priced with 5Y ROIC")
                                        .to_owned(),
                                })
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(when(
                            move || store.universe_selected_row.with(|r| r.is_some()),
                            move || {
                                view()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded_xs()
                                    .bg(rgb(BG_WELL))
                                    .border_1()
                                    .border_color(rgb(BORDER_PILL))
                                    .child(
                                        text(move || {
                                            let l = store.lang.get();
                                            store.universe_selected_row.with(|opt| {
                                                let Some(r) = opt.as_ref() else {
                                                    return String::new();
                                                };
                                                let roic_s =
                                                    fmt_pct_clean(r.roic_5y_avg_pct, false).0;
                                                let fcf_s =
                                                    fmt_pct_clean(r.strict_fcf_yield_pct, false).0;
                                                match l {
                                                    UiLang::Zh => format!(
                                                        "选中 {} · ROIC {roic_s} · FCF% {fcf_s}",
                                                        r.ticker
                                                    ),
                                                    UiLang::En => format!(
                                                        "Selected {} · ROIC {roic_s} · FCF% {fcf_s}",
                                                        r.ticker
                                                    ),
                                                }
                                            })
                                        })
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(INK_HERO)),
                                    )
                            },
                        ))
                        .child(scatter_legend_dot(EMERALD_FG, move || {
                            store.lang.get().t("击球区", "Strike Zone").to_owned()
                        }))
                        .child(scatter_legend_dot(SKY_FG, move || {
                            store.lang.get().t("能力圈", "Circle").to_owned()
                        }))
                        .child(scatter_legend_dot(GOLD_FG, move || {
                            store.lang.get().t("筛选池", "Screened").to_owned()
                        })),
                ),
        )
        .child(when(
            move || !has_data.get(),
            move || {
                view()
                    .h(px(56.0))
                    .rounded_md()
                    .bg(rgb(BG_VIEWPORT))
                    .border_1()
                    .border_color(rgb(BORDER_HAIRLINE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(12.0))
                    .child(
                        text(move || {
                            store
                                .lang
                                .get()
                                .t(
                                    "当前筛选结果暂无同时具备 5Y ROIC 与日级报价的标的（可同步 Top 400 报价池）。",
                                    "No filtered companies have both 5Y ROIC and daily quotes yet.",
                                )
                                .to_owned()
                        })
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(INK_MUTED)),
                    )
            },
        ))
        .child(when(
            move || has_data.get(),
            move || {
                view()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        view()
                            .flex()
                            .flex_row()
                            .gap(px(6.0))
                            .child(
                                view()
                                    .w(px(34.0))
                                    .h(px(SCATTER_CANVAS_H))
                                    .flex_shrink_0()
                                    .py(px(2.0))
                                    .flex()
                                    .flex_col()
                                    .justify_between()
                                    .items_end()
                                    .child(axis_tick_text(move || {
                                        scatter_data.with(|d| {
                                            d.as_ref()
                                                .map(|m| format!("{:.0}%", m.y_max))
                                                .unwrap_or_default()
                                        })
                                    }))
                                    .child(axis_tick_text(move || {
                                        scatter_data.with(|d| {
                                            d.as_ref()
                                                .map(|m| {
                                                    format!(
                                                        "{:.0}%",
                                                        m.y_min + (m.y_max - m.y_min) * 0.5
                                                    )
                                                })
                                                .unwrap_or_default()
                                        })
                                    }))
                                    .child(axis_tick_text(move || {
                                        scatter_data.with(|d| {
                                            d.as_ref()
                                                .map(|m| format!("{:.0}%", m.y_min))
                                                .unwrap_or_default()
                                        })
                                    })),
                            )
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h(px(SCATTER_CANVAS_H))
                                    .rounded_md()
                                    .bg(rgb(BG_VIEWPORT))
                                    .border_1()
                                    .border_color(rgb(BORDER_HAIRLINE))
                                    .overflow_hidden()
                                    .cursor_pointer()
                                    .child(
                                        canvas(move || {
                                            let sz = canvas_size.get();
                                            let hover = hover_pt.get();
                                            let l = store.lang.get();
                                            scatter_data.with(|opt| match opt {
                                                Some(m) => paint_scatter_canvas(m, sz, hover, l),
                                                None => Painter::new(),
                                            })
                                        })
                                        .w_full()
                                        .h_full()
                                        .on_layout(move |b: &Bounds<Points>| canvas_size.set(b.size))
                                        .on_mouse_move(move |e: &MouseEvent| {
                                            hover_pt.set(Some(e.position));
                                        })
                                        .on_mouse_leave(move |_: &MouseEvent| {
                                            hover_pt.set(None);
                                        })
                                        .on_press(move |e: &PressEvent| {
                                            let sz = canvas_size.get_untracked();
                                            let hit_cik = scatter_data.with_untracked(|opt| {
                                                let m = opt.as_ref()?;
                                                let idx = hit_test_scatter_point(
                                                    &m.points,
                                                    e.position.x.0,
                                                    e.position.y.0,
                                                    sz.width.0,
                                                    sz.height.0,
                                                    14.0,
                                                )?;
                                                m.points.get(idx).map(|p| p.cik.clone())
                                            });
                                            if let Some(cik) = hit_cik
                                                && let Some(row) = store.universe_rows.with_untracked(|rows| {
                                                    rows.iter().find(|r| r.cik == cik).cloned()
                                                })
                                            {
                                                store.select_universe_row(&row);
                                            }
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        view()
                            .pl(px(40.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(axis_tick_text(move || {
                                scatter_data.with(|d| {
                                    d.as_ref()
                                        .map(|m| format!("ROIC {:.0}%", m.x_min))
                                        .unwrap_or_default()
                                })
                            }))
                            .child(axis_tick_text(move || {
                                scatter_data.with(|d| {
                                    d.as_ref()
                                        .map(|m| format!("{:.0}%", (m.x_min + m.x_max) * 0.5))
                                        .unwrap_or_default()
                                })
                            }))
                            .child(axis_tick_text(move || {
                                scatter_data.with(|d| {
                                    d.as_ref()
                                        .map(|m| format!("{:.0}%", m.x_max))
                                        .unwrap_or_default()
                                })
                            })),
                    )
            },
        ))
}

fn paint_scatter_canvas(
    model: &ScatterPlotModel,
    sz: Size<Points>,
    hover: Option<Point<Points>>,
    lang: UiLang,
) -> Painter {
    let mut p = Painter::new();
    let w = sz.width.0;
    let h = sz.height.0;
    if w <= 24.0 || h <= 24.0 {
        return p;
    }

    p = p.fill_color(rgb(BG_VIEWPORT)).fill_rect(0.0, 0.0, w, h);

    // Subtle grid lines
    for frac in [0.0f32, 0.5, 1.0] {
        let (_, gy) = scatter_point_px(0.0, frac, w, h);
        p = p
            .fill_color(rgb(BORDER_HAIRLINE))
            .fill_rect(0.0, gy, w, 1.0);
        let (gx, _) = scatter_point_px(frac, 0.0, w, h);
        p = p
            .fill_color(rgb(BORDER_HAIRLINE))
            .fill_rect(gx, 0.0, 1.0, h);
    }

    // Sweet-spot quadrant: ROIC >= 15% & Strict FCF Yield >= 4%
    if let (Some(rx), Some(ry)) = (model.ref_roic_norm, model.ref_fcf_norm) {
        let (x0, y0) = scatter_point_px(rx, 1.0, w, h);
        let (x1, y1) = scatter_point_px(1.0, ry, w, h);
        p = p.fill_color(rgba(EMERALD_BAND)).fill_rect(
            x0,
            y0,
            (x1 - x0).max(1.0),
            (y1 - y0).max(1.0),
        );
    }

    // Threshold reference lines: 15% ROIC & 4% FCF Yield
    if let Some(rx) = model.ref_roic_norm {
        let (gx, _) = scatter_point_px(rx, 0.0, w, h);
        p = p
            .stroke_color(rgba(GOLD_BORDER))
            .line_width(1.0)
            .line_dash(&[px(4.0), px(4.0)])
            .begin_path()
            .move_to(gx, 0.0)
            .line_to(gx, h)
            .stroke()
            .line_dash(&[])
            .font(&format!("9px {FONT_MONO_NUM}"))
            .font_variant_numeric(FontVariant(FontVariant::TABULAR_NUMS))
            .text_align(CanvasTextAlign::Left)
            .text_baseline(CanvasTextBaseline::Bottom)
            .fill_color(rgb(GOLD_FG))
            .fill_text("15%", gx + 3.0, h - 3.0);
    }
    if let Some(ry) = model.ref_fcf_norm {
        let (_, gy) = scatter_point_px(0.0, ry, w, h);
        p = p
            .stroke_color(rgba(EMERALD_BORDER))
            .line_width(1.0)
            .line_dash(&[px(4.0), px(4.0)])
            .begin_path()
            .move_to(0.0, gy)
            .line_to(w, gy)
            .stroke()
            .line_dash(&[])
            .font(&format!("9px {FONT_MONO_NUM}"))
            .font_variant_numeric(FontVariant(FontVariant::TABULAR_NUMS))
            .text_align(CanvasTextAlign::Left)
            .text_baseline(CanvasTextBaseline::Bottom)
            .fill_color(rgb(EMERALD_FG))
            .fill_text("4%", 6.0, gy - 2.0);
    }
    if let Some(zy) = model.zero_fcf_norm {
        let (_, gy) = scatter_point_px(0.0, zy, w, h);
        p = p.fill_color(rgb(BORDER_SUBTLE)).fill_rect(0.0, gy, w, 1.0);
    }

    let hovered_idx =
        hover.and_then(|pos| hit_test_scatter_point(&model.points, pos.x.0, pos.y.0, w, h, 14.0));

    for (idx, pt) in model.points.iter().enumerate() {
        let (cx_p, cy_p) = scatter_point_px(pt.x_norm, pt.y_norm, w, h);
        let col = if pt.in_strike_zone {
            EMERALD_FG
        } else if pt.in_watchlist {
            SKY_FG
        } else {
            GOLD_FG
        };
        let is_hovered = hovered_idx == Some(idx);

        if pt.is_selected {
            p = p
                .fill_color(rgba(0xFFFFFF22))
                .fill_rect(0.0, cy_p - 0.5, w, 1.0)
                .fill_rect(cx_p - 0.5, 0.0, 1.0, h)
                .fill_color(rgb(INK_HERO))
                .begin_path()
                .arc(cx_p, cy_p, 5.5, radians(0.0), radians(TAU))
                .fill()
                .fill_color(rgb(col))
                .begin_path()
                .arc(cx_p, cy_p, 3.5, radians(0.0), radians(TAU))
                .fill();
        } else {
            let r = if pt.in_strike_zone || pt.in_watchlist {
                3.0f32
            } else {
                2.2f32
            };
            if is_hovered {
                p = p
                    .fill_color(rgb(INK_HERO))
                    .begin_path()
                    .arc(cx_p, cy_p, r + 2.0, radians(0.0), radians(TAU))
                    .fill();
            }
            p = p
                .fill_color(rgb(col))
                .begin_path()
                .arc(cx_p, cy_p, r, radians(0.0), radians(TAU))
                .fill();
        }
    }

    // Floating hover tooltip pill
    if let Some(idx) = hovered_idx
        && let Some(pt) = model.points.get(idx)
    {
        let (cx_p, cy_p) = scatter_point_px(pt.x_norm, pt.y_norm, w, h);
        let tier_tag = if pt.in_strike_zone {
            lang.t("击球区", "Strike")
        } else if pt.in_watchlist {
            lang.t("能力圈", "Circle")
        } else {
            lang.t("点击选中", "Click")
        };
        let tip = format!(
            "{} · ROIC {:.1}% · FCF {:.1}% · {tier_tag}",
            pt.ticker, pt.roic_pct, pt.fcf_yield_pct
        );
        let tip_w = (tip.chars().count() as f32 * 5.9 + 16.0).clamp(150.0, w - 12.0);
        let tip_h = 20.0f32;
        let tip_x = if cx_p + 10.0 + tip_w <= w - 6.0 {
            cx_p + 10.0
        } else {
            (cx_p - tip_w - 10.0).clamp(6.0, (w - tip_w - 6.0).max(6.0))
        };
        let tip_y = if cy_p - tip_h - 8.0 >= 6.0 {
            cy_p - tip_h - 8.0
        } else {
            (cy_p + 8.0).clamp(6.0, (h - tip_h - 6.0).max(6.0))
        };
        let tip_col = if pt.in_strike_zone {
            EMERALD_FG
        } else if pt.in_watchlist {
            SKY_FG
        } else {
            INK_HERO
        };
        let tip_rect = bounds(point(px(tip_x), px(tip_y)), size(px(tip_w), px(tip_h)));
        p = p
            .fill_color(rgba(0x13131CEB))
            .stroke_color(rgb(BORDER_STRONG))
            .line_width(1.0)
            .begin_path()
            .round_rect(tip_rect, px(4.0))
            .fill()
            .stroke()
            .font(&format!("10px {FONT_MONO_NUM}"))
            .font_variant_numeric(FontVariant(FontVariant::TABULAR_NUMS))
            .text_align(CanvasTextAlign::Left)
            .text_baseline(CanvasTextBaseline::Middle)
            .fill_color(rgb(tip_col))
            .fill_text(&tip, tip_x + 8.0, tip_y + tip_h * 0.5);
    }

    p
}

fn axis_tick_text(label: impl Fn() -> String + 'static) -> Text {
    text(label)
        .font_family(FONT_MONO_NUM)
        .text_size(px(TEXT_2XS))
        .text_color(rgb(INK_MUTED))
}

fn scatter_legend_dot(color: u32, label: impl Fn() -> String + 'static) -> View {
    view()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(view().w(px(6.0)).h(px(6.0)).rounded_xs().bg(rgb(color)))
        .child(
            text(label)
                .font_family(FONT_UI)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED)),
        )
}

// ── Virtualized 6,000-Row Screener Table ─────────────────────────────────────

fn render_universe_table_card(store: AppStore) -> View {
    let quality_sorted = move || {
        store
            .universe_state
            .with(|s| s.sort_by == UniverseSortBy::QualityScore)
    };

    obsidian_card()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            view()
                .flex_shrink_0()
                .px(px(12.0))
                .py(px(7.0))
                .bg(rgb(BG_WELL))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .min_w(px(0.0))
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                let n = store.universe_rows.with(|r| r.len());
                                match l {
                                    UiLang::Zh => format!("全市场筛选榜单 ({n} 家)"),
                                    UiLang::En => format!("Screened Universe ({n} companies)"),
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        )
                        .child(
                            text(move || {
                                store
                                    .lang
                                    .get()
                                    .t(
                                        "点击列头排序 · 点击行右侧下钻 5Y 财报",
                                        "Click headers to sort · Select row for 5Y financials",
                                    )
                                    .to_owned()
                            })
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_MUTED))
                            .truncate()
                            .number_of_lines(1),
                        ),
                )
                .child(
                    view()
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded_xs()
                        .cursor_pointer()
                        .border_1()
                        .bg(move || {
                            if quality_sorted() {
                                rgba(SKY_BG)
                            } else {
                                rgb(BG_SURFACE)
                            }
                        })
                        .border_color(move || {
                            if quality_sorted() {
                                rgba(SKY_BORDER)
                            } else {
                                rgb(BORDER_SUBTLE)
                            }
                        })
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .on_press(move || store.toggle_universe_sort(UniverseSortBy::QualityScore))
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                let arrow = store.universe_state.with(|s| {
                                    if s.sort_by == UniverseSortBy::QualityScore {
                                        if s.sort_asc { " ↑" } else { " ↓" }
                                    } else {
                                        ""
                                    }
                                });
                                match l {
                                    UiLang::Zh => format!("按质量分{arrow}"),
                                    UiLang::En => format!("By Quality{arrow}"),
                                }
                            })
                            .font_family(FONT_MONO_TERM)
                            .text_size(px(TEXT_2XS))
                            .text_color(move || {
                                rgb(if quality_sorted() { SKY_FG } else { INK_MUTED })
                            }),
                        ),
                ),
        )
        .child(render_universe_table_header(store))
        .child(
            virtual_list(
                move || store.universe_rows.with(|r| r.len()),
                move |pos: Position| render_universe_virtual_row(store, pos),
            )
            .flex_1()
            .min_h(px(0.0))
            .estimated_item_size(px(TABLE_ROW_HEIGHT))
            .keys(move || {
                store
                    .universe_rows
                    .with(|r| r.iter().map(|x| x.cik.clone()).collect::<Vec<_>>())
            })
            .empty(render_universe_no_matches(store)),
        )
}

fn render_universe_table_header(store: AppStore) -> View {
    let cols: &[(&str, &str, f32, bool, Option<UniverseSortBy>)] = &[
        (
            "标的 / 能力圈 · SIC",
            "Ticker / Circle · SIC",
            0.0,
            false,
            Some(UniverseSortBy::Ticker),
        ),
        (
            "5Y ROIC",
            "5Y ROIC",
            COL_ROIC_W,
            true,
            Some(UniverseSortBy::Roic5yAvg),
        ),
        (
            "FCF%",
            "FCF Yield",
            COL_FCF_W,
            true,
            Some(UniverseSortBy::StrictFcfYield),
        ),
        (
            "3Y 股本Δ",
            "3Y Shrs Δ",
            COL_SHRS_W,
            true,
            Some(UniverseSortBy::ShareChange3y),
        ),
        (
            "SBC/OCF",
            "SBC/OCF",
            COL_SBC_W,
            true,
            Some(UniverseSortBy::SbcToOcf),
        ),
        ("净债/OE", "ND/OE", COL_NDOE_W, true, None),
        (
            "现价",
            "Price",
            COL_PRICE_W,
            true,
            Some(UniverseSortBy::LatestPrice),
        ),
        (
            "击球价",
            "Strike",
            COL_STRIKE_W,
            true,
            Some(UniverseSortBy::StrikePrice),
        ),
        (
            "距击球",
            "Strike %",
            COL_GAP_W,
            true,
            Some(UniverseSortBy::DistanceToStrike),
        ),
        (
            "预警",
            "Warn",
            COL_WARN_W,
            true,
            Some(UniverseSortBy::WarningsCount),
        ),
    ];

    view()
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .px(px(12.0))
        .py(px(6.0))
        .bg(rgb(BG_ROW_ALT))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .children(cols.iter().map(|(zh, en, width, right, sort_opt)| {
            let zh = *zh;
            let en = *en;
            let width = *width;
            let right = *right;
            let sort_opt = *sort_opt;

            let mut cell = view()
                .px(px(4.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0));
            if right {
                cell = cell.justify_end();
            }
            if width > 0.0 {
                cell = cell.w(px(width)).flex_shrink_0();
            } else {
                cell = cell.flex_1().min_w(px(130.0));
            }
            if let Some(col) = sort_opt {
                cell = cell
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .on_press(move || store.toggle_universe_sort(col));
            }

            let is_sorted =
                move || sort_opt.is_some_and(|col| store.universe_state.with(|s| s.sort_by == col));

            cell.child(
                text(move || {
                    let base = store.lang.get().t(zh, en);
                    if let Some(col) = sort_opt {
                        let arrow = store.universe_state.with(|s| {
                            if s.sort_by == col {
                                if s.sort_asc { " ↑" } else { " ↓" }
                            } else {
                                ""
                            }
                        });
                        format!("{base}{arrow}")
                    } else {
                        base.to_owned()
                    }
                })
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || rgb(if is_sorted() { INK_PRIMARY } else { INK_MUTED }))
                .truncate()
                .number_of_lines(1),
            )
        }))
}

/// Renders one virtualized table cell.
/// IMPORTANT: Every dynamic closure reads `pos.index()` at evaluation time so recycled cells
/// never display stale captured row state.
fn render_universe_virtual_row(store: AppStore, pos: Position) -> View {
    view()
        .h(px(TABLE_ROW_HEIGHT))
        .px(px(12.0))
        .flex()
        .flex_row()
        .items_center()
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .cursor_pointer()
        .bg(move || {
            let idx = pos.index();
            let sel = store.universe_selected_cik.get();
            let is_selected = store.universe_rows.with(|r| {
                r.get(idx)
                    .is_some_and(|x| sel.as_deref() == Some(x.cik.as_str()))
            });
            if is_selected {
                rgb(BG_SELECTED_ROW)
            } else if idx.is_multiple_of(2) {
                rgb(BG_VIEWPORT)
            } else {
                rgb(BG_ROW_ALT)
            }
        })
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .active(|s| s.bg(rgb(BG_ACTIVE)))
        .on_press(move || {
            let idx = pos.index();
            if let Some(row) = store.universe_rows.with_untracked(|r| r.get(idx).cloned()) {
                store.select_universe_row(&row);
            }
        })
        // Col 1: Ticker, Issuer Name, Circle / Strike / FIN / SIC badges, Exchange · SIC subline
        .child(
            view()
                .flex_1()
                .min_w(px(130.0))
                .px(px(4.0))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(2.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .min_w(px(0.0))
                        .child(
                            text(move || {
                                let idx = pos.index();
                                store.universe_rows.with(|r| {
                                    r.get(idx).map(|x| x.ticker.clone()).unwrap_or_default()
                                })
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_XS))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(INK_HERO))
                            .number_of_lines(1),
                        )
                        .child(
                            view().flex_1().min_w(px(0.0)).child(
                                text(move || {
                                    let idx = pos.index();
                                    store.universe_rows.with(|r| {
                                        r.get(idx).map(|x| x.name.clone()).unwrap_or_default()
                                    })
                                })
                                .font_family(FONT_UI)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_PRIMARY))
                                .truncate()
                                .number_of_lines(1),
                            ),
                        )
                        .child(when(
                            move || row_live_bucket_at(store, pos.index()).is_some(),
                            move || {
                                view()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded_xs()
                                    .border_1()
                                    .bg(move || {
                                        let (_, bg, _) = row_live_bucket_at(store, pos.index())
                                            .map(bucket_colors)
                                            .unwrap_or((SLATE_FG, SLATE_BG, SLATE_BORDER));
                                        rgba(bg)
                                    })
                                    .border_color(move || {
                                        let (_, _, br) = row_live_bucket_at(store, pos.index())
                                            .map(bucket_colors)
                                            .unwrap_or((SLATE_FG, SLATE_BG, SLATE_BORDER));
                                        rgba(br)
                                    })
                                    .child(
                                        text(move || {
                                            row_live_bucket_at(store, pos.index())
                                                .map(|b| b.code().to_owned())
                                                .unwrap_or_default()
                                        })
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            let (fg, _, _) = row_live_bucket_at(store, pos.index())
                                                .map(bucket_colors)
                                                .unwrap_or((SLATE_FG, SLATE_BG, SLATE_BORDER));
                                            rgb(fg)
                                        })
                                        .number_of_lines(1),
                                    )
                            },
                        ))
                        .child(when(
                            move || row_strike_badge_at(store, pos.index()).is_some(),
                            move || {
                                view()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded_xs()
                                    .border_1()
                                    .bg(move || match row_strike_badge_at(store, pos.index()) {
                                        Some(StrikeBadgeKind::CircleStrike) => rgba(EMERALD_BG),
                                        _ => rgba(SLATE_BG),
                                    })
                                    .border_color(move || {
                                        match row_strike_badge_at(store, pos.index()) {
                                            Some(StrikeBadgeKind::CircleStrike) => {
                                                rgba(EMERALD_BORDER)
                                            }
                                            _ => rgba(SLATE_BORDER),
                                        }
                                    })
                                    .child(
                                        text(move || {
                                            let l = store.lang.get();
                                            match row_strike_badge_at(store, pos.index()) {
                                                Some(StrikeBadgeKind::CircleStrike) => {
                                                    l.t("击球区", "STRIKE").to_owned()
                                                }
                                                Some(StrikeBadgeKind::NonCircleDiscount) => {
                                                    l.t("≤18x 折价", "≤18x Ref").to_owned()
                                                }
                                                None => String::new(),
                                            }
                                        })
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            rgb(match row_strike_badge_at(store, pos.index()) {
                                                Some(StrikeBadgeKind::CircleStrike) => EMERALD_FG,
                                                _ => SLATE_FG,
                                            })
                                        })
                                        .number_of_lines(1),
                                    )
                            },
                        ))
                        .child(when(
                            move || {
                                let idx = pos.index();
                                store
                                    .universe_rows
                                    .with(|r| r.get(idx).is_some_and(|x| x.is_financial))
                            },
                            move || badge_pill("FIN", ORANGE_FG, ORANGE_BG, ORANGE_BORDER),
                        ))
                        .child(when(
                            move || {
                                let idx = pos.index();
                                store
                                    .universe_rows
                                    .with(|r| r.get(idx).is_some_and(|x| x.sic.is_none()))
                            },
                            move || {
                                badge_pill(
                                    move || store.lang.get().t("未分类", "Unclassified").to_owned(),
                                    SLATE_FG,
                                    SLATE_BG,
                                    SLATE_BORDER,
                                )
                            },
                        )),
                )
                .child(
                    text(move || {
                        let idx = pos.index();
                        let l = store.lang.get();
                        store.universe_rows.with(|r| {
                            r.get(idx)
                                .map(|x| format_sic_line(x, l))
                                .unwrap_or_default()
                        })
                    })
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED))
                    .truncate()
                    .number_of_lines(1),
                ),
        )
        // Col 2: 5Y ROIC
        .child(virtual_num_cell(
            COL_ROIC_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| fmt_pct_clean(x.roic_5y_avg_pct, false).0)
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                store
                    .universe_rows
                    .with(|r| match r.get(idx).and_then(|x| x.roic_5y_avg_pct) {
                        Some(v) if v >= Decimal::from(20) => EMERALD_FG,
                        Some(v) if v >= Decimal::from(15) => INK_PRIMARY,
                        Some(_) => GOLD_FG,
                        None => INK_MUTED,
                    })
            },
        ))
        // Col 3: Strict FCF Yield
        .child(virtual_num_cell(
            COL_FCF_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| fmt_pct_clean(x.strict_fcf_yield_pct, false).0)
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    let val = r.get(idx).and_then(|x| x.strict_fcf_yield_pct);
                    let (_, pol) = fmt_pct_clean(val, false);
                    match val {
                        Some(_) if pol < 0 => CORAL_FG,
                        Some(v) if v >= Decimal::from(5) => EMERALD_FG,
                        Some(_) => INK_PRIMARY,
                        None => INK_MUTED,
                    }
                })
            },
        ))
        // Col 4: 3Y Share Change
        .child(virtual_num_cell(
            COL_SHRS_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| fmt_pct_clean(x.share_change_3y_pct, true).0)
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    let val = r.get(idx).and_then(|x| x.share_change_3y_pct);
                    let (_, pol) = fmt_pct_clean(val, true);
                    share_delta_color(val, pol)
                })
            },
        ))
        // Col 5: SBC / OCF
        .child(virtual_num_cell(
            COL_SBC_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| fmt_pct_clean(x.sbc_to_ocf_pct, false).0)
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                store
                    .universe_rows
                    .with(|r| match r.get(idx).and_then(|x| x.sbc_to_ocf_pct) {
                        Some(v) if v > Decimal::from(25) => GOLD_FG,
                        Some(v) if v <= Decimal::from(10) => EMERALD_FG,
                        Some(_) => INK_SECONDARY,
                        None => INK_MUTED,
                    })
            },
        ))
        // Col 6: Leverage (Net Debt / Strict OE)
        .child(virtual_num_cell(
            COL_NDOE_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .and_then(|x| x.net_debt_to_oe)
                        .map(|v| format!("{v:.1}x"))
                        .unwrap_or_else(|| "—".to_owned())
                })
            },
            move || {
                let idx = pos.index();
                store
                    .universe_rows
                    .with(|r| match r.get(idx).and_then(|x| x.net_debt_to_oe) {
                        Some(v) if v <= Decimal::ZERO => EMERALD_FG,
                        Some(v) if v > Decimal::from(3) => GOLD_FG,
                        Some(_) => INK_SECONDARY,
                        None => INK_MUTED,
                    })
            },
        ))
        // Col 7: Price
        .child(virtual_num_cell(
            COL_PRICE_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| fmt_price(x.latest_price, "USD"))
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                let has = store
                    .universe_rows
                    .with(|r| r.get(idx).is_some_and(|x| x.latest_price.is_some()));
                if has { INK_HERO } else { INK_MUTED }
            },
        ))
        // Col 8: Strike
        .child(virtual_num_cell(
            COL_STRIKE_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| fmt_price(x.strike_price, "USD"))
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                let has = store
                    .universe_rows
                    .with(|r| r.get(idx).is_some_and(|x| x.strike_price.is_some()));
                if has { INK_SECONDARY } else { INK_MUTED }
            },
        ))
        // Col 9: Strike % / Gap (Circle-gated)
        .child(virtual_num_cell(
            COL_GAP_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| fmt_pct_clean(x.distance_to_strike_pct, true).0)
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    store.universe_strike_zone_ciks.with(|ciks| {
                        r.get(idx)
                            .map(|row| universe_distance_color(row, ciks))
                            .unwrap_or(INK_MUTED)
                    })
                })
            },
        ))
        // Col 10: Warnings Count
        .child(virtual_num_cell(
            COL_WARN_W,
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    r.get(idx)
                        .map(|x| {
                            if x.warnings_count == 0 {
                                "0".to_owned()
                            } else {
                                format!("{}", x.warnings_count)
                            }
                        })
                        .unwrap_or_default()
                })
            },
            move || {
                let idx = pos.index();
                store.universe_rows.with(|r| {
                    let Some(row) = r.get(idx) else {
                        return INK_MUTED;
                    };
                    if row.warnings_count > 0 {
                        let has_red = row
                            .parsed_warnings()
                            .iter()
                            .any(|w| w.severity == WarningSeverity::Red);
                        if has_red { CORAL_FG } else { GOLD_FG }
                    } else {
                        INK_MUTED
                    }
                })
            },
        ))
}

fn virtual_num_cell(
    width: f32,
    content: impl Fn() -> String + 'static,
    color: impl Fn() -> u32 + 'static,
) -> View {
    view()
        .w(px(width))
        .flex_shrink_0()
        .px(px(4.0))
        .flex()
        .flex_row()
        .justify_end()
        .child(
            text(content)
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .text_color(move || rgb(color()))
                .truncate()
                .number_of_lines(1),
        )
}

fn row_live_bucket_at(store: AppStore, idx: usize) -> Option<Bucket> {
    store.universe_rows.with(|rows| {
        let row = rows.get(idx)?;
        live_watchlist_bucket(store, row)
    })
}

fn row_strike_badge_at(store: AppStore, idx: usize) -> Option<StrikeBadgeKind> {
    store.universe_rows.with(|rows| {
        let row = rows.get(idx)?;
        let dist = row.distance_to_strike_pct?;
        if dist > Decimal::ZERO {
            return None;
        }
        let in_circle_strike = store
            .universe_strike_zone_ciks
            .with(|ciks| row.in_price_pool && ciks.contains(&row.cik));
        if in_circle_strike {
            Some(StrikeBadgeKind::CircleStrike)
        } else {
            Some(StrikeBadgeKind::NonCircleDiscount)
        }
    })
}

fn live_watchlist_bucket(store: AppStore, row: &UniverseRow) -> Option<Bucket> {
    store
        .reports
        .with(|reps| {
            reps.iter()
                .find(|r| r.ticker.eq_ignore_ascii_case(&row.ticker))
                .map(|r| r.bucket)
        })
        .or_else(|| {
            row.in_watchlist
                .then_some(row.bucket_opt.unwrap_or(Bucket::Watch))
        })
}

fn universe_distance_color(row: &UniverseRow, strike_zone_ciks: &HashSet<String>) -> u32 {
    match row.distance_to_strike_pct {
        Some(d)
            if d <= Decimal::ZERO && row.in_price_pool && strike_zone_ciks.contains(&row.cik) =>
        {
            EMERALD_FG
        }
        Some(d) if d <= Decimal::ZERO => INK_SECONDARY,
        Some(d) if d <= Decimal::from(15) => GOLD_FG,
        Some(_) => INK_SECONDARY,
        None => INK_MUTED,
    }
}

fn format_sic_line(row: &UniverseRow, lang: UiLang) -> String {
    match (
        row.sic,
        row.sic_description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (Some(sic), Some(desc)) => format!("{} · SIC {sic} ({desc})", row.exchange),
        (Some(sic), None) => format!("{} · SIC {sic}", row.exchange),
        (None, Some(desc)) => format!(
            "{} · {desc} ({})",
            row.exchange,
            lang.t("未分类SIC", "Unclassified SIC")
        ),
        (None, None) => format!(
            "{} · {}",
            row.exchange,
            lang.t("未分类SIC", "Unclassified SIC")
        ),
    }
}

// ── Right Inspection Drawer ──────────────────────────────────────────────────

fn render_universe_drawer_shell(store: AppStore) -> View {
    view()
        .w(px(DRAWER_WIDTH))
        .flex_shrink_0()
        .h_full()
        .bg(rgb(0x111118))
        .border_l_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .p(px(16.0))
                .child(dynamic(move || {
                    let Some(row) = store.universe_selected_row.get() else {
                        return view().into_element();
                    };
                    let l = store.lang.get();
                    let strike_ciks = store.universe_strike_zone_ciks.get();
                    let live_bkt = live_watchlist_bucket(store, &row);
                    let annuals = store.universe_selected_annuals.get();
                    let peers = store.universe_selected_peers.get();
                    render_universe_drawer_content(
                        store,
                        &row,
                        live_bkt,
                        &strike_ciks,
                        &annuals,
                        peers.as_ref(),
                        l,
                    )
                    .into_element()
                })),
        )
}

fn render_universe_drawer_content(
    store: AppStore,
    row: &UniverseRow,
    live_bkt: Option<Bucket>,
    strike_ciks: &HashSet<String>,
    annuals: &[LakeAnnualRow],
    peers: Option<&m3_core::lake::PeerComparison>,
    l: UiLang,
) -> View {
    let warnings = row.parsed_warnings();
    let roic_txt = match (row.roic_5y_avg_pct, row.roic_5y_stddev_pct) {
        (Some(avg), Some(sd)) => format!("{avg:.1}% ± {sd:.1}%"),
        (Some(avg), None) => format!("{avg:.1}%"),
        _ => "—".to_owned(),
    };
    let gm_txt = match (row.gross_margin_5y_avg_pct, row.gross_margin_5y_stddev_pct) {
        (Some(avg), Some(sd)) => format!("{avg:.1}% ± {sd:.1}%"),
        (Some(avg), None) => format!("{avg:.1}%"),
        _ => "—".to_owned(),
    };
    let (fcf_txt, fcf_pol) = fmt_pct_clean(row.strict_fcf_yield_pct, false);
    let (share_txt, share_pol) = fmt_pct_clean(row.share_change_3y_pct, true);
    let (sbc_txt, _) = fmt_pct_clean(row.sbc_to_ocf_pct, false);
    let (dist_txt, _) = fmt_pct_clean(row.distance_to_strike_pct, true);

    let in_circle_strike = row.in_price_pool
        && strike_ciks.contains(&row.cik)
        && row
            .distance_to_strike_pct
            .is_some_and(|d| d <= Decimal::ZERO);
    let below_18x_ref = row
        .distance_to_strike_pct
        .is_some_and(|d| d <= Decimal::ZERO);

    let (elig_title, elig_detail, elig_fg, elig_bg, elig_br) = if in_circle_strike {
        (
            l.t("击球区达标 · Strike Zone", "Strike Zone Eligible"),
            l.t(
                "已纳入能力圈且最新报价低于/等于击球价（7 日报价校验通过）。",
                "Inside Circle & latest quote at or below Strike Price (7-day quote check passed).",
            ),
            EMERALD_FG,
            EMERALD_BG,
            EMERALD_BORDER,
        )
    } else if below_18x_ref && live_bkt.is_none() {
        (
            l.t("非能力圈折价 · 门控未放行", "Non-Circle Discount · Gated"),
            l.t(
                "当前价格低于 18x 统一参考价，但尚未加入能力圈或行业未毕业，不触发击球区信号。",
                "Below 18x uniform ref price, but outside your Circle — Strike Zone requires Circle membership.",
            ),
            GOLD_FG,
            GOLD_BG,
            GOLD_BORDER,
        )
    } else if let Some(bkt) = live_bkt {
        (
            l.t(
                "已纳入能力圈 · 等待击球价",
                "In Circle · Waiting for Strike",
            ),
            match bkt {
                Bucket::Core => l.t(
                    "已处于 CORE 核心池，当前尚未进入击球折扣区间。",
                    "In CORE tier; waiting for price to reach Strike Zone.",
                ),
                Bucket::Watch => l.t(
                    "处于 WATCH 观察池，持续跟踪基本面与估值锚。",
                    "In WATCH tier; tracking fundamentals & valuation anchor.",
                ),
                Bucket::Out | Bucket::TooHard => l.t(
                    "已标记为能力圈外或 Too Hard。",
                    "Marked Outside Circle or Too Hard.",
                ),
            },
            SKY_FG,
            SKY_BG,
            SKY_BORDER,
        )
    } else {
        (
            l.t("未入池 · 18x 统一参考口径", "Not in Circle · 18x Ref Basis"),
            l.t(
                "可先加入 WATCH 观察池，或进入 Industry Studio 完成行业研究毕业。",
                "Add to WATCH or graduate its industry in Industry Studio to unlock Circle Strike alerts.",
            ),
            SLATE_FG,
            SLATE_BG,
            SLATE_BORDER,
        )
    };

    let ticker_dossier = row.ticker.clone();
    let ticker_similar = row.ticker.clone();
    let row_watch = row.clone();
    let row_research = row.clone();
    let row_ind_tk = row.ticker.clone();
    let row_ind_nm = row.name.clone();
    let row_ind_sic = row.sic;
    let row_ind_desc = row.sic_description.clone();

    let mut drawer = view()
        .flex()
        .flex_col()
        .gap(px(14.0))
        // Header
        .child(
            view()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .child(
                            text(row.ticker.clone())
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_LG))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(INK_HERO)),
                        )
                        .child({
                            let mut badges = view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .flex_wrap()
                                .gap(px(4.0));
                            if row.is_financial {
                                badges = badges.child(badge_pill(
                                    l.t("金融口径预警", "Financials Flag"),
                                    ORANGE_FG,
                                    ORANGE_BG,
                                    ORANGE_BORDER,
                                ));
                            }
                            if row.sic.is_none() {
                                badges = badges.child(badge_pill(
                                    l.t("未分类SIC", "Unclassified SIC"),
                                    SLATE_FG,
                                    SLATE_BG,
                                    SLATE_BORDER,
                                ));
                            }
                            if let Some(bkt) = live_bkt {
                                let (fg, bg, br) = bucket_colors(bkt);
                                badges = badges.child(badge_pill(
                                    match l {
                                        UiLang::Zh => format!("已入池 · {}", bkt.code()),
                                        UiLang::En => format!("In Circle · {}", bkt.code()),
                                    },
                                    fg,
                                    bg,
                                    br,
                                ));
                            }
                            badges
                        }),
                )
                .child(
                    text(row.name.clone())
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(INK_PRIMARY)),
                )
                .child(
                    text(format_sic_line(row, l))
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
        // Strike Zone & Circle Status Banner
        .child(
            view()
                .p(px(10.0))
                .rounded_md()
                .bg(rgba(elig_bg))
                .border_1()
                .border_color(rgba(elig_br))
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    text(elig_title)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(elig_fg)),
                )
                .child(
                    text(elig_detail)
                        .font_family(FONT_UI)
                        .text_size(px(TEXT_2XS))
                        .line_height(px(15.0))
                        .text_color(rgb(INK_PRIMARY)),
                ),
        )
        // Quick Actions
        .child({
            let mut actions = view().flex().flex_col().gap(px(6.0));
            if live_bkt.is_some() {
                actions = actions.child(
                    view()
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .child(
                            drawer_button(
                                l.t("打开研报 Dossier →", "Open Dossier →"),
                                CTA_INK,
                                CTA_BG,
                                CTA_BG,
                                move || {
                                    store.open_company(
                                        &ticker_dossier,
                                        CompanyViewTab::FinancialsMemo,
                                    );
                                },
                            )
                            .flex_1(),
                        )
                        .child(drawer_button(
                            l.t("Deep Research", "Deep Research"),
                            VIOLET_FG,
                            VIOLET_BG,
                            VIOLET_BORDER,
                            move || store.dispatch_universe_company_research(&row_research),
                        )),
                );
            } else {
                actions = actions
                    .child(drawer_button(
                        l.t("+ WATCH · 加入观察池", "+ WATCH · Add to Watch"),
                        CTA_INK,
                        CTA_BG,
                        CTA_BG,
                        move || store.add_universe_company_to_watch(&row_watch),
                    ))
                    .child(drawer_button(
                        l.t(
                            "入池并派 Alan 深研 · Deep Research",
                            "Add & Dispatch Deep Research",
                        ),
                        VIOLET_FG,
                        VIOLET_BG,
                        VIOLET_BORDER,
                        move || store.dispatch_universe_company_research(&row_research),
                    ));
            }
            actions
                .child(drawer_button(
                    l.t(
                        "研究所在行业 · Industry Studio (⌘3)",
                        "Research Industry · Industry Studio (⌘3)",
                    ),
                    SKY_FG,
                    SKY_BG,
                    SKY_BORDER,
                    move || {
                        store.open_or_create_industry_for_company(
                            &row_ind_tk,
                            &row_ind_nm,
                            row_ind_sic,
                            row_ind_desc.as_deref(),
                        );
                    },
                ))
                .child(drawer_button(
                    l.t("找相似公司 · Find Similar", "Find Similar · Lookalike"),
                    VIOLET_FG,
                    VIOLET_BG,
                    VIOLET_BORDER,
                    move || store.find_similar_for_company(&ticker_similar),
                ))
        })
        // 5Y Deterministic Snapshot
        .child(
            prop_section("5Y DETERMINISTIC SNAPSHOT")
                .child(kv_line(
                    if live_bkt.is_some() {
                        l.t("现价 / 击球价", "Price / Strike")
                    } else {
                        l.t("现价 / 参考价 (18x)", "Price / Ref (18x)")
                    },
                    format!(
                        "{} / {}",
                        fmt_price(row.latest_price, "USD"),
                        fmt_price(row.strike_price, "USD")
                    ),
                    INK_HERO,
                ))
                .child(kv_line(
                    if live_bkt.is_some() {
                        l.t("距击球价", "Dist. to Strike")
                    } else {
                        l.t("距参考价", "Dist. to Ref")
                    },
                    dist_txt,
                    universe_distance_color(row, strike_ciks),
                ))
                .child(kv_line(
                    l.t("严苛 FCF 收益率", "Strict FCF Yield"),
                    fcf_txt,
                    if fcf_pol < 0 {
                        CORAL_FG
                    } else if row.strict_fcf_yield_pct.is_some() {
                        INK_PRIMARY
                    } else {
                        INK_MUTED
                    },
                ))
                .child(kv_line(
                    l.t("最新严苛 OE", "Latest Strict OE"),
                    row.strict_owner_earnings
                        .map(|oe| fmt_money_compact(oe, "USD"))
                        .unwrap_or_else(|| "—".to_owned()),
                    match row.strict_owner_earnings {
                        Some(oe) if oe > Decimal::ZERO => EMERALD_FG,
                        Some(_) => CORAL_FG,
                        None => INK_MUTED,
                    },
                ))
                .child(kv_line("5Y ROIC", roic_txt, INK_HERO))
                .child(kv_line(
                    l.t("5Y 毛利率", "5Y Gross Margin"),
                    gm_txt,
                    INK_PRIMARY,
                ))
                .child(kv_line(
                    l.t("3Y 股本变化", "3Y Share Change"),
                    share_txt,
                    share_delta_color(row.share_change_3y_pct, share_pol),
                ))
                .child(kv_line("SBC / OCF", sbc_txt, INK_SECONDARY))
                .child(kv_line(
                    l.t("净负债 / 严苛 OE", "Net Debt / Strict OE"),
                    row.net_debt_to_oe
                        .map(|v| format!("{v:.2}x"))
                        .unwrap_or_else(|| "—".to_owned()),
                    INK_SECONDARY,
                ))
                .child(kv_line(
                    l.t("质量综合分", "Quality Score"),
                    format!("{:.1}", row.quality_score),
                    SKY_FG,
                )),
        )
        // 5Y Annuals Mini-Table
        .child(render_universe_annuals_table(annuals, &row.cik, l));

    // SIC Peer Comparison
    if let Some(peers_sec) = render_universe_drawer_peers(store, peers, &row.cik, l) {
        drawer = drawer.child(peers_sec);
    }

    // Warnings
    if !warnings.is_empty() {
        drawer = drawer.child(render_universe_drawer_warnings(&warnings, l));
    }

    // XBRL Provenance
    drawer.child(render_universe_drawer_provenance(store, row, l))
}

fn drawer_button(
    label: &str,
    fg: u32,
    bg: u32,
    border: u32,
    on_press: impl Fn() + 'static,
) -> View {
    let is_solid = bg == CTA_BG;
    view()
        .px(px(10.0))
        .py(px(6.0))
        .rounded_md()
        .cursor_pointer()
        .bg(if is_solid { rgb(bg) } else { rgba(bg) })
        .border_1()
        .border_color(if is_solid { rgb(border) } else { rgba(border) })
        .hover(move |s| {
            s.bg(if is_solid {
                rgb(CTA_BG_HOVER)
            } else {
                rgb(BG_HOVER)
            })
        })
        .active(move |s| {
            s.bg(if is_solid {
                rgb(CTA_BG_ACTIVE)
            } else {
                rgb(BG_ACTIVE)
            })
        })
        .flex()
        .items_center()
        .justify_center()
        .on_press(on_press)
        .child(
            text(label.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(fg))
                .number_of_lines(1),
        )
}

fn render_universe_annuals_table(annuals: &[LakeAnnualRow], target_cik: &str, l: UiLang) -> View {
    let sec = prop_section(l.t("ANNUAL METRICS · 5Y 底表", "ANNUAL METRICS · 5Y Ledger"));
    let recent: Vec<&LakeAnnualRow> = annuals
        .iter()
        .filter(|a| a.cik == target_cik)
        .rev()
        .take(5)
        .collect();
    if recent.is_empty() {
        return sec.child(
            text(l.t("暂无年度明细行", "No annual rows yet").to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED)),
        );
    }

    let recent_len = recent.len();
    let header = view()
        .flex()
        .flex_row()
        .items_center()
        .py(px(4.0))
        .border_b_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .child(
            view().w(px(46.0)).flex_shrink_0().child(
                text(l.t("财年", "FY").to_owned())
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED)),
            ),
        )
        .child(
            view()
                .w(px(54.0))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(
                    text(l.t("营收", "Rev").to_owned())
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
        .child(
            view()
                .w(px(42.0))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(
                    text(l.t("毛利%", "GM%").to_owned())
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
        .child(
            view()
                .w(px(44.0))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .child(
                    text("ROIC")
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(INK_MUTED)),
                ),
        )
        .child(
            view().flex_1().min_w(px(0.0)).flex().justify_end().child(
                text(l.t("严苛OE", "Strict OE").to_owned())
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED)),
            ),
        );

    let mut box_view = view()
        .rounded_sm()
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .px(px(8.0))
        .py(px(4.0))
        .flex()
        .flex_col()
        .child(header);

    for (idx, a) in recent.into_iter().enumerate() {
        let gm = a
            .gross_margin_pct
            .map(|v| format!("{v:.0}%"))
            .unwrap_or_else(|| "—".to_owned());
        let roic = a
            .roic_pct
            .map(|v| format!("{v:.0}%"))
            .unwrap_or_else(|| "—".to_owned());
        let roic_col = match a.roic_pct {
            Some(v) if v >= Decimal::from(15) => EMERALD_FG,
            Some(_) => GOLD_FG,
            None => INK_MUTED,
        };
        let (oe_str, oe_col) = match a.strict_owner_earnings {
            Some(oe) => {
                let col = if oe > Decimal::ZERO {
                    EMERALD_FG
                } else {
                    CORAL_FG
                };
                (fmt_money_compact(oe, "USD"), col)
            }
            None => ("—".to_owned(), INK_MUTED),
        };
        let mut row_el = view().flex().flex_row().items_center().py(px(4.0));
        if idx + 1 < recent_len {
            row_el = row_el.border_b_1().border_color(rgb(BORDER_HAIRLINE));
        }
        box_view = box_view.child(
            row_el
                .child(
                    view().w(px(46.0)).flex_shrink_0().child(
                        text(a.label.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_PRIMARY)),
                    ),
                )
                .child(
                    view()
                        .w(px(54.0))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .child(
                            text(fmt_money_compact(a.revenue, "USD"))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                )
                .child(
                    view()
                        .w(px(42.0))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .child(
                            text(gm)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(INK_SECONDARY)),
                        ),
                )
                .child(
                    view()
                        .w(px(44.0))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .child(
                            text(roic)
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_2XS))
                                .text_color(rgb(roic_col)),
                        ),
                )
                .child(
                    view().flex_1().min_w(px(0.0)).flex().justify_end().child(
                        text(oe_str)
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(oe_col)),
                    ),
                ),
        );
    }

    sec.child(box_view)
}

fn render_universe_drawer_peers(
    store: AppStore,
    peers: Option<&m3_core::lake::PeerComparison>,
    target_cik: &str,
    l: UiLang,
) -> Option<View> {
    let peers = peers?;
    if peers.target.cik != target_cik || peers.peer_count <= 1 {
        return None;
    }
    let roic_p = peers
        .roic_percentile
        .map(|p| format!("P{p:.0}"))
        .unwrap_or_else(|| "—".to_owned());
    let gm_p = peers
        .gross_margin_percentile
        .map(|p| format!("P{p:.0}"))
        .unwrap_or_else(|| "—".to_owned());
    let fcf_p = peers
        .strict_fcf_yield_percentile
        .map(|p| format!("P{p:.0}"))
        .unwrap_or_else(|| "—".to_owned());

    let mut sec = prop_section(&format!("SIC PEERS · N={}", peers.peer_count)).child(kv_line(
        l.t(
            "同业 ROIC / 毛利 / FCF 分位",
            "Peer ROIC / Margin / FCF Pct",
        ),
        format!("{roic_p} / {gm_p} / {fcf_p}"),
        SKY_FG,
    ));

    if !peers.top_peers.is_empty() {
        sec = sec.child(view().flex().flex_row().flex_wrap().gap(px(5.0)).children(
            peers.top_peers.iter().take(4).map(|p| {
                let peer_row = p.clone();
                view()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded_xs()
                    .bg(rgb(BG_WELL))
                    .border_1()
                    .border_color(rgb(BORDER_SUBTLE))
                    .hover(|s| s.bg(rgb(BG_HOVER)))
                    .cursor_pointer()
                    .on_press(move || store.select_universe_row(&peer_row))
                    .child(
                        text(p.ticker.clone())
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_SECONDARY)),
                    )
            }),
        ));
    }

    Some(sec)
}

fn render_universe_drawer_warnings(warnings: &[WarningFlag], l: UiLang) -> View {
    let heading = match l {
        UiLang::Zh => format!("WARNINGS · {} 项预警", warnings.len()),
        UiLang::En => format!("WARNINGS · {} Flags", warnings.len()),
    };
    prop_section(&heading).children(warnings.iter().map(|w| {
        let (fg, bg, br) = match w.severity {
            WarningSeverity::Red => (CORAL_FG, CORAL_BG, CORAL_BORDER),
            WarningSeverity::Amber => (GOLD_FG, GOLD_BG, GOLD_BORDER),
            WarningSeverity::Info => (SLATE_FG, SLATE_BG, SLATE_BORDER),
        };
        view()
            .p(px(8.0))
            .rounded_sm()
            .bg(rgba(bg))
            .border_1()
            .border_color(rgba(br))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                text(w.title.clone())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(fg)),
            )
            .child(
                text(w.detail.clone())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_PRIMARY)),
            )
    }))
}

fn render_universe_drawer_provenance(store: AppStore, row: &UniverseRow, l: UiLang) -> View {
    let sha_short = row
        .raw_sha256
        .as_deref()
        .map(|s| &s[..s.len().min(16)])
        .unwrap_or("—");
    let peers_cli = format!("m3 universe peers {}", row.ticker);
    let edgar_url = edgar::normalize_cik(&row.cik)
        .ok()
        .map(|norm| format!("https://www.sec.gov/edgar/browse/?CIK={norm}"));

    let mut action_row = view()
        .mt(px(4.0))
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap(px(6.0))
        .child(
            view()
                .px(px(10.0))
                .py(px(5.0))
                .rounded_sm()
                .bg(rgb(BG_WELL))
                .border_1()
                .border_color(rgb(BORDER_PILL))
                .hover(|s| s.bg(rgb(BG_HOVER)))
                .cursor_pointer()
                .on_press(move || {
                    copy_text_with_notice(store, peers_cli.clone(), "universe peers");
                })
                .child(
                    text(match l {
                        UiLang::Zh => format!("复制 CLI: m3 universe peers {}", row.ticker),
                        UiLang::En => format!("Copy CLI: m3 universe peers {}", row.ticker),
                    })
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_SECONDARY))
                    .truncate()
                    .number_of_lines(1),
                ),
        );
    if let Some(url) = edgar_url {
        action_row = action_row.child(
            view()
                .px(px(10.0))
                .py(px(5.0))
                .rounded_sm()
                .bg(rgba(SKY_BG))
                .border_1()
                .border_color(rgba(SKY_BORDER))
                .hover(|s| s.bg(rgb(BG_HOVER)))
                .cursor_pointer()
                .on_press(move || open_external_url(&url))
                .child(
                    text(l.t("打开 SEC EDGAR ↗", "Open SEC EDGAR ↗"))
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_2XS))
                        .text_color(rgb(SKY_FG))
                        .number_of_lines(1),
                ),
        );
    }

    prop_section(l.t("PROVENANCE · XBRL 溯源", "PROVENANCE · XBRL Trace"))
        .child(kv_line("CIK", row.cik.clone(), INK_SECONDARY))
        .child(kv_line(
            l.t("最近财报", "Latest Filing"),
            row.latest_filing_end
                .clone()
                .unwrap_or_else(|| "—".to_owned()),
            INK_SECONDARY,
        ))
        .child(kv_line(
            l.t("年报年数", "Annual Years"),
            match l {
                UiLang::Zh => format!("{} 年", row.years_of_data),
                UiLang::En => format!("{} yr", row.years_of_data),
            },
            INK_SECONDARY,
        ))
        .child(kv_line("RAW_SHA256", sha_short.to_owned(), INK_MUTED))
        .child(action_row)
}

// ── Empty States (NeedsSync / Loading / NoMatches) ───────────────────────────

fn render_universe_empty_state(store: AppStore) -> View {
    let sync_cli = "m3 universe sync --full --prices-top 400";

    obsidian_card()
        .p(px(24.0))
        .flex()
        .flex_col()
        .gap(px(16.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .pb(px(12.0))
                .border_b_1()
                .border_color(rgb(BORDER_HAIRLINE))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(view().w(px(7.0)).h(px(7.0)).rounded_full().bg(rgb(SKY_FG)))
                        .child(
                            text(move || {
                                let l = store.lang.get();
                                if store.universe_syncing.get() {
                                    l.t(
                                        "正在同步全美股 L0 确定性湖仓…",
                                        "Syncing US Universe L0 Lakehouse…",
                                    )
                                    .to_owned()
                                } else {
                                    l.t(
                                        "全美股 L0 确定性湖仓尚未同步",
                                        "US Universe L0 Lakehouse Not Synced Yet",
                                    )
                                    .to_owned()
                                }
                            })
                            .font_family(FONT_UI)
                            .text_size(px(TEXT_MD))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(INK_HERO)),
                        ),
                )
                .child(badge_pill("MILESTONE A · ZERO-LLM L0", SKY_FG, SKY_BG, SKY_BORDER)),
        )
        .child(
            text(move || {
                store
                    .lang
                    .get()
                    .t(
                        "点击「立即同步全美股 L0」或在终端运行 CLI，即可从 SEC EDGAR 官方全量归档（company_tickers_exchange.json + companyfacts.zip）拉取美股财报，使用与 m3 report 完全一致的 rust_decimal 引擎并行计算 5,000+ 公司护城河指标，并对 Top 400 质量候选池拉取免费日级股价。",
                        "Click \"Sync US Universe L0 Now\" or run the CLI to ingest SEC EDGAR bulk archives (company_tickers_exchange.json + companyfacts.zip), compute moat metrics across 5,000+ US companies with the deterministic rust_decimal engine, and fetch daily prices for the Top 400 quality pool.",
                    )
                    .to_owned()
            })
            .font_family(FONT_UI)
            .text_size(px(TEXT_BASE))
            .line_height(px(22.0))
            .text_color(rgb(INK_SECONDARY)),
        )
        .child(when(
            move || store.universe_sync_progress.with(|p| p.is_some()),
            move || {
                view()
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded_md()
                    .bg(rgba(SKY_BG))
                    .border_1()
                    .border_color(rgba(SKY_BORDER))
                    .child(
                        text(move || {
                            let l = store.lang.get();
                            let msg = store.universe_sync_progress.get().unwrap_or_default();
                            match l {
                                UiLang::Zh => format!("同步进度：{msg}"),
                                UiLang::En => format!("Sync progress: {msg}"),
                            }
                        })
                        .font_family(FONT_MONO_TERM)
                        .text_size(px(TEXT_XS))
                        .text_color(rgb(SKY_FG)),
                    )
            },
        ))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(10.0))
                .child(sync_action_pill(
                    move || {
                        let l = store.lang.get();
                        if store.universe_syncing.get() {
                            l.t("正在后台同步 SEC 湖仓…", "Syncing SEC Lakehouse…")
                                .to_owned()
                        } else {
                            l.t("立即同步全美股 L0 (Full SEC)", "Sync US Universe L0 (Full SEC)")
                                .to_owned()
                        }
                    },
                    true,
                    move || store.universe_syncing.get(),
                    move || store.start_universe_sync(true),
                ))
                .child(sync_action_pill(
                    move || {
                        store
                            .lang
                            .get()
                            .t("增量同步 L0", "Incremental Sync L0")
                            .to_owned()
                    },
                    false,
                    move || store.universe_syncing.get(),
                    move || store.start_universe_sync(false),
                ))
                .child(
                    view()
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded_md()
                        .bg(rgb(BG_WELL))
                        .border_1()
                        .border_color(rgb(BORDER_PILL))
                        .hover(|s| s.bg(rgb(BG_HOVER)))
                        .cursor_pointer()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .on_press(move || {
                            copy_text_with_notice(store, sync_cli.to_owned(), "universe sync");
                        })
                        .child(
                            text(sync_cli)
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_PRIMARY)),
                        )
                        .child(kbd(move || store.lang.get().t("复制", "Copy").to_owned())),
                ),
        )
}

fn render_universe_no_matches(store: AppStore) -> View {
    view()
        .p(px(24.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(
            text(move || {
                let l = store.lang.get();
                let req_5y = store.universe_state.with(|s| s.require_5y_history);
                if req_5y {
                    l.t(
                        "当前芒格滤镜（含 5Y+ 完整财报）或搜索词下无匹配公司。可关闭「5Y+ 完整财报」或重置。",
                        "No companies match the active filters (including 5Y+ Filings) or search query.",
                    )
                    .to_owned()
                } else {
                    l.t(
                        "当前芒格滤镜或搜索词下无匹配公司。可放宽筛选条件或重置。",
                        "No companies match the active filters or search query. Relax filters or reset.",
                    )
                    .to_owned()
                }
            })
            .font_family(FONT_UI)
            .text_size(px(TEXT_XS))
            .text_color(rgb(INK_SECONDARY))
            .flex_1()
            .min_w(px(0.0)),
        )
        .child(action_button(
            move || {
                store
                    .lang
                    .get()
                    .t("重置全部滤镜", "Reset All Filters")
                    .to_owned()
            },
            false,
            move || {
                store.universe_state.update(|s| s.reset());
                store.reload_universe_from_lake();
            },
        ))
}

// ── Async Lookalike Semantic Search Pane ─────────────────────────────────────

fn render_universe_similar_pane(store: AppStore) -> View {
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .w_full()
                .child(dynamic(move || {
                    let query_empty = store.universe_state.with(|s| s.query.trim().is_empty());
                    if query_empty {
                        return render_universe_similar_prompt(store).into_element();
                    }
                    if store.universe_search_running.get() {
                        return render_universe_similar_loading(store).into_element();
                    }
                    let hits = store.universe_similar_hits.get();
                    if hits.is_empty() {
                        return render_universe_similar_empty(store).into_element();
                    }
                    render_universe_similar_results(store, &hits).into_element()
                })),
        )
}

fn render_universe_similar_prompt(store: AppStore) -> View {
    let l = store.lang.get();
    obsidian_card()
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_header(
            l.t(
                "混合语义相似搜索 · Lookalike Discovery",
                "Hybrid Semantic Search · Lookalike Discovery",
            ),
            "L1 SQL HARD FILTER × LATENT COSINE × BM25".to_owned(),
        ))
        .child(
            text(
                l.t(
                    "在右上角搜索框输入商业模式关键词（例如 \"recurring industrial CAD subscription\"）或直接输入美股 Ticker（如 ADSK），也可在右侧详情抽屉点击「找相似公司 · Lookalike」。前置叠加当前左侧芒格财务硬过滤条件（剔除金融 / 报价池 / ROIC / SBC），零大模型 Token 异步毫秒级返回可溯源相似标的。",
                    "Enter business keywords (e.g., \"recurring industrial CAD subscription\") or a US ticker (e.g., ADSK) above, or click \"Find Similar · Lookalike\" in the drawer. Applies active financial hard filters and returns traceable lookalikes asynchronously with 0 LLM tokens.",
                )
                .to_owned(),
            )
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .line_height(px(20.0))
            .text_color(rgb(INK_SECONDARY)),
        )
}

fn render_universe_similar_loading(store: AppStore) -> View {
    let l = store.lang.get();
    let q = store.universe_state.with(|s| s.query.trim().to_owned());
    obsidian_card()
        .p(px(20.0))
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
                .child(
                    view()
                        .w(px(7.0))
                        .h(px(7.0))
                        .rounded_full()
                        .bg(rgb(VIOLET_FG)),
                )
                .child(
                    text(match l {
                        UiLang::Zh => format!("正在后台检索与「{q}」相似的护城河公司…"),
                        UiLang::En => format!("Searching lookalike moat companies for \"{q}\"…"),
                    })
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(INK_PRIMARY)),
                ),
        )
        .child(badge_pill("ASYNC L1", VIOLET_FG, VIOLET_BG, VIOLET_BORDER))
}

fn render_universe_similar_empty(store: AppStore) -> View {
    let l = store.lang.get();
    let l1_chunks = store
        .inbox_summary
        .with(|s| s.as_ref().map(|x| x.doc_chunks_count).unwrap_or(0));
    let hint = if l1_chunks == 0 {
        l.t(
            "本地 L1 语义索引片段尚为空（doc_chunks=0）。请先到「简报收件箱」点击「追平事件流」或运行 `m3 events catchup --bootstrap` 构建文本索引。",
            "Local L1 semantic index is empty (doc_chunks=0). Open Inbox and click Catch-Up or run `m3 events catchup --bootstrap` to build text chunks.",
        )
    } else {
        l.t(
            "当前相似搜索或财务硬过滤条件无匹配公司，可放宽过滤条件或更换关键词。",
            "No companies match this similarity query and financial hard filters. Relax filters or adjust keywords.",
        )
    };

    let mut actions = view()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .flex_shrink_0();
    if l1_chunks == 0 {
        actions = actions.child(action_button(
            l.t("追平 L1 文本索引", "Build L1 Text Index").to_owned(),
            true,
            move || {
                store.open_tab(WorkspaceTab::Inbox);
                store.start_l1_catchup(true);
            },
        ));
    }
    actions = actions.child(action_button(
        l.t("返回关键词筛选", "Back to Keyword Screen").to_owned(),
        false,
        move || store.set_universe_search_mode(UniverseSearchMode::Keyword),
    ));

    obsidian_card()
        .p(px(20.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(
            text(hint.to_owned())
                .font_family(FONT_UI)
                .text_size(px(TEXT_SM))
                .text_color(rgb(INK_SECONDARY))
                .flex_1()
                .min_w(px(0.0)),
        )
        .child(actions)
}

fn render_universe_similar_results(store: AppStore, hits: &[SimilarCompanyHit]) -> View {
    let l = store.lang.get();
    let q = store.universe_state.with(|s| s.query.trim().to_owned());
    let elapsed = store
        .universe_similar_query_ms
        .get()
        .map(|ms| format!("{ms}ms"))
        .unwrap_or_else(|| "—".to_owned());
    let subtitle = match l {
        UiLang::Zh => format!("命中 {} 家 · 查询 {elapsed} · 0 Flagship Token", hits.len()),
        UiLang::En => format!("{} hits · {elapsed} · 0 Flagship Tokens", hits.len()),
    };
    let selected_cik = store.universe_selected_cik.get();

    view()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(obsidian_card().p(px(14.0)).child(section_header(
            &match l {
                UiLang::Zh => format!("Lookalike 相似公司 · {q}"),
                UiLang::En => format!("Lookalike Companies · {q}"),
            },
            subtitle,
        )))
        .children(hits.iter().enumerate().map(|(idx, hit)| {
            render_universe_similar_card(store, hit, idx, selected_cik.as_deref(), l)
        }))
}

fn render_universe_similar_card(
    store: AppStore,
    hit: &SimilarCompanyHit,
    idx: usize,
    selected_cik: Option<&str>,
    l: UiLang,
) -> View {
    let row = &hit.company;
    let is_selected = selected_cik == Some(row.cik.as_str());
    let row_click = row.clone();
    let row_watch = row.clone();
    let ticker_dossier = row.ticker.clone();
    let live_bkt = live_watchlist_bucket(store, row);
    let (roic_s, _) = fmt_pct_clean(row.roic_5y_avg_pct, false);
    let (fcf_s, _) = fmt_pct_clean(row.strict_fcf_yield_pct, false);
    let (sbc_s, _) = fmt_pct_clean(row.sbc_to_ocf_pct, false);

    let mut header_left = view()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .min_w(px(0.0))
        .flex_1()
        .child(
            text(format!("#{}", idx + 1))
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_XS))
                .text_color(rgb(INK_MUTED)),
        )
        .child(
            text(row.ticker.clone())
                .font_family(FONT_MONO_NUM)
                .text_size(px(TEXT_MD))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(INK_HERO)),
        )
        .child(
            view().flex_1().min_w(px(0.0)).child(
                text(row.name.clone())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .text_color(rgb(INK_PRIMARY))
                    .truncate()
                    .number_of_lines(1),
            ),
        );
    if let Some(bkt) = live_bkt {
        let (fg, bg, br) = bucket_colors(bkt);
        header_left = header_left.child(badge_pill(bkt.code().to_owned(), fg, bg, br));
    }

    let action_btn = if live_bkt.is_some() {
        action_button(
            l.t("打开 Dossier →", "Open Dossier →").to_owned(),
            false,
            move || store.open_company(&ticker_dossier, CompanyViewTab::FinancialsMemo),
        )
    } else {
        action_button(
            l.t("+ 入观察池", "+ Add to Watch").to_owned(),
            false,
            move || store.add_universe_company_to_watch(&row_watch),
        )
    };

    view()
        .rounded_lg()
        .bg(rgb(if is_selected {
            BG_SELECTED_ROW
        } else {
            BG_SURFACE
        }))
        .border_1()
        .border_color(if is_selected {
            rgba(VIOLET_BORDER)
        } else {
            rgb(BORDER_SUBTLE)
        })
        .hover(|s| s.bg(rgb(BG_HOVER)))
        .cursor_pointer()
        .p(px(14.0))
        .flex()
        .flex_col()
        .gap(px(8.0))
        .on_press(move || store.select_universe_row(&row_click))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(header_left)
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .flex_shrink_0()
                        .child(badge_pill(
                            match l {
                                UiLang::Zh => format!("综合 {:.0}%", hit.score * 100.0),
                                UiLang::En => format!("Score {:.0}%", hit.score * 100.0),
                            },
                            VIOLET_FG,
                            VIOLET_BG,
                            VIOLET_BORDER,
                        ))
                        .child(
                            text(match l {
                                UiLang::Zh => format!(
                                    "语义 {:.0}% · BM25 {:.2} · {} 片段",
                                    hit.semantic_score * 100.0,
                                    hit.lexical_score,
                                    hit.matched_chunk_count
                                ),
                                UiLang::En => format!(
                                    "Semantic {:.0}% · BM25 {:.2} · {} chunks",
                                    hit.semantic_score * 100.0,
                                    hit.lexical_score,
                                    hit.matched_chunk_count
                                ),
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_MUTED)),
                        ),
                ),
        )
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(10.0))
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            text(format!("ROIC {roic_s}"))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(
                            text(format!("FCF% {fcf_s}"))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(
                            text(format!("SBC/OCF {sbc_s}"))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_SECONDARY)),
                        )
                        .child(
                            text(format_sic_line(row, l))
                                .font_family(FONT_MONO_TERM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED))
                                .truncate()
                                .number_of_lines(1),
                        ),
                )
                .child(action_btn),
        )
        .children(
            hit.evidence
                .iter()
                .take(2)
                .map(|ev| render_evidence_card(store, ev, l)),
        )
}

fn render_evidence_card(store: AppStore, ev: &SignalEvidence, l: UiLang) -> View {
    let date_str = ev.filing_date.as_deref().unwrap_or("—");
    let copy_url = ev.source_url.clone();
    let open_url_str = ev.source_url.clone();
    let copy_label = format!("{} {}", ev.ticker, ev.doc_type);

    let mut right = view().flex().flex_row().items_center().gap(px(8.0)).child(
        text(format!(
            "{} {:.0}%",
            l.t("相似度", "Sim"),
            ev.similarity * 100.0
        ))
        .font_family(FONT_MONO_NUM)
        .text_size(px(TEXT_XS))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(EMERALD_FG)),
    );
    if !ev.source_url.trim().is_empty() {
        right = right
            .child(action_button(
                l.t("复制 URL", "Copy URL").to_owned(),
                false,
                move || copy_text_with_notice(store, copy_url.clone(), &copy_label),
            ))
            .child(action_button(
                l.t("打开原文 ↗", "Open SEC ↗").to_owned(),
                false,
                move || open_external_url(&open_url_str),
            ));
    }

    let mut card = obsidian_card()
        .p(px(12.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(6.0))
                        .child(badge_pill(ev.doc_type.clone(), SKY_FG, SKY_BG, SKY_BORDER))
                        .child(
                            view()
                                .px(px(6.0))
                                .py(px(1.0))
                                .rounded_xs()
                                .bg(rgb(BG_WELL))
                                .child(
                                    text(ev.section.clone())
                                        .font_family(FONT_MONO_TERM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                ),
                        )
                        .child(
                            text(format!("{} · {date_str}", ev.ticker))
                                .font_family(FONT_MONO_NUM)
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(INK_MUTED)),
                        ),
                )
                .child(right),
        );

    if !ev.title.trim().is_empty() {
        card = card.child(
            text(ev.title.clone())
                .font_family(FONT_UI)
                .text_size(px(TEXT_XS))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(INK_HERO)),
        );
    }

    card = card.child(
        view()
            .px(px(10.0))
            .py(px(8.0))
            .rounded_md()
            .bg(rgb(BG_SHELL))
            .border_1()
            .border_color(rgb(BORDER_HAIRLINE))
            .child(
                text(ev.quote.clone())
                    .font_family(FONT_UI)
                    .text_size(px(TEXT_XS))
                    .line_height(px(18.0))
                    .text_color(rgb(INK_PRIMARY)),
            ),
    );

    if !ev.source_url.trim().is_empty() {
        card = card.child(
            text(ev.source_url.clone())
                .font_family(FONT_MONO_TERM)
                .text_size(px(TEXT_2XS))
                .text_color(rgb(INK_MUTED))
                .truncate()
                .number_of_lines(1),
        );
    }

    card
}
