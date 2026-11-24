use super::{PaletteItem, SortCol, StatusQuickFilter, TodayPulse};
use crate::i18n::UiLang;
use crate::model::{
    AnnualMetricRow, Bucket, MungerReport, OhlcBar, StrikeZoneStatus, ValuationSnapshot,
    WarningSeverity, primary_circle_tag,
};
use rust_decimal::Decimal;
use std::{cmp::Ordering, collections::BTreeMap};

pub fn compute_today_pulse(reports: &[MungerReport]) -> TodayPulse {
    let mut strike_tickers = Vec::new();
    let mut red_core_tickers = Vec::new();
    let mut watch_count = 0usize;

    for r in reports {
        match r.bucket {
            Bucket::Core => {
                if r.status == StrikeZoneStatus::InStrikeZone {
                    strike_tickers.push(r.ticker.clone());
                }
                if r.status == StrikeZoneStatus::WarningBlocked
                    || r.warnings
                        .iter()
                        .any(|w| w.severity == WarningSeverity::Red)
                {
                    red_core_tickers.push(r.ticker.clone());
                }
            }
            Bucket::Watch => watch_count += 1,
            Bucket::Out | Bucket::TooHard => {}
        }
    }

    TodayPulse {
        strike_tickers,
        red_core_tickers,
        watch_count,
    }
}

pub fn collect_circle_tags(reports: &[MungerReport]) -> Vec<(String, usize)> {
    let mut map: BTreeMap<String, usize> = BTreeMap::new();
    for rep in reports {
        let tag = primary_circle_tag(&rep.circle);
        if !tag.is_empty() {
            *map.entry(tag).or_insert(0) += 1;
        }
    }
    let mut pairs: Vec<(String, usize)> = map.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs
}

pub fn filter_reports_for_basket<'a>(
    reports: &'a [MungerReport],
    bucket: Bucket,
    circle_filter: Option<&str>,
    status_filter: StatusQuickFilter,
    sort_col: SortCol,
    sort_asc: bool,
) -> Vec<&'a MungerReport> {
    let mut rows: Vec<&'a MungerReport> = reports
        .iter()
        .filter(|r| r.bucket == bucket)
        .filter(|r| circle_filter.is_none_or(|t| primary_circle_tag(&r.circle) == t))
        .filter(|r| match status_filter {
            StatusQuickFilter::All => true,
            StatusQuickFilter::StrikeOnly => r.status == StrikeZoneStatus::InStrikeZone,
            StatusQuickFilter::WarnedOnly => {
                r.status == StrikeZoneStatus::WarningBlocked
                    || r.warnings
                        .iter()
                        .any(|w| w.severity == WarningSeverity::Red)
            }
        })
        .collect();
    sort_reports(&mut rows, sort_col, sort_asc);
    rows
}

pub fn build_alan_interrogation_prompt_for(
    ticker: &str,
    name: &str,
    bucket: Bucket,
    lang: crate::i18n::UiLang,
) -> String {
    match lang {
        crate::i18n::UiLang::Zh => format!(
            "请作为我的芒格跨学科研究幕僚 Alan，执行 [{ticker} · {name}]（当前分桶: {}）的审讯工作流：\n\
            1) 先运行 `./apps/m3-desktop/m3 report {ticker}` 与 `./apps/m3-desktop/m3 dossier {ticker}` 获取确定性财务与现有草稿（严禁口算或改动数字）。\n\
            2) 基于 10-K/电话会更新三句话业务过滤、跨学科思维格栅与 5 年逆向事前验尸。\n\
            3) 针对我提出的质疑进行一轮费曼答辩追问；保留 workbench/dossiers/{ticker}.json 中的 human_judgment 待我本人拍板，严禁输出买卖指令。",
            bucket.code()
        ),
        crate::i18n::UiLang::En => format!(
            "Act as Alan, my interdisciplinary research chief of staff, and run the interrogation workflow for [{ticker} · {name}] (tier: {}):\n\
            1) Run `./apps/m3-desktop/m3 report {ticker}` and `./apps/m3-desktop/m3 dossier {ticker}` first for deterministic financials and existing notes (never alter or fabricate numbers).\n\
            2) Update the 3-sentence business filter, multidisciplinary latticework, and 5-year pre-mortem from primary filings.\n\
            3) Press me with one Feynman interrogation round; preserve human_judgment in workbench/dossiers/{ticker}.json for my decision only and never emit trade orders.",
            bucket.code()
        ),
    }
}

