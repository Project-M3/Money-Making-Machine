use crate::model::{
    AnnualFinancials, FieldProvenance, RawCompanyFactsSummary, TtmFinancials, YtdPeriodFinancials,
};
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path, str::FromStr, thread, time::Duration};

const REV_TAGS: &[&str] = &[
    "RevenueFromContractWithCustomerExcludingAssessedTax",
    "Revenues",
    "SalesRevenueNet",
    "RevenueFromContractWithCustomerIncludingAssessedTax",
];
const COST_TAGS: &[&str] = &[
    "CostOfRevenue",
    "CostOfGoodsAndServicesSold",
    "CostOfGoodsAndServiceExcludingDepreciationDepletionAndAmortization",
];
const GROSS_PROFIT_TAGS: &[&str] = &["GrossProfit"];
const EBIT_TAGS: &[&str] = &["OperatingIncomeLoss"];
const TAX_TAGS: &[&str] = &["IncomeTaxExpenseBenefit"];
const PRETAX_TAGS: &[&str] = &[
    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest",
    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesMinorityInterestAndIncomeLossFromEquityMethodInvestments",
];
const OCF_TAGS: &[&str] = &["NetCashProvidedByUsedInOperatingActivities"];
const CAPEX_TAGS: &[&str] = &[
    "PaymentsToAcquirePropertyPlantAndEquipment",
    "PaymentsToAcquireProductiveAssets",
];
const SBC_TAGS: &[&str] = &[
    "ShareBasedCompensation",
    "AllocatedShareBasedCompensationExpense",
];
const BUYBACK_TAGS: &[&str] = &[
    "PaymentsForRepurchaseOfCommonStock",
    "PaymentsForRepurchaseOfEquity",
];
const DILUTED_SHARES_TAGS: &[&str] = &["WeightedAverageNumberOfDilutedSharesOutstanding"];
const CASH_COMBINED_TAGS: &[&str] = &["CashCashEquivalentsAndShortTermInvestments"];
const CASH_ONLY_TAGS: &[&str] = &[
    "CashAndCashEquivalentsAtCarryingValue",
    "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents",
];
const ST_INVESTMENT_TAGS: &[&str] = &[
    "MarketableSecuritiesCurrent",
    "ShortTermInvestments",
    "AvailableForSaleSecuritiesDebtSecuritiesCurrent",
    "MarketableSecurities",
];
const LT_DEBT_NONCURRENT_TAGS: &[&str] = &["LongTermDebtNoncurrent"];
const LT_DEBT_COMBINED_TAGS: &[&str] = &["LongTermDebt", "LongTermDebtAndCapitalLeaseObligations"];
const LT_DEBT_CURRENT_TAGS: &[&str] = &["LongTermDebtCurrent"];
const ST_BORROWINGS_TAGS: &[&str] = &["ShortTermBorrowings"];
const COMMERCIAL_PAPER_TAGS: &[&str] = &["CommercialPaper"];
const DEBT_CURRENT_TAGS: &[&str] = &["DebtCurrent"];
const EQUITY_TAGS: &[&str] = &[
    "StockholdersEquity",
    "StockholdersEquityIncludingPortionAttributableToNoncontrollingInterest",
];
const ENDING_SHARES_GAAP_TAGS: &[&str] = &["CommonStockSharesOutstanding"];
const ENDING_SHARES_DEI_TAGS: &[&str] = &["EntityCommonStockSharesOutstanding"];

#[derive(Debug, Clone)]
struct FactItem {
    tag: String,
    start: Option<String>,
    end: String,
    end_days: i64,
    duration_days: Option<i64>,
    val: Decimal,
    filed: String,
    fp: Option<String>,
}

#[derive(Debug, Clone)]
struct PickedValue {
    val: Decimal,
    tag: String,
    filed: String,
    period_end: String,
    note: Option<String>,
}

pub fn sec_user_agent() -> String {
    std::env::var("M3_SEC_USER_AGENT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "M3Research m3-research@example.com".to_owned())
}

pub fn sec_http_client() -> Result<Client> {
    Client::builder()
        .user_agent(sec_user_agent())
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to build SEC HTTP client")
}

/// Howard Hinnant civil-from-days inverse: YYYY-MM-DD -> days since Unix epoch.
pub fn parse_date_days(iso: &str) -> Result<i64> {
    let parts: Vec<&str> = iso.trim().split('-').collect();
    ensure!(parts.len() == 3, "invalid ISO date: {iso}");
    let year: i64 = parts[0].parse().with_context(|| format!("bad year in {iso}"))?;
    let month: i64 = parts[1].parse().with_context(|| format!("bad month in {iso}"))?;
    let day: i64 = parts[2].parse().with_context(|| format!("bad day in {iso}"))?;
    ensure!((1..=12).contains(&month) && (1..=31).contains(&day), "out-of-range date: {iso}");

    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(era * 146_097 + doe - 719_468)
}

pub fn normalize_cik(raw: &str) -> Result<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    ensure!(!digits.is_empty() && digits.len() <= 10, "invalid CIK: {raw}");
    Ok(format!("{digits:0>10}"))
}

