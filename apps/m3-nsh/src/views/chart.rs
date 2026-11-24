//! Interactive Intrinsic-Value K-Line candlestick + volume + Strike/Fair-Value overlay canvas.

use crate::components::action_button;
use crate::format::{
    civil_ym_from_ms, civil_ymd_from_ms, compute_kline_domain, currency_prefix, dec_to_f64,
    fmt_pct_clean, fmt_price, fmt_volume_compact, primary_val,
};
use crate::stores::{AppStore, ChartTimeframe};
use crate::theme::*;
use m3_core::calculator;
use m3_core::i18n::UiLang;
use m3_core::model::{MungerReport, OhlcBar, PriceHistorySnapshot};
use nsh_core::FontVariant;
use nsh_core::canvas::{CanvasTextAlign, CanvasTextBaseline};
use nsh_core::prelude::*;
use rust_decimal::Decimal;
use std::f32::consts::TAU;

const CHART_HEIGHT: f32 = 228.0;
const PAD_X: f32 = 10.0;

/// Live valuation overlay computed from the selected company and Charlie Sandbox assumptions.
#[derive(Debug, Clone, PartialEq)]
pub struct KlineValuationOverlay {
    pub strike_price: Option<Decimal>,
    pub fv_high: Option<Decimal>,
    pub dist_to_strike_pct: Option<Decimal>,
}

/// Recomputes live Strike Price, Fair Value Band ceiling, and % vs Strike from Charlie Sandbox.
pub fn compute_kline_valuation_overlay(
    rep: &MungerReport,
    use_ttm_basis: bool,
    sandbox_multiple: Decimal,
    sandbox_target_return: Decimal,
    sandbox_expected_growth: Decimal,
) -> KlineValuationOverlay {
    let val_opt = primary_val(rep, use_ttm_basis);
    let cur_price = rep.price.as_ref().map(|p| p.price);
    let oe_ps = val_opt.and_then(|v| v.owner_earnings_per_share);
    let sb = calculator::evaluate_sandbox(
        oe_ps,
        cur_price,
        sandbox_multiple,
        sandbox_target_return,
        sandbox_expected_growth,
    );
    let strike_price = sb
        .fair_value
        .or_else(|| val_opt.and_then(|v| v.strike_price));
    let fv_high = strike_price.and_then(|s| {
        if sandbox_multiple > Decimal::ZERO {
            Some(s * (sandbox_multiple + Decimal::from(4)) / sandbox_multiple)
        } else {
            None
        }
    });
    let dist_to_strike_pct = match (cur_price, strike_price) {
        (Some(p), Some(s)) if s > Decimal::ZERO => Some(((p - s) / s) * Decimal::from(100)),
        _ => val_opt.and_then(|v| v.distance_to_strike_pct),
    };
    KlineValuationOverlay {
        strike_price,
        fv_high,
        dist_to_strike_pct,
    }
}

/// Slices OHLC bars according to the active chart timeframe (`5Y Wk`, `3Y Wk`, `1Y Day`).
pub fn select_kline_bars(
    history_wk: Option<&PriceHistorySnapshot>,
    history_1d: Option<&PriceHistorySnapshot>,
    tf: ChartTimeframe,
) -> Vec<OhlcBar> {
    match tf {
        ChartTimeframe::FiveYearWeekly => history_wk.map(|h| h.bars.clone()).unwrap_or_default(),
        ChartTimeframe::ThreeYearWeekly => history_wk
            .map(|h| {
                let keep = 156usize.min(h.bars.len());
                h.bars[h.bars.len() - keep..].to_vec()
            })
            .unwrap_or_default(),
        ChartTimeframe::OneYearDaily => history_1d.map(|h| h.bars.clone()).unwrap_or_default(),
    }
}

/// Computes normalized OHLC candle geometry bounds for unit tests & chart rendering.
pub fn compute_kline_canvas_bounds(
    bars: &[OhlcBar],
    strike: Option<Decimal>,
    fv_high: Option<Decimal>,
    width: f32,
    height: f32,
) -> Option<(f64, f64, u64, usize)> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let (lo, hi, max_vol) = compute_kline_domain(bars, strike, fv_high)?;
    let _span = (hi - lo).max(1e-6);
    let valid_bars = bars
        .iter()
        .filter(|b| dec_to_f64(b.open).is_some() && dec_to_f64(b.close).is_some())
        .count();
    Some((lo, hi, max_vol, valid_bars))
}

