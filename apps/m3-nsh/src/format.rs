//! Pure formatting, sorting, filtering, and memo-parsing helpers for `m3-nsh`.

use crate::stores::types::{
    DeepResearchPerspectiveUi, InboxFilter, MemoSection, PaletteItem, ParsedResearchMemo,
    PerspectiveUiStatus, SidebarDockNotice, SortCol, StatusQuickFilter, StoredAppMessage,
    TodayPulse, TopbarMessage, TopbarMessageAction, TopbarMessageStatus,
};
use crate::theme::{
    BG_WELL, BORDER_SUBTLE, CORAL_BG, CORAL_BORDER, CORAL_FG, EMERALD_BG, EMERALD_BORDER,
    EMERALD_FG, GOLD_FG, INK_MUTED, SKY_BG, SKY_BORDER, SKY_FG, SLATE_BG, SLATE_BORDER, SLATE_FG,
    VIOLET_BG, VIOLET_BORDER, VIOLET_FG,
};
use m3_core::alan_interrogator::{TuringTapeEntry, TuringTapeKind};
use m3_core::budget::ThemeClusterMember;
use m3_core::i18n::UiLang;
use m3_core::lake_l1::{SignalInboxRow, SignalKind, SignalStatus};
use m3_core::lake_l2::BudgetStatusReport;
use m3_core::model::{
    AnnualMetricRow, Bucket, MungerReport, OhlcBar, StrikeZoneStatus, ValuationSnapshot,
    WarningSeverity, primary_circle_tag,
};
use m3_core::research::{DeepResearchEvent, DeepResearchResult};
use m3_core::storage::format_bytes;
use m3_core::universe::{UniverseSyncProgress, UniverseSyncStage};
use m3_core::update::PassiveUpdateNotice;
use rust_decimal::Decimal;
use std::{cmp::Ordering, collections::BTreeMap, path::Path};

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
    lang: UiLang,
) -> String {
    match lang {
        UiLang::Zh => format!(
            "请作为我的芒格跨学科研究幕僚 Alan，执行 [{ticker} · {name}]（当前分桶: {}）的审讯工作流：\n\
            1) 先运行 `m3 report {ticker}` 与 `m3 dossier {ticker}` 获取确定性财务与现有草稿（严禁口算或改动数字）。\n\
            2) 基于 10-K/电话会更新三句话业务过滤、跨学科思维格栅与 5 年逆向事前验尸。\n\
            3) 针对我提出的质疑进行一轮费曼答辩追问；保留 workbench/dossiers/{ticker}.json 中的 human_judgment 待我本人拍板，严禁输出买卖指令。",
            bucket.code()
        ),
        UiLang::En => format!(
            "Act as Alan, my interdisciplinary research chief of staff, and run the interrogation workflow for [{ticker} · {name}] (tier: {}):\n\
            1) Run `m3 report {ticker}` and `m3 dossier {ticker}` first for deterministic financials and existing notes (never alter or fabricate numbers).\n\
            2) Update the 3-sentence business filter, multidisciplinary latticework, and 5-year pre-mortem from primary filings.\n\
            3) Press me with one Feynman interrogation round; preserve human_judgment in workbench/dossiers/{ticker}.json for my decision only and never emit trade orders.",
            bucket.code()
        ),
    }
}

pub fn dec_to_f64(d: Decimal) -> Option<f64> {
    d.to_string().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Computes `(y_min, y_max, max_volume)` for OHLC candlestick rendering.
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

pub fn currency_prefix(currency: &str) -> &'static str {
    match currency {
        "USD" => "$",
        "EUR" => "€",
        "HKD" => "HK$",
        "JPY" => "¥",
        "CNY" => "¥",
        _ => "",
    }
}

pub fn fmt_price(val: Option<Decimal>, currency: &str) -> String {
    match val {
        Some(v) => format!("{}{:.2}", currency_prefix(currency), v),
        None => "—".into(),
    }
}