pub fn lookup_cik_by_ticker(
    client: &Client,
    cache_path: &Path,
    ticker: &str,
    refresh: bool,
) -> Result<String> {
    let target = ticker.trim().to_ascii_uppercase();
    let bytes = if cache_path.exists() && !refresh {
        fs::read(cache_path).with_context(|| format!("read {}", cache_path.display()))?
    } else {
        thread::sleep(Duration::from_millis(160));
        let resp = client
            .get("https://www.sec.gov/files/company_tickers.json")
            .send()
            .context("request SEC company_tickers.json")?;
        ensure!(
            resp.status().is_success(),
            "SEC company_tickers.json returned HTTP {}",
            resp.status()
        );
        let raw = resp.bytes().context("read SEC company_tickers body")?.to_vec();
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = cache_path.with_extension("tmp");
        fs::write(&tmp, &raw)?;
        fs::rename(&tmp, cache_path)?;
        raw
    };
    let root: Value = serde_json::from_slice(&bytes).context("parse company_tickers.json")?;
    let map = root
        .as_object()
        .context("company_tickers.json must be JSON object")?;
    for entry in map.values() {
        if entry["ticker"]
            .as_str()
            .is_some_and(|t| t.eq_ignore_ascii_case(&target))
        {
            if let Some(cik_num) = entry["cik_str"].as_u64() {
                return Ok(format!("{cik_num:010}"));
            }
            if let Some(cik_str) = entry["cik_str"].as_str() {
                return normalize_cik(cik_str);
            }
        }
    }
    if !refresh {
        return lookup_cik_by_ticker(client, cache_path, ticker, true);
    }
    anyhow::bail!("ticker {target} not found in SEC company_tickers.json")
}

pub fn fetch_and_extract(
    client: &Client,
    ticker: &str,
    cik: &str,
    raw_cache_path: &Path,
    fetched_at_ms: u64,
) -> Result<RawCompanyFactsSummary> {
    let norm_cik = normalize_cik(cik)?;
    let url = format!("https://data.sec.gov/api/xbrl/companyfacts/CIK{norm_cik}.json");
    thread::sleep(Duration::from_millis(160));
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("fetch SEC XBRL for {ticker} ({norm_cik})"))?;
    ensure!(
        resp.status().is_success(),
        "SEC XBRL returned HTTP {} for {ticker} ({url})",
        resp.status()
    );
    let raw_bytes = resp
        .bytes()
        .with_context(|| format!("read SEC XBRL bytes for {ticker}"))?
        .to_vec();
    let _: Value = serde_json::from_slice(&raw_bytes)
        .with_context(|| format!("SEC response for {ticker} was not valid JSON"))?;

    if let Some(parent) = raw_cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = raw_cache_path.with_extension("tmp");
    fs::write(&tmp, &raw_bytes)?;
    fs::rename(&tmp, raw_cache_path)?;

    parse_companyfacts_bytes(&raw_bytes, ticker, &norm_cik, &url, fetched_at_ms)
}