/// Maps a local canvas X coordinate to the hovered candle index.
pub fn kline_hover_bar_index(x: f32, width: f32, bar_count: usize) -> Option<usize> {
    if bar_count == 0 || width <= 20.0 || x < 0.0 || x > width {
        return None;
    }
    let plot_w = (width - PAD_X * 2.0).max(10.0);
    let step = plot_w / bar_count as f32;
    let idx = ((x - PAD_X) / step).floor() as isize;
    Some(idx.clamp(0, bar_count as isize - 1) as usize)
}

/// Projects a price inside `[y_min, y_max]` to canvas Y within the price sub-region.
pub fn kline_price_to_y(price: f64, y_min: f64, y_max: f64, height: f32) -> f32 {
    let price_top = 8.0f32;
    let price_h = (height * 0.72).max(10.0);
    let y_span = (y_max - y_min).max(1e-6);
    let ratio = ((price - y_min) / y_span).clamp(0.0, 1.0);
    price_top + price_h - (ratio as f32) * price_h
}

#[derive(Clone)]
struct KlineRenderState {
    ticker: String,
    currency: String,
    bars: Vec<OhlcBar>,
    overlay: KlineValuationOverlay,
    domain: Option<(f64, f64, u64)>,
}

impl PartialEq for KlineRenderState {
    fn eq(&self, other: &Self) -> bool {
        self.ticker == other.ticker
            && self.currency == other.currency
            && self.overlay == other.overlay
            && self.domain == other.domain
            && self.bars.len() == other.bars.len()
            && self.bars.iter().zip(other.bars.iter()).all(|(a, b)| {
                a.ts_ms == b.ts_ms
                    && a.open == b.open
                    && a.high == b.high
                    && a.low == b.low
                    && a.close == b.close
                    && a.volume == b.volume
            })
    }
}

fn read_kline_state(store: AppStore) -> KlineRenderState {
    let ticker = store.selected_ticker.get();
    let tf = store.chart_timeframe.get();
    let use_ttm = store.use_ttm_basis.get();
    let mult = store.sandbox_multiple.get();
    let ret = store.sandbox_target_return.get();
    let gro = store.sandbox_expected_growth.get();

    let (currency, overlay) =
        store
            .reports
            .with(|reps| match reps.iter().find(|r| r.ticker == ticker) {
                Some(rep) => (
                    rep.currency.clone(),
                    compute_kline_valuation_overlay(rep, use_ttm, mult, ret, gro),
                ),
                None => (
                    "USD".to_owned(),
                    KlineValuationOverlay {
                        strike_price: None,
                        fv_high: None,
                        dist_to_strike_pct: None,
                    },
                ),
            });

    let bars = match tf {
        ChartTimeframe::FiveYearWeekly => store
            .selected_history_wk
            .with(|h| select_kline_bars(h.as_ref(), None, tf)),
        ChartTimeframe::ThreeYearWeekly => store
            .selected_history_wk
            .with(|h| select_kline_bars(h.as_ref(), None, tf)),
        ChartTimeframe::OneYearDaily => store
            .selected_history_1d
            .with(|h| select_kline_bars(None, h.as_ref(), tf)),
    };

    let domain = compute_kline_domain(&bars, overlay.strike_price, overlay.fv_high);

    KlineRenderState {
        ticker,
        currency,
        bars,
        overlay,
        domain,
    }
}