pub fn fmt_money_compact(val: Decimal, currency: &str) -> String {
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

pub fn fmt_money_compact_opt(val: Option<Decimal>, currency: &str) -> String {
    match val {
        Some(v) => fmt_money_compact(v, currency),
        None => "—".into(),
    }
}

pub fn fmt_shares_compact(val: Decimal) -> String {
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
pub fn fmt_pct_clean(val: Option<Decimal>, signed: bool) -> (String, i8) {
    let Some(v) = val else {
        return ("—".into(), 0);
    };
    let threshold = Decimal::new(5, 3);
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

fn civil_ymd_tuple_from_ms(ms: u64) -> (i64, i64, i64) {
    let days = (ms / 1000) as i64 / 86_400;
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
    (yr, m, d)
}

pub fn civil_ym_from_ms(ms: u64) -> String {
    let (yr, m, _) = civil_ymd_tuple_from_ms(ms);
    format!("{yr:04}-{m:02}")
}

pub fn civil_ymd_from_ms(ms: u64) -> String {
    let (yr, m, d) = civil_ymd_tuple_from_ms(ms);
    format!("{yr:04}-{m:02}-{d:02}")
}

pub fn fmt_volume_compact(vol: u64) -> String {
    if vol >= 1_000_000_000 {
        format!("{:.2}B", vol as f64 / 1_000_000_000.0)
    } else if vol >= 1_000_000 {
        format!("{:.2}M", vol as f64 / 1_000_000.0)
    } else if vol >= 1_000 {
        format!("{:.1}K", vol as f64 / 1_000.0)
    } else {
        format!("{vol}")
    }
}

pub fn fmt_time_cn_for(ms: Option<u64>, lang: UiLang) -> String {
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

pub fn cmp_opt_dec(a: Option<Decimal>, b: Option<Decimal>, asc: bool) -> Ordering {
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
                    Bucket::Watch => 0,
                    Bucket::Core => 1,
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

pub fn primary_val(rep: &MungerReport, prefer_ttm: bool) -> Option<&ValuationSnapshot> {
    if prefer_ttm {
        rep.ttm_valuation.as_ref().or(rep.fy_valuation.as_ref())
    } else {
        rep.fy_valuation.as_ref().or(rep.ttm_valuation.as_ref())
    }
}

pub fn primary_row(rep: &MungerReport, prefer_ttm: bool) -> Option<&AnnualMetricRow> {
    if prefer_ttm {
        rep.ttm_row.as_ref().or_else(|| rep.annual_rows.last())
    } else {
        rep.annual_rows.last()
    }
}

pub fn filter_inbox_signals(
    signals: &[SignalInboxRow],
    filter: InboxFilter,
) -> Vec<&SignalInboxRow> {
    signals
        .iter()
        .filter(|s| match filter {
            InboxFilter::Unread => s.status == SignalStatus::Unread,
            InboxFilter::All => true,
            InboxFilter::Discovery => {
                s.kind == SignalKind::Discovery && s.status != SignalStatus::Archived
            }
            InboxFilter::Ripple => {
                s.kind == SignalKind::Ripple && s.status != SignalStatus::Archived
            }
            InboxFilter::StrikeAndRedFlag => {
                matches!(s.kind, SignalKind::Strike | SignalKind::RedFlag)
                    && s.status != SignalStatus::Archived
            }
            InboxFilter::Archived => s.status == SignalStatus::Archived,
        })
        .collect()
}

pub fn count_inbox_filter(signals: &[SignalInboxRow], filter: InboxFilter) -> usize {
    filter_inbox_signals(signals, filter).len()
}

pub fn resolve_active_inbox_signal<'a>(
    visible: &[&'a SignalInboxRow],
    selected_id: Option<&str>,
    cursor: usize,
) -> Option<(usize, &'a SignalInboxRow)> {
    if visible.is_empty() {
        return None;
    }
    if let Some(id) = selected_id
        && let Some((idx, sig)) = visible.iter().enumerate().find(|(_, s)| s.signal_id == id)
    {
        return Some((idx, *sig));
    }
    let idx = cursor.min(visible.len() - 1);
    Some((idx, visible[idx]))
}

pub fn step_inbox_selection(
    visible: &[&SignalInboxRow],
    selected_id: Option<&str>,
    cursor: usize,
    delta: isize,
) -> Option<(usize, String)> {
    if visible.is_empty() {
        return None;
    }
    let cur_idx = resolve_active_inbox_signal(visible, selected_id, cursor)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let next_idx = (cur_idx as isize + delta).clamp(0, (visible.len() - 1) as isize) as usize;
    Some((next_idx, visible[next_idx].signal_id.clone()))
}

pub fn signal_kind_badge_for(kind: SignalKind, lang: UiLang) -> (&'static str, u32, u32, u32) {
    match kind {
        SignalKind::Discovery => (
            lang.t("量化候选", "Quant Screen"),
            SKY_FG,
            SKY_BG,
            SKY_BORDER,
        ),
        SignalKind::Ripple => (
            lang.t("隐空间传导", "Ripple"),
            VIOLET_FG,
            VIOLET_BG,
            VIOLET_BORDER,
        ),
        SignalKind::Strike => (
            lang.t("击球区", "Strike"),
            EMERALD_FG,
            EMERALD_BG,
            EMERALD_BORDER,
        ),
        SignalKind::RedFlag => (
            lang.t("红灯预警", "Red Flag"),
            CORAL_FG,
            CORAL_BG,
            CORAL_BORDER,
        ),
    }
}

pub fn signal_status_badge_for(
    status: SignalStatus,
    lang: UiLang,
) -> (&'static str, u32, u32, u32) {
    match status {
        SignalStatus::Unread => (lang.t("未读", "Unread"), SKY_FG, SKY_BG, SKY_BORDER),
        SignalStatus::Researching => (
            lang.t("Alan 研判", "Alan"),
            VIOLET_FG,
            VIOLET_BG,
            VIOLET_BORDER,
        ),
        SignalStatus::Watch => (
            lang.t("观察池", "Watch"),
            EMERALD_FG,
            EMERALD_BG,
            EMERALD_BORDER,
        ),
        SignalStatus::TooHard => ("TOO HARD", SLATE_FG, SLATE_BG, SLATE_BORDER),
        SignalStatus::Archived => (
            lang.t("已归档", "Archived"),
            INK_MUTED,
            BG_WELL,
            BORDER_SUBTLE,
        ),
    }
}

pub fn budget_watermark_summary_for(
    report: Option<&BudgetStatusReport>,
    lang: UiLang,
) -> (String, u32) {
    let prefix = lang.t("本月 AI 预算", "AI Budget");
    let Some(r) = report else {
        return (format!("{prefix} $0.00 / $10.00"), EMERALD_FG);
    };
    let used = r.usage.total_cost_usd.round_dp(2).normalize();
    let limit = r.config.monthly_budget_usd.round_dp(2).normalize();
    let exhausted = r.remaining_monthly_usd <= Decimal::ZERO || r.remaining_monthly_tokens == 0;
    let color = if exhausted {
        CORAL_FG
    } else if r.config.monthly_budget_usd > Decimal::ZERO
        && used * Decimal::from(10) >= r.config.monthly_budget_usd * Decimal::from(8)
    {
        GOLD_FG
    } else {
        EMERALD_FG
    };
    (format!("{prefix} ${used:.2} / ${limit:.2}"), color)
}

pub fn should_skip_auto_catchup(lake_present: bool) -> bool {
    let env_val = std::env::var("M3_NO_AUTO_CATCHUP").ok();
    should_skip_auto_catchup_with(cfg!(test), env_val.as_deref(), lake_present)
}

pub fn should_skip_auto_catchup_with(
    is_test: bool,
    no_auto_env: Option<&str>,
    lake_present: bool,
) -> bool {
    if is_test || !lake_present {
        return true;
    }
    no_auto_env.is_some_and(|raw| {
        let trimmed = raw.trim();
        trimmed == "1" || trimmed.eq_ignore_ascii_case("true")
    })
}

pub fn parse_research_memo(raw: &str) -> ParsedResearchMemo {
    let mut parsed = ParsedResearchMemo::default();
    let mut current_heading: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();
    let mut in_preamble = true;

    let flush_section = |parsed: &mut ParsedResearchMemo, heading: String, lines: Vec<String>| {
        let body = lines.join("\n").trim().to_owned();
        let lower = heading.to_ascii_lowercase();
        if lower.contains("evidence & citations") || lower == "citations" {
            let mut cites = Vec::new();
            for line in body.lines() {
                let t = line.trim();
                if t.is_empty() || (t.starts_with('_') && t.ends_with('_')) {
                    continue;
                }
                cites.push(t.to_owned());
            }
            parsed.citations = cites;
        } else if lower.starts_with("human notes") {
            let cleaned: Vec<&str> = body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("<!--"))
                .collect();
            if !cleaned.is_empty() {
                parsed.human_notes = Some(cleaned.join("\n"));
            }
        }

        let id = if lower.contains("(moat)") {
            "moat".to_owned()
        } else if lower.contains("(accounting)") {
            "accounting".to_owned()
        } else if lower.contains("(latent_peers)") {
            "latent_peers".to_owned()
        } else if lower.contains("(premortem)") {
            "premortem".to_owned()
        } else if lower.contains("deterministic financial snapshot") {
            "snapshot".to_owned()
        } else if lower.contains("evidence & citations") {
            "citations".to_owned()
        } else if lower.starts_with("human notes") {
            "human_notes".to_owned()
        } else {
            lower.replace(' ', "_")
        };

        parsed.sections.push(MemoSection { id, heading, body });
    };

    for line in raw.lines() {
        let trimmed = line.trim_end();
        if in_preamble && parsed.title.is_empty() && trimmed.starts_with("# ") {
            parsed.title = trimmed.trim_start_matches("# ").trim().to_owned();
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            in_preamble = false;
            if let Some(prev) = current_heading.replace(heading.trim().to_owned()) {
                let lines = std::mem::take(&mut current_lines);
                flush_section(&mut parsed, prev, lines);
            }
            continue;
        }
        if in_preamble {
            if let Some(rest) = trimmed.trim().strip_prefix("- **")
                && let Some((k, v)) = rest.split_once("**:")
            {
                parsed
                    .metadata
                    .push((k.trim().to_owned(), v.trim().trim_matches('`').to_owned()));
            }
        } else {
            current_lines.push(trimmed.to_owned());
        }
    }

    if let Some(last) = current_heading {
        flush_section(&mut parsed, last, current_lines);
    }

    parsed
}

pub fn format_theme_member_roic(member: &ThemeClusterMember) -> String {
    match member.roic_5y_avg_pct {
        Some(v) => format!("ROIC {v:.0}%"),
        None => "ROIC —".to_owned(),
    }
}

pub fn format_theme_member_distance(member: &ThemeClusterMember) -> String {
    fmt_pct_clean(member.distance_to_strike_pct, true).0
}

pub fn compact_sidebar_version_label(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('v');
    if let Some((base, rest)) = trimmed.split_once("-beta.")
        && let Some((_, build)) = rest.rsplit_once('.')
        && !build.is_empty()
    {
        return format!("v{base}-beta.{build}");
    }
    format!("v{trimmed}")
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_sidebar_dock_notice_for(
    banner: Option<&(String, bool)>,
    universe_syncing: bool,
    universe_progress: Option<&str>,
    inbox_catching_up: bool,
    inbox_progress: Option<&str>,
    deep_research_running: bool,
    industry_research_running: bool,
    industry_progress: Option<&str>,
    syncing: bool,
    themes_syncing: bool,
    update_checking: bool,
    update_applying: bool,
    lang: UiLang,
) -> Option<SidebarDockNotice> {
    if let Some((msg, true)) = banner {
        return Some(SidebarDockNotice {
            tag: "ERROR",
            message: msg.clone(),
            dot_color: CORAL_FG,
            border_color: CORAL_BORDER,
            dismissible: true,
        });
    }
    if update_applying {
        return Some(SidebarDockNotice {
            tag: "UPDATING",
            message: lang
                .t(
                    "正在下载并校验新版 M3.app…",
                    "Downloading and verifying M3.app update…",
                )
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if update_checking {
        let message = match banner {
            Some((msg, false)) => msg.clone(),
            _ => lang
                .t("正在检查新版本…", "Checking for updates…")
                .to_owned(),
        };
        return Some(SidebarDockNotice {
            tag: "UPDATE",
            message,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if universe_syncing {
        return Some(SidebarDockNotice {
            tag: "LAKE SYNC",
            message: universe_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    lang.t(
                        "正在后台同步全美股 L0 确定性湖仓…",
                        "Syncing US L0 deterministic lakehouse…",
                    )
                })
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if inbox_catching_up {
        return Some(SidebarDockNotice {
            tag: "CATCH-UP",
            message: inbox_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| lang.t("正在追平 SEC 事件流…", "Catching up L1 SEC event feed…"))
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if industry_research_running {
        return Some(SidebarDockNotice {
            tag: "INDUSTRY L2",
            message: industry_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    lang.t(
                        "Alan 4 视角行业深研运行中…",
                        "Running 4-perspective industry research…",
                    )
                })
                .to_owned(),
            dot_color: VIOLET_FG,
            border_color: VIOLET_BORDER,
            dismissible: false,
        });
    }
    if deep_research_running {
        return Some(SidebarDockNotice {
            tag: "ALAN L2",
            message: lang
                .t(
                    "4 视角并行深研运行中…",
                    "Running 4-perspective Deep Research…",
                )
                .to_owned(),
            dot_color: VIOLET_FG,
            border_color: VIOLET_BORDER,
            dismissible: false,
        });
    }
    if syncing {
        return Some(SidebarDockNotice {
            tag: "SYNCING",
            message: lang
                .t(
                    "正在同步核心池行情与财务档案…",
                    "Syncing Circle quotes and SEC financials…",
                )
                .to_owned(),
            dot_color: EMERALD_FG,
            border_color: EMERALD_BORDER,
            dismissible: false,
        });
    }
    if themes_syncing {
        return Some(SidebarDockNotice {
            tag: "THEMES",
            message: lang
                .t(
                    "正在同步商业原型主题图谱…",
                    "Syncing business archetype graph…",
                )
                .to_owned(),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            dismissible: false,
        });
    }
    if let Some((msg, false)) = banner {
        return Some(SidebarDockNotice {
            tag: "NOTICE",
            message: msg.clone(),
            dot_color: EMERALD_FG,
            border_color: EMERALD_BORDER,
            dismissible: true,
        });
    }
    None
}

/// Snapshot of store signals synthesized into unified [`TopbarMessage`] entries.
pub struct TopbarMessageInputs<'a> {
    pub lang: UiLang,
    pub update_applied_version: Option<&'a str>,
    pub update_applying: bool,
    pub update_progress: Option<&'a str>,
    pub update_progress_fraction: Option<f32>,
    pub update_checking: bool,
    pub update_notice: Option<&'a PassiveUpdateNotice>,
    pub universe_syncing: bool,
    pub universe_progress: Option<&'a str>,
    pub universe_progress_fraction: Option<f32>,
    pub inbox_catching_up: bool,
    pub inbox_progress: Option<&'a str>,
    pub deep_research_running: bool,
    pub deep_research_ticker: Option<&'a str>,
    pub industry_research_running: bool,
    pub industry_progress: Option<&'a str>,
    pub syncing: bool,
    pub sync_stage: Option<&'a str>,
    pub sync_progress: Option<f32>,
    pub themes_syncing: bool,
    pub storage_scanning: bool,
    pub messages: &'a [StoredAppMessage],
}

/// Builds the ordered unified list of active tasks, update prompts, errors, and notices
/// shown inside the Topbar Message Box pill and popover.
pub fn build_topbar_messages(inputs: &TopbarMessageInputs<'_>) -> Vec<TopbarMessage> {
    let l = inputs.lang;
    let mut out = Vec::new();

    if let Some(ver) = inputs.update_applied_version {
        out.push(TopbarMessage {
            id: None,
            tag: "UPDATE",
            title: match l {
                UiLang::Zh => format!("M3 v{ver} 已就绪，重启以生效"),
                UiLang::En => format!("M3 v{ver} ready — restart to apply"),
            },
            detail: None,
            status: TopbarMessageStatus::ActionRequired,
            progress: Some(1.0),
            dot_color: EMERALD_FG,
            border_color: EMERALD_BORDER,
            action: Some(TopbarMessageAction::RestartAfterUpdate),
            dismissible: false,
        });
    } else if inputs.update_applying {
        out.push(TopbarMessage {
            id: None,
            tag: "UPDATING",
            title: inputs
                .update_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    l.t(
                        "正在下载并校验新版 M3.app…",
                        "Downloading and verifying M3.app update…",
                    )
                })
                .to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: inputs.update_progress_fraction,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            action: None,
            dismissible: false,
        });
    } else if inputs.update_checking {
        out.push(TopbarMessage {
            id: None,
            tag: "UPDATE",
            title: l.t("正在检查软件更新…", "Checking for updates…").to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: None,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            action: None,
            dismissible: false,
        });
    } else if let Some(notice) = inputs.update_notice {
        out.push(TopbarMessage {
            id: None,
            tag: "UPDATE",
            title: match l {
                UiLang::Zh => format!(
                    "发现新版本 v{}（{}）",
                    notice.latest_version,
                    notice.channel.as_str()
                ),
                UiLang::En => format!(
                    "Update v{} available ({})",
                    notice.latest_version,
                    notice.channel.as_str()
                ),
            },
            detail: None,
            status: TopbarMessageStatus::ActionRequired,
            progress: None,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            action: Some(TopbarMessageAction::ApplyUpdate),
            dismissible: false,
        });
    }

    if inputs.universe_syncing {
        out.push(TopbarMessage {
            id: None,
            tag: "LAKE SYNC",
            title: inputs
                .universe_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    l.t(
                        "正在后台同步全美股 L0 确定性湖仓…",
                        "Syncing US L0 deterministic lakehouse…",
                    )
                })
                .to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: inputs.universe_progress_fraction.map(|p| p.clamp(0.0, 1.0)),
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            action: None,
            dismissible: false,
        });
    }
    if inputs.inbox_catching_up {
        out.push(TopbarMessage {
            id: None,
            tag: "CATCH-UP",
            title: inputs
                .inbox_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| l.t("正在追平 SEC 事件流…", "Catching up L1 SEC event feed…"))
                .to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: None,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            action: None,
            dismissible: false,
        });
    }
    if inputs.deep_research_running {
        let title = match (
            l,
            inputs.deep_research_ticker.filter(|t| !t.trim().is_empty()),
        ) {
            (UiLang::Zh, Some(t)) => format!("Alan 4 视角深研 {t} 运行中…"),
            (UiLang::En, Some(t)) => format!("Running 4-lens Deep Research on {t}…"),
            (UiLang::Zh, None) => "4 视角并行深研运行中…".to_owned(),
            (UiLang::En, None) => "Running 4-perspective Deep Research…".to_owned(),
        };
        out.push(TopbarMessage {
            id: None,
            tag: "ALAN L2",
            title,
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: None,
            dot_color: VIOLET_FG,
            border_color: VIOLET_BORDER,
            action: None,
            dismissible: false,
        });
    }
    if inputs.industry_research_running {
        out.push(TopbarMessage {
            id: None,
            tag: "INDUSTRY L2",
            title: inputs
                .industry_progress
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    l.t(
                        "Alan 4 视角行业深研运行中…",
                        "Running 4-perspective industry research…",
                    )
                })
                .to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: None,
            dot_color: VIOLET_FG,
            border_color: VIOLET_BORDER,
            action: None,
            dismissible: false,
        });
    }
    if inputs.syncing {
        out.push(TopbarMessage {
            id: None,
            tag: "SYNCING",
            title: inputs
                .sync_stage
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    l.t(
                        "正在同步核心池行情与财务档案…",
                        "Syncing Circle quotes and SEC financials…",
                    )
                })
                .to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: inputs.sync_progress.map(|p| p.clamp(0.0, 1.0)),
            dot_color: EMERALD_FG,
            border_color: EMERALD_BORDER,
            action: None,
            dismissible: false,
        });
    }
    if inputs.themes_syncing {
        out.push(TopbarMessage {
            id: None,
            tag: "THEMES",
            title: l
                .t(
                    "正在同步商业原型主题图谱…",
                    "Syncing business archetype graph…",
                )
                .to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: None,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            action: None,
            dismissible: false,
        });
    }
    if inputs.storage_scanning {
        out.push(TopbarMessage {
            id: None,
            tag: "STORAGE",
            title: l
                .t("正在扫描本地存储占用…", "Scanning local storage usage…")
                .to_owned(),
            detail: None,
            status: TopbarMessageStatus::Running,
            progress: None,
            dot_color: SKY_FG,
            border_color: SKY_BORDER,
            action: Some(TopbarMessageAction::OpenSettings),
            dismissible: false,
        });
    }

    for msg in inputs.messages {
        let (tag, status, dot_color, border_color) = if msg.is_error {
            ("ERROR", TopbarMessageStatus::Error, CORAL_FG, CORAL_BORDER)
        } else {
            (
                "NOTICE",
                TopbarMessageStatus::Info,
                EMERALD_FG,
                EMERALD_BORDER,
            )
        };
        out.push(TopbarMessage {
            id: Some(msg.id),
            tag,
            title: msg.message.clone(),
            detail: None,
            status,
            progress: None,
            dot_color,
            border_color,
            action: None,
            dismissible: true,
        });
    }

    out
}

fn perspective_title_for(id: &str, fallback_cn: &str, lang: UiLang) -> String {
    match (lang, id) {
        (UiLang::En, "moat") => "Moat".to_owned(),
        (UiLang::En, "accounting") => "Accounting".to_owned(),
        (UiLang::En, "latent_peers") => "Latent Peers".to_owned(),
        (UiLang::En, "premortem") => "5Y Pre-Mortem".to_owned(),
        _ => fallback_cn.to_owned(),
    }
}

/// Applies one `DeepResearchEvent` onto store-backed UI state, returning `(reload_context, banner)`.
#[allow(clippy::too_many_arguments)]
pub fn apply_deep_research_event_compat(
    running: &mut bool,
    run_id: &mut Option<String>,
    status_note: &mut Option<String>,
    budget_blocked: &mut Option<String>,
    perspectives: &mut [DeepResearchPerspectiveUi],
    last_result: &mut Option<DeepResearchResult>,
    tape: &mut Vec<TuringTapeEntry>,
    ev: &DeepResearchEvent,
    lang: UiLang,
) -> (bool, Option<(String, bool)>) {
    match ev {
        DeepResearchEvent::Started {
            run_id: rid,
            ticker,
        } => {
            *running = true;
            *run_id = Some(rid.clone());
            *budget_blocked = None;
            *status_note = Some(match lang {
                UiLang::Zh => format!("正在启动 4 视角并行深研 ({ticker})…"),
                UiLang::En => format!("Starting 4-perspective parallel Deep Research ({ticker})…"),
            });
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id: Some(rid.clone()),
                kind: TuringTapeKind::Input,
                summary: match lang {
                    UiLang::Zh => format!("Deep Research 启动 · {ticker} ({rid})"),
                    UiLang::En => format!("Deep Research started · {ticker} ({rid})"),
                },
                detail: Some(
                    lang.t(
                        "4 并行视角：护城河 / 会计排雷 / 隐空间同行 / 5年逆向验尸",
                        "4 parallel lenses: Moat / Accounting / Latent Peers / 5Y Pre-Mortem",
                    )
                    .to_owned(),
                ),
                status: "started".to_owned(),
            });
            (false, None)
        }
        DeepResearchEvent::BudgetBlocked {
            run_id: rid,
            ticker,
            reason,
        } => {
            *running = false;
            *run_id = Some(rid.clone());
            *budget_blocked = Some(reason.clone());
            *status_note = Some(match lang {
                UiLang::Zh => format!("预算闸门硬拦截：{reason}"),
                UiLang::En => format!("Budget gate blocked: {reason}"),
            });
            for p in perspectives.iter_mut() {
                if p.status == PerspectiveUiStatus::Pending {
                    p.status = PerspectiveUiStatus::Failed;
                }
            }
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id: Some(rid.clone()),
                kind: TuringTapeKind::Policy,
                summary: match lang {
                    UiLang::Zh => format!("预算硬拦截 · {ticker}（0 LLM Token）"),
                    UiLang::En => format!("Budget blocked · {ticker} (0 LLM tokens)"),
                },
                detail: Some(reason.clone()),
                status: "blocked_budget".to_owned(),
            });
            (
                true,
                Some((
                    match lang {
                        UiLang::Zh => format!("{ticker} Deep Research 已被预算闸门拦截：{reason}"),
                        UiLang::En => {
                            format!("{ticker} Deep Research blocked by budget gate: {reason}")
                        }
                    },
                    true,
                )),
            )
        }
        DeepResearchEvent::PerspectiveStarted { perspective, label } => {
            let localized_label = perspective_title_for(perspective, label, lang);
            *status_note = Some(match lang {
                UiLang::Zh => format!("4 视角并行推进中 · {localized_label}…"),
                UiLang::En => format!("4-perspective run in progress · {localized_label}…"),
            });
            if let Some(p) = perspectives.iter_mut().find(|item| item.id == *perspective) {
                p.status = PerspectiveUiStatus::Running;
            }
            (false, None)
        }
        DeepResearchEvent::ToolStarted {
            perspective,
            name,
            title,
        } => {
            if let Some(p) = perspectives.iter_mut().find(|item| item.id == *perspective)
                && !p.tools_used.iter().any(|t| t == name)
            {
                p.tools_used.push(name.clone());
            }
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id: Some(format!("dr-{perspective}-{name}-{seq}")),
                kind: TuringTapeKind::Tool,
                summary: if title.is_empty() || title == name {
                    format!("[{perspective}] {name}")
                } else {
                    format!("[{perspective}] {name} · {title}")
                },
                detail: None,
                status: "running".to_owned(),
            });
            (false, None)
        }
        DeepResearchEvent::ToolFinished {
            perspective,
            name,
            status,
        } => {
            let is_denied = status.contains("denied") || status.contains("blocked");
            let prefix = format!("[{perspective}] {name}");
            if let Some(row) = tape
                .iter_mut()
                .rev()
                .find(|r| r.summary.starts_with(&prefix) && r.status == "running")
            {
                row.status = status.clone();
                if is_denied {
                    row.kind = TuringTapeKind::Policy;
                }
            } else {
                let seq = tape.len() + 1;
                tape.push(TuringTapeEntry {
                    seq,
                    call_id: None,
                    kind: if is_denied {
                        TuringTapeKind::Policy
                    } else {
                        TuringTapeKind::Tool
                    },
                    summary: prefix,
                    detail: None,
                    status: status.clone(),
                });
            }
            (false, None)
        }
        DeepResearchEvent::PerspectiveDelta { perspective, chunk } => {
            if let Some(p) = perspectives.iter_mut().find(|item| item.id == *perspective) {
                p.status = PerspectiveUiStatus::Running;
                p.stream_text.push_str(chunk);
            }
            (false, None)
        }
        DeepResearchEvent::PerspectiveFinished {
            perspective,
            summary_chars,
        } => {
            if let Some(p) = perspectives.iter_mut().find(|item| item.id == *perspective) {
                p.status = PerspectiveUiStatus::Completed;
                p.summary_chars = *summary_chars;
            }
            (false, None)
        }
        DeepResearchEvent::Synthesizing { ticker } => {
            *status_note = Some(match lang {
                UiLang::Zh => {
                    format!("正在合成 {ticker} Living Research Memo（保留 ## Human Notes）…")
                }
                UiLang::En => {
                    format!(
                        "Synthesizing {ticker} Living Research Memo (preserving ## Human Notes)…"
                    )
                }
            });
            (false, None)
        }
        DeepResearchEvent::Completed { result } => {
            *running = false;
            *run_id = Some(result.run_id.clone());
            *last_result = Some((**result).clone());
            for outcome in &result.perspectives {
                if let Some(p) = perspectives
                    .iter_mut()
                    .find(|item| item.id == outcome.perspective)
                {
                    p.status = PerspectiveUiStatus::Completed;
                    p.tools_used = outcome.tools_used.clone();
                    if p.stream_text.is_empty() {
                        p.stream_text = outcome.content.clone();
                    }
                    p.summary_chars = outcome.content.chars().count();
                    p.input_tokens = outcome.input_tokens;
                    p.output_tokens = outcome.output_tokens;
                }
            }
            let cost_str = result.cost_usd.round_dp(4).normalize();
            let memo_label = result.memo_path.as_deref().unwrap_or("workbench/memos");
            *status_note = Some(match lang {
                UiLang::Zh => format!(
                    "完成 · {} in / {} out · ${cost_str} · {memo_label}",
                    result.input_tokens, result.output_tokens
                ),
                UiLang::En => format!(
                    "Completed · {} in / {} out · ${cost_str} · {memo_label}",
                    result.input_tokens, result.output_tokens
                ),
            });
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id: Some(result.run_id.clone()),
                kind: TuringTapeKind::Out,
                summary: match lang {
                    UiLang::Zh => format!(
                        "Deep Research 完成 · {} (${cost_str} · {} 引用)",
                        result.ticker,
                        result.citations.len()
                    ),
                    UiLang::En => format!(
                        "Deep Research completed · {} (${cost_str} · {} citations)",
                        result.ticker,
                        result.citations.len()
                    ),
                },
                detail: Some(memo_label.to_owned()),
                status: "completed".to_owned(),
            });
            (
                true,
                Some((
                    match lang {
                        UiLang::Zh => format!(
                            "{} Deep Research 完成（{} in / {} out · ${cost_str}）· 已写入 {memo_label}",
                            result.ticker, result.input_tokens, result.output_tokens
                        ),
                        UiLang::En => format!(
                            "{} Deep Research complete ({} in / {} out · ${cost_str}) · wrote {memo_label}",
                            result.ticker, result.input_tokens, result.output_tokens
                        ),
                    },
                    false,
                )),
            )
        }
        DeepResearchEvent::Failed {
            run_id: rid,
            ticker,
            error,
        } => {
            *running = false;
            if !rid.is_empty() {
                *run_id = Some(rid.clone());
            }
            *status_note = Some(match lang {
                UiLang::Zh => format!("Deep Research 未完成：{error}"),
                UiLang::En => format!("Deep Research incomplete: {error}"),
            });
            for p in perspectives.iter_mut() {
                if matches!(
                    p.status,
                    PerspectiveUiStatus::Pending | PerspectiveUiStatus::Running
                ) {
                    p.status = PerspectiveUiStatus::Failed;
                }
            }
            let seq = tape.len() + 1;
            tape.push(TuringTapeEntry {
                seq,
                call_id: (!rid.is_empty()).then(|| rid.clone()),
                kind: TuringTapeKind::Error,
                summary: match lang {
                    UiLang::Zh => format!("Deep Research 失败 · {ticker}"),
                    UiLang::En => format!("Deep Research failed · {ticker}"),
                },
                detail: Some(error.clone()),
                status: "failed".to_owned(),
            });
            (
                true,
                Some((
                    match lang {
                        UiLang::Zh => format!("{ticker} Deep Research 失败：{error}"),
                        UiLang::En => format!("{ticker} Deep Research failed: {error}"),
                    },
                    true,
                )),
            )
        }
    }
}