pub fn parse_companyfacts_bytes(
    raw_bytes: &[u8],
    ticker: &str,
    cik: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<RawCompanyFactsSummary> {
    let sha256 = hex_digest(raw_bytes);
    let root: Value = serde_json::from_slice(raw_bytes).context("parse companyfacts JSON")?;
    let entity_name = root["entityName"]
        .as_str()
        .unwrap_or(ticker)
        .to_owned();
    let facts = root["facts"]
        .as_object()
        .context("companyfacts missing facts object")?;
    let us_gaap = facts.get("us-gaap").and_then(Value::as_object);
    let dei = facts.get("dei").and_then(Value::as_object);

    let annual_ends = discover_annual_periods(us_gaap, 8)?;
    ensure!(
        !annual_ends.is_empty(),
        "no annual 10-K revenue periods discovered for {ticker}"
    );

    let mut annuals = Vec::with_capacity(annual_ends.len());
    for (period_end, end_days) in &annual_ends {
        if let Some(row) = build_annual_row(us_gaap, dei, period_end, *end_days)? {
            annuals.push(row);
        }
    }
    ensure!(
        !annuals.is_empty(),
        "could not extract complete annual periods for {ticker}"
    );

    let ttm = build_ttm(us_gaap, dei, annuals.last())?;

    Ok(RawCompanyFactsSummary {
        schema_version: 1,
        ticker: ticker.to_owned(),
        entity_name,
        cik: normalize_cik(cik)?,
        source_url: source_url.to_owned(),
        raw_sha256: sha256,
        fetched_at_ms,
        annuals,
        ttm,
        manual_note: None,
    })
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn collect_items(
    ns: Option<&serde_json::Map<String, Value>>,
    tags: &[&str],
    unit_key: &str,
    allowed_forms: &[&str],
) -> Vec<FactItem> {
    let Some(ns_map) = ns else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for tag in tags {
        let Some(tag_obj) = ns_map.get(*tag) else {
            continue;
        };
        let Some(units) = tag_obj["units"].get(unit_key).and_then(Value::as_array) else {
            continue;
        };
        for item in units {
            let Some(form) = item["form"].as_str() else {
                continue;
            };
            if !allowed_forms.contains(&form) {
                continue;
            }
            let Some(end) = item["end"].as_str() else {
                continue;
            };
            let Ok(end_days) = parse_date_days(end) else {
                continue;
            };
            let start = item["start"].as_str().map(str::to_owned);
            let duration_days = match &start {
                Some(s) => match parse_date_days(s) {
                    Ok(sd) => Some(end_days - sd),
                    Err(_) => continue,
                },
                None => None,
            };
            let val = if let Some(n) = item["val"].as_i64() {
                Decimal::from(n)
            } else if let Some(n) = item["val"].as_u64() {
                Decimal::from(n)
            } else if let Some(n) = item["val"].as_f64() {
                let Ok(d) = Decimal::from_str(&n.to_string()) else {
                    continue;
                };
                d
            } else {
                continue;
            };
            out.push(FactItem {
                tag: (*tag).to_owned(),
                start,
                end: end.to_owned(),
                end_days,
                duration_days,
                val,
                filed: item["filed"].as_str().unwrap_or("").to_owned(),
                fp: item["fp"].as_str().map(str::to_owned),
            });
        }
    }
    out
}

fn discover_annual_periods(
    us_gaap: Option<&serde_json::Map<String, Value>>,
    max_years: usize,
) -> Result<Vec<(String, i64)>> {
    let forms = &["10-K", "10-K/A"];
    let items = collect_items(us_gaap, REV_TAGS, "USD", forms);
    let mut by_bucket: BTreeMap<i64, (String, i64)> = BTreeMap::new();
    for item in items {
        let Some(dur) = item.duration_days else {
            continue;
        };
        if !(330..=395).contains(&dur) {
            continue;
        }
        // Group 52/53-week fiscal end dates within +-15 days into one period.
        let existing_key = by_bucket
            .keys()
            .copied()
            .find(|k| (k - item.end_days).abs() <= 15);
        match existing_key {
            Some(k) => {
                if item.end_days > by_bucket[&k].1 {
                    by_bucket.insert(k, (item.end.clone(), item.end_days));
                }
            }
            None => {
                by_bucket.insert(item.end_days, (item.end.clone(), item.end_days));
            }
        }
    }
    let mut periods: Vec<(String, i64)> = by_bucket.into_values().collect();
    periods.sort_by_key(|(_, d)| *d);
    if periods.len() > max_years {
        periods = periods.split_off(periods.len() - max_years);
    }
    Ok(periods)
}

#[derive(Clone, Copy)]
struct PeriodWindow {
    target_end_days: i64,
    min_dur: i64,
    max_dur: i64,
    max_end_delta: i64,
}

fn pick_flow_for_period(
    ns: Option<&serde_json::Map<String, Value>>,
    tags: &[&str],
    unit_key: &str,
    forms: &[&str],
    window: PeriodWindow,
) -> Option<PickedValue> {
    for tag in tags {
        let items = collect_items(ns, &[tag], unit_key, forms);
        let best = items
            .into_iter()
            .filter(|it| {
                it.duration_days
                    .is_some_and(|d| (window.min_dur..=window.max_dur).contains(&d))
                    && (it.end_days - window.target_end_days).abs() <= window.max_end_delta
            })
            .max_by(|a, b| {
                // Prefer exact period end match first, then latest filing date (restatements).
                let da = (a.end_days - window.target_end_days).abs();
                let db = (b.end_days - window.target_end_days).abs();
                db.cmp(&da).then_with(|| a.filed.cmp(&b.filed))
            });
        if let Some(it) = best {
            return Some(PickedValue {
                val: it.val,
                tag: it.tag,
                filed: it.filed,
                period_end: it.end,
                note: None,
            });
        }
    }
    None
}

fn pick_instant_for_period(
    ns: Option<&serde_json::Map<String, Value>>,
    tags: &[&str],
    unit_key: &str,
    forms: &[&str],
    target_end_days: i64,
    max_end_delta: i64,
) -> Option<PickedValue> {
    for tag in tags {
        let items = collect_items(ns, &[tag], unit_key, forms);
        let best = items
            .into_iter()
            .filter(|it| it.start.is_none() && (it.end_days - target_end_days).abs() <= max_end_delta)
            .max_by(|a, b| {
                let da = (a.end_days - target_end_days).abs();
                let db = (b.end_days - target_end_days).abs();
                db.cmp(&da).then_with(|| a.filed.cmp(&b.filed))
            });
        if let Some(it) = best {
            return Some(PickedValue {
                val: it.val,
                tag: it.tag,
                filed: it.filed,
                period_end: it.end,
                note: None,
            });
        }
    }
    None
}

fn build_annual_row(
    us_gaap: Option<&serde_json::Map<String, Value>>,
    dei: Option<&serde_json::Map<String, Value>>,
    period_end: &str,
    end_days: i64,
) -> Result<Option<AnnualFinancials>> {
    let forms = &["10-K", "10-K/A"];
    let annual_win = PeriodWindow {
        target_end_days: end_days,
        min_dur: 330,
        max_dur: 395,
        max_end_delta: 15,
    };
    let Some(rev) = pick_flow_for_period(us_gaap, REV_TAGS, "USD", forms, annual_win) else {
        return Ok(None);
    };
    let Some(ebit) = pick_flow_for_period(us_gaap, EBIT_TAGS, "USD", forms, annual_win) else {
        return Ok(None);
    };
    let Some(ocf) = pick_flow_for_period(us_gaap, OCF_TAGS, "USD", forms, annual_win) else {
        return Ok(None);
    };
    let Some(capex_raw) = pick_flow_for_period(us_gaap, CAPEX_TAGS, "USD", forms, annual_win)
    else {
        return Ok(None);
    };

    let gross_profit =
        if let Some(gp) = pick_flow_for_period(us_gaap, GROSS_PROFIT_TAGS, "USD", forms, annual_win)
        {
            gp
        } else if let Some(cost) = pick_flow_for_period(us_gaap, COST_TAGS, "USD", forms, annual_win)
        {
            PickedValue {
                val: rev.val - cost.val,
                tag: format!("derived({} - {})", rev.tag, cost.tag),
                filed: cost.filed,
                period_end: period_end.to_owned(),
                note: Some("derived_from_cost_of_revenue".to_owned()),
            }
        } else {
            return Ok(None);
        };

    let tax_expense = pick_flow_for_period(us_gaap, TAX_TAGS, "USD", forms, annual_win).unwrap_or(
        PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some("absent_in_xbrl_assumed_zero".to_owned()),
        },
    );
    let pretax_income = pick_flow_for_period(us_gaap, PRETAX_TAGS, "USD", forms, annual_win)
        .unwrap_or(PickedValue {
            val: ebit.val,
            tag: "fallback_ebit".to_owned(),
            filed: ebit.filed.clone(),
            period_end: period_end.to_owned(),
            note: Some("pretax_absent_used_ebit".to_owned()),
        });
    let sbc = pick_flow_for_period(us_gaap, SBC_TAGS, "USD", forms, annual_win).unwrap_or(
        PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some("absent_in_xbrl_assumed_zero".to_owned()),
        },
    );
    let buybacks_raw = pick_flow_for_period(us_gaap, BUYBACK_TAGS, "USD", forms, annual_win)
        .unwrap_or(PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some("absent_in_xbrl_assumed_zero".to_owned()),
        });

    let Some(equity) =
        pick_instant_for_period(us_gaap, EQUITY_TAGS, "USD", forms, end_days, 15)
    else {
        return Ok(None);
    };

    let cash_and_st = extract_cash_and_st(us_gaap, forms, end_days, period_end)?;
    let total_debt = extract_total_debt(us_gaap, forms, end_days, period_end);

    let ending_shares = pick_instant_for_period(
        us_gaap,
        ENDING_SHARES_GAAP_TAGS,
        "shares",
        forms,
        end_days,
        20,
    )
    .or_else(|| pick_instant_for_period(dei, ENDING_SHARES_DEI_TAGS, "shares", forms, end_days, 45));

    let diluted_shares =
        pick_flow_for_period(us_gaap, DILUTED_SHARES_TAGS, "shares", forms, annual_win)
            .or_else(|| ending_shares.clone())
            .context("missing both diluted_shares and ending_shares")?;

    let ending_shares = ending_shares.unwrap_or_else(|| diluted_shares.clone());

    let fiscal_year = period_end
        .get(..4)
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);

    let filed = [
        &rev.filed,
        &ocf.filed,
        &equity.filed,
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .max()
    .cloned();

    let provenance = vec![
        prov("revenue", &rev),
        prov("gross_profit", &gross_profit),
        prov("ebit", &ebit),
        prov("ocf", &ocf),
        prov("capex", &capex_raw),
        prov("sbc", &sbc),
        prov("buybacks", &buybacks_raw),
        prov("cash_and_st", &cash_and_st),
        prov("total_debt", &total_debt),
        prov("equity", &equity),
        prov("diluted_shares", &diluted_shares),
    ];

    Ok(Some(AnnualFinancials {
        fiscal_year,
        period_end: period_end.to_owned(),
        filed,
        revenue: rev.val,
        gross_profit: gross_profit.val,
        ebit: ebit.val,
        tax_expense: tax_expense.val,
        pretax_income: pretax_income.val,
        ocf: ocf.val,
        capex: capex_raw.val.abs(),
        sbc: sbc.val.abs(),
        buybacks: buybacks_raw.val.abs(),
        cash_and_st: cash_and_st.val,
        total_debt: total_debt.val,
        equity: equity.val,
        diluted_shares: diluted_shares.val,
        ending_shares: ending_shares.val,
        provenance,
    }))
}