/// Interactive K-line candlestick, volume, and live Charlie Sandbox valuation overlay canvas.
pub fn chart_placeholder(store: AppStore) -> impl IntoElement {
    let chart_size = signal(size(px(0.0), px(0.0)));
    let hover_pt: Signal<Option<Point<Points>>> = signal(None);

    let kline_state = computed(move || read_kline_state(store));
    let has_domain = computed(move || kline_state.with(|s| s.domain.is_some()));

    view()
        .flex()
        .flex_col()
        .gap(px(8.0))
        // Live Strike & Fair Value Band readout bar
        .child(
            view()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .flex_wrap()
                .gap(px(10.0))
                .child(
                    view()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap(px(12.0))
                        .child(
                            view()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(5.0))
                                .child(view().w(px(10.0)).h(px(2.0)).bg(rgb(EMERALD_FG)))
                                .child(
                                    text(move || store.lang.get().t("击球线", "Strike").to_owned())
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                )
                                .child(
                                    text(move || {
                                        kline_state.with(|s| {
                                            fmt_price(s.overlay.strike_price, &s.currency)
                                        })
                                    })
                                    .font_family(FONT_MONO_NUM)
                                    .text_size(px(TEXT_XS))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || {
                                        let has =
                                            kline_state.with(|s| s.overlay.strike_price.is_some());
                                        rgb(if has { EMERALD_FG } else { INK_MUTED })
                                    }),
                                )
                                .child(when(
                                    move || {
                                        kline_state.with(|s| s.overlay.dist_to_strike_pct.is_some())
                                    },
                                    move || {
                                        text(move || {
                                            kline_state.with(|s| {
                                                let (d, _) = fmt_pct_clean(
                                                    s.overlay.dist_to_strike_pct,
                                                    true,
                                                );
                                                format!("({d})")
                                            })
                                        })
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED))
                                    },
                                )),
                        )
                        .child(when(
                            move || {
                                kline_state.with(|s| {
                                    s.overlay.strike_price.is_some() && s.overlay.fv_high.is_some()
                                })
                            },
                            move || {
                                view()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(5.0))
                                    .child(
                                        view()
                                            .w(px(10.0))
                                            .h(px(8.0))
                                            .rounded_xs()
                                            .bg(rgba(EMERALD_BG))
                                            .border_1()
                                            .border_color(rgba(EMERALD_BORDER)),
                                    )
                                    .child(
                                        text(move || {
                                            store.lang.get().t("合理区间", "Fair Band").to_owned()
                                        })
                                        .font_family(FONT_UI)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_MUTED)),
                                    )
                                    .child(
                                        text(move || {
                                            kline_state.with(|s| {
                                                format!(
                                                    "{} – {}",
                                                    fmt_price(s.overlay.strike_price, &s.currency),
                                                    fmt_price(s.overlay.fv_high, &s.currency)
                                                )
                                            })
                                        })
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_XS))
                                        .text_color(rgb(INK_SECONDARY)),
                                    )
                            },
                        )),
                )
                .child(
                    text(move || {
                        let l = store.lang.get();
                        if has_domain.get() {
                            l.t(
                                "悬停十字线查看 OHLCV 与击球价偏离",
                                "Hover crosshair for OHLCV & % vs Strike",
                            )
                            .to_owned()
                        } else {
                            String::new()
                        }
                    })
                    .font_family(FONT_MONO_TERM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED)),
                ),
        )
        // Main chart or empty/loading placeholder
        .child(when(
            move || has_domain.get(),
            move || render_kline_canvas_block(store, kline_state, chart_size, hover_pt),
        ))
        .child(when(
            move || !has_domain.get(),
            move || render_kline_empty_placeholder(store, kline_state),
        ))
}