/// Formats a live [`m3_core::update::UpdateProgress`] payload for Settings & Sidebar Dock.
pub fn format_update_progress(p: &m3_core::update::UpdateProgress, lang: UiLang) -> String {
    use m3_core::storage::format_bytes;
    use m3_core::update::UpdateStage;

    match p.stage {
        UpdateStage::Resolving => lang
            .t("正在获取版本清单…", "Resolving release manifest…")
            .to_owned(),
        UpdateStage::Downloading => match p.total_bytes {
            Some(total) if total > 0 => {
                let pct =
                    ((p.downloaded_bytes.min(total) as f64 / total as f64) * 100.0).round() as u32;
                let got = format_bytes(p.downloaded_bytes);
                let all = format_bytes(total);
                match lang {
                    UiLang::Zh => format!("下载中 {got} / {all} ({pct}%)"),
                    UiLang::En => format!("Downloading {got} / {all} ({pct}%)"),
                }
            }
            _ => {
                let got = format_bytes(p.downloaded_bytes);
                match lang {
                    UiLang::Zh => format!("下载中 {got}…"),
                    UiLang::En => format!("Downloading {got}…"),
                }
            }
        },
        UpdateStage::Verifying => lang.t("正在校验 SHA-256…", "Verifying SHA-256…").to_owned(),
        UpdateStage::Extracting => lang.t("正在解压 M3.app…", "Extracting M3.app…").to_owned(),
        UpdateStage::Installing => lang.t("正在原子替换安装…", "Installing update…").to_owned(),
        UpdateStage::Completed => lang.t("安装完成", "Install complete").to_owned(),
    }
}