fn extract_cash_and_st(
    us_gaap: Option<&serde_json::Map<String, Value>>,
    forms: &[&str],
    end_days: i64,
    period_end: &str,
) -> Result<PickedValue> {
    if let Some(combined) =
        pick_instant_for_period(us_gaap, CASH_COMBINED_TAGS, "USD", forms, end_days, 15)
    {
        return Ok(combined);
    }
    let cash = pick_instant_for_period(us_gaap, CASH_ONLY_TAGS, "USD", forms, end_days, 15)
        .context("missing cash_and_equivalents")?;
    let st = pick_instant_for_period(us_gaap, ST_INVESTMENT_TAGS, "USD", forms, end_days, 15);
    let (val, tag) = match st {
        Some(st_val) => (cash.val + st_val.val, format!("{} + {}", cash.tag, st_val.tag)),
        None => (cash.val, cash.tag),
    };
    Ok(PickedValue {
        val,
        tag,
        filed: cash.filed,
        period_end: period_end.to_owned(),
        note: None,
    })
}

fn extract_total_debt(
    us_gaap: Option<&serde_json::Map<String, Value>>,
    forms: &[&str],
    end_days: i64,
    period_end: &str,
) -> PickedValue {
    let lt_noncurrent =
        pick_instant_for_period(us_gaap, LT_DEBT_NONCURRENT_TAGS, "USD", forms, end_days, 15);
    let lt_current =
        pick_instant_for_period(us_gaap, LT_DEBT_CURRENT_TAGS, "USD", forms, end_days, 15);
    let cp = pick_instant_for_period(us_gaap, COMMERCIAL_PAPER_TAGS, "USD", forms, end_days, 15);
    let stb = pick_instant_for_period(us_gaap, ST_BORROWINGS_TAGS, "USD", forms, end_days, 15);
    let debt_current =
        pick_instant_for_period(us_gaap, DEBT_CURRENT_TAGS, "USD", forms, end_days, 15);

    let mut total = Decimal::ZERO;
    let mut parts = Vec::new();
    let mut latest_filed = String::new();

    if let Some(lt) = lt_noncurrent {
        total += lt.val;
        parts.push(lt.tag);
        latest_filed = latest_filed.max(lt.filed);
        if lt_current.is_some() || cp.is_some() || stb.is_some() {
            for p in [lt_current, cp, stb].into_iter().flatten() {
                total += p.val;
                parts.push(p.tag);
                latest_filed = latest_filed.max(p.filed);
            }
        } else if let Some(dc) = debt_current {
            total += dc.val;
            parts.push(dc.tag);
            latest_filed = latest_filed.max(dc.filed);
        }
    } else {
        let lt_combined =
            pick_instant_for_period(us_gaap, LT_DEBT_COMBINED_TAGS, "USD", forms, end_days, 15);
        if let Some(lt) = lt_combined {
            total += lt.val;
            parts.push(lt.tag);
            latest_filed = latest_filed.max(lt.filed);
            // Only add commercial paper / short-term borrowings; LongTermDebt generally includes LongTermDebtCurrent.
            for p in [cp, stb].into_iter().flatten() {
                total += p.val;
                parts.push(p.tag);
                latest_filed = latest_filed.max(p.filed);
            }
        } else {
            for p in [lt_current, cp, stb, debt_current].into_iter().flatten() {
                total += p.val;
                parts.push(p.tag);
                latest_filed = latest_filed.max(p.filed);
            }
        }
    }

    if parts.is_empty() {
        PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some("absent_in_xbrl_assumed_zero".to_owned()),
        }
    } else {
        PickedValue {
            val: total,
            tag: parts.join(" + "),
            filed: latest_filed,
            period_end: period_end.to_owned(),
            note: None,
        }
    }
}