fn render_kline_canvas_block(
    store: AppStore,
    kline_state: Memo<KlineRenderState>,
    chart_size: Signal<Size<Points>>,
    hover_pt: Signal<Option<Point<Points>>>,
) -> View {
    let strike_below_visible = computed(move || {
        kline_state.with(|s| {
            let Some((y_min, _, _)) = s.domain else {
                return false;
            };
            s.overlay
                .strike_price
                .and_then(dec_to_f64)
                .is_some_and(|st| st < y_min)
        })
    });

    view()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            view()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(
                    view()
                        .relative()
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(CHART_HEIGHT))
                        .rounded_md()
                        .bg(rgb(BG_VIEWPORT))
                        .border_1()
                        .border_color(rgb(BORDER_HAIRLINE))
                        .overflow_hidden()
                        .child(
                            canvas(move || {
                                let sz = chart_size.get();
                                let hover = hover_pt.get();
                                let l = store.lang.get();
                                kline_state.with(|st| paint_kline_canvas(st, sz, hover, l))
                            })
                            .w_full()
                            .h_full()
                            .on_layout(move |b: &Bounds<Points>| chart_size.set(b.size))
                            .on_mouse_move(move |e: &MouseEvent| hover_pt.set(Some(e.position)))
                            .on_mouse_leave(move |_: &MouseEvent| hover_pt.set(None)),
                        )
                        .child(when(
                            move || strike_below_visible.get(),
                            move || {
                                view()
                                    .absolute()
                                    .bottom(px(8.0))
                                    .right(px(8.0))
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded_xs()
                                    .bg(rgba(EMERALD_BG))
                                    .border_1()
                                    .border_color(rgba(EMERALD_BORDER))
                                    .child(
                                        text(move || {
                                            let l = store.lang.get();
                                            kline_state.with(|s| {
                                                format!(
                                                    "↓ {} {} ({})",
                                                    l.t("击球线", "Strike"),
                                                    fmt_price(s.overlay.strike_price, &s.currency),
                                                    l.t("低于区间底价", "below visible floor")
                                                )
                                            })
                                        })
                                        .font_family(FONT_MONO_NUM)
                                        .text_size(px(TEXT_2XS))
                                        .text_color(rgb(EMERALD_FG)),
                                    )
                            },
                        )),
                )
                // Right Y-axis price labels
                .child(
                    view()
                        .w(px(72.0))
                        .h(px(CHART_HEIGHT))
                        .flex_shrink_0()
                        .py(px(4.0))
                        .flex()
                        .flex_col()
                        .justify_between()
                        .items_end()
                        .children([1.0f64, 0.667, 0.333, 0.0].into_iter().map(|frac| {
                            text(move || {
                                kline_state.with(|s| {
                                    let Some((y_min, y_max, _)) = s.domain else {
                                        return String::new();
                                    };
                                    let pfx = currency_prefix(&s.currency);
                                    let val = y_min + (y_max - y_min) * frac;
                                    format!("{pfx}{val:.2}")
                                })
                            })
                            .font_family(FONT_MONO_NUM)
                            .text_size(px(TEXT_2XS))
                            .text_color(rgb(INK_MUTED))
                            .tabular_nums()
                        })),
                ),
        )
        // Bottom X-axis date ticks
        .child(
            view()
                .pr(px(80.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .children((0..5usize).map(|slot| {
                    text(move || {
                        kline_state.with(|s| {
                            if s.bars.is_empty() {
                                return String::new();
                            }
                            let n = s.bars.len();
                            let idx = match slot {
                                0 => 0,
                                1 => n / 4,
                                2 => n / 2,
                                3 => (n * 3) / 4,
                                _ => n - 1,
                            };
                            civil_ym_from_ms(s.bars[idx].ts_ms)
                        })
                    })
                    .font_family(FONT_MONO_NUM)
                    .text_size(px(TEXT_2XS))
                    .text_color(rgb(INK_MUTED))
                    .tabular_nums()
                })),
        )
}

fn render_kline_empty_placeholder(store: AppStore, kline_state: Memo<KlineRenderState>) -> View {
    view()
        .h(px(128.0))
        .rounded_md()
        .bg(rgb(BG_VIEWPORT))
        .border_1()
        .border_color(rgb(BORDER_HAIRLINE))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .child(
            text(move || {
                let l = store.lang.get();
                let syncing = store.sync_running.get();
                let tk = kline_state.with(|s| s.ticker.clone());
                if syncing {
                    match l {
                        UiLang::Zh => format!("正在同步 {tk} 的行情与历史 K 线…"),
                        UiLang::En => format!("Syncing quotes & K-lines for {tk}…"),
                    }
                } else {
                    match l {
                        UiLang::Zh => format!("尚未同步 {tk} 的历史 K 线缓存"),
                        UiLang::En => format!("No historical K-line cache synced for {tk}"),
                    }
                }
            })
            .font_family(FONT_UI)
            .text_size(px(TEXT_XS))
            .text_color(rgb(INK_MUTED)),
        )
        .child(action_button(
            move || {
                let l = store.lang.get();
                if store.sync_running.get() {
                    l.t("同步中…", "Syncing…").to_owned()
                } else {
                    l.t("同步行情 K 线", "Sync Quotes & K-Lines").to_owned()
                }
            },
            false,
            move || store.start_sync(),
        ))
}

fn paint_kline_canvas(
    st: &KlineRenderState,
    sz: Size<Points>,
    hover: Option<Point<Points>>,
    lang: UiLang,
) -> Painter {
    let mut p = Painter::new();
    let w = sz.width.0;
    let h = sz.height.0;
    if w <= 20.0 || h <= 40.0 || st.bars.is_empty() {
        return p;
    }
    let Some((y_min, y_max, max_vol)) = st.domain else {
        return p;
    };

    p = p.fill_color(rgb(BG_VIEWPORT)).fill_rect(0.0, 0.0, w, h);

    let plot_w = (w - PAD_X * 2.0).max(10.0);
    let price_top = 8.0f32;
    let price_h = (h * 0.72).max(10.0);
    let vol_top = h * 0.78;
    let vol_h = (h * 0.19).max(6.0);

    // 1. Subtle horizontal grid lines
    for frac in [0.0f32, 0.333, 0.667, 1.0] {
        let gy = price_top + price_h * frac;
        p = p
            .fill_color(rgb(BORDER_HAIRLINE))
            .fill_rect(0.0, gy, w, 1.0);
    }

    // 2. Shaded translucent Fair Value band (Strike -> fv_high)
    let strike_f64 = st.overlay.strike_price.and_then(dec_to_f64);
    let fv_high_f64 = st.overlay.fv_high.and_then(dec_to_f64);
    if let Some(s) = strike_f64 {
        let band_top = fv_high_f64.unwrap_or(s);
        if band_top >= y_min && s <= y_max {
            let y_hi = kline_price_to_y(band_top.min(y_max), y_min, y_max, h);
            let y_lo = kline_price_to_y(s.max(y_min), y_min, y_max, h);
            let band_h = (y_lo - y_hi).max(2.0);
            p = p
                .fill_color(rgba(EMERALD_BAND))
                .fill_rect(0.0, y_hi, w, band_h);
        }
    }

    // 3. OHLC wicks, candle bodies, and bottom volume bars
    let n = st.bars.len() as f32;
    let step = plot_w / n;
    let body_w = (step * 0.62).clamp(1.2, 8.0);
    let max_v_f = (max_vol as f64).max(1.0);

    for (idx, bar) in st.bars.iter().enumerate() {
        let cx_p = PAD_X + (idx as f32 + 0.5) * step;
        let (Some(o), Some(hi), Some(lo), Some(c)) = (
            dec_to_f64(bar.open),
            dec_to_f64(bar.high),
            dec_to_f64(bar.low),
            dec_to_f64(bar.close),
        ) else {
            continue;
        };
        let is_up = c >= o;
        let col = if is_up { EMERALD_FG } else { CORAL_FG };
        let y_hi = kline_price_to_y(hi, y_min, y_max, h);
        let y_lo = kline_price_to_y(lo, y_min, y_max, h);
        let y_o = kline_price_to_y(o, y_min, y_max, h);
        let y_c = kline_price_to_y(c, y_min, y_max, h);

        // Wick
        p = p
            .fill_color(rgb(col))
            .fill_rect(cx_p - 0.5, y_hi, 1.0, (y_lo - y_hi).max(1.0));

        // Body
        let body_top = y_o.min(y_c);
        let body_h = (y_o - y_c).abs().max(1.2);
        p = p
            .fill_color(rgb(col))
            .fill_rect(cx_p - body_w * 0.5, body_top, body_w, body_h);

        // Volume bar
        let vh = (((bar.volume as f64 / max_v_f) * vol_h as f64) as f32)
            .max(1.0)
            .min(vol_h);
        let vy = vol_top + (vol_h - vh);
        p = p
            .fill_color(rgba(if is_up { EMERALD_VOL } else { CORAL_VOL }))
            .fill_rect(cx_p - body_w * 0.5, vy, body_w, vh);
    }

    // 4. Dashed Strike Price line + badge label
    if let Some(s) = strike_f64
        && s >= y_min
        && s <= y_max
    {
        let sy = kline_price_to_y(s, y_min, y_max, h);
        p = p
            .stroke_color(rgba(EMERALD_LINE))
            .line_width(1.5)
            .line_dash(&[px(6.0), px(4.0)])
            .begin_path()
            .move_to(0.0, sy)
            .line_to(w, sy)
            .stroke()
            .line_dash(&[]);

        let badge_txt = format!(
            "{} {}",
            lang.t("击球线", "Strike"),
            fmt_price(st.overlay.strike_price, &st.currency)
        );
        let badge_w = (badge_txt.chars().count() as f32 * 6.2 + 12.0).clamp(84.0, 170.0);
        let badge_h = 16.0f32;
        let badge_y = (sy - badge_h - 3.0).clamp(price_top + 2.0, price_top + price_h - badge_h);
        let badge_rect = bounds(point(px(8.0), px(badge_y)), size(px(badge_w), px(badge_h)));
        p = p
            .fill_color(rgba(0x0D1614E6))
            .stroke_color(rgba(EMERALD_BORDER))
            .line_width(1.0)
            .begin_path()
            .round_rect(badge_rect, px(3.0))
            .fill()
            .stroke()
            .font(&format!("10px {FONT_MONO_NUM}"))
            .font_variant_numeric(FontVariant(FontVariant::TABULAR_NUMS))
            .text_align(CanvasTextAlign::Left)
            .text_baseline(CanvasTextBaseline::Middle)
            .fill_color(rgb(EMERALD_FG))
            .fill_text(&badge_txt, 14.0, badge_y + badge_h * 0.5);
    }

    // 5. Interactive hover crosshair + floating OHLCV + date + % vs Strike readout
    if let Some(pos) = hover
        && let Some(idx) = kline_hover_bar_index(pos.x.0, w, st.bars.len())
        && let Some(bar) = st.bars.get(idx)
    {
        let cx_p = PAD_X + (idx as f32 + 0.5) * step;
        let cursor_y = pos.y.0.clamp(price_top, price_top + price_h);

        // Vertical & horizontal crosshair hairlines
        p = p
            .stroke_color(rgba(0xFFFFFF38))
            .line_width(1.0)
            .line_dash(&[px(3.0), px(3.0)])
            .begin_path()
            .move_to(cx_p, 0.0)
            .line_to(cx_p, h)
            .move_to(0.0, cursor_y)
            .line_to(w, cursor_y)
            .stroke()
            .line_dash(&[]);

        // Close dot highlight
        if let Some(close_f) = dec_to_f64(bar.close) {
            let cy_c = kline_price_to_y(close_f, y_min, y_max, h);
            let is_up = bar.close >= bar.open;
            p = p
                .fill_color(rgb(INK_HERO))
                .begin_path()
                .arc(cx_p, cy_c, 4.0, radians(0.0), radians(TAU))
                .fill()
                .fill_color(rgb(if is_up { EMERALD_FG } else { CORAL_FG }))
                .begin_path()
                .arc(cx_p, cy_c, 2.5, radians(0.0), radians(TAU))
                .fill();
        }

        // Floating OHLCV + Date + % vs Strike HUD card (auto-flips left/right)
        let pfx = currency_prefix(&st.currency);
        let date_str = civil_ymd_from_ms(bar.ts_ms);
        let vs_strike = match st.overlay.strike_price {
            Some(s) if s > Decimal::ZERO => {
                let diff = ((bar.close - s) / s) * Decimal::from(100);
                let (pct_s, _) = fmt_pct_clean(Some(diff), true);
                let in_zone = diff <= Decimal::ZERO;
                (
                    format!("{} {pct_s}", lang.t("距击球", "vs Strike")),
                    if in_zone { EMERALD_FG } else { GOLD_FG },
                )
            }
            _ => (lang.t("无击球锚", "No Strike").to_owned(), INK_MUTED),
        };
        let line1 = format!("{date_str}   C {pfx}{:.2}   {}", bar.close, vs_strike.0);
        let line2 = format!(
            "O {pfx}{:.2}   H {pfx}{:.2}   L {pfx}{:.2}   Vol {}",
            bar.open,
            bar.high,
            bar.low,
            fmt_volume_compact(bar.volume)
        );
        let max_chars = line1.chars().count().max(line2.chars().count()) as f32;
        let box_w = (max_chars * 6.1 + 20.0).min(w - 16.0).max(210.0);
        let box_h = 38.0f32;
        let box_x = if cx_p > w * 0.5 {
            8.0
        } else {
            (w - box_w - 8.0).max(8.0)
        };
        let box_y = 8.0f32;
        let box_rect = bounds(point(px(box_x), px(box_y)), size(px(box_w), px(box_h)));

        p = p
            .fill_color(rgba(0x13131CEB))
            .stroke_color(rgb(BORDER_STRONG))
            .line_width(1.0)
            .begin_path()
            .round_rect(box_rect, px(5.0))
            .fill()
            .stroke()
            .font(&format!("10px {FONT_MONO_NUM}"))
            .font_variant_numeric(FontVariant(FontVariant::TABULAR_NUMS))
            .text_align(CanvasTextAlign::Left)
            .text_baseline(CanvasTextBaseline::Middle)
            .fill_color(rgb(vs_strike.1))
            .fill_text(&line1, box_x + 10.0, box_y + 12.0)
            .fill_color(rgb(INK_SECONDARY))
            .fill_text(&line2, box_x + 10.0, box_y + 26.0);
    }

    p
}