/// Formats a live [`UniverseSyncProgress`] update for the Topbar Message Box and Radar banner.
pub fn format_universe_sync_progress(prog: &UniverseSyncProgress, lang: UiLang) -> String {
    let pct = (prog.fraction.clamp(0.0, 1.0) * 100.0).round() as u32;
    match lang {
        UiLang::Zh => match prog.stage {
            UniverseSyncStage::Index => {
                if let Some(dl) = prog.downloaded_bytes {
                    if let Some(tot) = prog.total_bytes.filter(|&t| t > 0) {
                        format!(
                            "拉取 SEC 索引 {} / {} ({pct}%)",
                            format_bytes(dl),
                            format_bytes(tot)
                        )
                    } else {
                        format!("拉取 SEC 索引 {} ({pct}%)", format_bytes(dl))
                    }
                } else {
                    format!("{} ({pct}%)", prog.message)
                }
            }
            UniverseSyncStage::DownloadSec => {
                if let Some(dl) = prog.downloaded_bytes {
                    if let Some(tot) = prog.total_bytes.filter(|&t| t > 0) {
                        format!(
                            "下载全美股财报包 {} / {} ({pct}%)",
                            format_bytes(dl),
                            format_bytes(tot)
                        )
                    } else {
                        format!("下载全美股财报包 {} ({pct}%)", format_bytes(dl))
                    }
                } else {
                    format!("{} ({pct}%)", prog.message)
                }
            }
            UniverseSyncStage::ParseAndCommit => format!("{} ({pct}%)", prog.message),
            UniverseSyncStage::Quotes => {
                if let (Some(done), Some(tot)) = (prog.quotes_done, prog.quotes_total) {
                    format!("L0 已就绪 · 补充 Top 候选报价 {done}/{tot} ({pct}%)")
                } else {
                    format!("{} ({pct}%)", prog.message)
                }
            }
            UniverseSyncStage::Exporting => format!("L0 已就绪 · 导出 Parquet 视图 ({pct}%)"),
        },
        UiLang::En => match prog.stage {
            UniverseSyncStage::Index => {
                if let Some(dl) = prog.downloaded_bytes {
                    if let Some(tot) = prog.total_bytes.filter(|&t| t > 0) {
                        format!(
                            "Fetching SEC index {} / {} ({pct}%)",
                            format_bytes(dl),
                            format_bytes(tot)
                        )
                    } else {
                        format!("Fetching SEC index {} ({pct}%)", format_bytes(dl))
                    }
                } else {
                    format!("Preparing US SEC index ({pct}%)…")
                }
            }
            UniverseSyncStage::DownloadSec => {
                if let Some(dl) = prog.downloaded_bytes {
                    if let Some(tot) = prog.total_bytes.filter(|&t| t > 0) {
                        format!(
                            "Downloading SEC XBRL {} / {} ({pct}%)",
                            format_bytes(dl),
                            format_bytes(tot)
                        )
                    } else {
                        format!("Downloading SEC XBRL {} ({pct}%)", format_bytes(dl))
                    }
                } else {
                    format!("Checking SEC bulk archive ({pct}%)…")
                }
            }
            UniverseSyncStage::ParseAndCommit => {
                if let Some(n) = prog.parsed_companies {
                    format!("Parsing US XBRL issuers ({n}) ({pct}%)…")
                } else {
                    format!("Parsing US XBRL lakehouse ({pct}%)…")
                }
            }
            UniverseSyncStage::Quotes => {
                if let (Some(done), Some(tot)) = (prog.quotes_done, prog.quotes_total) {
                    format!("Lake ready · fetching Top quotes {done}/{tot} ({pct}%)")
                } else {
                    format!("Lake ready · syncing Top pool quotes ({pct}%)…")
                }
            }
            UniverseSyncStage::Exporting => format!("Lake ready · exporting Parquet ({pct}%)…"),
        },
    }
}