fn build_ttm(
    us_gaap: Option<&serde_json::Map<String, Value>>,
    dei: Option<&serde_json::Map<String, Value>>,
    latest_annual: Option<&AnnualFinancials>,
) -> Result<Option<TtmFinancials>> {
    let Some(annual) = latest_annual else {
        return Ok(None);
    };
    let annual_end_days = parse_date_days(&annual.period_end)?;
    let q_forms = &["10-Q", "10-Q/A"];
    let ocf_q = collect_items(us_gaap, OCF_TAGS, "USD", q_forms);

    // Find the newest 10-Q period whose start begins right after the latest 10-K period_end.
    let newest_q = ocf_q
        .iter()
        .filter(|it| {
            let Some(start_str) = &it.start else {
                return false;
            };
            let Ok(start_days) = parse_date_days(start_str) else {
                return false;
            };
            let Some(dur) = it.duration_days else {
                return false;
            };
            it.end_days > annual_end_days + 45
                && (start_days - annual_end_days).abs() <= 15
                && (70..=310).contains(&dur)
        })
        .max_by(|a, b| a.end_days.cmp(&b.end_days).then_with(|| a.filed.cmp(&b.filed)));

    let Some(curr_q) = newest_q else {
        return Ok(None);
    };
    let curr_end = curr_q.end.clone();
    let curr_end_days = curr_q.end_days;
    let curr_start = curr_q.start.clone().unwrap_or_default();
    let curr_start_days = parse_date_days(&curr_start)?;
    let curr_dur = curr_q.duration_days.unwrap_or(90);

    // Matching prior-year YTD (~365 days earlier, similar duration).
    let prior_end_target = curr_end_days - 365;
    let prior_dur_min = curr_dur - 15;
    let prior_dur_max = curr_dur + 15;

    let pick_ytd = |tags: &[&str], abs_val: bool, default_zero: bool| -> Option<PickedValue> {
        let cur = pick_ytd_exact(us_gaap, tags, curr_start_days, curr_end_days, curr_dur);
        let cur = match cur {
            Some(v) => v,
            None if default_zero => PickedValue {
                val: Decimal::ZERO,
                tag: "absent".to_owned(),
                filed: String::new(),
                period_end: curr_end.clone(),
                note: Some("absent_in_10q_assumed_zero".to_owned()),
            },
            None => return None,
        };
        let prev = pick_flow_for_period(
            us_gaap,
            tags,
            "USD",
            &["10-Q", "10-Q/A"],
            PeriodWindow {
                target_end_days: prior_end_target,
                min_dur: prior_dur_min,
                max_dur: prior_dur_max,
                max_end_delta: 15,
            },
        );
        let prev = match prev {
            Some(v) => v,
            None if default_zero => PickedValue {
                val: Decimal::ZERO,
                tag: "absent".to_owned(),
                filed: String::new(),
                period_end: curr_end.clone(),
                note: Some("prior_ytd_absent_assumed_zero".to_owned()),
            },
            None => return None,
        };
        let cv = if abs_val { cur.val.abs() } else { cur.val };
        let pv = if abs_val { prev.val.abs() } else { prev.val };
        Some(PickedValue {
            val: cv - pv,
            tag: cur.tag,
            filed: cur.filed,
            period_end: curr_end.clone(),
            note: Some(format!("ytd_curr={cv},ytd_prev={pv}")),
        })
    };

    let pick_curr_ytd = |tags: &[&str], abs_val: bool, default_zero: bool| -> Option<PickedValue> {
        match pick_ytd_exact(us_gaap, tags, curr_start_days, curr_end_days, curr_dur) {
            Some(mut v) => {
                if abs_val {
                    v.val = v.val.abs();
                }
                Some(v)
            }
            None if default_zero => Some(PickedValue {
                val: Decimal::ZERO,
                tag: "absent".to_owned(),
                filed: String::new(),
                period_end: curr_end.clone(),
                note: Some("absent_in_10q_assumed_zero".to_owned()),
            }),
            None => None,
        }
    };

    let Some(rev_delta) = pick_ytd(REV_TAGS, false, false) else {
        return Ok(None);
    };
    let Some(ebit_delta) = pick_ytd(EBIT_TAGS, false, false) else {
        return Ok(None);
    };
    let Some(ocf_delta) = pick_ytd(OCF_TAGS, false, false) else {
        return Ok(None);
    };
    let Some(capex_delta) = pick_ytd(CAPEX_TAGS, true, false) else {
        return Ok(None);
    };

    let gp_delta = if let Some(gp) = pick_ytd(GROSS_PROFIT_TAGS, false, false) {
        gp
    } else if let Some(cost) = pick_ytd(COST_TAGS, false, false) {
        PickedValue {
            val: rev_delta.val - cost.val,
            tag: format!("derived({} - {})", rev_delta.tag, cost.tag),
            filed: cost.filed,
            period_end: curr_end.clone(),
            note: Some("derived_from_cost_of_revenue".to_owned()),
        }
    } else {
        return Ok(None);
    };

    let tax_delta = pick_ytd(TAX_TAGS, false, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: None,
    });
    let pretax_delta = pick_ytd(PRETAX_TAGS, false, false).unwrap_or_else(|| ebit_delta.clone());
    let sbc_delta = pick_ytd(SBC_TAGS, true, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: None,
    });
    let buybacks_delta = pick_ytd(BUYBACK_TAGS, true, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: None,
    });

    // Current YTD figures for warning detection (e.g. GOOGL 2026 YTD CapEx > OCF).
    let ytd_rev = pick_curr_ytd(REV_TAGS, false, true).map(|p| p.val).unwrap_or(Decimal::ZERO);
    let ytd_ocf = pick_curr_ytd(OCF_TAGS, false, true).map(|p| p.val).unwrap_or(Decimal::ZERO);
    let ytd_capex = pick_curr_ytd(CAPEX_TAGS, true, true).map(|p| p.val).unwrap_or(Decimal::ZERO);
    let ytd_sbc = pick_curr_ytd(SBC_TAGS, true, true).map(|p| p.val).unwrap_or(Decimal::ZERO);
    let ytd_buybacks = pick_curr_ytd(BUYBACK_TAGS, true, true).map(|p| p.val).unwrap_or(Decimal::ZERO);

    // Derive standalone latest quarter from current YTD minus prior-quarter YTD when curr_dur > 110 days.
    let (lq_label, lq_ocf, lq_capex, lq_sbc, lq_buybacks) = if curr_dur <= 110 {
        (
            Some(curr_q.fp.clone().unwrap_or_else(|| "Q1".to_owned())),
            Some(ytd_ocf),
            Some(ytd_capex),
            Some(ytd_sbc),
            Some(ytd_buybacks),
        )
    } else if let Some(prev_q_end_days) = find_previous_ytd_end(us_gaap, curr_start_days, curr_end_days) {
        let prev_dur = prev_q_end_days - curr_start_days;
        let p_ocf = pick_ytd_exact(us_gaap, OCF_TAGS, curr_start_days, prev_q_end_days, prev_dur)
            .map(|v| ytd_ocf - v.val);
        let p_capex = pick_ytd_exact(us_gaap, CAPEX_TAGS, curr_start_days, prev_q_end_days, prev_dur)
            .map(|v| ytd_capex - v.val.abs());
        let p_sbc = pick_ytd_exact(us_gaap, SBC_TAGS, curr_start_days, prev_q_end_days, prev_dur)
            .map(|v| ytd_sbc - v.val.abs());
        let p_buybacks = pick_ytd_exact(us_gaap, BUYBACK_TAGS, curr_start_days, prev_q_end_days, prev_dur)
            .map(|v| ytd_buybacks - v.val.abs());
        (
            Some(curr_q.fp.clone().unwrap_or_else(|| "LatestQ".to_owned())),
            p_ocf,
            p_capex,
            p_sbc,
            p_buybacks,
        )
    } else {
        (None, None, None, None, None)
    };

    let all_q_forms = &["10-Q", "10-Q/A"];
    let cash_and_st = extract_cash_and_st(us_gaap, all_q_forms, curr_end_days, &curr_end)
        .map(|p| p.val)
        .unwrap_or(annual.cash_and_st);
    let debt_picked = extract_total_debt(us_gaap, all_q_forms, curr_end_days, &curr_end);
    let total_debt = if debt_picked.tag == "absent" {
        annual.total_debt
    } else {
        debt_picked.val
    };
    let equity = pick_instant_for_period(us_gaap, EQUITY_TAGS, "USD", all_q_forms, curr_end_days, 15)
        .map(|p| p.val)
        .unwrap_or(annual.equity);
    let ending_shares = pick_instant_for_period(
        us_gaap,
        ENDING_SHARES_GAAP_TAGS,
        "shares",
        all_q_forms,
        curr_end_days,
        20,
    )
    .or_else(|| pick_instant_for_period(dei, ENDING_SHARES_DEI_TAGS, "shares", all_q_forms, curr_end_days, 45))
    .map(|p| p.val)
    .unwrap_or(annual.ending_shares);

    let diluted_shares = pick_flow_for_period(
        us_gaap,
        DILUTED_SHARES_TAGS,
        "shares",
        all_q_forms,
        PeriodWindow {
            target_end_days: curr_end_days,
            min_dur: 65,
            max_dur: 115,
            max_end_delta: 15,
        },
    )
    .or_else(|| pick_ytd_exact(us_gaap, DILUTED_SHARES_TAGS, curr_start_days, curr_end_days, curr_dur))
    .map(|p| p.val)
    .unwrap_or(annual.diluted_shares);

    let fp_label = curr_q.fp.clone().unwrap_or_else(|| "YTD".to_owned());
    let latest_ytd = YtdPeriodFinancials {
        period_label: format!("{} {fp_label} YTD", &curr_end[..4]),
        start: curr_start,
        end: curr_end.clone(),
        filed: (!curr_q.filed.is_empty()).then(|| curr_q.filed.clone()),
        revenue: ytd_rev,
        ocf: ytd_ocf,
        capex: ytd_capex,
        sbc: ytd_sbc,
        buybacks: ytd_buybacks,
        latest_quarter_label: lq_label,
        latest_quarter_ocf: lq_ocf,
        latest_quarter_capex: lq_capex,
        latest_quarter_sbc: lq_sbc,
        latest_quarter_buybacks: lq_buybacks,
    };

    Ok(Some(TtmFinancials {
        as_of_end: curr_end.clone(),
        label: format!("TTM ({curr_end})"),
        revenue: annual.revenue + rev_delta.val,
        gross_profit: annual.gross_profit + gp_delta.val,
        ebit: annual.ebit + ebit_delta.val,
        tax_expense: annual.tax_expense + tax_delta.val,
        pretax_income: annual.pretax_income + pretax_delta.val,
        ocf: annual.ocf + ocf_delta.val,
        capex: annual.capex + capex_delta.val,
        sbc: annual.sbc + sbc_delta.val,
        buybacks: annual.buybacks + buybacks_delta.val,
        cash_and_st,
        total_debt,
        equity,
        diluted_shares,
        ending_shares,
        latest_ytd,
        provenance: vec![
            prov("ttm_revenue", &rev_delta),
            prov("ttm_gross_profit", &gp_delta),
            prov("ttm_ocf", &ocf_delta),
            prov("ttm_capex", &capex_delta),
            prov("ttm_sbc", &sbc_delta),
            prov("ttm_buybacks", &buybacks_delta),
        ],
    }))
}