pub(super) fn dec_to_f64(d: Decimal) -> Option<f64> {
    d.to_string().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Computes `(y_min, y_max, max_volume)` for OHLC candlestick rendering.
/// Includes Munger Strike Price and Fair Value band smoothly when within reasonable proximity
/// of historical prices so candlesticks never get flattened.
pub fn compute_kline_domain(
    bars: &[OhlcBar],
    strike: Option<Decimal>,
    fv_high: Option<Decimal>,
) -> Option<(f64, f64, u64)> {
    if bars.is_empty() {
        return None;
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut max_vol: u64 = 1;
    for bar in bars {
        if let Some(l) = dec_to_f64(bar.low) {
            lo = lo.min(l);
        }
        if let Some(h) = dec_to_f64(bar.high) {
            hi = hi.max(h);
        }
        max_vol = max_vol.max(bar.volume);
    }
    if !lo.is_finite() || !hi.is_finite() || lo <= 0.0 || hi <= 0.0 {
        return None;
    }
    let orig_lo = lo;
    let orig_hi = hi;
    if let Some(s) = strike.and_then(dec_to_f64)
        && s > 0.0
        && s >= orig_lo * 0.42
        && s <= orig_hi * 2.2
    {
        lo = lo.min(s);
        hi = hi.max(s);
    }
    if let Some(fh) = fv_high.and_then(dec_to_f64)
        && fh > 0.0
        && fh >= orig_lo * 0.42
        && fh <= orig_hi * 2.2
    {
        lo = lo.min(fh);
        hi = hi.max(fh);
    }
    let span = (hi - lo).max(hi * 0.05);
    let pad = span * 0.08;
    Some(((lo - pad).max(0.01), hi + pad, max_vol))
}

pub(super) fn currency_prefix(currency: &str) -> &'static str {
    match currency {
        "USD" => "$",
        "EUR" => "€",
        "HKD" => "HK$",
        "JPY" => "¥",
        "CNY" => "¥",
        _ => "",
    }
}

pub(super) fn fmt_price(val: Option<Decimal>, currency: &str) -> String {
    match val {
        Some(v) => format!("{}{:.2}", currency_prefix(currency), v),
        None => "—".into(),
    }
}

pub(super) fn fmt_money_compact(val: Decimal, currency: &str) -> String {
    let sign = if val < Decimal::ZERO { "-" } else { "" };
    let abs = val.abs();
    let pfx = currency_prefix(currency);
    let billion = Decimal::from(1_000_000_000i64);
    let million = Decimal::from(1_000_000i64);
    if abs >= billion {
        format!("{sign}{pfx}{:.2}B", abs / billion)
    } else if abs >= million {
        format!("{sign}{pfx}{:.1}M", abs / million)
    } else {
        format!("{sign}{pfx}{:.0}", abs)
    }
}

/// Formats an optional monetary amount compactly, returning `"—"` when unavailable.
pub(super) fn fmt_money_compact_opt(val: Option<Decimal>, currency: &str) -> String {
    match val {
        Some(v) => fmt_money_compact(v, currency),
        None => "—".into(),
    }
}

pub(super) fn fmt_shares_compact(val: Decimal) -> String {
    let billion = Decimal::from(1_000_000_000i64);
    let million = Decimal::from(1_000_000i64);
    if val >= billion {
        format!("{:.2}B", val / billion)
    } else if val >= million {
        format!("{:.1}M", val / million)
    } else {
        format!("{:.0}", val)
    }
}

/// Returns `(formatted, polarity)` where `polarity` is `-1`, `0`, or `1`.
/// Magnitudes `< 0.005` clamp to neutral `"0.00%"` (`polarity = 0`).
pub fn fmt_pct_clean(val: Option<Decimal>, signed: bool) -> (String, i8) {
    let Some(v) = val else {
        return ("—".into(), 0);
    };
    let threshold = Decimal::new(5, 3); // 0.005
    if v.abs() < threshold {
        return ("0.00%".into(), 0);
    }
    let pol = if v > Decimal::ZERO { 1 } else { -1 };
    let text = if signed {
        format!("{:+.2}%", v)
    } else {
        format!("{:.2}%", v)
    };
    (text, pol)
}

pub(super) fn civil_ym_from_ms(ms: u64) -> String {
    let days = (ms / 1000) as i64 / 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let yr = y + i64::from(m <= 2);
    format!("{yr:04}-{m:02}")
}

pub(super) fn fmt_time_cn_for(ms: Option<u64>, lang: UiLang) -> String {
    let Some(ms) = ms else {
        return lang.t("未同步", "Never").into();
    };
    let secs = (ms / 1000 + 8 * 3600) as i64;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let yr = y + i64::from(m <= 2);
    format!(
        "{yr:04}-{m:02}-{d:02} {:02}:{:02}",
        tod / 3600,
        (tod % 3600) / 60
    )
}

pub(super) fn cmp_opt_dec(a: Option<Decimal>, b: Option<Decimal>, asc: bool) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => {
            if asc {
                x.cmp(&y)
            } else {
                y.cmp(&x)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub fn sort_reports(rows: &mut [&MungerReport], col: SortCol, asc: bool) {
    rows.sort_by(|a, b| {
        let va = primary_val(a, true);
        let vb = primary_val(b, true);
        let ord = match col {
            SortCol::Default => {
                let rank = |bkt: Bucket| match bkt {
                    Bucket::Core => 0,
                    Bucket::Watch => 1,
                    Bucket::Out => 2,
                    Bucket::TooHard => 3,
                };
                rank(a.bucket).cmp(&rank(b.bucket))
            }
            SortCol::Ticker => {
                if asc {
                    a.ticker.cmp(&b.ticker)
                } else {
                    b.ticker.cmp(&a.ticker)
                }
            }
            SortCol::Price => cmp_opt_dec(
                a.price.as_ref().map(|p| p.price),
                b.price.as_ref().map(|p| p.price),
                asc,
            ),
            SortCol::Margin => cmp_opt_dec(
                va.and_then(|v| v.distance_to_strike_pct),
                vb.and_then(|v| v.distance_to_strike_pct),
                asc,
            ),
            SortCol::Roic => cmp_opt_dec(a.roic_5y_avg_pct, b.roic_5y_avg_pct, asc),
            SortCol::FcfYield => cmp_opt_dec(
                va.and_then(|v| v.strict_fcf_yield_mcap_pct),
                vb.and_then(|v| v.strict_fcf_yield_mcap_pct),
                asc,
            ),
            SortCol::ShareDelta => cmp_opt_dec(a.share_change_3y_pct, b.share_change_3y_pct, asc),
            SortCol::NetDebt => cmp_opt_dec(
                a.net_debt_to_owner_earnings,
                b.net_debt_to_owner_earnings,
                asc,
            ),
        };
        ord.then_with(|| a.ticker.cmp(&b.ticker))
    });
}

pub fn filter_palette_items<'a>(items: &'a [PaletteItem], query: &str) -> Vec<&'a PaletteItem> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return items.iter().collect();
    }
    let terms: Vec<&str> = q.split_whitespace().collect();
    items
        .iter()
        .filter(|it| {
            let hay = format!("{} {} {}", it.group, it.title, it.subtitle).to_ascii_lowercase();
            terms.iter().all(|t| hay.contains(t))
        })
        .collect()
}

pub(super) fn primary_val(rep: &MungerReport, prefer_ttm: bool) -> Option<&ValuationSnapshot> {
    if prefer_ttm {
        rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref())
    } else {
        rep.fy_valuation.as_ref().or(rep.ttm_valuation.as_ref())
    }
}

pub(super) fn primary_row(rep: &MungerReport, prefer_ttm: bool) -> Option<&AnnualMetricRow> {
    if prefer_ttm {
        rep.ttm_row.as_ref().or_else(|| rep.annual_rows.last())
    } else {
        rep.annual_rows.last()
    }
}
