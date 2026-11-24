//! Intrinsic-value K-line & scatter canvas helpers (Phase 1 spike + skeleton).

use crate::format::{compute_kline_domain, dec_to_f64};
use crate::stores::AppStore;
use crate::theme::*;
use m3_core::model::OhlcBar;
use nsh_core::prelude::*;
use rust_decimal::Decimal;

/// Computes normalized OHLC candle geometry bounds for unit tests & Phase 2/3 chart rendering.
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

/// Phase 1 canvas spike verifying `canvas()` + `Painter` bounds rendering.
pub fn chart_placeholder(store: AppStore) -> impl IntoElement {
    let chart_size = signal(size(px(0.0), px(0.0)));
    view()
        .h(px(140.0))
        .mx(px(24.0))
        .mt(px(16.0))
        .rounded_lg()
        .bg(rgb(BG_SURFACE))
        .border_1()
        .border_color(rgb(BORDER_SUBTLE))
        .overflow_hidden()
        .child(
            canvas(move || {
                let sz = chart_size.get();
                let p = Painter::new();
                if sz.width.0 > 1.0 && sz.height.0 > 1.0 {
                    let bars = store
                        .selected_history_wk
                        .with(|h| h.as_ref().map(|x| x.bars.len()).unwrap_or(0));
                    let mid_y = sz.height.0 * 0.5;
                    return p
                        .fill_color(rgb(BG_SURFACE))
                        .fill_rect(0.0, 0.0, sz.width.0, sz.height.0)
                        .stroke_color(rgba(EMERALD_LINE))
                        .line_width(1.5)
                        .begin_path()
                        .move_to(16.0, mid_y)
                        .line_to((sz.width.0 - 16.0).max(16.0), mid_y)
                        .stroke()
                        .fill_color(rgb(INK_MUTED))
                        .fill_text(&format!("K-Line Canvas Ready ({bars} wk bars)"), 16.0, 24.0);
                }
                p
            })
            .w_full()
            .h_full()
            .on_layout(move |b: &Bounds<Points>| chart_size.set(b.size)),
        )
}