fn pick_ytd_exact(
    us_gaap: Option<&serde_json::Map<String, Value>>,
    tags: &[&str],
    start_days: i64,
    end_days: i64,
    dur_days: i64,
) -> Option<PickedValue> {
    for tag in tags {
        let items = collect_items(us_gaap, &[tag], "USD", &["10-Q", "10-Q/A"])
            .into_iter()
            .chain(collect_items(us_gaap, &[tag], "shares", &["10-Q", "10-Q/A"]));
        let best = items
            .filter(|it| {
                let Some(s) = &it.start else {
                    return false;
                };
                let Ok(sd) = parse_date_days(s) else {
                    return false;
                };
                let Some(d) = it.duration_days else {
                    return false;
                };
                (sd - start_days).abs() <= 7
                    && (it.end_days - end_days).abs() <= 7
                    && (d - dur_days).abs() <= 10
            })
            .max_by(|a, b| a.filed.cmp(&b.filed));
        if let Some(it) = best {
            return Some(PickedValue {
                val: it.val,
                tag: it.tag,
                filed: it.filed,
                period_end: it.end,
                note: None,
            });
        }
    }
    None
}

fn find_previous_ytd_end(
    us_gaap: Option<&serde_json::Map<String, Value>>,
    start_days: i64,
    curr_end_days: i64,
) -> Option<i64> {
    let ocf_q = collect_items(us_gaap, OCF_TAGS, "USD", &["10-Q", "10-Q/A"]);
    ocf_q
        .into_iter()
        .filter_map(|it| {
            let sd = parse_date_days(it.start.as_deref()?).ok()?;
            let gap = curr_end_days - it.end_days;
            if (sd - start_days).abs() <= 7 && (70..=115).contains(&gap) {
                Some(it.end_days)
            } else {
                None
            }
        })
        .max()
}