/// Target external agent format for one-click MCP config copying in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpClientTarget {
    #[default]
    ClaudeCode,
    Codex,
    JsonClients,
}

fn shell_quote_arg(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b':'))
    {
        return arg.to_owned();
    }
    let escaped = arg.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

/// Builds a ready-to-paste `m3 mcp` client configuration snippet for the active workspace.
pub fn build_mcp_config_snippet(
    target: McpClientTarget,
    workspace_root: &Path,
    has_custom_workspace: bool,
) -> String {
    let ws = workspace_root.display().to_string();
    match target {
        McpClientTarget::ClaudeCode => {
            if has_custom_workspace {
                format!(
                    "claude mcp add m3 -- m3 mcp --workspace {}",
                    shell_quote_arg(&ws)
                )
            } else {
                "claude mcp add m3 -- m3 mcp".to_owned()
            }
        }
        McpClientTarget::Codex => {
            if has_custom_workspace {
                let ws_toml = serde_json::to_string(&ws).unwrap_or_else(|_| {
                    format!("\"{}\"", ws.replace('\\', "\\\\").replace('"', "\\\""))
                });
                format!(
                    "[mcp_servers.m3]\ncommand = \"m3\"\nargs = [\"mcp\", \"--workspace\", {ws_toml}]"
                )
            } else {
                "[mcp_servers.m3]\ncommand = \"m3\"\nargs = [\"mcp\"]".to_owned()
            }
        }
        McpClientTarget::JsonClients => {
            let args: Vec<&str> = if has_custom_workspace {
                vec!["mcp", "--workspace", &ws]
            } else {
                vec!["mcp"]
            };
            let payload = serde_json::json!({
                "mcpServers": {
                    "m3": {
                        "command": "m3",
                        "args": args,
                    }
                }
            });
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| {
                "{\n  \"mcpServers\": {\n    \"m3\": {\n      \"command\": \"m3\",\n      \"args\": [\"mcp\"]\n    }\n  }\n}".to_owned()
            })
        }
    }
}
