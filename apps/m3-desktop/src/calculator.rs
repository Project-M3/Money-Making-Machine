use crate::{
    edgar,
    model::{
        AnnualFinancials, AnnualMetricRow, Bucket, CompanyEntry, FieldProvenance, MungerReport,
        PriceSnapshot, RawCompanyFactsSummary, SandboxResult, StrikeZoneStatus, TtmFinancials,
        ValuationAssumptions, ValuationSnapshot, WarningFlag, WarningSeverity,
    },
};
use rust_decimal::Decimal;

pub const REPORT_SCHEMA_VERSION: u32 = 1;

fn hundred() -> Decimal {
    Decimal::from(100)
}

pub fn decimal_sqrt(value: Decimal) -> Option<Decimal> {
    if value < Decimal::ZERO {
        return None;
    }
    if value.is_zero() {
        return Some(Decimal::ZERO);
    }
    let two = Decimal::from(2);
    let mut guess = if value > Decimal::ONE {
        value / two
    } else {
        Decimal::ONE
    };
    for _ in 0..24 {
        if guess.is_zero() {
            return Some(Decimal::ZERO);
        }
        let next = (guess + value / guess) / two;
        if (next - guess).abs() <= Decimal::new(1, 14) {
            return Some(next);
        }
        guess = next;
    }
    Some(guess)
}

fn mean_and_stddev(values: &[Decimal]) -> Option<(Decimal, Decimal)> {
    if values.is_empty() {
        return None;
    }
    let count = Decimal::from(values.len() as u64);
    let sum: Decimal = values.iter().copied().sum();
    let mean = sum / count;
    let var = values
        .iter()
        .map(|v| {
            let diff = *v - mean;
            diff * diff
        })
        .sum::<Decimal>()
        / count;
    let stddev = decimal_sqrt(var).unwrap_or(Decimal::ZERO);
    Some((mean, stddev))
}

const COMMON_SHARE_CLASS_SUFFIXES: [&str; 4] = ["A", "B", "C", "V"];
const YAHOO_EXCHANGE_DOT_SUFFIXES: [&str; 53] = [
    "PA", "HK", "L", "TO", "CN", "NE", "DE", "F", "BE", "HM", "MU", "DU", "SW", "AS", "BR", "LS",
    "MI", "MC", "OL", "ST", "HE", "CO", "IC", "IR", "VI", "WA", "AT", "BD", "T", "SS", "SZ", "BJ",
    "AX", "NZ", "SI", "KS", "KQ", "TW", "TWO", "NS", "BO", "BK", "JK", "KL", "PS", "VN", "SA",
    "MX", "BA", "SN", "LM", "CR", "TA",
];

fn split_common_class_base(upper: &str) -> (&str, Option<&str>) {
    if let Some((base, suffix)) = upper.split_once('-')
        && COMMON_SHARE_CLASS_SUFFIXES.contains(&suffix)
    {
        return (base, Some(suffix));
    }
    if let Some((base, suffix)) = upper.split_once('.')
        && COMMON_SHARE_CLASS_SUFFIXES.contains(&suffix)
    {
        return (base, Some(suffix));
    }
    (upper, None)
}

fn is_multi_series_note_or_pref(stem: &str, sibling_tickers: &[String]) -> bool {
    if sibling_tickers.is_empty() || stem.len() < 3 {
        return false;
    }
    sibling_tickers.iter().any(|s| {
        let sib = s.trim().to_ascii_uppercase();
        sib.len() == stem.len() + 1
            && sib.starts_with(stem)
            && matches!(sib.as_bytes().last(), Some(b'C'..=b'J' | b'L'..=b'Z'))
            && !matches!(
                (stem, sib.as_str()),
                ("LBTY", "LBTYK")
                    | ("FWON", "FWONK")
                    | ("BATR", "BATRK")
                    | ("DISC", "DISCK")
                    | ("LSXM", "LSXMK")
                    | ("LILA", "LILAK")
            )
    })
}

/// Returns true only when two distinct tickers represent verified common-stock share classes of
/// the same issuer (e.g. `GOOG`/`GOOGL`, `BRK-A`/`BRK-B`, `KELYA`/`KELYB`), never baby-bonds,
/// exchange-traded debentures, preferred shares, or warrants (e.g. `CCZ` vs `CMCSA`).
pub(crate) fn are_verified_common_share_class_siblings(
    a: &str,
    b: &str,
    sibling_tickers: &[String],
) -> bool {
    let au = a.trim().to_ascii_uppercase();
    let bu = b.trim().to_ascii_uppercase();
    if au.is_empty()
        || bu.is_empty()
        || au == bu
        || au.contains('+')
        || au.contains('/')
        || bu.contains('+')
        || bu.contains('/')
    {
        return false;
    }
    if is_non_common_ticker(&au, &[]) || is_non_common_ticker(&bu, &[]) {
        return false;
    }

    let (a_base, a_cls) = split_common_class_base(&au);
    let (b_base, b_cls) = split_common_class_base(&bu);
    if (a_cls.is_some() || b_cls.is_some()) && a_base == b_base {
        return true;
    }
    if au.contains('-') || au.contains('.') || bu.contains('-') || bu.contains('.') {
        return false;
    }

    const DUAL_CLASS_COMMON_PAIRS: [(&str, &str); 10] = [
        ("GOOG", "GOOGL"),
        ("FOX", "FOXA"),
        ("NWS", "NWSA"),
        ("Z", "ZG"),
        ("UA", "UAA"),
        ("CENT", "CENTA"),
        ("WLY", "WLYB"),
        ("RDI", "RDIB"),
        ("CWEN", "CWENA"),
        ("LILA", "LILAK"),
    ];
    if DUAL_CLASS_COMMON_PAIRS
        .iter()
        .any(|(x, y)| (au == *x && bu == *y) || (au == *y && bu == *x))
    {
        return true;
    }

    if au.len() == bu.len()
        && au.len() >= 4
        && au[..au.len() - 1] == bu[..bu.len() - 1]
        && matches!(au.as_bytes().last(), Some(b'A' | b'B' | b'K'))
        && matches!(bu.as_bytes().last(), Some(b'A' | b'B' | b'K'))
    {
        let stem = &au[..au.len() - 1];
        return !is_multi_series_note_or_pref(stem, sibling_tickers);
    }

    false
}

fn is_non_common_syntax_only(upper: &str) -> bool {
    if upper.is_empty() {
        return false;
    }
    if upper.contains('+') || upper.contains('/') {
        return true;
    }
    if let Some((_, suffix)) = upper.split_once('-') {
        return !COMMON_SHARE_CLASS_SUFFIXES.contains(&suffix);
    }
    if let Some((_, suffix)) = upper.split_once('.') {
        return !(COMMON_SHARE_CLASS_SUFFIXES.contains(&suffix)
            || YAHOO_EXCHANGE_DOT_SUFFIXES.contains(&suffix));
    }
    upper.is_ascii()
        && upper.len() >= 5
        && (matches!(upper.as_bytes().last(), Some(b'W' | b'R' | b'U' | b'Z'))
            || upper.ends_with("WW")
            || upper.ends_with("WS")
            || upper.ends_with("WT"))
}