fn prov(field: &str, picked: &PickedValue) -> FieldProvenance {
    FieldProvenance {
        field: field.to_owned(),
        source_tag: picked.tag.clone(),
        filed: (!picked.filed.is_empty()).then(|| picked.filed.clone()),
        period_end: picked.period_end.clone(),
        note: picked.note.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros_helper::*;

    mod rust_decimal_macros_helper {
        use rust_decimal::Decimal;
        pub fn dec(v: i64) -> Decimal {
            Decimal::from(v)
        }
    }

    #[test]
    fn parses_multi_tag_merge_and_dedups_by_end_date_not_fy() {
        // Simulates GOOGL switching Revenue tag between FY2024 and FY2025, plus a restated FY2024 entry in a later 10-K.
        let sample = serde_json::json!({
            "cik": 1652044,
            "entityName": "Alphabet Test Inc.",
            "facts": {
                "us-gaap": {
                    "RevenueFromContractWithCustomerExcludingAssessedTax": {
                        "units": { "USD": [
                            {"start":"2024-01-01","end":"2024-12-31","val":350000,"form":"10-K","fy":2024,"fp":"FY","filed":"2025-02-04"},
                            {"start":"2024-01-01","end":"2024-12-31","val":350018,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"}
                        ]}
                    },
                    "Revenues": {
                        "units": { "USD": [
                            {"start":"2025-01-01","end":"2025-12-31","val":402836,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2026-01-01","end":"2026-09-30","val":357269,"form":"10-Q","fy":2026,"fp":"Q3","filed":"2026-10-29"},
                            {"start":"2025-01-01","end":"2025-09-30","val":290000,"form":"10-Q","fy":2025,"fp":"Q3","filed":"2025-10-30"}
                        ]}
                    },
                    "CostOfRevenue": {
                        "units": { "USD": [
                            {"start":"2024-01-01","end":"2024-12-31","val":146306,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2025-01-01","end":"2025-12-31","val":162535,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2026-01-01","end":"2026-09-30","val":138633,"form":"10-Q","fy":2026,"fp":"Q3","filed":"2026-10-29"},
                            {"start":"2025-01-01","end":"2025-09-30","val":110000,"form":"10-Q","fy":2025,"fp":"Q3","filed":"2025-10-30"}
                        ]}
                    },
                    "OperatingIncomeLoss": {
                        "units": { "USD": [
                            {"start":"2024-01-01","end":"2024-12-31","val":112390,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2025-01-01","end":"2025-12-31","val":129039,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2026-01-01","end":"2026-09-30","val":121042,"form":"10-Q","fy":2026,"fp":"Q3","filed":"2026-10-29"},
                            {"start":"2025-01-01","end":"2025-09-30","val":95000,"form":"10-Q","fy":2025,"fp":"Q3","filed":"2025-10-30"}
                        ]}
                    },
                    "NetCashProvidedByUsedInOperatingActivities": {
                        "units": { "USD": [
                            {"start":"2024-01-01","end":"2024-12-31","val":125299,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2025-01-01","end":"2025-12-31","val":164713,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2026-01-01","end":"2026-06-30","val":84859,"form":"10-Q","fy":2026,"fp":"Q2","filed":"2026-07-30"},
                            {"start":"2026-01-01","end":"2026-09-30","val":127149,"form":"10-Q","fy":2026,"fp":"Q3","filed":"2026-10-29"},
                            {"start":"2025-01-01","end":"2025-09-30","val":100000,"form":"10-Q","fy":2025,"fp":"Q3","filed":"2025-10-30"}
                        ]}
                    },
                    "PaymentsToAcquirePropertyPlantAndEquipment": {
                        "units": { "USD": [
                            {"start":"2024-01-01","end":"2024-12-31","val":52535,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2025-01-01","end":"2025-12-31","val":91447,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2026-01-01","end":"2026-06-30","val":80598,"form":"10-Q","fy":2026,"fp":"Q2","filed":"2026-07-30"},
                            {"start":"2026-01-01","end":"2026-09-30","val":131953,"form":"10-Q","fy":2026,"fp":"Q3","filed":"2026-10-29"},
                            {"start":"2025-01-01","end":"2025-09-30","val":60000,"form":"10-Q","fy":2025,"fp":"Q3","filed":"2025-10-30"}
                        ]}
                    },
                    "ShareBasedCompensation": {
                        "units": { "USD": [
                            {"start":"2024-01-01","end":"2024-12-31","val":22785,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2025-01-01","end":"2025-12-31","val":24953,"form":"10-K","fy":2025,"fp":"FY","filed":"2026-02-05"},
                            {"start":"2026-01-01","end":"2026-09-30","val":23338,"form":"10-Q","fy":2026,"fp":"Q3","filed":"2026-10-29"},
                            {"start":"2025-01-01","end":"2025-09-30","val":18000,"form":"10-Q","fy":2025,"fp":"Q3","filed":"2025-10-30"}
                        ]}
                    },
                    "CashCashEquivalentsAndShortTermInvestments": {
                        "units": { "USD": [
                            {"end":"2024-12-31","val":95657,"form":"10-K","filed":"2025-02-04"},
                            {"end":"2025-12-31","val":126843,"form":"10-K","filed":"2026-02-05"}
                        ]}
                    },
                    "LongTermDebtNoncurrent": {
                        "units": { "USD": [
                            {"end":"2024-12-31","val":10883,"form":"10-K","filed":"2025-02-04"},
                            {"end":"2025-12-31","val":46547,"form":"10-K","filed":"2026-02-05"},
                            {"end":"2026-09-30","val":125102,"form":"10-Q","filed":"2026-10-29"}
                        ]}
                    },
                    "LongTermDebt": {
                        "units": { "USD": [
                            {"end":"2025-12-31","val":49085,"form":"10-K","filed":"2026-02-05"}
                        ]}
                    },
                    "LongTermDebtCurrent": {
                        "units": { "USD": [
                            {"end":"2025-12-31","val":1996,"form":"10-K","filed":"2026-02-05"},
                            {"end":"2026-09-30","val":1016,"form":"10-Q","filed":"2026-10-29"}
                        ]}
                    },
                    "StockholdersEquity": {
                        "units": { "USD": [
                            {"end":"2024-12-31","val":325084,"form":"10-K","filed":"2025-02-04"},
                            {"end":"2025-12-31","val":415265,"form":"10-K","filed":"2026-02-05"},
                            {"end":"2026-09-30","val":480000,"form":"10-Q","filed":"2026-10-29"}
                        ]}
                    },
                    "WeightedAverageNumberOfDilutedSharesOutstanding": {
                        "units": { "shares": [
                            {"start":"2024-01-01","end":"2024-12-31","val":12447,"form":"10-K","filed":"2026-02-05"},
                            {"start":"2025-01-01","end":"2025-12-31","val":12230,"form":"10-K","filed":"2026-02-05"},
                            {"start":"2026-07-01","end":"2026-09-30","val":12150,"form":"10-Q","filed":"2026-10-29"}
                        ]}
                    }
                }
            }
        });
        let bytes = serde_json::to_vec(&sample).unwrap();
        let parsed = parse_companyfacts_bytes(&bytes, "GOOGL", "1652044", "https://sec.test", 123).unwrap();
        assert_eq!(parsed.annuals.len(), 2);
        // Picked restated 350018 instead of 350000
        assert_eq!(parsed.annuals[0].revenue, dec(350018));
        // Derived gross profit when GrossProfit tag absent
        assert_eq!(parsed.annuals[1].revenue, dec(402836));
        assert_eq!(parsed.annuals[1].gross_profit, dec(402836 - 162535));
        // No double counting: LongTermDebtNoncurrent (46547) + LongTermDebtCurrent (1996), NOT + LongTermDebt (49085)
        assert_eq!(parsed.annuals[1].total_debt, dec(46547 + 1996));
        // TTM = 2025 FY + 2026 Q3 YTD - 2025 Q3 YTD
        let ttm = parsed.ttm.unwrap();
        assert_eq!(ttm.ocf, dec(164713 + 127149 - 100000));
        assert_eq!(ttm.capex, dec(91447 + 131953 - 60000));
        // Standalone Q3 derived from Q3 YTD - Q2 YTD
        assert_eq!(ttm.latest_ytd.latest_quarter_ocf, Some(dec(127149 - 84859)));
        assert_eq!(ttm.latest_ytd.latest_quarter_capex, Some(dec(131953 - 80598)));
    }
}