/// Returns true when a ticker aligns with `issuer_name` as a common equity symbol (word,
/// prefix, ordered consonant stem, or initials), used to keep canonical common selection
/// independent of SEC row ordering when exchange-traded debentures share a CIK.
pub(crate) fn ticker_matches_issuer_name(ticker: &str, issuer_name: &str) -> bool {
    let raw = ticker.trim().to_ascii_uppercase();
    let (base, _) = split_common_class_base(&raw);
    let clean_ticker: String = base.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if clean_ticker.is_empty() {
        return false;
    }
    let words: Vec<String> = issuer_name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|w| w.to_ascii_uppercase())
        .filter(|w| {
            !w.is_empty()
                && !matches!(
                    w.as_str(),
                    "CORP"
                        | "CORPORATION"
                        | "INC"
                        | "INCORPORATED"
                        | "CO"
                        | "COMPANY"
                        | "LTD"
                        | "LIMITED"
                        | "LLC"
                        | "LP"
                        | "PLC"
                        | "GROUP"
                        | "HOLDINGS"
                        | "HOLDING"
                        | "THE"
                        | "OF"
                        | "AND"
                )
        })
        .collect();
    if words.is_empty() {
        return false;
    }
    if words.iter().any(|w| w == &clean_ticker) {
        return true;
    }
    let first = &words[0];
    let stem =
        if clean_ticker.len() >= 4 && matches!(clean_ticker.as_bytes().last(), Some(b'A' | b'B')) {
            &clean_ticker[..clean_ticker.len() - 1]
        } else {
            clean_ticker.as_str()
        };
    if stem.len() >= 3 && first.starts_with(stem) {
        return true;
    }
    if words.len() >= clean_ticker.len()
        && clean_ticker.len() >= 2
        && words
            .iter()
            .zip(clean_ticker.bytes())
            .all(|(w, b)| w.as_bytes().first() == Some(&b))
    {
        return true;
    }
    if stem.len() >= 3 && first.as_bytes().first() == stem.as_bytes().first() {
        let mut idx = 0usize;
        let sb = stem.as_bytes();
        for &b in first.as_bytes() {
            if b == sb[idx] {
                idx += 1;
                if idx == sb.len() {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn is_non_common_ticker(ticker: &str, sibling_tickers: &[String]) -> bool {
    let upper = ticker.trim().to_ascii_uppercase();
    if is_non_common_syntax_only(&upper) {
        return true;
    }
    if upper.is_empty() || !upper.is_ascii() {
        return false;
    }
    let norm_siblings: Vec<String> = sibling_tickers
        .iter()
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect();

    if norm_siblings.iter().any(|sib| {
        sib.len() < upper.len()
            && upper.starts_with(sib)
            && matches!(
                &upper[sib.len()..],
                "W" | "WW" | "WS" | "WT" | "WU" | "WZ" | "R" | "U" | "Z"
            )
    }) {
        return true;
    }

    if norm_siblings.iter().any(|sib| {
        (2..=4).contains(&sib.len())
            && !is_non_common_syntax_only(sib)
            && sib.len() < upper.len()
            && matches!(upper.len() - sib.len(), 1 | 2)
            && upper.starts_with(sib)
            && !are_verified_common_share_class_siblings(&upper, sib, &norm_siblings)
    }) {
        return true;
    }

    // If a sibling is `<stem>-P*` or `<stem>-WT*` and `<stem>` itself is listed, non-`<stem>`
    // plain tickers not verified as common share-class siblings (e.g. AMJB vs JPM/JPM-PC) are
    // supplemental notes/ETNs.
    let has_pref_anchored_root = norm_siblings.iter().any(|root| {
        root != &upper
            && !is_non_common_syntax_only(root)
            && norm_siblings.iter().any(|s| {
                s.strip_prefix(root.as_str())
                    .and_then(|rest| rest.strip_prefix('-'))
                    .is_some_and(|suf| {
                        suf.starts_with('P') || suf.starts_with("WT") || suf.starts_with("WS")
                    })
            })
            && !are_verified_common_share_class_siblings(&upper, root, &norm_siblings)
    });
    if has_pref_anchored_root {
        return true;
    }

    // Exchange-traded debenture / baby-bond symbol ending in `'Z'` (e.g. CCZ vs CMCSA, RZC vs
    // RGA) alongside a non-`Z` plain common sibling under the same CIK.
    if upper.len() >= 3
        && upper.ends_with('Z')
        && norm_siblings.iter().any(|sib| {
            sib != &upper
                && !is_non_common_syntax_only(sib)
                && !sib.ends_with('Z')
                && !are_verified_common_share_class_siblings(&upper, sib, &norm_siblings)
        })
    {
        return true;
    }

    false
}

pub(crate) const UNRESOLVED_SHARE_BASIS_TITLE: &str = "权威普通股代码待确认";

pub(crate) fn is_unresolved_facts_ticker(ticker: &str) -> bool {
    let trimmed = ticker.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case(crate::edgar::UNRESOLVED_CANONICAL_TICKER)
}

pub(crate) fn is_supplemental_security_ticker(ticker: &str, common_ticker: &str) -> bool {
    let upper = ticker.trim().to_ascii_uppercase();
    let base = common_ticker.trim().to_ascii_uppercase();
    if base.eq_ignore_ascii_case(crate::edgar::UNRESOLVED_CANONICAL_TICKER) {
        return true;
    }
    if upper.is_empty() || upper == base {
        return false;
    }
    if upper.contains('+') || upper.contains('/') {
        return true;
    }
    if let Some((_, suffix)) = upper.split_once('-') {
        if suffix.starts_with('P')
            || suffix.starts_with("WT")
            || suffix.starts_with("WS")
            || matches!(suffix, "W" | "WW" | "U" | "UN" | "R" | "RT" | "Z")
        {
            return true;
        }
        if COMMON_SHARE_CLASS_SUFFIXES.contains(&suffix) || suffix == "NEW" {
            return false;
        }
    } else if let Some((_, suffix)) = upper.split_once('.') {
        if YAHOO_EXCHANGE_DOT_SUFFIXES.contains(&suffix) {
            return !base.is_empty()
                && !are_verified_common_share_class_siblings(&upper, &base, &[]);
        }
        if suffix.starts_with("PR")
            || suffix.starts_with("PF")
            || suffix.starts_with("WT")
            || suffix.starts_with("WS")
            || matches!(suffix, "W" | "WW" | "U" | "UN" | "R" | "RT" | "Z")
        {
            return true;
        }
    }
    if !base.is_empty() {
        return is_non_common_ticker(&upper, std::slice::from_ref(&base))
            || !are_verified_common_share_class_siblings(&upper, &base, &[]);
    }
    upper.is_ascii()
        && upper.len() >= 5
        && (upper.ends_with("WW") || upper.ends_with("WS") || upper.ends_with("WT"))
}

pub(crate) fn is_share_basis_mismatched(ticker: &str, is_non_common_listing: bool) -> bool {
    is_non_common_listing
        || ticker.eq_ignore_ascii_case("BRK-B")
        || ticker.eq_ignore_ascii_case("BRK.B")
        || is_non_common_ticker(ticker, &[])
}

pub(crate) fn latest_summary_filing_end(facts: &RawCompanyFactsSummary) -> Option<&str> {
    facts
        .ttm
        .as_ref()
        .map(|t| t.as_of_end.as_str())
        .or_else(|| facts.annuals.last().map(|a| a.period_end.as_str()))
}

struct RowInputs {
    label: String,
    period_end: String,
    revenue: Decimal,
    gross_profit: Decimal,
    ebit: Decimal,
    tax_expense: Decimal,
    pretax_income: Decimal,
    ocf: Decimal,
    capex: Decimal,
    sbc: Decimal,
    buybacks: Decimal,
    cash_and_st: Option<Decimal>,
    total_debt: Decimal,
    equity: Decimal,
    diluted_shares: Decimal,
    tax_unavailable: bool,
    sbc_unavailable: bool,
    buybacks_unavailable: bool,
    debt_unavailable: bool,
    cash_unavailable: bool,
}

fn compute_row(inp: RowInputs) -> AnnualMetricRow {
    let gross_margin_pct = if inp.revenue > Decimal::ZERO {
        Some((inp.gross_profit / inp.revenue) * hundred())
    } else {
        None
    };

    let effective_tax_rate = if inp.pretax_income > Decimal::ZERO {
        (inp.tax_expense / inp.pretax_income).clamp(Decimal::ZERO, Decimal::new(35, 2))
    } else {
        Decimal::new(21, 2)
    };

    let cash_unavailable = inp.cash_unavailable || inp.cash_and_st.is_none();
    let cash_val = inp.cash_and_st.unwrap_or(Decimal::ZERO);
    let nopat_val = inp.ebit * (Decimal::ONE - effective_tax_rate);
    let invested_capital_val = inp.total_debt + inp.equity - cash_val;
    let (roic_pct, roic_note) = if inp.tax_unavailable {
        (None, Some("tax_expense_unavailable".to_owned()))
    } else if inp.debt_unavailable {
        (None, Some("total_debt_unavailable".to_owned()))
    } else if cash_unavailable {
        (None, Some("cash_and_st_unavailable".to_owned()))
    } else if invested_capital_val > Decimal::ZERO {
        (Some((nopat_val / invested_capital_val) * hundred()), None)
    } else {
        (None, Some("net_cash_exceeds_book_capital".to_owned()))
    };

    let total_capital = inp.total_debt + inp.equity;
    let roic_total_capital_pct =
        if !inp.tax_unavailable && !inp.debt_unavailable && total_capital > Decimal::ZERO {
            Some((nopat_val / total_capital) * hundred())
        } else {
            None
        };

    let fcf = inp.ocf - inp.capex;
    let strict_oe_val = inp.ocf - inp.capex - inp.sbc;
    let sbc_to_ocf_pct = if !inp.sbc_unavailable && inp.ocf > Decimal::ZERO {
        Some((inp.sbc / inp.ocf) * hundred())
    } else {
        None
    };
    let net_debt_val = inp.total_debt - cash_val;

    AnnualMetricRow {
        label: inp.label,
        period_end: inp.period_end,
        revenue: inp.revenue,
        gross_profit: inp.gross_profit,
        gross_margin_pct,
        ebit: inp.ebit,
        nopat: (!inp.tax_unavailable).then_some(nopat_val),
        invested_capital: (!inp.debt_unavailable && !cash_unavailable)
            .then_some(invested_capital_val),
        roic_pct,
        roic_note,
        roic_total_capital_pct,
        ocf: inp.ocf,
        capex: inp.capex,
        fcf,
        sbc: (!inp.sbc_unavailable).then_some(inp.sbc),
        strict_owner_earnings: (!inp.sbc_unavailable).then_some(strict_oe_val),
        sbc_to_ocf_pct,
        buybacks: (!inp.buybacks_unavailable).then_some(inp.buybacks),
        cash_and_st: (!cash_unavailable).then_some(inp.cash_and_st).flatten(),
        total_debt: (!inp.debt_unavailable).then_some(inp.total_debt),
        net_debt: (!inp.debt_unavailable && !cash_unavailable).then_some(net_debt_val),
        equity: inp.equity,
        diluted_shares: inp.diluted_shares,
    }
}

pub(crate) fn has_absent_provenance(prov: &[FieldProvenance], fields: &[&str]) -> bool {
    prov.iter()
        .any(|p| fields.contains(&p.field.as_str()) && p.source_tag == "absent")
}

pub(crate) fn prov_has_share_mismatch(prov: &[FieldProvenance]) -> bool {
    prov.iter().any(|p| {
        matches!(p.field.as_str(), "diluted_shares" | "ttm_diluted_shares")
            && p.note
                .as_deref()
                .is_some_and(|n| n.contains("share_basis_magnitude_mismatch"))
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ValuationGuards {
    pub strict_oe_available: bool,
    pub share_basis_valid: bool,
    pub debt_available: bool,
    pub cash_available: bool,
}

impl Default for ValuationGuards {
    fn default() -> Self {
        Self {
            strict_oe_available: true,
            share_basis_valid: true,
            debt_available: true,
            cash_available: true,
        }
    }
}

pub(crate) fn guards_from_provenance(
    ticker: &str,
    prov: &[FieldProvenance],
    diluted_shares: Decimal,
    ending_shares: Option<Decimal>,
) -> ValuationGuards {
    let strict_oe_available = !has_absent_provenance(prov, &["ttm_sbc", "sbc"]);
    let share_basis_valid = !is_share_basis_mismatched(ticker, false)
        && !prov_has_share_mismatch(prov)
        && !ending_shares
            .is_some_and(|ending| edgar::has_share_magnitude_mismatch(diluted_shares, ending));
    let debt_available = !has_absent_provenance(prov, &["ttm_total_debt", "total_debt"]);
    let cash_available = !has_absent_provenance(prov, &["ttm_cash_and_st", "cash_and_st"]);
    ValuationGuards {
        strict_oe_available,
        share_basis_valid,
        debt_available,
        cash_available,
    }
}

pub(crate) fn are_consecutive_fiscal_rows(rows_asc: &[&AnnualMetricRow]) -> bool {
    if rows_asc.len() <= 1 {
        return true;
    }
    for pair in rows_asc.windows(2) {
        let (Ok(d0), Ok(d1)) = (
            edgar::parse_date_days(&pair[0].period_end),
            edgar::parse_date_days(&pair[1].period_end),
        ) else {
            return false;
        };
        if !(300..=430).contains(&(d1 - d0)) {
            return false;
        }
    }
    true
}

pub(crate) fn last_5_years_consecutive(rows: &[AnnualMetricRow]) -> bool {
    if rows.len() < 5 {
        return false;
    }
    let slice: Vec<&AnnualMetricRow> = rows[rows.len() - 5..].iter().collect();
    are_consecutive_fiscal_rows(&slice)
}

fn from_annual(a: &AnnualFinancials) -> AnnualMetricRow {
    let cash_unavailable =
        a.cash_and_st.is_none() || has_absent_provenance(&a.provenance, &["cash_and_st"]);
    compute_row(RowInputs {
        label: format!("FY{}", a.fiscal_year),
        period_end: a.period_end.clone(),
        revenue: a.revenue,
        gross_profit: a.gross_profit,
        ebit: a.ebit,
        tax_expense: a.tax_expense,
        pretax_income: a.pretax_income,
        ocf: a.ocf,
        capex: a.capex,
        sbc: a.sbc,
        buybacks: a.buybacks,
        cash_and_st: (!cash_unavailable).then_some(a.cash_and_st).flatten(),
        total_debt: a.total_debt,
        equity: a.equity,
        diluted_shares: a.diluted_shares,
        tax_unavailable: has_absent_provenance(&a.provenance, &["tax_expense"]),
        sbc_unavailable: has_absent_provenance(&a.provenance, &["sbc"]),
        buybacks_unavailable: has_absent_provenance(&a.provenance, &["buybacks"]),
        debt_unavailable: has_absent_provenance(&a.provenance, &["total_debt"]),
        cash_unavailable,
    })
}

fn from_ttm(t: &TtmFinancials) -> AnnualMetricRow {
    let cash_unavailable = t.cash_and_st.is_none()
        || has_absent_provenance(&t.provenance, &["ttm_cash_and_st", "cash_and_st"]);
    compute_row(RowInputs {
        label: t.label.clone(),
        period_end: t.as_of_end.clone(),
        revenue: t.revenue,
        gross_profit: t.gross_profit,
        ebit: t.ebit,
        tax_expense: t.tax_expense,
        pretax_income: t.pretax_income,
        ocf: t.ocf,
        capex: t.capex,
        sbc: t.sbc,
        buybacks: t.buybacks,
        cash_and_st: (!cash_unavailable).then_some(t.cash_and_st).flatten(),
        total_debt: t.total_debt,
        equity: t.equity,
        diluted_shares: t.diluted_shares,
        tax_unavailable: has_absent_provenance(&t.provenance, &["ttm_tax_expense", "tax_expense"]),
        sbc_unavailable: has_absent_provenance(&t.provenance, &["ttm_sbc", "sbc"]),
        buybacks_unavailable: has_absent_provenance(&t.provenance, &["ttm_buybacks", "buybacks"]),
        debt_unavailable: has_absent_provenance(&t.provenance, &["ttm_total_debt", "total_debt"]),
        cash_unavailable,
    })
}

pub(crate) fn compute_valuation(
    basis_label: &str,
    row: &AnnualMetricRow,
    price_opt: Option<&PriceSnapshot>,
    assumptions: &ValuationAssumptions,
    guards: ValuationGuards,
) -> ValuationSnapshot {
    let shares = row.diluted_shares;
    let oe_opt = guards
        .strict_oe_available
        .then_some(row.strict_owner_earnings)
        .flatten();
    let per_share_allowed = guards.strict_oe_available && guards.share_basis_valid;
    let oe_per_share = if per_share_allowed && shares > Decimal::ZERO {
        oe_opt.map(|oe| oe / shares)
    } else {
        None
    };

    let fair_value_low = oe_per_share
        .filter(|v| *v > Decimal::ZERO)
        .map(|v| v * assumptions.multiple_low);
    let fair_value_high = oe_per_share
        .filter(|v| *v > Decimal::ZERO)
        .map(|v| v * assumptions.multiple_high);
    let strike_price = if per_share_allowed {
        assumptions.strike_price_override.or(fair_value_low)
    } else {
        None
    };

    let (
        market_cap,
        enterprise_value,
        yield_mcap,
        yield_ev,
        distance,
        implied_growth,
        expected_ret,
    ) = if let Some(p) = price_opt {
        let mcap = if guards.share_basis_valid && shares > Decimal::ZERO {
            Some(p.price * shares)
        } else {
            None
        };
        let ev = if guards.debt_available && guards.cash_available {
            match (mcap, row.net_debt) {
                (Some(m), Some(nd)) => Some(m + nd),
                _ => None,
            }
        } else {
            None
        };
        let ym = if per_share_allowed {
            match (oe_opt, mcap.filter(|m| *m > Decimal::ZERO)) {
                (Some(oe), Some(m)) => Some((oe / m) * hundred()),
                _ => None,
            }
        } else {
            None
        };
        let ye = if per_share_allowed {
            match (oe_opt, ev.filter(|e| *e > Decimal::ZERO)) {
                (Some(oe), Some(e)) => Some((oe / e) * hundred()),
                _ => None,
            }
        } else {
            None
        };
        let dist = strike_price
            .filter(|s| *s > Decimal::ZERO)
            .map(|s| ((p.price - s) / s) * hundred());
        let imp = ym.map(|y| assumptions.target_return_pct - y);
        let exp = ym.map(|y| y + assumptions.expected_growth_pct);
        (mcap, ev, ym, ye, dist, imp, exp)
    } else {
        (None, None, None, None, None, None, None)
    };

    ValuationSnapshot {
        basis_label: basis_label.to_owned(),
        period_end: row.period_end.clone(),
        strict_owner_earnings: oe_opt,
        shares_basis: shares,
        owner_earnings_per_share: oe_per_share,
        market_cap,
        enterprise_value,
        strict_fcf_yield_mcap_pct: yield_mcap,
        strict_fcf_yield_ev_pct: yield_ev,
        fair_value_low,
        fair_value_high,
        strike_price,
        distance_to_strike_pct: distance,
        implied_growth_for_target_pct: implied_growth,
        expected_total_return_pct: expected_ret,
    }
}

pub(crate) fn share_basis_mismatch_warning(ticker: &str, non_common: bool) -> WarningFlag {
    WarningFlag {
        code: "SHARE_BASIS_MISMATCH".to_owned(),
        severity: WarningSeverity::Amber,
        title: if non_common {
            "股本口径与报价类别不一致".to_owned()
        } else {
            "财报稀释股本与流通股本口径异常".to_owned()
        },
        detail: if non_common {
            format!(
                "{ticker} 属于非普通股或与财报稀释股本口径不一致（如 BRK-B/权证），已抑制每股估值"
            )
        } else {
            "稀释股本与期末普通股本存在量级差异（可能为千股/普通股混用），已抑制每股估值".to_owned()
        },
    }
}

pub(crate) fn unresolved_share_basis_warning(ticker: &str) -> WarningFlag {
    WarningFlag {
        code: "SHARE_BASIS_MISMATCH".to_owned(),
        severity: WarningSeverity::Amber,
        title: UNRESOLVED_SHARE_BASIS_TITLE.to_owned(),
        detail: format!(
            "{ticker} 暂无法从 SEC 索引或湖仓确认所属发行人权威普通股代码，已暂时抑制每股估值"
        ),
    }
}

pub(crate) fn derive_strike_status(
    bucket: Bucket,
    has_annuals: bool,
    has_valid_price: bool,
    warnings: &[WarningFlag],
    fy_valuation: Option<&ValuationSnapshot>,
    ttm_valuation: Option<&ValuationSnapshot>,
) -> StrikeZoneStatus {
    if bucket == Bucket::TooHard {
        StrikeZoneStatus::TooHard
    } else if bucket == Bucket::Out {
        StrikeZoneStatus::Out
    } else if !has_annuals {
        StrikeZoneStatus::NoFinancials
    } else if warnings.iter().any(|w| w.severity == WarningSeverity::Red) {
        StrikeZoneStatus::WarningBlocked
    } else if !has_valid_price {
        StrikeZoneStatus::Wait
    } else {
        let primary = ttm_valuation.as_ref().or(fy_valuation.as_ref());
        match primary
            .filter(|v| v.strike_price.is_some())
            .and_then(|v| v.distance_to_strike_pct)
        {
            Some(dist) if dist <= Decimal::ZERO => StrikeZoneStatus::InStrikeZone,
            Some(dist) if dist <= Decimal::from(15) => StrikeZoneStatus::NearStrikeZone,
            _ => StrikeZoneStatus::Wait,
        }
    }
}

pub fn evaluate_sandbox(
    owner_earnings_per_share: Option<Decimal>,
    price: Option<Decimal>,
    multiple: Decimal,
    target_return_pct: Decimal,
    expected_growth_pct: Decimal,
) -> SandboxResult {
    let fair_value = owner_earnings_per_share
        .filter(|v| *v > Decimal::ZERO)
        .map(|v| v * multiple);
    let distance_to_strike_pct = match (price, fair_value) {
        (Some(p), Some(fv)) if fv > Decimal::ZERO => Some(((p - fv) / fv) * hundred()),
        _ => None,
    };
    let strict_fcf_yield_pct = match (owner_earnings_per_share, price) {
        (Some(oe), Some(p)) if p > Decimal::ZERO => Some((oe / p) * hundred()),
        _ => None,
    };
    let implied_growth_for_target_pct = strict_fcf_yield_pct.map(|y| target_return_pct - y);
    let expected_total_return_pct = strict_fcf_yield_pct.map(|y| y + expected_growth_pct);

    SandboxResult {
        fair_value,
        distance_to_strike_pct,
        strict_fcf_yield_pct,
        implied_growth_for_target_pct,
        expected_total_return_pct,
    }
}

pub(crate) struct AnnualReportBasis {
    pub row: AnnualMetricRow,
    pub ending_shares: Decimal,
    pub provenance: Vec<FieldProvenance>,
}

pub(crate) struct TtmReportBasis {
    pub row: AnnualMetricRow,
    pub ending_shares: Option<Decimal>,
    pub provenance: Vec<FieldProvenance>,
    pub latest_ytd: crate::model::YtdPeriodFinancials,
}

pub(crate) struct MetricReportInputs {
    pub raw_sha256: Option<String>,
    pub annuals: Vec<AnnualReportBasis>,
    pub ttm: Option<TtmReportBasis>,
}

pub fn build_report(
    company: &CompanyEntry,
    facts: Option<&RawCompanyFactsSummary>,
    price: Option<&PriceSnapshot>,
    computed_at_ms: u64,
) -> MungerReport {
    let inputs = MetricReportInputs {
        raw_sha256: facts.map(|f| f.raw_sha256.clone()),
        annuals: facts
            .map(|f| {
                f.annuals
                    .iter()
                    .map(|a| AnnualReportBasis {
                        row: from_annual(a),
                        ending_shares: a.ending_shares,
                        provenance: a.provenance.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ttm: facts.and_then(|f| {
            f.ttm.as_ref().map(|t| TtmReportBasis {
                row: from_ttm(t),
                ending_shares: Some(t.ending_shares),
                provenance: t.provenance.clone(),
                latest_ytd: t.latest_ytd.clone(),
            })
        }),
    };
    let mut rep = build_report_from_metrics_with_basis(
        company,
        &inputs,
        price,
        computed_at_ms,
        facts.map(|f| f.ticker.as_str()),
    );
    if let Some(f) = facts {
        rep.latest_filing_end = latest_summary_filing_end(f).map(str::to_owned);
    }
    rep
}

pub(crate) fn build_report_from_metrics(
    company: &CompanyEntry,
    inputs: &MetricReportInputs,
    price: Option<&PriceSnapshot>,
    computed_at_ms: u64,
) -> MungerReport {
    build_report_from_metrics_with_basis(company, inputs, price, computed_at_ms, None)
}

fn build_report_from_metrics_with_basis(
    company: &CompanyEntry,
    inputs: &MetricReportInputs,
    price: Option<&PriceSnapshot>,
    computed_at_ms: u64,
    canonical_facts_ticker: Option<&str>,
) -> MungerReport {
    let annual_rows: Vec<AnnualMetricRow> = inputs.annuals.iter().map(|a| a.row.clone()).collect();
    let ttm_row = inputs.ttm.as_ref().map(|t| t.row.clone());
    let latest_ytd = inputs.ttm.as_ref().map(|t| t.latest_ytd.clone());

    let last5_asc: Vec<&AnnualMetricRow> = annual_rows
        .iter()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let consecutive_5y = last5_asc.len() == 5 && are_consecutive_fiscal_rows(&last5_asc);
    let roic_vals: Vec<Decimal> = last5_asc.iter().filter_map(|r| r.roic_pct).collect();
    let (roic_5y_avg_pct, roic_5y_stddev_pct) = if consecutive_5y && roic_vals.len() == 5 {
        mean_and_stddev(&roic_vals)
            .map(|(m, s)| (Some(m), Some(s)))
            .unwrap_or((None, None))
    } else {
        (None, None)
    };

    let gm_vals: Vec<Decimal> = last5_asc
        .iter()
        .filter_map(|r| r.gross_margin_pct)
        .collect();
    let (gross_margin_5y_avg_pct, gross_margin_5y_stddev_pct) =
        if consecutive_5y && gm_vals.len() == 5 {
            mean_and_stddev(&gm_vals)
                .map(|(m, s)| (Some(m), Some(s)))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

    let share_span_ok = |start_idx: usize, end_idx: usize, min_days: i64, max_days: i64| -> bool {
        let slice: Vec<&AnnualMetricRow> = annual_rows[start_idx..=end_idx].iter().collect();
        if !are_consecutive_fiscal_rows(&slice) {
            return false;
        }
        let (Ok(d0), Ok(d1)) = (
            edgar::parse_date_days(&annual_rows[start_idx].period_end),
            edgar::parse_date_days(&annual_rows[end_idx].period_end),
        ) else {
            return false;
        };
        if !(min_days..=max_days).contains(&(d1 - d0)) {
            return false;
        }
        let start_bad = inputs.annuals.get(start_idx).is_some_and(|a| {
            prov_has_share_mismatch(&a.provenance)
                || edgar::has_share_magnitude_mismatch(a.row.diluted_shares, a.ending_shares)
        });
        let end_bad = inputs.annuals.get(end_idx).is_some_and(|a| {
            prov_has_share_mismatch(&a.provenance)
                || edgar::has_share_magnitude_mismatch(a.row.diluted_shares, a.ending_shares)
        });
        !(start_bad || end_bad)
    };

    let share_change_3y_pct = if annual_rows.len() >= 4 {
        let start_idx = annual_rows.len() - 4;
        let end_idx = annual_rows.len() - 1;
        let latest = &annual_rows[end_idx];
        let base = &annual_rows[start_idx];
        if base.diluted_shares > Decimal::ZERO && share_span_ok(start_idx, end_idx, 950, 1220) {
            Some(((latest.diluted_shares - base.diluted_shares) / base.diluted_shares) * hundred())
        } else {
            None
        }
    } else {
        None
    };

    let share_change_5y_pct = if annual_rows.len() >= 6 {
        let start_idx = annual_rows.len() - 6;
        let end_idx = annual_rows.len() - 1;
        let latest = &annual_rows[end_idx];
        let base = &annual_rows[start_idx];
        if base.diluted_shares > Decimal::ZERO && share_span_ok(start_idx, end_idx, 1650, 2000) {
            Some(((latest.diluted_shares - base.diluted_shares) / base.diluted_shares) * hundred())
        } else {
            None
        }
    } else {
        None
    };

    let latest_row = ttm_row.as_ref().or_else(|| annual_rows.last());
    let latest_prov: &[FieldProvenance] = inputs
        .ttm
        .as_ref()
        .map(|t| t.provenance.as_slice())
        .or_else(|| inputs.annuals.last().map(|a| a.provenance.as_slice()))
        .unwrap_or(&[]);
    let debt_unavailable = has_absent_provenance(latest_prov, &["ttm_total_debt", "total_debt"]);
    let cash_unavailable = has_absent_provenance(latest_prov, &["ttm_cash_and_st", "cash_and_st"])
        || latest_row.is_none_or(|r| r.cash_and_st.is_none());
    let sbc_unavailable = has_absent_provenance(latest_prov, &["ttm_sbc", "sbc"]);
    let tax_unavailable = has_absent_provenance(latest_prov, &["ttm_tax_expense", "tax_expense"]);

    let net_debt = (!debt_unavailable && !cash_unavailable)
        .then(|| latest_row.and_then(|r| r.net_debt))
        .flatten();
    let net_debt_to_owner_earnings = if debt_unavailable || cash_unavailable || sbc_unavailable {
        None
    } else {
        latest_row.and_then(|r| match (r.net_debt, r.strict_owner_earnings) {
            (Some(nd), Some(oe)) if oe > Decimal::ZERO => Some(nd / oe),
            _ => None,
        })
    };

    let unresolved_facts_ticker = canonical_facts_ticker.is_some_and(is_unresolved_facts_ticker);
    let supplemental_to_facts = canonical_facts_ticker.is_some_and(|ft| {
        !is_unresolved_facts_ticker(ft) && is_supplemental_security_ticker(&company.ticker, ft)
    });
    let share_basis_blocked = unresolved_facts_ticker || supplemental_to_facts;
    let fy_guards = inputs
        .annuals
        .last()
        .map(|a| {
            let mut g = guards_from_provenance(
                &company.ticker,
                &a.provenance,
                a.row.diluted_shares,
                Some(a.ending_shares),
            );
            g.cash_available &= a.row.cash_and_st.is_some();
            g.share_basis_valid &= !share_basis_blocked;
            g
        })
        .unwrap_or_else(|| {
            let mut g = guards_from_provenance(&company.ticker, &[], Decimal::ZERO, None);
            g.share_basis_valid &= !share_basis_blocked;
            g
        });
    let ttm_guards = inputs
        .ttm
        .as_ref()
        .map(|t| {
            let mut g = guards_from_provenance(
                &company.ticker,
                &t.provenance,
                t.row.diluted_shares,
                t.ending_shares,
            );
            g.cash_available &= t.row.cash_and_st.is_some();
            g.share_basis_valid &= !share_basis_blocked;
            g
        })
        .unwrap_or_else(|| {
            let mut g = guards_from_provenance(&company.ticker, &[], Decimal::ZERO, None);
            g.share_basis_valid &= !share_basis_blocked;
            g
        });
    let active_guards = if ttm_row.is_some() {
        ttm_guards
    } else {
        fy_guards
    };

    let fy_valuation = annual_rows
        .last()
        .map(|row| compute_valuation("FY", row, price, &company.assumptions, fy_guards));
    let ttm_valuation = ttm_row
        .as_ref()
        .map(|row| compute_valuation("TTM", row, price, &company.assumptions, ttm_guards));

    let mut warnings = Vec::new();
    if let Some(row) = latest_row {
        if !sbc_unavailable
            && let Some(strict_oe) = row.strict_owner_earnings
            && strict_oe <= Decimal::ZERO
        {
            warnings.push(WarningFlag {
                code: "NEGATIVE_STRICT_OE".to_owned(),
                severity: WarningSeverity::Red,
                title: "严苛股东盈余为负".to_owned(),
                detail: format!(
                    "{} 期间 OCF - CapEx - SBC = {:.2}M，经营现金流未能覆盖资本开支与股权激励",
                    row.label,
                    strict_oe / Decimal::from(1_000_000)
                ),
            });
        }
        if let Some(sbc_pct) = row.sbc_to_ocf_pct
            && sbc_pct > Decimal::from(25)
        {
            warnings.push(WarningFlag {
                code: "HIGH_SBC_BURDEN".to_owned(),
                severity: WarningSeverity::Amber,
                title: "股权激励 (SBC) 侵蚀显著".to_owned(),
                detail: format!("SBC 占经营现金流 {:.1}%（高于 25% 警戒线）", sbc_pct),
            });
        }
    }

    if let Some(ytd) = &latest_ytd {
        let ytd_flow_absent = inputs
            .ttm
            .as_ref()
            .is_some_and(|t| has_absent_provenance(&t.provenance, &["ytd_ocf", "ytd_capex"]));
        if !ytd_flow_absent
            && let (Some(ytd_ocf), Some(ytd_capex)) = (ytd.ocf, ytd.capex)
            && ytd_capex > ytd_ocf
        {
            warnings.push(WarningFlag {
                code: "YTD_CAPEX_EXCEEDS_OCF".to_owned(),
                severity: WarningSeverity::Red,
                title: "最新季报期资本开支超过经营现金流".to_owned(),
                detail: format!(
                    "{} CapEx ({:.2}M) > OCF ({:.2}M)，自由现金流转为净消耗",
                    ytd.period_label,
                    ytd_capex / Decimal::from(1_000_000),
                    ytd_ocf / Decimal::from(1_000_000)
                ),
            });
        } else if !ytd_flow_absent
            && let (Some(q_ocf), Some(q_capex)) = (ytd.latest_quarter_ocf, ytd.latest_quarter_capex)
            && q_capex > q_ocf
        {
            warnings.push(WarningFlag {
                code: "LATEST_Q_CAPEX_EXCEEDS_OCF".to_owned(),
                severity: WarningSeverity::Red,
                title: "最近单季资本开支超过经营现金流".to_owned(),
                detail: format!(
                    "{} 单季 CapEx ({:.2}M) > OCF ({:.2}M)",
                    ytd.latest_quarter_label.as_deref().unwrap_or("单季"),
                    q_capex / Decimal::from(1_000_000),
                    q_ocf / Decimal::from(1_000_000)
                ),
            });
        }

        let ytd_buybacks_absent = inputs
            .ttm
            .as_ref()
            .is_some_and(|t| has_absent_provenance(&t.provenance, &["ytd_buybacks"]));
        let fy_buybacks_absent = inputs
            .annuals
            .last()
            .is_some_and(|a| has_absent_provenance(&a.provenance, &["buybacks"]));
        if !ytd_buybacks_absent
            && !fy_buybacks_absent
            && ytd.buybacks.is_some_and(|b| b.is_zero())
            && annual_rows
                .last()
                .and_then(|fy| fy.buybacks)
                .is_some_and(|b| b > Decimal::ZERO)
        {
            warnings.push(WarningFlag {
                code: "BUYBACKS_HALTED".to_owned(),
                severity: WarningSeverity::Amber,
                title: "最新财年迄今股票回购归零".to_owned(),
                detail: format!(
                    "{} 期未发生普通股回购，需核查资本配置转向",
                    ytd.period_label
                ),
            });
        }
    }

    if let Some(ch3) = share_change_3y_pct
        && ch3 > Decimal::from(2)
    {
        warnings.push(WarningFlag {
            code: "SHARE_DILUTION".to_owned(),
            severity: WarningSeverity::Amber,
            title: "近 3 年总股本净稀释".to_owned(),
            detail: format!("近 3 年稀释股本净增长 {:+.2}%", ch3),
        });
    }

    if let Some(roic_avg) = roic_5y_avg_pct
        && roic_avg < Decimal::from(15)
    {
        warnings.push(WarningFlag {
            code: "LOW_ROIC".to_owned(),
            severity: WarningSeverity::Amber,
            title: "5 年平均 ROIC 低于 15%".to_owned(),
            detail: format!("5 年 ROIC 均值 {:.2}%，未达到 15% 护城河门槛", roic_avg),
        });
    }

    if let Some(nd_oe) = net_debt_to_owner_earnings
        && nd_oe > Decimal::from(3)
    {
        warnings.push(WarningFlag {
            code: "HIGH_NET_DEBT".to_owned(),
            severity: WarningSeverity::Amber,
            title: "净负债高于 3 年严苛股东盈余".to_owned(),
            detail: format!("净负债 / 严苛股东盈余 = {:.2}x", nd_oe),
        });
    } else if !sbc_unavailable
        && net_debt.is_some_and(|nd| nd > Decimal::ZERO)
        && latest_row
            .and_then(|r| r.strict_owner_earnings)
            .is_some_and(|oe| oe <= Decimal::ZERO)
    {
        warnings.push(WarningFlag {
            code: "NET_DEBT_WITH_NEG_OE".to_owned(),
            severity: WarningSeverity::Amber,
            title: "存在净负债且严苛股东盈余为负".to_owned(),
            detail: "负现金流期间同时背负净债务，需警惕再融资风险".to_owned(),
        });
    }

    if let Some(gm_std) = gross_margin_5y_stddev_pct
        && gm_std > Decimal::from(4)
    {
        warnings.push(WarningFlag {
            code: "UNSTABLE_GROSS_MARGIN".to_owned(),
            severity: WarningSeverity::Amber,
            title: "毛利率 5 年波动偏大".to_owned(),
            detail: format!("5 年毛利率标准差 {:.2}% (> 4%)", gm_std),
        });
    }

    if !active_guards.share_basis_valid {
        warnings.push(if unresolved_facts_ticker {
            unresolved_share_basis_warning(&company.ticker)
        } else {
            share_basis_mismatch_warning(
                &company.ticker,
                supplemental_to_facts || is_share_basis_mismatched(&company.ticker, false),
            )
        });
    }

    if sbc_unavailable || debt_unavailable || cash_unavailable || tax_unavailable {
        let mut missing = Vec::new();
        if tax_unavailable {
            missing.push("所得税");
        }
        if sbc_unavailable {
            missing.push("SBC");
        }
        if debt_unavailable {
            missing.push("总债务");
        }
        if cash_unavailable {
            missing.push("现金及短期投资");
        }
        warnings.push(WarningFlag {
            code: "DATA_UNAVAILABLE".to_owned(),
            severity: WarningSeverity::Info,
            title: "部分 XBRL 科目缺失".to_owned(),
            detail: format!(
                "最新财报未提取到 {} 标签，相关比率按不可用处理",
                missing.join(" / ")
            ),
        });
    }

    let status = derive_strike_status(
        company.bucket,
        !annual_rows.is_empty(),
        price.is_some(),
        &warnings,
        fy_valuation.as_ref(),
        ttm_valuation.as_ref(),
    );

    let provenance_summary = inputs
        .annuals
        .last()
        .map(|a| a.provenance.clone())
        .unwrap_or_default();

    let latest_filing_end = latest_row.map(|r| r.period_end.clone());

    MungerReport {
        schema_version: REPORT_SCHEMA_VERSION,
        computed_at_ms,
        ticker: company.ticker.clone(),
        name: company.name.clone(),
        bucket: company.bucket,
        source: company.source,
        cik: company.cik.clone(),
        currency: company.currency.clone(),
        circle: company.circle.clone(),
        too_hard_reason: company.too_hard_reason.clone(),
        assumptions: company.assumptions.clone(),
        price: price.cloned(),
        raw_sha256: inputs.raw_sha256.clone(),
        latest_filing_end,
        annual_rows,
        ttm_row,
        latest_ytd,
        roic_5y_avg_pct,
        roic_5y_stddev_pct,
        gross_margin_5y_avg_pct,
        gross_margin_5y_stddev_pct,
        share_change_3y_pct,
        share_change_5y_pct,
        net_debt,
        net_debt_to_owner_earnings,
        fy_valuation,
        ttm_valuation,
        warnings,
        status,
        provenance_summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, YtdPeriodFinancials};

    fn sample_company() -> CompanyEntry {
        CompanyEntry {
            ticker: "TEST".into(),
            name: "Test Co".into(),
            bucket: Bucket::Core,
            source: DataSource::SecEdgar,
            cik: Some("0000000001".into()),
            currency: "USD".into(),
            circle: "软件".into(),
            too_hard_reason: None,
            assumptions: ValuationAssumptions {
                multiple_low: Decimal::from(20),
                multiple_high: Decimal::from(25),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(7),
                strike_price_override: None,
            },
        }
    }

    fn annual_row(yr: i32, shares: i64, ocf: i64, capex: i64, sbc: i64) -> AnnualFinancials {
        AnnualFinancials {
            fiscal_year: yr,
            period_end: format!("{yr}-12-31"),
            filed: None,
            revenue: Decimal::from(1000),
            gross_profit: Decimal::from(700),
            ebit: Decimal::from(300),
            tax_expense: Decimal::from(60),
            pretax_income: Decimal::from(300),
            ocf: Decimal::from(ocf),
            capex: Decimal::from(capex),
            sbc: Decimal::from(sbc),
            buybacks: Decimal::from(50),
            cash_and_st: Some(Decimal::from(200)),
            total_debt: Decimal::from(400),
            equity: Decimal::from(800),
            diluted_shares: Decimal::from(shares),
            ending_shares: Decimal::from(shares),
            provenance: Vec::new(),
        }
    }

    #[test]
    fn calculates_strict_oe_roic_strike_and_warnings() {
        let company = sample_company();
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "TEST".into(),
            entity_name: "Test Co".into(),
            cik: "0000000001".into(),
            source_url: "https://test".into(),
            raw_sha256: "abc".into(),
            fetched_at_ms: 1,
            annuals: vec![
                annual_row(2021, 102, 290, 50, 30),
                annual_row(2022, 100, 300, 50, 30),
                annual_row(2023, 98, 310, 50, 30),
                annual_row(2024, 95, 320, 50, 30),
                annual_row(2025, 90, 350, 50, 100), // strict OE = 200; SBC/OCF = 28.57% -> Amber
            ],
            ttm: None,
            manual_note: None,
        };
        let price = PriceSnapshot {
            schema_version: 1,
            symbol: "TEST".into(),
            price: Decimal::from(40),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: None,
            fetched_at_ms: 2,
            source_url: "https://price".into(),
        };
        let report = build_report(&company, Some(&facts), Some(&price), 3);
        let fy = report.fy_valuation.unwrap();
        // strict OE = 200, shares = 90 -> OE/share = 2.2222...
        // fair_value_low (20x) = 44.4444... -> price 40 is IN STRIKE ZONE!
        assert_eq!(fy.strict_owner_earnings, Some(Decimal::from(200)));
        assert_eq!(report.status, StrikeZoneStatus::InStrikeZone);
        assert!(report.warnings.iter().any(|w| w.code == "HIGH_SBC_BURDEN"));
        // 3Y shares: (90 - 100) / 100 = -10%
        assert_eq!(report.share_change_3y_pct, Some(Decimal::from(-10)));
        // ROIC check: tax = 20%, NOPAT = 240, IC = 400 + 800 - 200 = 1000 => ROIC = 24%
        assert_eq!(report.roic_5y_avg_pct, Some(Decimal::from(24)));
    }

    #[test]
    fn capex_exceeding_ocf_in_ytd_triggers_warning_blocked() {
        let company = sample_company();
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "TEST".into(),
            entity_name: "Test Co".into(),
            cik: "0000000001".into(),
            source_url: "https://test".into(),
            raw_sha256: "abc".into(),
            fetched_at_ms: 1,
            annuals: vec![annual_row(2025, 100, 350, 50, 20)],
            ttm: Some(TtmFinancials {
                as_of_end: "2026-09-30".into(),
                label: "TTM (2026-09-30)".into(),
                revenue: Decimal::from(1100),
                gross_profit: Decimal::from(750),
                ebit: Decimal::from(310),
                tax_expense: Decimal::from(60),
                pretax_income: Decimal::from(310),
                ocf: Decimal::from(340),
                capex: Decimal::from(280),
                sbc: Decimal::from(30),
                buybacks: Decimal::ZERO,
                cash_and_st: Some(Decimal::from(200)),
                total_debt: Decimal::from(500),
                equity: Decimal::from(900),
                diluted_shares: Decimal::from(100),
                ending_shares: Decimal::from(100),
                latest_ytd: YtdPeriodFinancials {
                    period_label: "2026 Q3 YTD".into(),
                    start: "2026-01-01".into(),
                    end: "2026-09-30".into(),
                    filed: None,
                    revenue: Some(Decimal::from(800)),
                    ocf: Some(Decimal::from(127)),
                    capex: Some(Decimal::from(132)),
                    sbc: Some(Decimal::from(23)),
                    buybacks: Some(Decimal::ZERO),
                    latest_quarter_label: Some("Q3".into()),
                    latest_quarter_ocf: Some(Decimal::from(42)),
                    latest_quarter_capex: Some(Decimal::from(51)),
                    latest_quarter_sbc: Some(Decimal::from(8)),
                    latest_quarter_buybacks: Some(Decimal::ZERO),
                },
                provenance: Vec::new(),
            }),
            manual_note: None,
        };
        let report = build_report(&company, Some(&facts), None, 3);
        assert_eq!(report.status, StrikeZoneStatus::WarningBlocked);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "YTD_CAPEX_EXCEEDS_OCF")
        );
        assert!(report.warnings.iter().any(|w| w.code == "BUYBACKS_HALTED"));
    }

    #[test]
    fn sandbox_recalculates_reverse_growth_correctly() {
        let res = evaluate_sandbox(
            Some(Decimal::from(5)),
            Some(Decimal::from(100)),
            Decimal::from(18),
            Decimal::from(10),
            Decimal::from(7),
        );
        // yield = 5/100 = 5%; target 10% -> implied growth = 5%; expected = 5 + 7 = 12%
        // fair value = 5 * 18 = 90 -> distance = (100 - 90)/90 = 11.111...%
        assert_eq!(res.fair_value, Some(Decimal::from(90)));
        assert_eq!(res.strict_fcf_yield_pct, Some(Decimal::from(5)));
        assert_eq!(res.implied_growth_for_target_pct, Some(Decimal::from(5)));
        assert_eq!(res.expected_total_return_pct, Some(Decimal::from(12)));
    }

    #[test]
    fn out_bucket_sets_out_strike_status() {
        let mut company = sample_company();
        company.bucket = Bucket::Out;
        let report = build_report(&company, None, None, 10);
        assert_eq!(report.bucket, Bucket::Out);
        assert_eq!(report.status, StrikeZoneStatus::Out);
    }

    #[test]
    fn absent_sbc_or_debt_suppresses_dependent_metrics_and_strike() {
        let company = sample_company();
        let mut ann = annual_row(2025, 100, 350, 50, 20);
        ann.provenance = vec![
            FieldProvenance {
                field: "sbc".to_owned(),
                source_tag: "absent".to_owned(),
                filed: None,
                period_end: "2025-12-31".to_owned(),
                note: None,
            },
            FieldProvenance {
                field: "total_debt".to_owned(),
                source_tag: "absent".to_owned(),
                filed: None,
                period_end: "2025-12-31".to_owned(),
                note: None,
            },
        ];
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "AIT".into(),
            entity_name: "Test Co".into(),
            cik: "0000109563".into(),
            source_url: "https://sec.test".into(),
            raw_sha256: "abc".into(),
            fetched_at_ms: 1,
            annuals: vec![ann],
            ttm: None,
            manual_note: None,
        };
        let price = PriceSnapshot {
            schema_version: 1,
            symbol: "AIT".into(),
            price: Decimal::from(50),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NYQ".into(),
            market_time_ms: Some(1_760_000_000_000),
            fetched_at_ms: 10,
            source_url: "https://price.test".into(),
        };
        let report = build_report(&company, Some(&facts), Some(&price), 10);
        assert!(report.annual_rows[0].roic_pct.is_none());
        assert_eq!(
            report.annual_rows[0].roic_note.as_deref(),
            Some("total_debt_unavailable")
        );
        assert!(report.annual_rows[0].roic_total_capital_pct.is_none());
        assert!(report.annual_rows[0].sbc.is_none());
        assert!(report.annual_rows[0].strict_owner_earnings.is_none());
        assert!(report.annual_rows[0].total_debt.is_none());
        assert!(report.annual_rows[0].net_debt.is_none());
        assert!(report.annual_rows[0].invested_capital.is_none());
        assert!(report.roic_5y_avg_pct.is_none());
        let fy = report.fy_valuation.unwrap();
        assert!(fy.strict_owner_earnings.is_none());
        assert!(fy.strike_price.is_none());
        assert!(fy.strict_fcf_yield_mcap_pct.is_none());
    }

    #[test]
    fn non_consecutive_fiscal_years_and_share_magnitude_mismatch_suppress_metrics() {
        let company = sample_company();
        let mut gapped = vec![
            annual_row(2018, 100, 300, 50, 10),
            annual_row(2019, 100, 310, 50, 10),
            annual_row(2023, 100, 320, 50, 10),
            annual_row(2024, 100, 330, 50, 10),
            annual_row(2025, 100, 340, 50, 10),
        ];
        // Also simulate CIX-like thousandfold share mismatch on latest annual
        gapped[4].diluted_shares = Decimal::from(12_337);
        gapped[4].ending_shares = Decimal::from(12_336_657);
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "GTEC".into(),
            entity_name: "Gap Corp".into(),
            cik: "0001735041".into(),
            source_url: "https://sec.test".into(),
            raw_sha256: "gap".into(),
            fetched_at_ms: 1,
            annuals: gapped,
            ttm: None,
            manual_note: None,
        };
        let price = PriceSnapshot {
            schema_version: 1,
            symbol: "GTEC".into(),
            price: Decimal::from(40),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(1_760_000_000_000),
            fetched_at_ms: 10,
            source_url: "https://price.test".into(),
        };
        let report = build_report(&company, Some(&facts), Some(&price), 10);
        assert!(report.roic_5y_avg_pct.is_none());
        assert!(report.gross_margin_5y_avg_pct.is_none());
        assert!(report.share_change_3y_pct.is_none());
        let fy = report.fy_valuation.unwrap();
        assert!(fy.strike_price.is_none());
        assert!(fy.strict_fcf_yield_mcap_pct.is_none());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
    }

    #[test]
    fn partial_5y_null_roic_and_absent_debt_suppress_5y_average_and_ev() {
        let company = sample_company();
        let mut sfm = vec![
            annual_row(2021, 100, 300, 50, 10),
            annual_row(2022, 100, 310, 50, 10),
            annual_row(2023, 100, 320, 50, 10),
            annual_row(2024, 100, 330, 50, 10),
            annual_row(2025, 100, 340, 50, 10),
        ];
        for row in &mut sfm[3..5] {
            row.total_debt = Decimal::ZERO;
            row.provenance.push(FieldProvenance {
                field: "total_debt".into(),
                source_tag: "absent".into(),
                filed: None,
                period_end: row.period_end.clone(),
                note: Some("absent_in_xbrl_assumed_zero".into()),
            });
        }
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "SFM".into(),
            entity_name: "Sprouts Test".into(),
            cik: "0001575515".into(),
            source_url: "https://sec.test".into(),
            raw_sha256: "sfm".into(),
            fetched_at_ms: 1,
            annuals: sfm,
            ttm: None,
            manual_note: None,
        };
        let price = PriceSnapshot {
            schema_version: 1,
            symbol: "SFM".into(),
            price: Decimal::from(50),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(1_760_000_000_000),
            fetched_at_ms: 10,
            source_url: "https://price.test".into(),
        };
        let report = build_report(&company, Some(&facts), Some(&price), 10);
        assert!(report.roic_5y_avg_pct.is_none());
        assert!(report.roic_5y_stddev_pct.is_none());
        // Gross profit is present in all 5 consecutive years, so GM 5Y remains available.
        assert!(report.gross_margin_5y_avg_pct.is_some());
        assert!(report.net_debt.is_none());
        assert!(report.net_debt_to_owner_earnings.is_none());
        let fy = report.fy_valuation.unwrap();
        assert!(fy.enterprise_value.is_none());
        assert!(fy.strict_fcf_yield_ev_pct.is_none());
    }

    #[test]
    fn absent_ytd_buybacks_does_not_issue_buybacks_halted() {
        let company = sample_company();
        let fy = annual_row(2025, 100, 350, 50, 20);
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "BDX".into(),
            entity_name: "Becton Test".into(),
            cik: "0000010795".into(),
            source_url: "https://sec.test".into(),
            raw_sha256: "bdx".into(),
            fetched_at_ms: 1,
            annuals: vec![fy],
            ttm: Some(TtmFinancials {
                as_of_end: "2026-03-31".into(),
                label: "TTM (2026-03-31)".into(),
                revenue: Decimal::from(1000),
                gross_profit: Decimal::from(700),
                ebit: Decimal::from(300),
                tax_expense: Decimal::from(60),
                pretax_income: Decimal::from(300),
                ocf: Decimal::from(350),
                capex: Decimal::from(50),
                sbc: Decimal::from(20),
                buybacks: Decimal::from(50),
                cash_and_st: Some(Decimal::from(200)),
                total_debt: Decimal::from(400),
                equity: Decimal::from(800),
                diluted_shares: Decimal::from(100),
                ending_shares: Decimal::from(100),
                latest_ytd: YtdPeriodFinancials {
                    period_label: "2026 Q2 YTD".into(),
                    start: "2025-10-01".into(),
                    end: "2026-03-31".into(),
                    filed: None,
                    revenue: Some(Decimal::from(500)),
                    ocf: Some(Decimal::from(180)),
                    capex: Some(Decimal::from(25)),
                    sbc: Some(Decimal::from(10)),
                    buybacks: None,
                    latest_quarter_label: None,
                    latest_quarter_ocf: None,
                    latest_quarter_capex: None,
                    latest_quarter_sbc: None,
                    latest_quarter_buybacks: None,
                },
                provenance: vec![FieldProvenance {
                    field: "ytd_buybacks".into(),
                    source_tag: "absent".into(),
                    filed: None,
                    period_end: "2026-03-31".into(),
                    note: Some("absent_in_10q_assumed_zero".into()),
                }],
            }),
            manual_note: None,
        };
        let report = build_report(&company, Some(&facts), None, 10);
        assert!(
            report
                .latest_ytd
                .as_ref()
                .is_some_and(|y| y.buybacks.is_none())
        );
        assert!(
            !report.warnings.iter().any(|w| w.code == "BUYBACKS_HALTED"),
            "absent ytd_buybacks must never trigger BUYBACKS_HALTED"
        );
    }

    #[test]
    fn non_common_warrant_and_absent_cash_suppress_valuation_and_roic_uniformly() {
        let mut warrant = sample_company();
        warrant.ticker = "SBCWW".into();
        let fy = annual_row(2025, 100, 350, 50, 20);
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "SBCWW".into(),
            entity_name: "SBC Warrant Co".into(),
            cik: "0000000099".into(),
            source_url: "https://sec.test".into(),
            raw_sha256: "warrant".into(),
            fetched_at_ms: 1,
            annuals: vec![fy.clone()],
            ttm: Some(TtmFinancials {
                as_of_end: "2026-09-30".into(),
                label: "TTM (2026-09-30)".into(),
                revenue: Decimal::from(1000),
                gross_profit: Decimal::from(700),
                ebit: Decimal::from(300),
                tax_expense: Decimal::from(60),
                pretax_income: Decimal::from(300),
                ocf: Decimal::from(350),
                capex: Decimal::from(50),
                sbc: Decimal::from(20),
                buybacks: Decimal::from(10),
                cash_and_st: None,
                total_debt: Decimal::from(400),
                equity: Decimal::from(800),
                diluted_shares: Decimal::from(100),
                ending_shares: Decimal::from(100),
                latest_ytd: YtdPeriodFinancials {
                    period_label: "2026 Q3 YTD".into(),
                    start: "2026-01-01".into(),
                    end: "2026-09-30".into(),
                    filed: None,
                    revenue: Some(Decimal::from(700)),
                    ocf: Some(Decimal::from(250)),
                    capex: Some(Decimal::from(30)),
                    sbc: Some(Decimal::from(15)),
                    buybacks: Some(Decimal::from(5)),
                    latest_quarter_label: None,
                    latest_quarter_ocf: None,
                    latest_quarter_capex: None,
                    latest_quarter_sbc: None,
                    latest_quarter_buybacks: None,
                },
                provenance: vec![FieldProvenance {
                    field: "ttm_cash_and_st".into(),
                    source_tag: "absent".into(),
                    filed: None,
                    period_end: "2026-09-30".into(),
                    note: Some("ambiguous_marketable_securities".into()),
                }],
            }),
            manual_note: None,
        };
        let snap = PriceSnapshot {
            schema_version: 1,
            symbol: "SBCWW".into(),
            price: Decimal::from(5),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NASDAQ".into(),
            market_time_ms: Some(10),
            fetched_at_ms: 10,
            source_url: "https://test".into(),
        };
        let rep = build_report(&warrant, Some(&facts), Some(&snap), 10);
        assert_eq!(rep.latest_filing_end.as_deref(), Some("2026-09-30"));
        assert_eq!(
            latest_summary_filing_end(&facts),
            rep.latest_filing_end.as_deref()
        );
        assert!(
            rep.warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert!(rep.warnings.iter().any(|w| w.code == "DATA_UNAVAILABLE"));
        assert_eq!(rep.fy_valuation.as_ref().unwrap().strike_price, None);
        assert_eq!(rep.ttm_valuation.as_ref().unwrap().strike_price, None);
        assert_eq!(
            rep.ttm_valuation
                .as_ref()
                .unwrap()
                .strict_fcf_yield_mcap_pct,
            None
        );
        assert_eq!(rep.ttm_row.as_ref().unwrap().roic_pct, None);
        assert_eq!(rep.ttm_row.as_ref().unwrap().invested_capital, None);
        assert_eq!(rep.ttm_row.as_ref().unwrap().cash_and_st, None);
        assert_eq!(rep.net_debt, None);
        assert_eq!(rep.net_debt_to_owner_earnings, None);
    }

    #[test]
    fn international_exchange_dot_suffixes_stay_common_while_warrants_and_preferreds_separate() {
        assert!(!is_non_common_ticker("DSY.PA", &[]));
        assert!(!is_supplemental_security_ticker("DSY.PA", ""));
        assert!(!is_non_common_ticker("1810.HK", &[]));
        assert!(!is_supplemental_security_ticker("1810.HK", ""));
        assert!(!is_non_common_ticker("BRK.B", &[]));
        assert!(!is_supplemental_security_ticker("BRK.B", "BRK.B"));

        assert!(is_non_common_ticker("ALB-PA", &[]));
        assert!(is_supplemental_security_ticker("ALB-PA", "ALB"));
        assert!(is_supplemental_security_ticker("ALB.PRA", "ALB"));
        assert!(is_non_common_ticker("PGYWW", &["PGY".to_owned()]));
        assert!(is_supplemental_security_ticker("PGYWW", ""));
        assert!(is_supplemental_security_ticker("PGYWW", "PGY"));
    }

    #[test]
    fn round14_supplemental_debenture_against_common_facts_suppresses_valuation() {
        assert!(is_supplemental_security_ticker("CCZ", "CMCSA"));
        assert!(is_supplemental_security_ticker("DTB", "DTE"));
        assert!(is_supplemental_security_ticker("AMJB", "JPM"));
        assert!(!is_supplemental_security_ticker("GOOGL", "GOOG"));
        assert!(!are_verified_common_share_class_siblings(
            "CCZ",
            "CMCSA",
            &["CMCSA".into(), "CCZ".into()]
        ));
        assert!(are_verified_common_share_class_siblings(
            "GOOGL",
            "GOOG",
            &["GOOG".into(), "GOOGL".into()]
        ));

        let mut ccz = sample_company();
        ccz.ticker = "CCZ".into();
        ccz.cik = Some("0001166691".into());
        let fy = annual_row(2025, 1000, 3500, 500, 100);
        let facts = RawCompanyFactsSummary {
            schema_version: 1,
            extractor_version: crate::edgar::SEC_EXTRACTOR_VERSION,
            ticker: "CMCSA".into(),
            entity_name: "COMCAST CORP".into(),
            cik: "0001166691".into(),
            source_url: "https://sec.test".into(),
            raw_sha256: "cmcsa".into(),
            fetched_at_ms: 1,
            annuals: vec![fy],
            ttm: None,
            manual_note: None,
        };
        let snap = PriceSnapshot {
            schema_version: 1,
            symbol: "CCZ".into(),
            price: Decimal::from(25),
            previous_close: None,
            currency: "NYSE".into(),
            exchange: "NYSE".into(),
            market_time_ms: Some(10),
            fetched_at_ms: 10,
            source_url: "https://test".into(),
        };
        let rep = build_report(&ccz, Some(&facts), Some(&snap), 10);
        assert!(
            rep.warnings
                .iter()
                .any(|w| w.code == "SHARE_BASIS_MISMATCH")
        );
        assert_eq!(rep.fy_valuation.as_ref().unwrap().strike_price, None);
        assert_eq!(
            rep.fy_valuation.as_ref().unwrap().strict_fcf_yield_mcap_pct,
            None
        );

        assert!(is_non_common_ticker("CCZ", &["CCZ".into(), "CMCSA".into()]));
        assert!(is_non_common_ticker("CCZ", &["CMCSA".into(), "CCZ".into()]));
        assert!(!is_non_common_ticker(
            "CMCSA",
            &["CCZ".into(), "CMCSA".into()]
        ));
        assert!(ticker_matches_issuer_name("CMCSA", "COMCAST CORP"));
        assert!(!ticker_matches_issuer_name("CCZ", "COMCAST CORP"));
        assert!(ticker_matches_issuer_name("DTE", "DTE ENERGY CO"));
        assert!(!ticker_matches_issuer_name("DTB", "DTE ENERGY CO"));

        for bad_ticker in [crate::edgar::UNRESOLVED_CANONICAL_TICKER, ""] {
            let mut unresolved_facts = facts.clone();
            unresolved_facts.ticker = bad_ticker.into();
            let unresolved_rep = build_report(&ccz, Some(&unresolved_facts), Some(&snap), 10);
            assert!(unresolved_rep.warnings.iter().any(
                |w| w.code == "SHARE_BASIS_MISMATCH" && w.title == UNRESOLVED_SHARE_BASIS_TITLE
            ));
            assert_eq!(
                unresolved_rep.fy_valuation.as_ref().unwrap().strike_price,
                None
            );
            assert_eq!(
                unresolved_rep
                    .fy_valuation
                    .as_ref()
                    .unwrap()
                    .strict_fcf_yield_mcap_pct,
                None
            );
        }
    }
}
