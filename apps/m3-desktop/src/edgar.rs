use crate::{
    model::{
        AnnualFinancials, FieldProvenance, RawCompanyFactsSummary, TtmFinancials,
        YtdPeriodFinancials,
    },
    store,
};
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
    thread,
    time::Duration,
};

pub const SEC_FACTS_SCHEMA_VERSION: u32 = 1;
pub const SEC_EXTRACTOR_VERSION: u32 = 14;
pub(crate) const UNRESOLVED_CANONICAL_TICKER: &str = "__UNRESOLVED__";
pub(crate) const MAX_ANNUAL_YEARS: usize = 8;

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
const CASH_AND_EQUIV_TAG: &str = "CashAndCashEquivalentsAtCarryingValue";
const CASH_ONLY_TAG: &str = "Cash";
const CASH_EQUIVALENTS_ONLY_TAGS: &[&str] = &["CashEquivalentsAtCarryingValue"];
const CASH_RESTRICTED_COMBINED_TAGS: &[&str] = &[
    "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents",
    "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalentsIncludingDisposalGroupAndDiscontinuedOperations",
];
const RESTRICTED_CASH_CURRENT_TAGS: &[&str] = &["RestrictedCashCurrent"];
const RESTRICTED_CASH_NONCURRENT_TAGS: &[&str] = &[
    "RestrictedCashNoncurrent",
    "RestrictedCashAndCashEquivalentsNoncurrent",
];
const RESTRICTED_CASH_TOTAL_TAGS: &[&str] = &[
    "RestrictedCashAndCashEquivalents",
    "RestrictedCash",
    "RestrictedCashAndCashEquivalentsAtCarryingValue",
];
const ST_INVESTMENT_TAGS: &[&str] = &[
    "ShortTermInvestments",
    "MarketableSecuritiesCurrent",
    "AvailableForSaleSecuritiesDebtSecuritiesCurrent",
    "AvailableForSaleSecuritiesCurrent",
    "OtherShortTermInvestments",
];
const AFS_DISCLOSURE_CLUE_TAGS: &[&str] = &[
    "AvailableForSaleDebtSecuritiesAccumulatedGrossUnrealizedGainBeforeTax",
    "AvailableForSaleDebtSecuritiesAccumulatedGrossUnrealizedLossBeforeTax",
    "DebtSecuritiesAvailableForSaleAmortizedCostAfterAllowanceForCreditLossCurrent",
    "DebtSecuritiesAvailableForSaleAmortizedCostExcludingAccruedInterestAfterAllowanceForCreditLossCurrent",
];
const ST_INVESTMENT_FLOW_CLUE_TAGS: &[&str] = &[
    "PaymentsToAcquireInvestments",
    "ProceedsFromSaleMaturityAndCollectionsOfInvestments",
    "PaymentsToAcquireAvailableForSaleSecuritiesDebt",
    "ProceedsFromSaleAndMaturityOfAvailableForSaleSecurities",
    "PaymentsToAcquireShortTermInvestments",
    "ProceedsFromSaleOfShortTermInvestments",
    "ProceedsFromMaturitiesPrepaymentsAndCallsOfShorttermInvestments",
];
const INVENTORY_SUBTOTAL_FALLBACK_TAGS: &[&str] = &[
    "InventoryFinishedGoodsNetOfReserves",
    "InventoryFinishedGoods",
];
const AR_FALLBACK_TAGS: &[&str] = &["AccountsReceivableNet"];
const MONEY_MARKET_FUNDS_TAGS: &[&str] = &["MoneyMarketFundsAtCarryingValue"];
const DEPOSITS_ASSETS_CURRENT_TAGS: &[&str] = &["DepositsAssetsCurrent"];
const GENERIC_MARKETABLE_SECURITIES_TAGS: &[&str] = &["MarketableSecurities"];
const NONCURRENT_MARKETABLE_SECURITIES_TAGS: &[&str] = &[
    "MarketableSecuritiesNoncurrent",
    "AvailableForSaleSecuritiesDebtSecuritiesNoncurrent",
    "AvailableForSaleSecuritiesNoncurrent",
];
const ASSETS_TOTAL_TAGS: &[&str] = &["Assets"];
const ASSETS_CURRENT_TAGS: &[&str] = &["AssetsCurrent"];
const AR_CURRENT_TAGS: &[&str] = &[
    "AccountsReceivableNetCurrent",
    "ReceivablesNetCurrent",
    "AccountsAndNotesReceivableNet",
];
const INVENTORY_TAGS: &[&str] = &["InventoryNet", "Inventories"];
const PREPAID_AND_OTHER_CURRENT_TAGS: &[&str] = &["PrepaidExpenseAndOtherAssetsCurrent"];
const PREPAID_CURRENT_TAGS: &[&str] = &["PrepaidExpenseCurrent"];
const OTHER_CURRENT_ONLY_TAGS: &[&str] = &["OtherAssetsCurrent"];
const EXTRA_CURRENT_ASSET_GROUPS: &[&[&str]] = &[
    &[
        "AssetsOfDisposalGroupIncludingDiscontinuedOperationCurrent",
        "AssetsHeldForSaleNotPartOfDisposalGroupCurrent",
    ],
    &["ContractWithCustomerAssetNetCurrent"],
    &["DepositsAssetsCurrent"],
    &["DerivativeAssetsCurrent"],
    &["IncomeTaxesReceivableCurrent"],
];
const NONCURRENT_ASSET_GROUPS: &[&[&str]] = &[
    &[
        "PropertyPlantAndEquipmentNet",
        "PropertyPlantAndEquipmentAndFinanceLeaseRightOfUseAssetAfterAccumulatedDepreciationAndAmortization",
    ],
    &["Goodwill"],
    &["OperatingLeaseRightOfUseAsset"],
    &["FinanceLeaseRightOfUseAsset"],
    &["DeferredIncomeTaxAssetsNet", "DeferredTaxAssetsNet"],
    &[
        "RestrictedCashNoncurrent",
        "RestrictedCashAndCashEquivalentsNoncurrent",
    ],
    &["OtherAssetsNoncurrent"],
    &[
        "EquityMethodInvestments",
        "LongTermInvestments",
        "OtherInvestments",
    ],
];
const INTANGIBLE_EXCL_GW_TAGS: &[&str] = &["IntangibleAssetsNetExcludingGoodwill"];
const INTANGIBLE_FINITE_TAGS: &[&str] = &["FiniteLivedIntangibleAssetsNet"];
const INTANGIBLE_INDEF_TAGS: &[&str] = &["IndefiniteLivedIntangibleAssetsExcludingGoodwill"];
const LT_DEBT_NONCURRENT_TAGS: &[&str] = &[
    "LongTermDebtNoncurrent",
    "LongTermDebtAndCapitalLeaseObligationsNoncurrent",
    "LongTermDebtExcludingCurrentMaturities",
    "LongTermNotesAndLoans",
    "LongTermLoansPayable",
];
const LT_DEBT_PARTIAL_FALLBACK_TAGS: &[&str] = &["LongTermNotesAndLoans", "LongTermLoansPayable"];
const DEBT_TOTAL_AGGREGATE_TAGS: &[&str] = &[
    "DebtAndCapitalLeaseObligations",
    "DebtLongtermAndShorttermCombinedAmount",
];
const LT_DEBT_COMBINED_TAGS: &[&str] = &[
    "LongTermDebtAndCapitalLeaseObligationsIncludingCurrentMaturities",
    "LongTermDebt",
    "LongTermDebtAndCapitalLeaseObligations",
];
const DEBT_PARTIAL_INCOMPLETE_TAGS: &[&str] = &[
    "SeniorLongTermNotes",
    "UnsecuredLongTermDebt",
    "SecuredLongTermDebt",
    "SubordinatedLongTermDebt",
    "ConvertibleLongTermNotesPayable",
    "SeniorNotes",
];
const DEBT_LOC_TAGS: &[&str] = &["LongTermLineOfCredit", "LineOfCredit"];
const DEBT_MATURITY_12M_TAGS: &[&str] =
    &["LongTermDebtMaturitiesRepaymentsOfPrincipalInNextTwelveMonths"];
const DEBT_NONCURRENT_MATURITY_TAGS: &[&str] = &[
    "LongTermDebtMaturitiesRepaymentsOfPrincipalInYearTwo",
    "LongTermDebtMaturitiesRepaymentsOfPrincipalInYearThree",
    "LongTermDebtMaturitiesRepaymentsOfPrincipalInYearFour",
    "LongTermDebtMaturitiesRepaymentsOfPrincipalInYearFive",
    "LongTermDebtMaturitiesRepaymentsOfPrincipalAfterYearFive",
];
const DEBT_NOTES_PAYABLE_CURRENT_TAGS: &[&str] = &["NotesAndLoansPayableCurrent"];
const DEBT_CARRYING_AMOUNT_TAGS: &[&str] = &["DebtInstrumentCarryingAmount"];
const DEBT_DISCOUNT_ISSUANCE_TAGS: &[&str] = &[
    "DebtInstrumentUnamortizedDiscountPremiumAndDebtIssuanceCostsNet",
    "UnamortizedDebtIssuanceExpense",
    "DebtInstrumentUnamortizedDiscount",
];
const LT_DEBT_CURRENT_TAGS: &[&str] = &[
    "LongTermDebtCurrent",
    "LongTermDebtAndCapitalLeaseObligationsCurrent",
    "OtherLongTermDebtCurrent",
    "LoansPayableCurrent",
];
const ST_BORROWINGS_PARENT_TAGS: &[&str] = &["ShortTermBorrowings"];
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

pub(crate) fn has_share_magnitude_mismatch(a: Decimal, b: Decimal) -> bool {
    a > Decimal::ZERO
        && b > Decimal::ZERO
        && (a >= b * Decimal::from(4) || b >= a * Decimal::from(4))
}

/// Howard Hinnant civil-from-days inverse: YYYY-MM-DD -> days since Unix epoch.
pub fn parse_date_days(iso: &str) -> Result<i64> {
    let parts: Vec<&str> = iso.trim().split('-').collect();
    ensure!(parts.len() == 3, "invalid ISO date: {iso}");
    let year: i64 = parts[0]
        .parse()
        .with_context(|| format!("bad year in {iso}"))?;
    let month: i64 = parts[1]
        .parse()
        .with_context(|| format!("bad month in {iso}"))?;
    let day: i64 = parts[2]
        .parse()
        .with_context(|| format!("bad day in {iso}"))?;
    ensure!(
        (1..=12).contains(&month) && (1..=31).contains(&day),
        "out-of-range date: {iso}"
    );

    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok(era * 146_097 + doe - 719_468)
}

pub fn normalize_cik(raw: &str) -> Result<String> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    ensure!(
        !digits.is_empty() && digits.len() <= 10,
        "invalid CIK: {raw}"
    );
    Ok(format!("{digits:0>10}"))
}

const SEC_COMPANY_TICKERS_V1_URL: &str = "https://www.sec.gov/files/company_tickers.json";

pub(crate) fn ensure_v1_tickers_cache(
    client: &Client,
    cache_path: &Path,
    refresh: bool,
) -> Result<Vec<u8>> {
    ensure_v1_tickers_cache_with_url(client, cache_path, refresh, SEC_COMPANY_TICKERS_V1_URL)
}

pub(crate) fn ensure_v1_tickers_cache_with_url(
    client: &Client,
    cache_path: &Path,
    refresh: bool,
    url: &str,
) -> Result<Vec<u8>> {
    if cache_path.exists() && !refresh {
        return fs::read(cache_path).with_context(|| format!("read {}", cache_path.display()));
    }
    thread::sleep(Duration::from_millis(160));
    let resp = client
        .get(url)
        .send()
        .context("request SEC company_tickers.json")?;
    ensure!(
        resp.status().is_success(),
        "SEC company_tickers.json returned HTTP {}",
        resp.status()
    );
    let raw = resp
        .bytes()
        .context("read SEC company_tickers body")?
        .to_vec();
    validate_company_tickers_v1_json(&raw)?;
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = store::atomic_temp_path(cache_path);
    if let Err(err) = fs::write(&tmp, &raw).and_then(|_| fs::rename(&tmp, cache_path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(raw)
}

pub(crate) fn canonical_ticker_for_cik_from_v1_bytes(
    bytes: &[u8],
    norm_cik: &str,
) -> Option<String> {
    let root: Value = serde_json::from_slice(bytes).ok()?;
    let map = root.as_object()?;
    let mut entries: Vec<(usize, String, String)> = Vec::new();
    for (idx_key, entry) in map {
        let entry_cik = if let Some(n) = entry["cik_str"].as_u64() {
            format!("{n:010}")
        } else if let Some(s) = entry["cik_str"].as_str() {
            let Ok(n) = normalize_cik(s) else {
                continue;
            };
            n
        } else {
            continue;
        };
        if entry_cik != norm_cik {
            continue;
        }
        let Some(ticker) = entry["ticker"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_uppercase())
        else {
            continue;
        };
        let title = entry["title"]
            .as_str()
            .map(str::trim)
            .unwrap_or("")
            .to_owned();
        let rank = idx_key.parse::<usize>().unwrap_or(usize::MAX);
        entries.push((rank, ticker, title));
    }
    if entries.is_empty() {
        return None;
    }
    entries.sort_by_key(|(rank, _, _)| *rank);
    let siblings: Vec<String> = entries.iter().map(|(_, t, _)| t.clone()).collect();
    entries
        .into_iter()
        .min_by_key(|(rank, ticker, title)| {
            (
                u8::from(crate::calculator::is_non_common_ticker(ticker, &siblings)),
                u8::from(ticker.contains('-') || ticker.contains('.')),
                u8::from(!crate::calculator::ticker_matches_issuer_name(
                    ticker, title,
                )),
                *rank,
            )
        })
        .map(|(_, ticker, _)| ticker)
}

pub fn lookup_cik_by_ticker(
    client: &Client,
    cache_path: &Path,
    ticker: &str,
    refresh: bool,
) -> Result<String> {
    let target = ticker.trim().to_ascii_uppercase();
    let bytes = ensure_v1_tickers_cache(client, cache_path, refresh)?;
    let root = validate_company_tickers_v1_json(&bytes)?;
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

fn validate_company_tickers_v1_json(bytes: &[u8]) -> Result<Value> {
    let root: Value = serde_json::from_slice(bytes).context("parse company_tickers.json")?;
    let map = root
        .as_object()
        .context("company_tickers.json must be JSON object")?;
    let has_valid_entry = map.values().any(|entry| {
        let has_ticker = entry["ticker"]
            .as_str()
            .is_some_and(|t| !t.trim().is_empty());
        let has_cik = entry["cik_str"].as_u64().is_some()
            || entry["cik_str"]
                .as_str()
                .is_some_and(|s| normalize_cik(s).is_ok());
        has_ticker && has_cik
    });
    ensure!(
        has_valid_entry,
        "SEC company_tickers.json contains no valid ticker/CIK entries"
    );
    Ok(root)
}

pub(crate) fn fetch_and_extract_from_url(
    client: &Client,
    ticker: &str,
    norm_cik: &str,
    url: &str,
    raw_cache_path: &Path,
    fetched_at_ms: u64,
) -> Result<RawCompanyFactsSummary> {
    thread::sleep(Duration::from_millis(160));
    let resp = client
        .get(url)
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
    let summary = parse_companyfacts_bytes(&raw_bytes, ticker, norm_cik, url, fetched_at_ms)?;

    if let Some(parent) = raw_cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = store::atomic_temp_path(raw_cache_path);
    if let Err(err) = fs::write(&tmp, &raw_bytes).and_then(|_| fs::rename(&tmp, raw_cache_path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }

    Ok(summary)
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
    fetch_and_extract_from_url(
        client,
        ticker,
        &norm_cik,
        &url,
        raw_cache_path,
        fetched_at_ms,
    )
}

#[derive(Deserialize)]
struct RawCompanyFactsDoc<'a> {
    #[serde(rename = "entityName", default)]
    entity_name: Option<String>,
    #[serde(borrow, default)]
    facts: Option<HashMap<String, Option<HashMap<String, RawTagObj<'a>>>>>,
}

#[derive(Deserialize)]
struct RawTagObj<'a> {
    #[serde(borrow, default)]
    units: HashMap<String, Option<Vec<RawFactEntry<'a>>>>,
}

#[derive(Deserialize)]
struct RawFactEntry<'a> {
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
    #[serde(borrow, default)]
    val: Option<&'a RawValue>,
    #[serde(default)]
    form: Option<String>,
    #[serde(default)]
    filed: Option<String>,
    #[serde(default)]
    fp: Option<String>,
}

type FactNamespace<'a> = HashMap<String, RawTagObj<'a>>;

fn parse_raw_decimal(raw: &RawValue) -> Option<Decimal> {
    crate::model::parse_raw_json_decimal_token(raw.get())
        .ok()
        .flatten()
}

pub fn parse_companyfacts_bytes(
    raw_bytes: &[u8],
    ticker: &str,
    cik: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<RawCompanyFactsSummary> {
    let sha256 = hex_digest(raw_bytes);
    let root: RawCompanyFactsDoc<'_> =
        serde_json::from_slice(raw_bytes).context("parse companyfacts JSON")?;
    let entity_name = root
        .entity_name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| ticker.to_owned());
    let facts = root.facts.context("companyfacts missing facts object")?;
    let us_gaap = facts.get("us-gaap").and_then(|opt| opt.as_ref());
    let dei = facts.get("dei").and_then(|opt| opt.as_ref());

    let annual_ends = discover_annual_periods(us_gaap, MAX_ANNUAL_YEARS)?;
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
        schema_version: SEC_FACTS_SCHEMA_VERSION,
        extractor_version: SEC_EXTRACTOR_VERSION,
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

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn period_ends_match_allowing_calendar_shift(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (parse_date_days(a), parse_date_days(b)) {
        (Ok(da), Ok(db)) => (da - db).abs() <= 15,
        _ => false,
    }
}

/// Detect coverage regressions across period keys (annual `period_end`s and TTM `period_end`),
/// not merely counts, so losing an intermediate fiscal year while adding a newer year is rejected.
/// Only an oldest period `< new_min` is permitted to drop when `new_period_ends` saturates
/// the `MAX_ANNUAL_YEARS` sliding window on a newer filing.
pub(crate) fn is_annual_period_coverage_regression(
    prev_period_ends: &[String],
    prev_ttm_end: Option<&str>,
    new_period_ends: &[String],
    new_ttm_end: Option<&str>,
    new_latest_end: Option<&str>,
) -> bool {
    if prev_period_ends.is_empty() {
        return false;
    }
    if new_period_ends.is_empty() || new_period_ends.len() < prev_period_ends.len() {
        return true;
    }
    let prev_max = prev_period_ends.iter().map(String::as_str).max();
    let new_max = new_latest_end.or_else(|| new_period_ends.iter().map(String::as_str).max());
    if let (Some(p), Some(n)) = (prev_max, new_max)
        && n < p
    {
        return true;
    }
    let new_min = new_period_ends.iter().map(String::as_str).min();
    let window_rolled_forward = new_period_ends.len() >= MAX_ANNUAL_YEARS && new_max > prev_max;
    for prev_end in prev_period_ends {
        let preserved = new_period_ends
            .iter()
            .any(|n| period_ends_match_allowing_calendar_shift(prev_end, n));
        if preserved {
            continue;
        }
        let rolled_off_oldest =
            window_rolled_forward && new_min.is_some_and(|min_end| prev_end.as_str() < min_end);
        if !rolled_off_oldest {
            return true;
        }
    }
    if let Some(p_ttm) = prev_ttm_end {
        if let Some(n_ttm) = new_ttm_end {
            if n_ttm < p_ttm {
                return true;
            }
        } else if new_max.is_none_or(|n| n <= p_ttm) {
            return true;
        }
    }
    false
}

fn collect_items(
    ns: Option<&FactNamespace<'_>>,
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
        let Some(Some(units)) = tag_obj.units.get(unit_key) else {
            continue;
        };
        for item in units {
            let Some(form) = item.form.as_deref() else {
                continue;
            };
            if !allowed_forms.contains(&form) {
                continue;
            }
            let Some(end) = item.end.as_deref() else {
                continue;
            };
            let Ok(end_days) = parse_date_days(end) else {
                continue;
            };
            let start = item.start.clone();
            let duration_days = match &start {
                Some(s) => match parse_date_days(s) {
                    Ok(sd) => Some(end_days - sd),
                    Err(_) => continue,
                },
                None => None,
            };
            let Some(val) = item.val.and_then(parse_raw_decimal) else {
                continue;
            };
            out.push(FactItem {
                tag: (*tag).to_owned(),
                start,
                end: end.to_owned(),
                end_days,
                duration_days,
                val,
                filed: item.filed.clone().unwrap_or_default(),
                fp: item.fp.clone(),
            });
        }
    }
    out
}

fn discover_annual_periods(
    us_gaap: Option<&FactNamespace<'_>>,
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
    ns: Option<&FactNamespace<'_>>,
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
    ns: Option<&FactNamespace<'_>>,
    tags: &[&str],
    unit_key: &str,
    forms: &[&str],
    target_end_days: i64,
    max_end_delta: i64,
) -> Option<PickedValue> {
    pick_instant_bounded(
        ns,
        tags,
        unit_key,
        forms,
        target_end_days,
        max_end_delta,
        None,
    )
}

fn pick_instant_bounded(
    ns: Option<&FactNamespace<'_>>,
    tags: &[&str],
    unit_key: &str,
    forms: &[&str],
    target_end_days: i64,
    max_end_delta: i64,
    max_allowed: Option<Decimal>,
) -> Option<PickedValue> {
    for tag in tags {
        let items: Vec<FactItem> = collect_items(ns, &[tag], unit_key, forms)
            .into_iter()
            .filter(|it| {
                it.start.is_none() && (it.end_days - target_end_days).abs() <= max_end_delta
            })
            .collect();
        let best_cmp = |a: &&FactItem, b: &&FactItem| {
            let da = (a.end_days - target_end_days).abs();
            let db = (b.end_days - target_end_days).abs();
            db.cmp(&da).then_with(|| a.filed.cmp(&b.filed))
        };
        let best = max_allowed
            .and_then(|ceil| items.iter().filter(|it| it.val <= ceil).max_by(best_cmp))
            .or_else(|| items.iter().max_by(best_cmp))
            .cloned();
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
    us_gaap: Option<&FactNamespace<'_>>,
    dei: Option<&FactNamespace<'_>>,
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

    let gross_profit = if let Some(gp) =
        pick_flow_for_period(us_gaap, GROSS_PROFIT_TAGS, "USD", forms, annual_win)
    {
        gp
    } else if let Some(cost) = pick_flow_for_period(us_gaap, COST_TAGS, "USD", forms, annual_win) {
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

    let tax_expense =
        pick_flow_for_period(us_gaap, TAX_TAGS, "USD", forms, annual_win).unwrap_or(PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some("absent_in_xbrl_assumed_zero".to_owned()),
        });
    let pretax_income = pick_flow_for_period(us_gaap, PRETAX_TAGS, "USD", forms, annual_win)
        .unwrap_or(PickedValue {
            val: ebit.val,
            tag: "fallback_ebit".to_owned(),
            filed: ebit.filed.clone(),
            period_end: period_end.to_owned(),
            note: Some("pretax_absent_used_ebit".to_owned()),
        });
    let sbc =
        pick_flow_for_period(us_gaap, SBC_TAGS, "USD", forms, annual_win).unwrap_or(PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some("absent_in_xbrl_assumed_zero".to_owned()),
        });
    let buybacks_raw = pick_flow_for_period(us_gaap, BUYBACK_TAGS, "USD", forms, annual_win)
        .unwrap_or(PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some("absent_in_xbrl_assumed_zero".to_owned()),
        });

    let Some(equity) = pick_instant_for_period(us_gaap, EQUITY_TAGS, "USD", forms, end_days, 15)
    else {
        return Ok(None);
    };

    let (cash_and_st, cash_val_opt) =
        match extract_cash_and_st(us_gaap, forms, end_days, period_end) {
            CashExtraction::Present(p) => {
                let val = Some(p.val);
                (p, val)
            }
            CashExtraction::Ambiguous(p) => (p, None),
            CashExtraction::Missing => return Ok(None),
        };
    let total_debt = extract_total_debt(us_gaap, forms, end_days, period_end);

    let ending_shares = pick_instant_for_period(
        us_gaap,
        ENDING_SHARES_GAAP_TAGS,
        "shares",
        forms,
        end_days,
        20,
    )
    .or_else(|| {
        pick_instant_for_period(dei, ENDING_SHARES_DEI_TAGS, "shares", forms, end_days, 45)
    });

    let Some(diluted_shares) =
        pick_flow_for_period(us_gaap, DILUTED_SHARES_TAGS, "shares", forms, annual_win)
            .or_else(|| ending_shares.clone())
    else {
        return Ok(None);
    };

    let ending_shares_found = ending_shares.is_some();
    let ending_shares = ending_shares.unwrap_or_else(|| diluted_shares.clone());
    let mut diluted_prov = prov("diluted_shares", &diluted_shares);
    if ending_shares_found && has_share_magnitude_mismatch(diluted_shares.val, ending_shares.val) {
        diluted_prov.note = Some(format!(
            "share_basis_magnitude_mismatch:diluted={},ending={}",
            diluted_shares.val, ending_shares.val
        ));
    }

    let fiscal_year = period_end
        .get(..4)
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);

    let filed = [&rev.filed, &ocf.filed, &equity.filed]
        .into_iter()
        .filter(|s| !s.is_empty())
        .max()
        .cloned();

    let provenance = vec![
        prov("revenue", &rev),
        prov("gross_profit", &gross_profit),
        prov("ebit", &ebit),
        prov("tax_expense", &tax_expense),
        prov("pretax_income", &pretax_income),
        prov("ocf", &ocf),
        prov("capex", &capex_raw),
        prov("sbc", &sbc),
        prov("buybacks", &buybacks_raw),
        prov("cash_and_st", &cash_and_st),
        prov("total_debt", &total_debt),
        prov("equity", &equity),
        diluted_prov,
        prov("ending_shares", &ending_shares),
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
        cash_and_st: cash_val_opt,
        total_debt: total_debt.val,
        equity: equity.val,
        diluted_shares: diluted_shares.val,
        ending_shares: ending_shares.val,
        provenance,
    }))
}

enum CashExtraction {
    Present(PickedValue),
    Ambiguous(PickedValue),
    Missing,
}

enum GenericMarketableClass {
    Current(PickedValue),
    NoncurrentExcluded,
    Missing,
    Ambiguous(String),
}

fn sum_known_current_assets_excl_ms(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
    cash_val: Decimal,
    cash_is_unrestricted: bool,
) -> (Decimal, Decimal) {
    let rc_curr = if cash_is_unrestricted {
        pick_instant_for_period(
            us_gaap,
            RESTRICTED_CASH_CURRENT_TAGS,
            "USD",
            forms,
            end_days,
            15,
        )
        .map_or(Decimal::ZERO, |p| p.val)
    } else {
        Decimal::ZERO
    };
    let ar = pick_instant_for_period(us_gaap, AR_CURRENT_TAGS, "USD", forms, end_days, 15)
        .map_or(Decimal::ZERO, |p| p.val);
    let inv = pick_instant_for_period(us_gaap, INVENTORY_TAGS, "USD", forms, end_days, 15)
        .map_or(Decimal::ZERO, |p| p.val);
    let prepaid_and_other = pick_instant_for_period(
        us_gaap,
        PREPAID_AND_OTHER_CURRENT_TAGS,
        "USD",
        forms,
        end_days,
        15,
    )
    .map_or_else(
        || {
            let prepaid =
                pick_instant_for_period(us_gaap, PREPAID_CURRENT_TAGS, "USD", forms, end_days, 15)
                    .map_or(Decimal::ZERO, |p| p.val);
            let other = pick_instant_for_period(
                us_gaap,
                OTHER_CURRENT_ONLY_TAGS,
                "USD",
                forms,
                end_days,
                15,
            )
            .map_or(Decimal::ZERO, |p| p.val);
            prepaid + other
        },
        |p| p.val,
    );
    let core_without_ms = cash_val + rc_curr + ar + inv + prepaid_and_other;
    let extra: Decimal = EXTRA_CURRENT_ASSET_GROUPS
        .iter()
        .filter_map(|group| pick_instant_for_period(us_gaap, group, "USD", forms, end_days, 15))
        .map(|p| p.val)
        .sum();
    (core_without_ms, core_without_ms + extra)
}

fn current_assets_reconcile_with_addition(
    assets_current: Decimal,
    core_without_ms: Decimal,
    full_without_ms: Decimal,
    addition: Decimal,
    tol: Decimal,
) -> bool {
    let half_add = addition / Decimal::from(2);
    let core_with = (assets_current - (core_without_ms + addition)).abs() <= tol
        && (assets_current - core_without_ms).abs() > half_add;
    let full_with = (assets_current - (full_without_ms + addition)).abs() <= tol
        && (assets_current - full_without_ms).abs() > half_add;
    (core_with && (assets_current - full_without_ms).abs() > tol) || full_with
}

fn sum_known_noncurrent_assets(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
) -> Decimal {
    let base: Decimal = NONCURRENT_ASSET_GROUPS
        .iter()
        .filter_map(|group| pick_instant_for_period(us_gaap, group, "USD", forms, end_days, 15))
        .map(|p| p.val)
        .sum();
    let intangibles =
        pick_instant_for_period(us_gaap, INTANGIBLE_EXCL_GW_TAGS, "USD", forms, end_days, 15)
            .map_or_else(
                || {
                    let finite = pick_instant_for_period(
                        us_gaap,
                        INTANGIBLE_FINITE_TAGS,
                        "USD",
                        forms,
                        end_days,
                        15,
                    )
                    .map_or(Decimal::ZERO, |p| p.val);
                    let indef = pick_instant_for_period(
                        us_gaap,
                        INTANGIBLE_INDEF_TAGS,
                        "USD",
                        forms,
                        end_days,
                        15,
                    )
                    .map_or(Decimal::ZERO, |p| p.val);
                    finite + indef
                },
                |p| p.val,
            );
    base + intangibles
}

fn classify_generic_marketable_securities(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
    cash_val: Decimal,
    cash_is_unrestricted: bool,
) -> GenericMarketableClass {
    let Some(ms) = pick_instant_for_period(
        us_gaap,
        GENERIC_MARKETABLE_SECURITIES_TAGS,
        "USD",
        forms,
        end_days,
        15,
    ) else {
        return GenericMarketableClass::Missing;
    };
    if ms.val <= Decimal::ZERO {
        return GenericMarketableClass::NoncurrentExcluded;
    }

    let period_forms = &["10-K", "10-K/A", "10-Q", "10-Q/A"];
    let nc_opt = has_positive_instant_any(
        us_gaap,
        NONCURRENT_MARKETABLE_SECURITIES_TAGS,
        period_forms,
        end_days,
    );
    let assets_current =
        pick_instant_for_period(us_gaap, ASSETS_CURRENT_TAGS, "USD", forms, end_days, 15);
    let tol = assets_current
        .as_ref()
        .map_or(ms.val.abs(), |ac| ac.val.abs())
        / Decimal::from(500);
    let tol = tol.max(Decimal::from(1000));

    if let Some((nc_tag, nc_val)) = nc_opt {
        if ms.val <= nc_val + tol {
            return GenericMarketableClass::NoncurrentExcluded;
        }
        let current_rem = ms.val - nc_val;
        if let Some(ref ac) = assets_current {
            let (core_without_ms, full_without_ms) = sum_known_current_assets_excl_ms(
                us_gaap,
                forms,
                end_days,
                cash_val,
                cash_is_unrestricted,
            );
            if (ac.val - core_without_ms).abs() > tol
                && current_assets_reconcile_with_addition(
                    ac.val,
                    core_without_ms,
                    full_without_ms,
                    current_rem,
                    tol,
                )
            {
                return GenericMarketableClass::Current(PickedValue {
                    val: current_rem,
                    tag: format!("derived({} - {nc_tag})", ms.tag),
                    filed: ms.filed,
                    period_end: ms.period_end,
                    note: Some("generic_marketable_securities_net_of_noncurrent".to_owned()),
                });
            }
        }
        return GenericMarketableClass::Ambiguous(
            "ambiguous_marketable_securities_mixed_current_noncurrent".to_owned(),
        );
    }

    let Some(assets_current) = assets_current else {
        return GenericMarketableClass::Ambiguous(
            "ambiguous_marketable_securities_without_assets_current".to_owned(),
        );
    };

    let (core_without_ms, full_without_ms) =
        sum_known_current_assets_excl_ms(us_gaap, forms, end_days, cash_val, cash_is_unrestricted);
    let core_excl = (assets_current.val - core_without_ms).abs() <= tol;
    let full_excl = (assets_current.val - full_without_ms).abs() <= tol;
    let core_with = (assets_current.val - (core_without_ms + ms.val)).abs() <= tol
        && (assets_current.val - core_without_ms).abs() > ms.val / Decimal::from(2);

    if core_excl || (full_excl && !core_with) || core_without_ms + ms.val > assets_current.val + tol
    {
        return GenericMarketableClass::NoncurrentExcluded;
    }
    if current_assets_reconcile_with_addition(
        assets_current.val,
        core_without_ms,
        full_without_ms,
        ms.val,
        tol,
    ) {
        return GenericMarketableClass::Current(ms);
    }

    if let Some(assets) =
        pick_instant_for_period(us_gaap, ASSETS_TOTAL_TAGS, "USD", forms, end_days, 15)
    {
        let expected_nc = assets.val - assets_current.val;
        let known_nc = sum_known_noncurrent_assets(us_gaap, forms, end_days);
        if (expected_nc - known_nc) < (ms.val - tol) {
            return GenericMarketableClass::Current(ms);
        }
    }

    GenericMarketableClass::Ambiguous(
        "ambiguous_marketable_securities_unreconciled_current_assets".to_owned(),
    )
}

fn resolve_restricted_cash_for_combined(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
) -> Option<(Decimal, String)> {
    let rc_curr = pick_instant_for_period(
        us_gaap,
        RESTRICTED_CASH_CURRENT_TAGS,
        "USD",
        forms,
        end_days,
        15,
    );
    let rc_noncurr = pick_instant_for_period(
        us_gaap,
        RESTRICTED_CASH_NONCURRENT_TAGS,
        "USD",
        forms,
        end_days,
        15,
    );
    let rc_total = pick_instant_for_period(
        us_gaap,
        RESTRICTED_CASH_TOTAL_TAGS,
        "USD",
        forms,
        end_days,
        15,
    );
    match (&rc_curr, &rc_noncurr) {
        (Some(c), Some(n)) => Some((c.val + n.val, format!("{} + {}", c.tag, n.tag))),
        (Some(p), None) | (None, Some(p)) => match rc_total {
            Some(tot) if tot.val >= p.val => Some((tot.val, tot.tag)),
            _ => Some((p.val, p.tag.clone())),
        },
        (None, None) => rc_total.map(|r| (r.val, r.tag)),
    }
}

fn combine_disjoint_cash_and_equiv(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
    period_end: &str,
    cash_only: PickedValue,
    equiv: PickedValue,
    combined_rc_and_restricted: Option<(Decimal, Option<Decimal>)>,
) -> PickedValue {
    let sum_val = cash_only.val + equiv.val;
    let should_combine = if let Some((crc_val, rc_opt)) = combined_rc_and_restricted {
        let rc_val = rc_opt.unwrap_or(Decimal::ZERO);
        let net_rc = crc_val - rc_val;
        let tol = (crc_val.abs().max(sum_val) / Decimal::from(500)).max(Decimal::from(1000));
        (net_rc - sum_val).abs() <= tol
            && (net_rc - cash_only.val).abs() > equiv.val / Decimal::from(2)
    } else if cash_only.filed == equiv.filed
        && let Some(ac) =
            pick_instant_for_period(us_gaap, ASSETS_CURRENT_TAGS, "USD", forms, end_days, 15)
    {
        let (core_co, full_co) =
            sum_known_current_assets_excl_ms(us_gaap, forms, end_days, cash_only.val, true);
        let st_known =
            pick_instant_for_period(us_gaap, ST_INVESTMENT_TAGS, "USD", forms, end_days, 15)
                .map_or(Decimal::ZERO, |s| s.val);
        let tol = (ac.val.abs() / Decimal::from(500)).max(Decimal::from(1000));
        let already_reconciled = (ac.val - (core_co + st_known)).abs() <= tol
            || (ac.val - (full_co + st_known)).abs() <= tol;
        let reconciles_with_equiv = ((ac.val - (core_co + equiv.val + st_known)).abs() <= tol
            || (ac.val - (full_co + equiv.val + st_known)).abs() <= tol)
            && (ac.val - (core_co + st_known)).abs() > equiv.val / Decimal::from(2);
        !already_reconciled && reconciles_with_equiv
    } else {
        false
    };

    if should_combine {
        PickedValue {
            val: sum_val,
            tag: format!("{} + {}", cash_only.tag, equiv.tag),
            filed: cash_only.filed.max(equiv.filed),
            period_end: period_end.to_owned(),
            note: Some("disjoint_cash_and_equivalents".to_owned()),
        }
    } else {
        cash_only
    }
}

fn current_assets_tol(ac_val: Decimal) -> Decimal {
    (ac_val.abs() / Decimal::from(500)).max(Decimal::from(1000))
}

struct CashPeriodCtx<'a> {
    us_gaap: Option<&'a FactNamespace<'a>>,
    forms: &'a [&'a str],
    end_days: i64,
    period_end: &'a str,
    ac_opt: Option<&'a PickedValue>,
}

fn verify_mmf_overlap_st_split(
    ctx: &CashPeriodCtx<'_>,
    cash: &PickedValue,
    st_val: &PickedValue,
    mmf: &PickedValue,
) -> Option<PickedValue> {
    if mmf.val <= Decimal::ZERO || mmf.val > cash.val || st_val.val <= mmf.val {
        return None;
    }
    let st_outside = st_val.val - mmf.val;
    if let Some(ac) = ctx.ac_opt
        && cash.val + st_outside > ac.val + current_assets_tol(ac.val)
    {
        return None;
    }
    let cash_only = pick_instant_for_period(
        ctx.us_gaap,
        &[CASH_ONLY_TAG],
        "USD",
        ctx.forms,
        ctx.end_days,
        15,
    )?;
    let deposits_curr = pick_instant_for_period(
        ctx.us_gaap,
        DEPOSITS_ASSETS_CURRENT_TAGS,
        "USD",
        ctx.forms,
        ctx.end_days,
        15,
    )
    .map_or(Decimal::ZERO, |p| p.val);
    let cash_tol = current_assets_tol(cash.val);
    if (cash_only.val + deposits_curr + mmf.val - cash.val).abs() > cash_tol {
        return None;
    }
    let other_curr = pick_instant_for_period(
        ctx.us_gaap,
        PREPAID_AND_OTHER_CURRENT_TAGS,
        "USD",
        ctx.forms,
        ctx.end_days,
        15,
    )
    .or_else(|| {
        pick_instant_for_period(
            ctx.us_gaap,
            OTHER_CURRENT_ONLY_TAGS,
            "USD",
            ctx.forms,
            ctx.end_days,
            15,
        )
    })
    .map_or(Decimal::ZERO, |p| p.val);
    if other_curr < st_outside {
        return None;
    }
    Some(PickedValue {
        val: cash.val + st_outside,
        tag: format!("{} + derived({} - {})", cash.tag, st_val.tag, mmf.tag),
        filed: cash
            .filed
            .as_str()
            .max(st_val.filed.as_str())
            .max(mmf.filed.as_str())
            .to_owned(),
        period_end: ctx.period_end.to_owned(),
        note: Some("st_investments_net_of_mmf_in_cash".to_owned()),
    })
}

/// Selects the authoritative short-term investment candidate for a period, prioritizing the
/// reported aggregate (`ShortTermInvestments`) over partial component tags and reconciling
/// disjoint components against `AssetsCurrent` while preserving selected provenance.
fn select_best_st_investment_candidate(
    ctx: &CashPeriodCtx<'_>,
    cash: &PickedValue,
    cash_is_unrestricted: bool,
    ac_ceiling: Option<Decimal>,
) -> Option<PickedValue> {
    let mut candidates: Vec<PickedValue> = Vec::with_capacity(ST_INVESTMENT_TAGS.len());
    for tag in ST_INVESTMENT_TAGS {
        if let Some(cand) = pick_instant_bounded(
            ctx.us_gaap,
            &[tag],
            "USD",
            ctx.forms,
            ctx.end_days,
            15,
            ac_ceiling,
        ) && cand.val >= Decimal::ZERO
        {
            candidates.push(cand);
        }
    }
    if candidates.is_empty() {
        return None;
    }

    let (core_without_ms, full_without_ms) = if ctx.ac_opt.is_some() {
        sum_known_current_assets_excl_ms(
            ctx.us_gaap,
            ctx.forms,
            ctx.end_days,
            cash.val,
            cash_is_unrestricted,
        )
    } else {
        (Decimal::ZERO, Decimal::ZERO)
    };

    let fits_and_reconciles = |val: Decimal| -> bool {
        let Some(ac) = ctx.ac_opt else {
            return false;
        };
        let tol = current_assets_tol(ac.val);
        cash.val + val <= ac.val + tol
            && full_without_ms + val <= ac.val + tol
            && current_assets_reconcile_with_addition(
                ac.val,
                core_without_ms,
                full_without_ms,
                val,
                tol,
            )
    };

    if let Some(agg) = candidates.iter().find(|c| c.tag == "ShortTermInvestments") {
        if agg.val > Decimal::ZERO {
            if !fits_and_reconciles(agg.val)
                && let Some(alt) = candidates
                    .iter()
                    .filter(|c| {
                        c.tag != "ShortTermInvestments"
                            && c.val > agg.val
                            && fits_and_reconciles(c.val)
                    })
                    .max_by_key(|c| c.val)
            {
                return Some(alt.clone());
            }
            return Some(agg.clone());
        }
        if let Some(alt) = candidates
            .iter()
            .filter(|c| {
                c.tag != "ShortTermInvestments"
                    && c.val > Decimal::ZERO
                    && fits_and_reconciles(c.val)
            })
            .max_by_key(|c| c.val)
        {
            return Some(alt.clone());
        }
        return Some(agg.clone());
    }

    // Disjoint component sum when ShortTermInvestments is absent: combine a primary ST bucket
    // with OtherShortTermInvestments strictly when the sum reconciles AssetsCurrent and neither
    // part alone does.
    if let Some(other) = candidates
        .iter()
        .find(|c| c.tag == "OtherShortTermInvestments" && c.val > Decimal::ZERO)
    {
        for prim in candidates
            .iter()
            .filter(|c| c.tag != "OtherShortTermInvestments" && c.val > Decimal::ZERO)
        {
            let combined_val = prim.val + other.val;
            if fits_and_reconciles(combined_val)
                && !fits_and_reconciles(prim.val)
                && !fits_and_reconciles(other.val)
            {
                return Some(PickedValue {
                    val: combined_val,
                    tag: format!("{} + {}", prim.tag, other.tag),
                    filed: prim.filed.as_str().max(other.filed.as_str()).to_owned(),
                    period_end: ctx.period_end.to_owned(),
                    note: Some("short_term_investment_disjoint_components_reconciled".to_owned()),
                });
            }
        }
    }

    if candidates.first().is_some_and(|c| c.val == Decimal::ZERO)
        && let Some(alt) = candidates
            .iter()
            .find(|c| c.val > Decimal::ZERO && fits_and_reconciles(c.val))
    {
        return Some(alt.clone());
    }

    candidates.into_iter().next()
}

/// Detects when a missing standard ST investment tag reflects omitted current marketable
/// securities/deposits (e.g., extension tags hidden from SEC standard facts alongside an
/// unexplained AssetsCurrent shortfall) rather than a verified cash-only balance sheet.
fn detect_omitted_current_st_investments(
    ctx: &CashPeriodCtx<'_>,
    cash: &PickedValue,
    cash_is_unrestricted: bool,
) -> Option<String> {
    let ac = ctx.ac_opt?;
    let tol = current_assets_tol(ac.val);
    let (core_without_ms, full_without_ms) = sum_known_current_assets_excl_ms(
        ctx.us_gaap,
        ctx.forms,
        ctx.end_days,
        cash.val,
        cash_is_unrestricted,
    );
    let deposits_overlap = if cash.tag == CASH_AND_EQUIV_TAG {
        pick_instant_for_period(
            ctx.us_gaap,
            DEPOSITS_ASSETS_CURRENT_TAGS,
            "USD",
            ctx.forms,
            ctx.end_days,
            15,
        )
        .filter(|dep| dep.val > Decimal::ZERO && dep.val <= cash.val)
        .map_or(Decimal::ZERO, |dep| dep.val)
    } else {
        Decimal::ZERO
    };
    let mut adj_full = (full_without_ms - deposits_overlap).max(core_without_ms);

    if pick_instant_for_period(
        ctx.us_gaap,
        AR_CURRENT_TAGS,
        "USD",
        ctx.forms,
        ctx.end_days,
        15,
    )
    .is_none()
        && let Some(ar_fb) = pick_instant_for_period(
            ctx.us_gaap,
            AR_FALLBACK_TAGS,
            "USD",
            ctx.forms,
            ctx.end_days,
            15,
        )
    {
        adj_full += ar_fb.val;
    }
    if pick_instant_for_period(
        ctx.us_gaap,
        INVENTORY_TAGS,
        "USD",
        ctx.forms,
        ctx.end_days,
        15,
    )
    .is_none()
        && let Some(inv_fb) = pick_instant_for_period(
            ctx.us_gaap,
            INVENTORY_SUBTOTAL_FALLBACK_TAGS,
            "USD",
            ctx.forms,
            ctx.end_days,
            15,
        )
    {
        adj_full += inv_fb.val;
    }

    if ac.val - core_without_ms <= tol || ac.val - adj_full <= tol {
        return None;
    }

    let period_forms = &["10-K", "10-K/A", "10-Q", "10-Q/A"];
    if has_positive_instant_any(
        ctx.us_gaap,
        NONCURRENT_MARKETABLE_SECURITIES_TAGS,
        period_forms,
        ctx.end_days,
    )
    .is_some()
        || has_positive_instant_any(
            ctx.us_gaap,
            &["DebtSecuritiesAvailableForSaleExcludingAccruedInterestNoncurrent"],
            period_forms,
            ctx.end_days,
        )
        .is_some()
    {
        return None;
    }

    let has_afs_disclosure = pick_instant_for_period(
        ctx.us_gaap,
        AFS_DISCLOSURE_CLUE_TAGS,
        "USD",
        ctx.forms,
        ctx.end_days,
        15,
    )
    .is_some();

    let has_pos_inv_flow =
        collect_items(ctx.us_gaap, ST_INVESTMENT_FLOW_CLUE_TAGS, "USD", ctx.forms)
            .into_iter()
            .any(|it| (it.end_days - ctx.end_days).abs() <= 15 && it.val > Decimal::ZERO);

    let has_recent_prior_st = collect_items(ctx.us_gaap, ST_INVESTMENT_TAGS, "USD", period_forms)
        .into_iter()
        .any(|it| {
            it.start.is_none()
                && it.val > Decimal::ZERO
                && it.end_days < ctx.end_days - 15
                && (ctx.end_days - it.end_days) <= 450
        });

    if has_afs_disclosure || (has_recent_prior_st && has_pos_inv_flow) {
        return Some("missing_current_st_investments_unreconciled_current_assets".to_owned());
    }
    None
}

fn resolve_st_investment_with_cash(
    ctx: &CashPeriodCtx<'_>,
    cash: &PickedValue,
    cash_is_unrestricted: bool,
    st_val: &PickedValue,
) -> CashExtraction {
    let exceeds_ac = ctx.ac_opt.is_some_and(|ac| {
        let tol = current_assets_tol(ac.val);
        cash.val > ac.val + tol || cash.val + st_val.val > ac.val + tol
    });
    let mmf_opt = pick_instant_for_period(
        ctx.us_gaap,
        MONEY_MARKET_FUNDS_TAGS,
        "USD",
        ctx.forms,
        ctx.end_days,
        15,
    );
    let (core_without_ms, full_without_ms) = if mmf_opt.is_some() {
        sum_known_current_assets_excl_ms(
            ctx.us_gaap,
            ctx.forms,
            ctx.end_days,
            cash.val,
            cash_is_unrestricted,
        )
    } else {
        (Decimal::ZERO, Decimal::ZERO)
    };

    if let Some(ref mmf) = mmf_opt
        && mmf.val > Decimal::ZERO
        && mmf.val <= cash.val
    {
        let mmf_tol = current_assets_tol(st_val.val.max(mmf.val));
        let reconciles_full = ctx.ac_opt.is_some_and(|ac| {
            current_assets_reconcile_with_addition(
                ac.val,
                core_without_ms,
                full_without_ms,
                st_val.val,
                current_assets_tol(ac.val),
            )
        });
        if (st_val.val - mmf.val).abs() <= mmf_tol && (exceeds_ac || !reconciles_full) {
            return CashExtraction::Present(PickedValue {
                val: cash.val,
                tag: cash.tag.clone(),
                filed: cash.filed.clone(),
                period_end: ctx.period_end.to_owned(),
                note: cash.note.clone(),
            });
        }
        if (exceeds_ac || st_val.tag.starts_with("AvailableForSaleSecurities"))
            && (exceeds_ac || !reconciles_full)
        {
            if let Some(split) = verify_mmf_overlap_st_split(ctx, cash, st_val, mmf) {
                return CashExtraction::Present(split);
            }
            if exceeds_ac {
                return CashExtraction::Ambiguous(PickedValue {
                    val: Decimal::ZERO,
                    tag: "absent".to_owned(),
                    filed: cash.filed.as_str().max(st_val.filed.as_str()).to_owned(),
                    period_end: ctx.period_end.to_owned(),
                    note: Some("cash_and_st_exceeds_current_assets_unverified_split".to_owned()),
                });
            }
        }
    }

    if exceeds_ac {
        return CashExtraction::Ambiguous(PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: cash.filed.as_str().max(st_val.filed.as_str()).to_owned(),
            period_end: ctx.period_end.to_owned(),
            note: Some("cash_and_st_exceeds_current_assets_unverified_split".to_owned()),
        });
    }

    CashExtraction::Present(PickedValue {
        val: cash.val + st_val.val,
        tag: format!("{} + {}", cash.tag, st_val.tag),
        filed: cash.filed.as_str().max(st_val.filed.as_str()).to_owned(),
        period_end: ctx.period_end.to_owned(),
        note: cash.note.clone(),
    })
}

fn enforce_current_assets_ceiling(
    res: CashExtraction,
    ac_opt: Option<&PickedValue>,
    period_end: &str,
) -> CashExtraction {
    let CashExtraction::Present(ref p) = res else {
        return res;
    };
    if let Some(ac) = ac_opt
        && p.val > ac.val + current_assets_tol(ac.val)
    {
        return CashExtraction::Ambiguous(PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: p.filed.clone(),
            period_end: period_end.to_owned(),
            note: Some("cash_and_st_exceeds_current_assets".to_owned()),
        });
    }
    res
}

fn extract_component_cash_and_st(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
    period_end: &str,
    ac_opt: Option<&PickedValue>,
) -> (CashExtraction, bool) {
    let ac_ceiling = ac_opt.map(|ac| ac.val + current_assets_tol(ac.val));
    let cce = pick_instant_bounded(
        us_gaap,
        &[CASH_AND_EQUIV_TAG],
        "USD",
        forms,
        end_days,
        15,
        ac_ceiling,
    );
    let cash_only = pick_instant_bounded(
        us_gaap,
        &[CASH_ONLY_TAG],
        "USD",
        forms,
        end_days,
        15,
        ac_ceiling,
    );
    let equiv_only = pick_instant_bounded(
        us_gaap,
        CASH_EQUIVALENTS_ONLY_TAGS,
        "USD",
        forms,
        end_days,
        15,
        ac_ceiling,
    );
    let combined_rc = pick_instant_bounded(
        us_gaap,
        CASH_RESTRICTED_COMBINED_TAGS,
        "USD",
        forms,
        end_days,
        15,
        ac_ceiling,
    );
    let restricted = resolve_restricted_cash_for_combined(us_gaap, forms, end_days);

    let (cash, cash_is_unrestricted) = if let Some(unrestricted) = cce {
        (unrestricted, true)
    } else if let Some(co) = cash_only {
        let combined_rc_and_restricted = combined_rc
            .as_ref()
            .map(|c| (c.val, restricted.as_ref().map(|(v, _)| *v)));
        let resolved = match equiv_only.filter(|e| e.val > Decimal::ZERO) {
            Some(eq) => combine_disjoint_cash_and_equiv(
                us_gaap,
                forms,
                end_days,
                period_end,
                co,
                eq,
                combined_rc_and_restricted,
            ),
            None => co,
        };
        (resolved, true)
    } else if let Some(crc) = combined_rc {
        if let Some((rc_val, rc_tag)) = restricted {
            if crc.val >= rc_val {
                let derived = PickedValue {
                    val: crc.val - rc_val,
                    tag: format!("derived({} - {rc_tag})", crc.tag),
                    filed: crc.filed,
                    period_end: period_end.to_owned(),
                    note: Some("unrestricted_cash_net_of_restricted".to_owned()),
                };
                (derived, true)
            } else {
                return (
                    CashExtraction::Ambiguous(PickedValue {
                        val: Decimal::ZERO,
                        tag: "absent".to_owned(),
                        filed: crc.filed,
                        period_end: period_end.to_owned(),
                        note: Some("restricted_cash_exceeds_combined_cash".to_owned()),
                    }),
                    true,
                );
            }
        } else if let Some(eq) = equiv_only
            && (crc.val - eq.val).abs() <= current_assets_tol(crc.val.max(eq.val))
        {
            (eq, true)
        } else {
            return (
                CashExtraction::Ambiguous(PickedValue {
                    val: Decimal::ZERO,
                    tag: "absent".to_owned(),
                    filed: crc.filed,
                    period_end: period_end.to_owned(),
                    note: Some("combined_restricted_cash_missing_restricted_deduction".to_owned()),
                }),
                false,
            );
        }
    } else if let Some(eq) = equiv_only {
        (eq, true)
    } else {
        return (CashExtraction::Missing, false);
    };

    let ctx = CashPeriodCtx {
        us_gaap,
        forms,
        end_days,
        period_end,
        ac_opt,
    };
    if let Some(st_val) =
        select_best_st_investment_candidate(&ctx, &cash, cash_is_unrestricted, ac_ceiling)
    {
        let ext = resolve_st_investment_with_cash(&ctx, &cash, cash_is_unrestricted, &st_val);
        return (
            enforce_current_assets_ceiling(ext, ac_opt, period_end),
            cash_is_unrestricted,
        );
    }

    let ext = match classify_generic_marketable_securities(
        us_gaap,
        forms,
        end_days,
        cash.val,
        cash_is_unrestricted,
    ) {
        GenericMarketableClass::Current(st_val) => CashExtraction::Present(PickedValue {
            val: cash.val + st_val.val,
            tag: format!("{} + {}", cash.tag, st_val.tag),
            filed: cash.filed.max(st_val.filed),
            period_end: period_end.to_owned(),
            note: st_val.note.or(cash.note),
        }),
        GenericMarketableClass::NoncurrentExcluded => CashExtraction::Present(PickedValue {
            val: cash.val,
            tag: cash.tag,
            filed: cash.filed,
            period_end: period_end.to_owned(),
            note: cash.note,
        }),
        GenericMarketableClass::Missing => {
            if let Some(reason) =
                detect_omitted_current_st_investments(&ctx, &cash, cash_is_unrestricted)
            {
                CashExtraction::Ambiguous(PickedValue {
                    val: Decimal::ZERO,
                    tag: "absent".to_owned(),
                    filed: cash.filed,
                    period_end: period_end.to_owned(),
                    note: Some(reason),
                })
            } else {
                CashExtraction::Present(PickedValue {
                    val: cash.val,
                    tag: cash.tag,
                    filed: cash.filed,
                    period_end: period_end.to_owned(),
                    note: cash.note,
                })
            }
        }
        GenericMarketableClass::Ambiguous(reason) => CashExtraction::Ambiguous(PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: cash.filed,
            period_end: period_end.to_owned(),
            note: Some(reason),
        }),
    };
    (
        enforce_current_assets_ceiling(ext, ac_opt, period_end),
        cash_is_unrestricted,
    )
}

fn extract_cash_and_st(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
    period_end: &str,
) -> CashExtraction {
    let ac_opt = pick_instant_for_period(us_gaap, ASSETS_CURRENT_TAGS, "USD", forms, end_days, 15);
    let ac_ceiling = ac_opt
        .as_ref()
        .map(|ac| ac.val + current_assets_tol(ac.val));
    let (comp, comp_unrestricted) =
        extract_component_cash_and_st(us_gaap, forms, end_days, period_end, ac_opt.as_ref());
    if let Some(combined) = pick_instant_bounded(
        us_gaap,
        CASH_COMBINED_TAGS,
        "USD",
        forms,
        end_days,
        15,
        ac_ceiling,
    ) {
        if combined.val == Decimal::ZERO {
            let res = match comp {
                CashExtraction::Present(p) if p.val > Decimal::ZERO => CashExtraction::Present(p),
                CashExtraction::Ambiguous(p) => CashExtraction::Ambiguous(p),
                _ => CashExtraction::Present(combined),
            };
            return enforce_current_assets_ceiling(res, ac_opt.as_ref(), period_end);
        }
        if ac_opt
            .as_ref()
            .is_some_and(|ac| combined.val > ac.val + current_assets_tol(ac.val))
        {
            let res = match comp {
                CashExtraction::Present(p)
                    if ac_opt
                        .as_ref()
                        .is_none_or(|ac| p.val <= ac.val + current_assets_tol(ac.val)) =>
                {
                    CashExtraction::Present(p)
                }
                _ => CashExtraction::Ambiguous(PickedValue {
                    val: Decimal::ZERO,
                    tag: "absent".to_owned(),
                    filed: combined.filed,
                    period_end: period_end.to_owned(),
                    note: Some("combined_cash_exceeds_current_assets".to_owned()),
                }),
            };
            return enforce_current_assets_ceiling(res, ac_opt.as_ref(), period_end);
        }
        let combined_rc = pick_instant_bounded(
            us_gaap,
            CASH_RESTRICTED_COMBINED_TAGS,
            "USD",
            forms,
            end_days,
            15,
            ac_ceiling,
        );
        let restricted = resolve_restricted_cash_for_combined(us_gaap, forms, end_days);
        let combined_includes_restricted = combined_rc.as_ref().is_some_and(|crc| {
            let tol = (crc.val.abs().max(combined.val.abs()) / Decimal::from(500))
                .max(Decimal::from(1000));
            (crc.val - combined.val).abs() <= tol
                && restricted
                    .as_ref()
                    .is_some_and(|(rc_val, _)| *rc_val > Decimal::ZERO)
        });
        if comp_unrestricted
            && let CashExtraction::Present(ref p) = comp
            && (p.val > combined.val || (combined_includes_restricted && p.val < combined.val))
        {
            return enforce_current_assets_ceiling(
                CashExtraction::Present(p.clone()),
                ac_opt.as_ref(),
                period_end,
            );
        }
        return enforce_current_assets_ceiling(
            CashExtraction::Present(combined),
            ac_opt.as_ref(),
            period_end,
        );
    }

    enforce_current_assets_ceiling(comp, ac_opt.as_ref(), period_end)
}

fn has_positive_instant_any(
    us_gaap: Option<&FactNamespace<'_>>,
    tags: &[&str],
    forms: &[&str],
    end_days: i64,
) -> Option<(String, Decimal)> {
    for tag in tags {
        if let Some(p) = pick_instant_for_period(us_gaap, &[tag], "USD", forms, end_days, 15)
            && p.val > Decimal::ZERO
        {
            return Some((p.tag, p.val));
        }
    }
    None
}

fn extract_total_debt(
    us_gaap: Option<&FactNamespace<'_>>,
    forms: &[&str],
    end_days: i64,
    period_end: &str,
) -> PickedValue {
    let lt_noncurrent =
        pick_instant_for_period(us_gaap, LT_DEBT_NONCURRENT_TAGS, "USD", forms, end_days, 15);
    let lt_combined =
        pick_instant_for_period(us_gaap, LT_DEBT_COMBINED_TAGS, "USD", forms, end_days, 15);
    let lt_current =
        pick_instant_for_period(us_gaap, LT_DEBT_CURRENT_TAGS, "USD", forms, end_days, 15);
    let cp = pick_instant_for_period(us_gaap, COMMERCIAL_PAPER_TAGS, "USD", forms, end_days, 15);
    let stb_parent = pick_instant_for_period(
        us_gaap,
        ST_BORROWINGS_PARENT_TAGS,
        "USD",
        forms,
        end_days,
        15,
    );
    let other_stb = pick_instant_for_period(
        us_gaap,
        &["OtherShortTermBorrowings"],
        "USD",
        forms,
        end_days,
        15,
    );
    let st_bank_loans = pick_instant_for_period(
        us_gaap,
        &["ShortTermBankLoansAndNotesPayable"],
        "USD",
        forms,
        end_days,
        15,
    );
    let debt_current =
        pick_instant_for_period(us_gaap, DEBT_CURRENT_TAGS, "USD", forms, end_days, 15);
    let loc = pick_instant_for_period(us_gaap, DEBT_LOC_TAGS, "USD", forms, end_days, 15);
    let ltlp = pick_instant_for_period(
        us_gaap,
        &["LongTermLoansPayable"],
        "USD",
        forms,
        end_days,
        15,
    );
    let other_lt_combined = pick_instant_for_period(
        us_gaap,
        &["LongTermDebtAndCapitalLeaseObligations"],
        "USD",
        forms,
        end_days,
        15,
    );
    let carrying = pick_instant_for_period(
        us_gaap,
        DEBT_CARRYING_AMOUNT_TAGS,
        "USD",
        forms,
        end_days,
        15,
    );
    let discount_net = pick_instant_for_period(
        us_gaap,
        DEBT_DISCOUNT_ISSUANCE_TAGS,
        "USD",
        forms,
        end_days,
        15,
    );

    let push_st_borrowings = |total: &mut Decimal,
                              parts: &mut Vec<String>,
                              latest_filed: &mut String,
                              included_lt_curr: Option<Decimal>| {
        let is_dup_ltc =
            |val: Decimal| included_lt_curr.is_some_and(|ltc| ltc > Decimal::ZERO && val == ltc);
        // ShortTermBorrowings is the consolidated ST balance-sheet parent covering CommercialPaper,
        // OtherShortTermBorrowings, and ShortTermBankLoansAndNotesPayable unless clearly disjoint.
        if let Some(stb) = stb_parent.as_ref() {
            if !is_dup_ltc(stb.val) {
                *total += stb.val;
                parts.push(stb.tag.clone());
                if stb.filed > *latest_filed {
                    *latest_filed = stb.filed.clone();
                }
            }
            if let Some(c) = cp.as_ref()
                && c.val > Decimal::ZERO
                && !is_dup_ltc(c.val)
                && stb.val < c.val * Decimal::new(95, 2)
            {
                *total += c.val;
                parts.push(c.tag.clone());
                if c.filed > *latest_filed {
                    *latest_filed = c.filed.clone();
                }
            }
        } else {
            for p in [&cp, &other_stb, &st_bank_loans].into_iter().flatten() {
                if is_dup_ltc(p.val) {
                    continue;
                }
                *total += p.val;
                parts.push(p.tag.clone());
                if p.filed > *latest_filed {
                    *latest_filed = p.filed.clone();
                }
            }
        }
    };

    let push_disjoint_current =
        |total: &mut Decimal, parts: &mut Vec<String>, latest_filed: &mut String| {
            // DebtCurrent already aggregates current maturities of LT debt, CP, and ST borrowings.
            // Prefer DebtCurrent if present; otherwise combine lt_current + disjoint ST borrowings.
            if let Some(dc) = debt_current.as_ref() {
                *total += dc.val;
                parts.push(dc.tag.clone());
                if dc.filed > *latest_filed {
                    *latest_filed = dc.filed.clone();
                }
            } else {
                let ltc_val = lt_current.as_ref().map(|p| {
                    *total += p.val;
                    parts.push(p.tag.clone());
                    if p.filed > *latest_filed {
                        *latest_filed = p.filed.clone();
                    }
                    p.val
                });
                push_st_borrowings(total, parts, latest_filed, ltc_val);
            }
        };

    let cur_est = {
        let mut cur_t = Decimal::ZERO;
        let mut cur_p = Vec::new();
        let mut cur_f = String::new();
        push_disjoint_current(&mut cur_t, &mut cur_p, &mut cur_f);
        cur_t
    };

    let disjoint_loc_for = |lt: &PickedValue| -> Option<&PickedValue> {
        let l = loc
            .as_ref()
            .filter(|p| p.val > Decimal::ZERO && p.val != lt.val)?;
        let lt_is_loans_payable = lt.tag == "LongTermLoansPayable"
            || lt.tag.ends_with(":LongTermLoansPayable")
            || ltlp.as_ref().is_some_and(|p| p.val == lt.val);
        let carrying_needs_loc = carrying.as_ref().is_some_and(|ca| {
            let base = lt.val + cur_est;
            ca.val > base
                && (ca.val - (base + l.val)).abs() < (ca.val - base).abs()
                && (base + l.val) <= ca.val * Decimal::new(110, 2)
        });
        (lt_is_loans_payable || carrying_needs_loc).then_some(l)
    };

    let max_bs_component = [
        lt_noncurrent.as_ref().map(|p| p.val),
        other_lt_combined.as_ref().map(|p| p.val),
        debt_current.as_ref().map(|p| p.val),
        lt_current.as_ref().map(|p| p.val),
        cp.as_ref().map(|p| p.val),
        stb_parent.as_ref().map(|p| p.val),
        other_stb.as_ref().map(|p| p.val),
        st_bank_loans.as_ref().map(|p| p.val),
        loc.as_ref().map(|p| p.val),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(Decimal::ZERO);

    // Prefer consolidated balance-sheet carrying debt totals first when explicitly reported
    // (e.g. AMD DebtLongtermAndShorttermCombinedAmount = $3.222B; XYL $1.942B carrying below
    // $1.954B footnote principal due to $12M unamortized discount; HON $28.687B = LT+current LT
    // needing disjoint ShortTermBorrowings $5.893B -> $34.580B; while rejecting sub-component
    // lease aggregates smaller than verified debt components as in ON FY25).
    if let Some(agg) = pick_instant_for_period(
        us_gaap,
        DEBT_TOTAL_AGGREGATE_TAGS,
        "USD",
        forms,
        end_days,
        15,
    ) {
        let agg_contradicts_carrying = carrying.as_ref().is_some_and(|ca| {
            (agg.val == Decimal::ZERO && ca.val > Decimal::ZERO)
                || (agg.val > Decimal::ZERO && ca.val > agg.val * Decimal::new(115, 2))
        });
        let lt_combined_compatible_with_agg = lt_combined.as_ref().is_none_or(|ltc| {
            agg.val >= ltc.val
                || ltc.val <= agg.val * Decimal::new(105, 2)
                || discount_net.as_ref().is_some_and(|d| {
                    d.val.abs() > Decimal::ZERO
                        && (ltc.val - d.val.abs() - agg.val).abs()
                            <= agg.val * Decimal::new(2, 2) + Decimal::from(1000)
                })
        });
        if agg.val >= max_bs_component
            && lt_combined_compatible_with_agg
            && !agg_contradicts_carrying
        {
            if let Some(lt_lease) = other_lt_combined.as_ref()
                && let Some(ltc) = lt_current.as_ref()
                && ltc.val > Decimal::ZERO
                && (agg.val - (lt_lease.val + ltc.val)).abs() <= Decimal::from(1000)
            {
                let mut st_only = Decimal::ZERO;
                let mut st_parts = Vec::new();
                let mut latest_filed = agg.filed.clone();
                push_st_borrowings(
                    &mut st_only,
                    &mut st_parts,
                    &mut latest_filed,
                    Some(ltc.val),
                );
                if st_only > Decimal::ZERO
                    && carrying
                        .as_ref()
                        .is_none_or(|ca| agg.val + st_only <= ca.val * Decimal::new(105, 2))
                {
                    let mut all_parts = vec![agg.tag];
                    all_parts.extend(st_parts);
                    return PickedValue {
                        val: agg.val + st_only,
                        tag: all_parts.join(" + "),
                        filed: latest_filed,
                        period_end: period_end.to_owned(),
                        note: None,
                    };
                }
            }
            return PickedValue {
                val: agg.val,
                tag: agg.tag,
                filed: agg.filed,
                period_end: period_end.to_owned(),
                note: None,
            };
        }
    }

    // Check current and noncurrent debt coverage independently.
    let maturity_12m =
        pick_instant_for_period(us_gaap, DEBT_MATURITY_12M_TAGS, "USD", forms, end_days, 15)
            .filter(|p| p.val > Decimal::ZERO);
    let maturity_12m_pos = maturity_12m.is_some();
    let stb_matches_maturity_12m = maturity_12m.as_ref().is_some_and(|m| {
        stb_parent
            .as_ref()
            .is_some_and(|stb| (stb.val - m.val).abs() <= Decimal::from(1000))
    });
    let notes_curr_pos =
        has_positive_instant_any(us_gaap, DEBT_NOTES_PAYABLE_CURRENT_TAGS, forms, end_days)
            .is_some();
    let missing_current_lt = debt_current.is_none()
        && lt_current.is_none()
        && ((maturity_12m_pos && !stb_matches_maturity_12m) || notes_curr_pos);

    let has_partial =
        has_positive_instant_any(us_gaap, DEBT_PARTIAL_INCOMPLETE_TAGS, forms, end_days).is_some();
    let has_loc = loc.as_ref().is_some_and(|p| p.val > Decimal::ZERO);
    let has_noncurrent_maturities =
        has_positive_instant_any(us_gaap, DEBT_NONCURRENT_MATURITY_TAGS, forms, end_days).is_some();
    let disjoint_loc_nc = lt_noncurrent.as_ref().and_then(disjoint_loc_for);
    let lt_noncurrent_is_partial = lt_noncurrent.as_ref().is_some_and(|lt| {
        let is_partial_tag = LT_DEBT_PARTIAL_FALLBACK_TAGS
            .iter()
            .any(|t| lt.tag == *t || lt.tag.ends_with(&format!(":{t}")));
        if !is_partial_tag {
            return false;
        }
        let base_nc_with_loc = lt.val + disjoint_loc_nc.map_or(Decimal::ZERO, |p| p.val);
        has_partial
            || (has_loc && disjoint_loc_nc.is_none())
            || lt_combined
                .as_ref()
                .is_some_and(|c| c.val > base_nc_with_loc + cur_est)
            || carrying
                .as_ref()
                .is_some_and(|ca| ca.val > (base_nc_with_loc + cur_est) * Decimal::new(115, 2))
    });
    let usable_lt_noncurrent = if lt_noncurrent_is_partial {
        None
    } else {
        lt_noncurrent.clone()
    };
    let missing_noncurrent_lt = usable_lt_noncurrent.is_none()
        && (has_partial || has_loc || has_noncurrent_maturities || lt_noncurrent_is_partial);

    let mut total = Decimal::ZERO;
    let mut parts = Vec::new();
    let mut latest_filed = String::new();

    if let Some(lt) = usable_lt_noncurrent
        && !missing_current_lt
    {
        total += lt.val;
        parts.push(lt.tag);
        latest_filed = latest_filed.max(lt.filed);
        if let Some(l) = disjoint_loc_nc {
            total += l.val;
            parts.push(l.tag.clone());
            latest_filed = latest_filed.max(l.filed.clone());
        }
        push_disjoint_current(&mut total, &mut parts, &mut latest_filed);
    } else if let Some(lt) = lt_combined {
        // LongTermDebt / LongTermDebtAndCapitalLeaseObligations may exclude current maturities on
        // balance sheets (e.g. TSLA, ADBE, LAKE) or include them in footnotes (e.g. TDG, TMO, CE).
        // Verify whether current maturities are already included before choosing current addition.
        let disjoint_loc_c = disjoint_loc_for(&lt);
        let explicitly_includes_current = lt.tag
            == "LongTermDebtAndCapitalLeaseObligationsIncludingCurrentMaturities"
            || lt
                .tag
                .ends_with(":LongTermDebtAndCapitalLeaseObligationsIncludingCurrentMaturities");
        let matches_nc_plus_cur = lt_noncurrent.as_ref().is_some_and(|nc| {
            lt_current.as_ref().is_some_and(|lc| {
                lc.val > Decimal::ZERO && (lt.val - (nc.val + lc.val)).abs() <= Decimal::from(1000)
            })
        });
        let exceeds_other_combined_by_cur = other_lt_combined.as_ref().is_some_and(|other| {
            other.val > Decimal::ZERO
                && cur_est > Decimal::ZERO
                && lt.tag != other.tag
                && lt.val > other.val
                && (lt.val - other.val) * Decimal::from(2) >= cur_est
        });
        let carrying_confirms_included = carrying.as_ref().and_then(|ca| {
            if cur_est <= Decimal::ZERO {
                return None;
            }
            let diff_without = (ca.val - lt.val).abs();
            let diff_with = (ca.val - (lt.val + cur_est)).abs();
            if ca.val > lt.val
                && diff_with < diff_without
                && (lt.val + cur_est) <= ca.val * Decimal::new(108, 2)
            {
                Some(false)
            } else if diff_without <= diff_with && diff_without <= ca.val * Decimal::new(10, 2) {
                Some(true)
            } else {
                None
            }
        });
        let lt_already_includes_current = explicitly_includes_current
            || matches_nc_plus_cur
            || exceeds_other_combined_by_cur
            || carrying_confirms_included.unwrap_or(false);

        let bs_carrying_from_other = other_lt_combined
            .as_ref()
            .zip(lt_current.as_ref())
            .filter(|(other_lt, ltc)| {
                let sum = other_lt.val + ltc.val;
                sum > Decimal::ZERO
                    && lt.val > sum
                    && discount_net.as_ref().is_some_and(|d| {
                        d.val.abs() > Decimal::ZERO
                            && (lt.val - d.val.abs() - sum).abs() <= Decimal::from(1000)
                    })
            })
            .map(|(other_lt, ltc)| (other_lt, ltc, other_lt.val + ltc.val));

        if let Some((other_lt, ltc, carrying_sum)) = bs_carrying_from_other {
            total += carrying_sum;
            parts.push(other_lt.tag.clone());
            parts.push(ltc.tag.clone());
            latest_filed = latest_filed
                .max(other_lt.filed.clone())
                .max(ltc.filed.clone());
            push_st_borrowings(&mut total, &mut parts, &mut latest_filed, Some(ltc.val));
        } else {
            total += lt.val;
            parts.push(lt.tag);
            latest_filed = latest_filed.max(lt.filed);
            if let Some(l) = disjoint_loc_c {
                total += l.val;
                parts.push(l.tag.clone());
                latest_filed = latest_filed.max(l.filed.clone());
            }
            if lt_already_includes_current {
                push_st_borrowings(
                    &mut total,
                    &mut parts,
                    &mut latest_filed,
                    lt_current.as_ref().map(|p| p.val),
                );
            } else {
                push_disjoint_current(&mut total, &mut parts, &mut latest_filed);
            }
        }
    } else if missing_current_lt || missing_noncurrent_lt {
        let note = match (missing_current_lt, missing_noncurrent_lt) {
            (true, true) => "incomplete_current_and_noncurrent_debt_coverage",
            (true, false) => "missing_current_portion_of_lt_debt_with_nonzero_maturities",
            (false, true) => "missing_noncurrent_debt_with_partial_loc_or_maturities",
            _ => "incomplete_debt_coverage",
        };
        return PickedValue {
            val: Decimal::ZERO,
            tag: "absent".to_owned(),
            filed: String::new(),
            period_end: period_end.to_owned(),
            note: Some(note.to_owned()),
        };
    } else {
        push_disjoint_current(&mut total, &mut parts, &mut latest_filed);
        if carrying
            .as_ref()
            .is_some_and(|ca| ca.val > Decimal::ZERO && ca.val > total * Decimal::new(115, 2))
        {
            return PickedValue {
                val: Decimal::ZERO,
                tag: "absent".to_owned(),
                filed: String::new(),
                period_end: period_end.to_owned(),
                note: Some("missing_noncurrent_debt_below_carrying_amount".to_owned()),
            };
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
    us_gaap: Option<&FactNamespace<'_>>,
    dei: Option<&FactNamespace<'_>>,
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
        .max_by(|a, b| {
            a.end_days
                .cmp(&b.end_days)
                .then_with(|| a.filed.cmp(&b.filed))
        });

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
        let tag = if cur.tag == "absent" || prev.tag == "absent" {
            "absent".to_owned()
        } else if cur.tag == prev.tag {
            cur.tag.clone()
        } else {
            format!("{} / prior:{}", cur.tag, prev.tag)
        };
        Some(PickedValue {
            val: cv - pv,
            tag,
            filed: cur.filed.clone(),
            period_end: curr_end.clone(),
            note: Some(format!(
                "ytd_curr={cv}(end={},filed={});ytd_prev={pv}(end={},filed={},tag={})",
                cur.period_end, cur.filed, prev.period_end, prev.filed, prev.tag
            )),
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
            note: Some(format!(
                "derived_from_cost_of_revenue;rev=[{}];cost=[{}]",
                rev_delta.note.as_deref().unwrap_or(""),
                cost.note.as_deref().unwrap_or("")
            )),
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
    let pretax_delta = pick_ytd(PRETAX_TAGS, false, false).unwrap_or_else(|| PickedValue {
        val: ebit_delta.val,
        tag: "fallback_ebit".to_owned(),
        filed: ebit_delta.filed.clone(),
        period_end: curr_end.clone(),
        note: Some(format!(
            "pretax_absent_used_ebit;{}",
            ebit_delta.note.as_deref().unwrap_or("")
        )),
    });
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

    // Current YTD figures and explicit operand provenance (independent from prior-year YTD / FY).
    let curr_ytd_rev_picked = pick_curr_ytd(REV_TAGS, false, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: Some("absent_in_10q_assumed_zero".to_owned()),
    });
    let curr_ytd_ocf_picked = pick_curr_ytd(OCF_TAGS, false, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: Some("absent_in_10q_assumed_zero".to_owned()),
    });
    let curr_ytd_capex_picked = pick_curr_ytd(CAPEX_TAGS, true, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: Some("absent_in_10q_assumed_zero".to_owned()),
    });
    let curr_ytd_sbc_picked = pick_curr_ytd(SBC_TAGS, true, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: Some("absent_in_10q_assumed_zero".to_owned()),
    });
    let curr_ytd_buybacks_picked = pick_curr_ytd(BUYBACK_TAGS, true, true).unwrap_or(PickedValue {
        val: Decimal::ZERO,
        tag: "absent".to_owned(),
        filed: String::new(),
        period_end: curr_end.clone(),
        note: Some("absent_in_10q_assumed_zero".to_owned()),
    });

    let ytd_rev = curr_ytd_rev_picked.val;
    let ytd_ocf = curr_ytd_ocf_picked.val;
    let ytd_capex = curr_ytd_capex_picked.val;
    let ytd_sbc = curr_ytd_sbc_picked.val;
    let ytd_buybacks = curr_ytd_buybacks_picked.val;

    // Derive standalone latest quarter from current YTD minus prior-quarter YTD when curr_dur > 110 days.
    let (lq_label, lq_ocf, lq_capex, lq_sbc, lq_buybacks) = if curr_dur <= 110 {
        (
            Some(curr_q.fp.clone().unwrap_or_else(|| "Q1".to_owned())),
            (curr_ytd_ocf_picked.tag != "absent").then_some(ytd_ocf),
            (curr_ytd_capex_picked.tag != "absent").then_some(ytd_capex),
            (curr_ytd_sbc_picked.tag != "absent").then_some(ytd_sbc),
            (curr_ytd_buybacks_picked.tag != "absent").then_some(ytd_buybacks),
        )
    } else if let Some(prev_q_end_days) =
        find_previous_ytd_end(us_gaap, curr_start_days, curr_end_days)
    {
        let prev_dur = prev_q_end_days - curr_start_days;
        let p_ocf = (curr_ytd_ocf_picked.tag != "absent")
            .then(|| {
                pick_ytd_exact(
                    us_gaap,
                    OCF_TAGS,
                    curr_start_days,
                    prev_q_end_days,
                    prev_dur,
                )
                .map(|v| ytd_ocf - v.val)
            })
            .flatten();
        let p_capex = (curr_ytd_capex_picked.tag != "absent")
            .then(|| {
                pick_ytd_exact(
                    us_gaap,
                    CAPEX_TAGS,
                    curr_start_days,
                    prev_q_end_days,
                    prev_dur,
                )
                .map(|v| ytd_capex - v.val.abs())
            })
            .flatten();
        let p_sbc = (curr_ytd_sbc_picked.tag != "absent")
            .then(|| {
                pick_ytd_exact(
                    us_gaap,
                    SBC_TAGS,
                    curr_start_days,
                    prev_q_end_days,
                    prev_dur,
                )
                .map(|v| ytd_sbc - v.val.abs())
            })
            .flatten();
        let p_buybacks = (curr_ytd_buybacks_picked.tag != "absent")
            .then(|| {
                pick_ytd_exact(
                    us_gaap,
                    BUYBACK_TAGS,
                    curr_start_days,
                    prev_q_end_days,
                    prev_dur,
                )
                .map(|v| ytd_buybacks - v.val.abs())
            })
            .flatten();
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

    let carry_annual_picked = |val: Decimal, annual_field: &str| -> PickedValue {
        let p = annual.provenance.iter().find(|p| p.field == annual_field);
        PickedValue {
            val,
            tag: p
                .map(|item| item.source_tag.clone())
                .unwrap_or_else(|| "annual_fallback".to_owned()),
            filed: p.and_then(|item| item.filed.clone()).unwrap_or_default(),
            period_end: annual.period_end.clone(),
            note: Some(format!(
                "carried_from_latest_annual(end={})",
                annual.period_end
            )),
        }
    };

    let all_q_forms = &["10-Q", "10-Q/A"];
    let annual_cash_prov = annual.provenance.iter().find(|p| p.field == "cash_and_st");
    let annual_cash_absent = annual_cash_prov.is_some_and(|p| p.source_tag == "absent");
    let (cash_picked, ttm_cash_and_st, ttm_cash_prov) =
        match extract_cash_and_st(us_gaap, all_q_forms, curr_end_days, &curr_end) {
            CashExtraction::Present(p) => {
                let v = Some(p.val);
                let p_prov = prov("ttm_cash_and_st", &p);
                (p, v, p_prov)
            }
            CashExtraction::Ambiguous(p) => {
                let p_prov = FieldProvenance {
                    field: "ttm_cash_and_st".to_owned(),
                    source_tag: "absent".to_owned(),
                    filed: (!p.filed.is_empty()).then_some(p.filed.clone()),
                    period_end: curr_end.clone(),
                    note: p.note.clone(),
                };
                (p, None, p_prov)
            }
            CashExtraction::Missing => match annual.cash_and_st.filter(|_| !annual_cash_absent) {
                Some(ann_cash) => {
                    let carried = carry_annual_picked(ann_cash, "cash_and_st");
                    let p_prov = prov("ttm_cash_and_st", &carried);
                    (carried, Some(ann_cash), p_prov)
                }
                None => {
                    let abs = PickedValue {
                        val: Decimal::ZERO,
                        tag: "absent".to_owned(),
                        filed: String::new(),
                        period_end: curr_end.clone(),
                        note: Some("carried_absent_cash_from_annual".to_owned()),
                    };
                    let p_prov = prov("ttm_cash_and_st", &abs);
                    (abs, None, p_prov)
                }
            },
        };
    let _ = cash_picked;

    let debt_picked = extract_total_debt(us_gaap, all_q_forms, curr_end_days, &curr_end);
    // Refuse to carry stale FY annual total_debt into TTM whenever the quarterly filing itself
    // discloses debt schedule/carrying/incomplete components that cannot verify total debt or
    // contradict carrying annual debt forward (e.g. QXO-style large quarterly debt changes).
    let quarterly_carrying = has_positive_instant_any(
        us_gaap,
        DEBT_CARRYING_AMOUNT_TAGS,
        all_q_forms,
        curr_end_days,
    );
    let quarterly_has_contradictory_schedule =
        quarterly_carrying.as_ref().is_some_and(|(_, val)| {
            if debt_picked.tag == "absent" {
                *val > Decimal::ZERO
            } else {
                *val > debt_picked.val * Decimal::new(115, 2)
            }
        });
    let quarterly_incomplete_or_contradicts = quarterly_has_contradictory_schedule
        || (debt_picked.tag == "absent"
            && debt_picked.note.as_deref() != Some("absent_in_xbrl_assumed_zero"))
        || (debt_picked.tag == "absent"
            && (has_positive_instant_any(
                us_gaap,
                DEBT_MATURITY_12M_TAGS,
                all_q_forms,
                curr_end_days,
            )
            .is_some()
                || has_positive_instant_any(
                    us_gaap,
                    DEBT_NONCURRENT_MATURITY_TAGS,
                    all_q_forms,
                    curr_end_days,
                )
                .is_some()
                || has_positive_instant_any(
                    us_gaap,
                    DEBT_NOTES_PAYABLE_CURRENT_TAGS,
                    all_q_forms,
                    curr_end_days,
                )
                .is_some()));
    let total_debt = if quarterly_incomplete_or_contradicts {
        Decimal::ZERO
    } else if debt_picked.tag == "absent" {
        annual.total_debt
    } else {
        debt_picked.val
    };

    let equity_picked =
        pick_instant_for_period(us_gaap, EQUITY_TAGS, "USD", all_q_forms, curr_end_days, 15)
            .unwrap_or_else(|| carry_annual_picked(annual.equity, "equity"));
    let equity = equity_picked.val;

    let ending_picked = pick_instant_for_period(
        us_gaap,
        ENDING_SHARES_GAAP_TAGS,
        "shares",
        all_q_forms,
        curr_end_days,
        20,
    )
    .or_else(|| {
        pick_instant_for_period(
            dei,
            ENDING_SHARES_DEI_TAGS,
            "shares",
            all_q_forms,
            curr_end_days,
            45,
        )
    })
    .unwrap_or_else(|| carry_annual_picked(annual.ending_shares, "ending_shares"));
    let ending_shares = ending_picked.val;

    let diluted_picked = pick_flow_for_period(
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
    .or_else(|| {
        pick_ytd_exact(
            us_gaap,
            DILUTED_SHARES_TAGS,
            curr_start_days,
            curr_end_days,
            curr_dur,
        )
    })
    .unwrap_or_else(|| carry_annual_picked(annual.diluted_shares, "diluted_shares"));
    let diluted_shares = diluted_picked.val;

    let fp_label = curr_q.fp.clone().unwrap_or_else(|| "YTD".to_owned());
    let year_prefix = curr_end.get(..4).unwrap_or(curr_end.as_str());
    let latest_ytd = YtdPeriodFinancials {
        period_label: format!("{year_prefix} {fp_label} YTD"),
        start: curr_start,
        end: curr_end.clone(),
        filed: (!curr_q.filed.is_empty()).then(|| curr_q.filed.clone()),
        revenue: (curr_ytd_rev_picked.tag != "absent").then_some(ytd_rev),
        ocf: (curr_ytd_ocf_picked.tag != "absent").then_some(ytd_ocf),
        capex: (curr_ytd_capex_picked.tag != "absent").then_some(ytd_capex),
        sbc: (curr_ytd_sbc_picked.tag != "absent").then_some(ytd_sbc),
        buybacks: (curr_ytd_buybacks_picked.tag != "absent").then_some(ytd_buybacks),
        latest_quarter_label: lq_label,
        latest_quarter_ocf: lq_ocf,
        latest_quarter_capex: lq_capex,
        latest_quarter_sbc: lq_sbc,
        latest_quarter_buybacks: lq_buybacks,
    };

    let annual_tax_absent = annual
        .provenance
        .iter()
        .any(|p| p.field == "tax_expense" && p.source_tag == "absent");
    let annual_sbc_absent = annual
        .provenance
        .iter()
        .any(|p| p.field == "sbc" && p.source_tag == "absent");
    let annual_buybacks_absent = annual
        .provenance
        .iter()
        .any(|p| p.field == "buybacks" && p.source_tag == "absent");
    let annual_debt_prov = annual.provenance.iter().find(|p| p.field == "total_debt");

    let ttm_tax_prov = if tax_delta.tag == "absent" || annual_tax_absent {
        FieldProvenance {
            field: "ttm_tax_expense".to_owned(),
            source_tag: "absent".to_owned(),
            filed: None,
            period_end: curr_end.clone(),
            note: Some(format!(
                "absent_in_xbrl(annual_absent={annual_tax_absent};delta={})",
                tax_delta.note.as_deref().unwrap_or("")
            )),
        }
    } else {
        prov("ttm_tax_expense", &tax_delta)
    };
    let ttm_sbc_prov = if sbc_delta.tag == "absent" || annual_sbc_absent {
        FieldProvenance {
            field: "ttm_sbc".to_owned(),
            source_tag: "absent".to_owned(),
            filed: None,
            period_end: curr_end.clone(),
            note: Some(format!(
                "absent_in_xbrl(annual_absent={annual_sbc_absent};delta={})",
                sbc_delta.note.as_deref().unwrap_or("")
            )),
        }
    } else {
        prov("ttm_sbc", &sbc_delta)
    };
    let ttm_buybacks_prov = if buybacks_delta.tag == "absent" || annual_buybacks_absent {
        FieldProvenance {
            field: "ttm_buybacks".to_owned(),
            source_tag: "absent".to_owned(),
            filed: None,
            period_end: curr_end.clone(),
            note: Some(format!(
                "absent_in_xbrl(annual_absent={annual_buybacks_absent};delta={})",
                buybacks_delta.note.as_deref().unwrap_or("")
            )),
        }
    } else {
        prov("ttm_buybacks", &buybacks_delta)
    };
    let ttm_debt_prov = if quarterly_incomplete_or_contradicts {
        FieldProvenance {
            field: "ttm_total_debt".to_owned(),
            source_tag: "absent".to_owned(),
            filed: None,
            period_end: curr_end.clone(),
            note: Some(format!(
                "quarterly_debt_incomplete_or_contradicts_annual({})",
                debt_picked.note.as_deref().unwrap_or("carrying_schedule")
            )),
        }
    } else if debt_picked.tag != "absent" {
        prov("ttm_total_debt", &debt_picked)
    } else if annual_debt_prov.is_some_and(|p| p.source_tag == "absent") {
        FieldProvenance {
            field: "ttm_total_debt".to_owned(),
            source_tag: "absent".to_owned(),
            filed: None,
            period_end: curr_end.clone(),
            note: Some("absent_in_xbrl".to_owned()),
        }
    } else {
        FieldProvenance {
            field: "ttm_total_debt".to_owned(),
            source_tag: annual_debt_prov
                .map(|p| p.source_tag.clone())
                .unwrap_or_else(|| "absent".to_owned()),
            filed: annual_debt_prov.and_then(|p| p.filed.clone()),
            period_end: annual_debt_prov
                .map(|p| p.period_end.clone())
                .unwrap_or_else(|| annual.period_end.clone()),
            note: Some("carried_from_latest_annual".to_owned()),
        }
    };

    let mut ttm_diluted_prov = prov("ttm_diluted_shares", &diluted_picked);
    if has_share_magnitude_mismatch(diluted_shares, ending_shares)
        || has_share_magnitude_mismatch(diluted_shares, annual.diluted_shares)
    {
        let existing_note = ttm_diluted_prov.note.take().unwrap_or_default();
        ttm_diluted_prov.note = Some(format!(
            "share_basis_magnitude_mismatch:diluted={diluted_shares},ending={ending_shares},annual_diluted={};{existing_note}",
            annual.diluted_shares
        ));
    }

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
        cash_and_st: ttm_cash_and_st,
        total_debt,
        equity,
        diluted_shares,
        ending_shares,
        latest_ytd,
        provenance: vec![
            prov("ttm_revenue", &rev_delta),
            prov("ttm_gross_profit", &gp_delta),
            prov("ttm_ebit", &ebit_delta),
            ttm_tax_prov,
            prov("ttm_pretax_income", &pretax_delta),
            prov("ttm_ocf", &ocf_delta),
            prov("ttm_capex", &capex_delta),
            ttm_sbc_prov,
            ttm_buybacks_prov,
            prov("ytd_revenue", &curr_ytd_rev_picked),
            prov("ytd_ocf", &curr_ytd_ocf_picked),
            prov("ytd_capex", &curr_ytd_capex_picked),
            prov("ytd_sbc", &curr_ytd_sbc_picked),
            prov("ytd_buybacks", &curr_ytd_buybacks_picked),
            ttm_cash_prov,
            ttm_debt_prov,
            prov("ttm_equity", &equity_picked),
            ttm_diluted_prov,
            prov("ttm_ending_shares", &ending_picked),
        ],
    }))
}

fn pick_ytd_exact(
    us_gaap: Option<&FactNamespace<'_>>,
    tags: &[&str],
    start_days: i64,
    end_days: i64,
    dur_days: i64,
) -> Option<PickedValue> {
    for tag in tags {
        let items = collect_items(us_gaap, &[tag], "USD", &["10-Q", "10-Q/A"])
            .into_iter()
            .chain(collect_items(
                us_gaap,
                &[tag],
                "shares",
                &["10-Q", "10-Q/A"],
            ));
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
    us_gaap: Option<&FactNamespace<'_>>,
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
        let parsed =
            parse_companyfacts_bytes(&bytes, "GOOGL", "1652044", "https://sec.test", 123).unwrap();
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
        assert_eq!(
            ttm.latest_ytd.latest_quarter_capex,
            Some(dec(131953 - 80598))
        );
    }

    #[test]
    fn parses_xbrl_decimals_without_f64_loss_and_marks_absent_provenance() {
        let raw = br#"{
            "entityName": "Precision Test Corp.",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2024-01-01","end":"2024-12-31","val":12345678901234567.89,"form":"10-K","filed":"2025-02-04"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2024-01-01","end":"2024-12-31","val":700000,"form":"10-K","filed":"2025-02-04"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2024-01-01","end":"2024-12-31","val":300000,"form":"10-K","filed":"2025-02-04"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2024-01-01","end":"2024-12-31","val":350000,"form":"10-K","filed":"2025-02-04"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2024-01-01","end":"2024-12-31","val":50000,"form":"10-K","filed":"2025-02-04"}
                    ]}},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [
                        {"end":"2024-12-31","val":100000,"form":"10-K","filed":"2025-02-04"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2024-12-31","val":800000,"form":"10-K","filed":"2025-02-04"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2024-01-01","end":"2024-12-31","val":1000.123456789012345,"form":"10-K","filed":"2025-02-04"}
                    ]}}
                }
            }
        }"#;
        let parsed =
            parse_companyfacts_bytes(raw, "PREC", "123456", "https://sec.test", 999).unwrap();
        assert_eq!(
            parsed.annuals[0].revenue.to_string(),
            "12345678901234567.89"
        );
        assert_eq!(
            parsed.annuals[0].diluted_shares.to_string(),
            "1000.123456789012345"
        );
        let absent_fields: Vec<&str> = parsed.annuals[0]
            .provenance
            .iter()
            .filter(|p| p.source_tag == "absent")
            .map(|p| p.field.as_str())
            .collect();
        assert!(absent_fields.contains(&"tax_expense"));
        assert!(absent_fields.contains(&"sbc"));
        assert!(absent_fields.contains(&"total_debt"));
    }

    #[test]
    fn extract_total_debt_handles_teva_noncurrent_notes_and_avoids_double_counting_current_debt() {
        let teva_like = br#"{
            "entityName": "TEVA-Like Corp.",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":16000000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":8000000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":3000000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":2000000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [
                        {"end":"2025-12-31","val":2000000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":7000000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "LongTermNotesAndLoans": { "units": { "USD": [
                        {"end":"2025-12-31","val":14986000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "LongTermDebtCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":1798000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "DebtCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":1820000000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":1100000000,"form":"10-K","filed":"2026-02-03"}
                    ]}}
                }
            }
        }"#;
        let parsed =
            parse_companyfacts_bytes(teva_like, "TEVA", "0000818686", "https://sec.test", 1)
                .unwrap();
        assert_eq!(
            parsed.annuals[0].total_debt,
            dec(14_986_000_000 + 1_820_000_000)
        );
        let debt_prov = parsed.annuals[0]
            .provenance
            .iter()
            .find(|p| p.field == "total_debt")
            .unwrap();
        assert_eq!(debt_prov.source_tag, "LongTermNotesAndLoans + DebtCurrent");

        // Fallback when only current debt tags exist: DebtCurrent alone, never DebtCurrent + LongTermDebtCurrent.
        let current_only = br#"{
            "entityName": "Current Debt Only Corp.",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":1000,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":500,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":200,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":250,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":50,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [
                        {"end":"2025-12-31","val":100,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":400,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "LongTermDebtCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":1798,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "DebtCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":1820,"form":"10-K","filed":"2026-02-03"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":100,"form":"10-K","filed":"2026-02-03"}
                    ]}}
                }
            }
        }"#;
        let parsed_curr =
            parse_companyfacts_bytes(current_only, "CURR", "0000000002", "https://sec.test", 1)
                .unwrap();
        assert_eq!(parsed_curr.annuals[0].total_debt, dec(1820));
    }

    #[test]
    fn ttm_records_full_operand_and_prior_ytd_provenance_and_share_magnitude_mismatch() {
        let raw = br#"{
            "entityName": "CIX-Like Corp.",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":1000000,"form":"10-K","filed":"2026-03-01"},
                        {"start":"2025-01-01","end":"2025-09-30","val":700000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":800000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":400000,"form":"10-K","filed":"2026-03-01"},
                        {"start":"2025-01-01","end":"2025-09-30","val":280000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":320000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":150000,"form":"10-K","filed":"2026-03-01"},
                        {"start":"2025-01-01","end":"2025-09-30","val":100000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":120000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":155000,"form":"10-K","filed":"2026-03-01"},
                        {"start":"2025-01-01","end":"2025-09-30","val":102000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":125000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":200000,"form":"10-K","filed":"2026-03-01"},
                        {"start":"2025-01-01","end":"2025-09-30","val":140000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":160000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":30000,"form":"10-K","filed":"2026-03-01"},
                        {"start":"2025-01-01","end":"2025-09-30","val":20000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":25000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [
                        {"end":"2025-12-31","val":50000,"form":"10-K","filed":"2026-03-01"},
                        {"end":"2026-09-30","val":60000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":500000,"form":"10-K","filed":"2026-03-01"},
                        {"end":"2026-09-30","val":550000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":12321000,"form":"10-K","filed":"2026-03-01"},
                        {"start":"2026-07-01","end":"2026-09-30","val":12337,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}}
                },
                "dei": {
                    "EntityCommonStockSharesOutstanding": { "units": { "shares": [
                        {"end":"2026-02-28","val":12323057,"form":"10-K","filed":"2026-03-01"},
                        {"end":"2026-10-30","val":12336657,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                    ]}}
                }
            }
        }"#;
        let parsed =
            parse_companyfacts_bytes(raw, "CIX", "0001049606", "https://sec.test", 1).unwrap();
        let ttm = parsed.ttm.unwrap();
        let fields: HashMap<&str, &FieldProvenance> = ttm
            .provenance
            .iter()
            .map(|p| (p.field.as_str(), p))
            .collect();
        for req in [
            "ttm_revenue",
            "ttm_gross_profit",
            "ttm_ebit",
            "ttm_tax_expense",
            "ttm_pretax_income",
            "ttm_ocf",
            "ttm_capex",
            "ttm_sbc",
            "ttm_buybacks",
            "ttm_cash_and_st",
            "ttm_total_debt",
            "ttm_equity",
            "ttm_diluted_shares",
            "ttm_ending_shares",
        ] {
            assert!(fields.contains_key(req), "missing {req} in TTM provenance");
        }
        assert!(
            fields["ttm_ebit"].note.as_deref().unwrap().contains(
                "ytd_prev=100000(end=2025-09-30,filed=2025-11-01,tag=OperatingIncomeLoss)"
            )
        );
        assert!(
            fields["ttm_diluted_shares"]
                .note
                .as_deref()
                .unwrap()
                .contains("share_basis_magnitude_mismatch")
        );
    }

    #[test]
    fn hei_debt_prefers_aggregate_over_partial_senior_long_term_notes() {
        let raw = br#"{
            "entityName": "HEICO CORP",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2024-11-01","end":"2025-10-31","val":4200000000,"form":"10-K","filed":"2025-12-18"},
                        {"start":"2024-11-01","end":"2025-07-31","val":3100000000,"fp":"Q3","form":"10-Q","filed":"2025-09-02"},
                        {"start":"2025-11-01","end":"2026-07-31","val":3400000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2024-11-01","end":"2025-10-31","val":1650000000,"form":"10-K","filed":"2025-12-18"},
                        {"start":"2024-11-01","end":"2025-07-31","val":1200000000,"fp":"Q3","form":"10-Q","filed":"2025-09-02"},
                        {"start":"2025-11-01","end":"2026-07-31","val":1350000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2024-11-01","end":"2025-10-31","val":900000000,"form":"10-K","filed":"2025-12-18"},
                        {"start":"2024-11-01","end":"2025-07-31","val":660000000,"fp":"Q3","form":"10-Q","filed":"2025-09-02"},
                        {"start":"2025-11-01","end":"2026-07-31","val":750000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [
                        {"start":"2024-11-01","end":"2025-10-31","val":850000000,"form":"10-K","filed":"2025-12-18"},
                        {"start":"2024-11-01","end":"2025-07-31","val":620000000,"fp":"Q3","form":"10-Q","filed":"2025-09-02"},
                        {"start":"2025-11-01","end":"2026-07-31","val":710000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2024-11-01","end":"2025-10-31","val":820000000,"form":"10-K","filed":"2025-12-18"},
                        {"start":"2024-11-01","end":"2025-07-31","val":600000000,"fp":"Q3","form":"10-Q","filed":"2025-09-02"},
                        {"start":"2025-11-01","end":"2026-07-31","val":690000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2024-11-01","end":"2025-10-31","val":60000000,"form":"10-K","filed":"2025-12-18"},
                        {"start":"2024-11-01","end":"2025-07-31","val":45000000,"fp":"Q3","form":"10-Q","filed":"2025-09-02"},
                        {"start":"2025-11-01","end":"2026-07-31","val":50000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [
                        {"end":"2025-10-31","val":200000000,"form":"10-K","filed":"2025-12-18"},
                        {"end":"2026-07-31","val":250000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-10-31","val":3800000000,"form":"10-K","filed":"2025-12-18"},
                        {"end":"2026-07-31","val":4200000000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "DebtAndCapitalLeaseObligations": { "units": { "USD": [
                        {"end":"2025-10-31","val":2167945000,"form":"10-K","filed":"2025-12-18"},
                        {"end":"2026-07-31","val":2541173000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "LongTermDebtAndCapitalLeaseObligations": { "units": { "USD": [
                        {"end":"2025-10-31","val":2164587000,"form":"10-K","filed":"2025-12-18"},
                        {"end":"2026-07-31","val":2537660000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "SeniorLongTermNotes": { "units": { "USD": [
                        {"end":"2025-10-31","val":1190055000,"form":"10-K","filed":"2025-12-18"},
                        {"end":"2026-07-31","val":2380335000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "LongTermDebtCurrent": { "units": { "USD": [
                        {"end":"2025-10-31","val":3358000,"form":"10-K","filed":"2025-12-18"},
                        {"end":"2026-07-31","val":3513000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2024-11-01","end":"2025-10-31","val":140000000,"form":"10-K","filed":"2025-12-18"},
                        {"start":"2025-11-01","end":"2026-07-31","val":140500000,"fp":"Q3","form":"10-Q","filed":"2026-09-01"}
                    ]}}
                }
            }
        }"#;
        let parsed =
            parse_companyfacts_bytes(raw, "HEI", "0000046619", "https://sec.test", 1).unwrap();
        assert_eq!(parsed.annuals.len(), 1);
        assert_eq!(
            parsed.annuals[0].total_debt,
            Decimal::from(2_167_945_000i64)
        );
        let ttm = parsed.ttm.unwrap();
        assert_eq!(ttm.total_debt, Decimal::from(2_541_173_000i64));
        let ttm_debt_prov = ttm
            .provenance
            .iter()
            .find(|p| p.field == "ttm_total_debt")
            .unwrap();
        assert_eq!(ttm_debt_prov.source_tag, "DebtAndCapitalLeaseObligations");
    }

    #[test]
    fn ttm_carried_forward_debt_preserves_annual_period_end() {
        let raw = br#"{
            "entityName": "CAT TEST",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":60000000000,"form":"10-K","filed":"2026-02-15"},
                        {"start":"2025-01-01","end":"2025-09-30","val":44000000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":46000000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":20000000000,"form":"10-K","filed":"2026-02-15"},
                        {"start":"2025-01-01","end":"2025-09-30","val":14800000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":15600000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":12000000000,"form":"10-K","filed":"2026-02-15"},
                        {"start":"2025-01-01","end":"2025-09-30","val":9000000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":9500000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":11800000000,"form":"10-K","filed":"2026-02-15"},
                        {"start":"2025-01-01","end":"2025-09-30","val":8800000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":9300000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":10000000000,"form":"10-K","filed":"2026-02-15"},
                        {"start":"2025-01-01","end":"2025-09-30","val":7000000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":7500000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":2000000000,"form":"10-K","filed":"2026-02-15"},
                        {"start":"2025-01-01","end":"2025-09-30","val":1400000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                        {"start":"2026-01-01","end":"2026-09-30","val":1500000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [
                        {"end":"2025-12-31","val":6000000000,"form":"10-K","filed":"2026-02-15"},
                        {"end":"2026-09-30","val":6500000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "LongTermDebtNoncurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":30696000000,"form":"10-K","filed":"2026-02-15"}
                    ]}},
                    "ShortTermBorrowings": { "units": { "USD": [
                        {"end":"2025-12-31","val":5514000000,"form":"10-K","filed":"2026-02-15"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":20000000000,"form":"10-K","filed":"2026-02-15"},
                        {"end":"2026-09-30","val":21000000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-15"},
                        {"start":"2026-01-01","end":"2026-09-30","val":495000000,"fp":"Q3","form":"10-Q","filed":"2026-11-01"}
                    ]}}
                }
            }
        }"#;
        let parsed =
            parse_companyfacts_bytes(raw, "CAT", "0000018230", "https://sec.test", 1).unwrap();
        assert_eq!(
            parsed.annuals[0].total_debt,
            Decimal::from(36_210_000_000i64)
        );
        let ttm = parsed.ttm.unwrap();
        assert_eq!(ttm.as_of_end, "2026-09-30");
        assert_eq!(ttm.total_debt, Decimal::from(36_210_000_000i64));
        let prov = ttm
            .provenance
            .iter()
            .find(|p| p.field == "ttm_total_debt")
            .unwrap();
        assert_eq!(prov.note.as_deref(), Some("carried_from_latest_annual"));
        assert_eq!(prov.period_end, "2025-12-31");
        assert_eq!(prov.filed.as_deref(), Some("2026-02-15"));
    }

    #[test]
    fn cat_and_aent_incomplete_debt_mark_total_debt_absent() {
        // CAT FY25 has LongTermDebtNoncurrent + ShortTermBorrowings + $7.12B next-12M maturities
        // with no LongTermDebtCurrent or DebtCurrent in us-gaap -> must not undercount at $36.21B.
        let cat_raw = br#"{
            "entityName": "CAT INC",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":64800000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "GrossProfit": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":20000000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "OperatingIncomeLoss": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":12000000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":11800000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":10000000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":2000000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [{"end":"2025-12-31","val":6000000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "LongTermDebtNoncurrent": { "units": { "USD": [{"end":"2025-12-31","val":30696000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "ShortTermBorrowings": { "units": { "USD": [{"end":"2025-12-31","val":5514000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "LongTermDebtMaturitiesRepaymentsOfPrincipalInNextTwelveMonths": { "units": { "USD": [{"end":"2025-12-31","val":7120000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "StockholdersEquity": { "units": { "USD": [{"end":"2025-12-31","val":20000000000,"form":"10-K","filed":"2026-02-13"}] }},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [{"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-13"}] }}
                }
            }
        }"#;
        let cat =
            parse_companyfacts_bytes(cat_raw, "CAT", "0000018230", "https://sec.test", 1).unwrap();
        let cat_prov = cat.annuals[0]
            .provenance
            .iter()
            .find(|p| p.field == "total_debt")
            .unwrap();
        assert_eq!(cat_prov.source_tag, "absent");

        // AENT FY25 has partial SubordinatedLongTermDebt ($10M) + LongTermLineOfCredit ($55.27M)
        // with no aggregate LT debt tag -> must mark debt absent rather than emit $10M.
        let aent_raw = br#"{
            "entityName": "AENT",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [{"start":"2024-07-01","end":"2025-06-30","val":1000000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "GrossProfit": { "units": { "USD": [{"start":"2024-07-01","end":"2025-06-30","val":120000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "OperatingIncomeLoss": { "units": { "USD": [{"start":"2024-07-01","end":"2025-06-30","val":20000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [{"start":"2024-07-01","end":"2025-06-30","val":15000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [{"start":"2024-07-01","end":"2025-06-30","val":25000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [{"start":"2024-07-01","end":"2025-06-30","val":2000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [{"end":"2025-06-30","val":1236000,"form":"10-K","filed":"2025-09-10"}] }},
                    "SubordinatedLongTermDebt": { "units": { "USD": [{"end":"2025-06-30","val":10000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "LongTermLineOfCredit": { "units": { "USD": [{"end":"2025-06-30","val":55268000,"form":"10-K","filed":"2025-09-10"}] }},
                    "StockholdersEquity": { "units": { "USD": [{"end":"2025-06-30","val":100000000,"form":"10-K","filed":"2025-09-10"}] }},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [{"start":"2024-07-01","end":"2025-06-30","val":50000000,"form":"10-K","filed":"2025-09-10"}] }}
                }
            }
        }"#;
        let aent = parse_companyfacts_bytes(aent_raw, "AENT", "0001823584", "https://sec.test", 1)
            .unwrap();
        let aent_prov = aent.annuals[0]
            .provenance
            .iter()
            .find(|p| p.field == "total_debt")
            .unwrap();
        assert_eq!(aent_prov.source_tag, "absent");
    }

    #[test]
    fn ttm_tracks_period_specific_ytd_sbc_and_absent_ytd_buybacks_separately() {
        // SSP-like: current Q3 YTD reports SBC $15.7M, prior Q3 YTD omits SBC, buybacks absent in 10-Q.
        let raw = br#"{
            "entityName": "SSP TEST",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":2400000000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2025-01-01","end":"2025-09-30","val":1700000000,"fp":"Q3","form":"10-Q","filed":"2025-11-05"},
                        {"start":"2026-01-01","end":"2026-09-30","val":1552130000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":900000000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2025-01-01","end":"2025-09-30","val":650000000,"fp":"Q3","form":"10-Q","filed":"2025-11-05"},
                        {"start":"2026-01-01","end":"2026-09-30","val":600000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":200000000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2025-01-01","end":"2025-09-30","val":140000000,"fp":"Q3","form":"10-Q","filed":"2025-11-05"},
                        {"start":"2026-01-01","end":"2026-09-30","val":130000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":150000000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2025-01-01","end":"2025-09-30","val":100000000,"fp":"Q3","form":"10-Q","filed":"2025-11-05"},
                        {"start":"2026-01-01","end":"2026-09-30","val":95000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":180000000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2025-01-01","end":"2025-09-30","val":120000000,"fp":"Q3","form":"10-Q","filed":"2025-11-05"},
                        {"start":"2026-01-01","end":"2026-09-30","val":148868000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":40000000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2025-01-01","end":"2025-09-30","val":30000000,"fp":"Q3","form":"10-Q","filed":"2025-11-05"},
                        {"start":"2026-01-01","end":"2026-09-30","val":28000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "ShareBasedCompensation": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":13797000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2026-01-01","end":"2026-09-30","val":15700000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "PaymentsForRepurchaseOfCommonStock": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":25000000,"form":"10-K","filed":"2026-02-20"}
                    ]}},
                    "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [
                        {"end":"2025-12-31","val":50000000,"form":"10-K","filed":"2026-02-20"},
                        {"end":"2026-09-30","val":60000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "LongTermDebt": { "units": { "USD": [
                        {"end":"2025-12-31","val":2500000000,"form":"10-K","filed":"2026-02-20"},
                        {"end":"2026-09-30","val":2400000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":1000000000,"form":"10-K","filed":"2026-02-20"},
                        {"end":"2026-09-30","val":1050000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":85000000,"form":"10-K","filed":"2026-02-20"},
                        {"start":"2026-01-01","end":"2026-09-30","val":86000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                    ]}}
                }
            }
        }"#;
        let parsed =
            parse_companyfacts_bytes(raw, "SSP", "0000832428", "https://sec.test", 1).unwrap();
        let ttm = parsed.ttm.unwrap();
        assert_eq!(ttm.latest_ytd.sbc, Some(Decimal::from(15_700_000i64)));
        assert!(ttm.latest_ytd.buybacks.is_none());
        let fields: HashMap<&str, &FieldProvenance> = ttm
            .provenance
            .iter()
            .map(|p| (p.field.as_str(), p))
            .collect();
        assert_eq!(fields["ttm_sbc"].source_tag, "absent");
        assert_eq!(fields["ytd_sbc"].source_tag, "ShareBasedCompensation");
        assert_eq!(fields["ttm_buybacks"].source_tag, "absent");
        assert_eq!(fields["ytd_buybacks"].source_tag, "absent");
    }

    #[test]
    fn amd_axti_bio_and_qxo_debt_coverage_and_ttm_contradiction_work() {
        // 1. AMD: DebtLongtermAndShorttermCombinedAmount ($3.222B) wins before duplicate $874M current tags ($4.096B).
        let amd_raw = br#"{
            "entityName": "AMD TEST",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-27","val":25000000000,"form":"10-K","filed":"2026-02-04"}] }},
                "GrossProfit": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-27","val":13000000000,"form":"10-K","filed":"2026-02-04"}] }},
                "OperatingIncomeLoss": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-27","val":4000000000,"form":"10-K","filed":"2026-02-04"}] }},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-27","val":5000000000,"form":"10-K","filed":"2026-02-04"}] }},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-27","val":700000000,"form":"10-K","filed":"2026-02-04"}] }},
                "CashCashEquivalentsAndShortTermInvestments": { "units": { "USD": [{"end":"2025-12-27","val":5000000000,"form":"10-K","filed":"2026-02-04"}] }},
                "DebtLongtermAndShorttermCombinedAmount": { "units": { "USD": [{"end":"2025-12-27","val":3222000000,"form":"10-K","filed":"2026-02-04"}] }},
                "LongTermDebtNoncurrent": { "units": { "USD": [{"end":"2025-12-27","val":2348000000,"form":"10-K","filed":"2026-02-04"}] }},
                "LongTermDebtCurrent": { "units": { "USD": [{"end":"2025-12-27","val":874000000,"form":"10-K","filed":"2026-02-04"}] }},
                "ShortTermBorrowings": { "units": { "USD": [{"end":"2025-12-27","val":874000000,"form":"10-K","filed":"2026-02-04"}] }},
                "StockholdersEquity": { "units": { "USD": [{"end":"2025-12-27","val":55000000000,"form":"10-K","filed":"2026-02-04"}] }},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [{"start":"2025-01-01","end":"2025-12-27","val":1600000000,"form":"10-K","filed":"2026-02-04"}] }}
            }}
        }"#;
        let amd =
            parse_companyfacts_bytes(amd_raw, "AMD", "0000002488", "https://sec.test", 1).unwrap();
        assert_eq!(amd.annuals[0].total_debt, Decimal::from(3_222_000_000i64));

        // 2. AXTI FY25: DebtCurrent ($62.796M) with noncurrent LongTermLineOfCredit ($5.2M) + YearTwo ($1.179M)
        // and no LongTermDebtNoncurrent / combined tag marks total_debt absent rather than exporting $62.8M.
        let axti_raw = br#"{
            "entityName": "AXTI TEST",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-03-17"}] }},
                "GrossProfit": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":25000000,"form":"10-K","filed":"2026-03-17"}] }},
                "OperatingIncomeLoss": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":5000000,"form":"10-K","filed":"2026-03-17"}] }},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-03-17"}] }},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [{"start":"2025-01-01","end":"2025-12-31","val":4000000,"form":"10-K","filed":"2026-03-17"}] }},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [{"end":"2025-12-31","val":30000000,"form":"10-K","filed":"2026-03-17"}] }},
                "DebtCurrent": { "units": { "USD": [{"end":"2025-12-31","val":62796000,"form":"10-K","filed":"2026-03-17"}] }},
                "LongTermLineOfCredit": { "units": { "USD": [{"end":"2025-12-31","val":5200000,"form":"10-K","filed":"2026-03-17"}] }},
                "LongTermDebtMaturitiesRepaymentsOfPrincipalInYearTwo": { "units": { "USD": [{"end":"2025-12-31","val":1179000,"form":"10-K","filed":"2026-03-17"}] }},
                "StockholdersEquity": { "units": { "USD": [{"end":"2025-12-31","val":150000000,"form":"10-K","filed":"2026-03-17"}] }},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [{"start":"2025-01-01","end":"2025-12-31","val":45000000,"form":"10-K","filed":"2026-03-17"}] }}
            }}
        }"#;
        let axti = parse_companyfacts_bytes(axti_raw, "AXTI", "0001051627", "https://sec.test", 1)
            .unwrap();
        let axti_debt_prov = axti.annuals[0]
            .provenance
            .iter()
            .find(|p| p.field == "total_debt")
            .unwrap();
        assert_eq!(axti_debt_prov.source_tag, "absent");

        // 3. BIO FY25 & Q3'26: extracts $1.203B FY25 and $1.204B Q3'26 via combined LongTermDebt / OtherLongTermDebtCurrent.
        let bio_raw = br#"{
            "entityName": "BIO TEST",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":2600000000,"form":"10-K","filed":"2026-02-13"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1900000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-01-01","end":"2026-09-30","val":1950000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":1400000000,"form":"10-K","filed":"2026-02-13"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1020000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-01-01","end":"2026-09-30","val":1050000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":350000000,"form":"10-K","filed":"2026-02-13"},
                    {"start":"2025-01-01","end":"2025-09-30","val":250000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-01-01","end":"2026-09-30","val":260000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":420000000,"form":"10-K","filed":"2026-02-13"},
                    {"start":"2025-01-01","end":"2025-09-30","val":300000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-01-01","end":"2026-09-30","val":320000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":120000000,"form":"10-K","filed":"2026-02-13"},
                    {"start":"2025-01-01","end":"2025-09-30","val":85000000,"fp":"Q3","form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-01-01","end":"2026-09-30","val":90000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-12-31","val":1500000000,"form":"10-K","filed":"2026-02-13"},
                    {"end":"2026-09-30","val":1550000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":1201700000,"form":"10-K","filed":"2026-02-13"},
                    {"end":"2026-09-30","val":803100000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "OtherLongTermDebtCurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":1300000,"form":"10-K","filed":"2026-02-13"},
                    {"end":"2026-09-30","val":400900000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "LongTermDebt": { "units": { "USD": [
                    {"end":"2025-12-31","val":1203000000,"form":"10-K","filed":"2026-02-13"},
                    {"end":"2026-09-30","val":1204000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "LongTermDebtMaturitiesRepaymentsOfPrincipalInNextTwelveMonths": { "units": { "USD": [
                    {"end":"2025-12-31","val":1300000,"form":"10-K","filed":"2026-02-13"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-12-31","val":6000000000,"form":"10-K","filed":"2026-02-13"},
                    {"end":"2026-09-30","val":6100000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-01-01","end":"2025-12-31","val":28000000,"form":"10-K","filed":"2026-02-13"},
                    {"start":"2026-01-01","end":"2026-09-30","val":28000000,"fp":"Q3","form":"10-Q","filed":"2026-11-04"}
                ]}}
            }}
        }"#;
        let bio =
            parse_companyfacts_bytes(bio_raw, "BIO", "0000012208", "https://sec.test", 1).unwrap();
        assert_eq!(bio.annuals[0].total_debt, Decimal::from(1_203_000_000i64));
        assert_eq!(bio.ttm.unwrap().total_debt, Decimal::from(1_204_000_000i64));

        // 4. QXO-style TTM contradiction: when 10-Q omits combined LT balance and carries
        // incomplete noncurrent + maturities (without current portion), TTM marks ttm_total_debt
        // absent rather than stale FY25 $3.057B. When LongTermDebt ($8.978B) IS present, it uses $8.978B.
        let qxo_incomplete_raw = br#"{
            "entityName": "QXO TEST",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":6000000000,"form":"10-K","filed":"2026-02-27"},
                    {"start":"2025-01-01","end":"2025-09-30","val":4200000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":7800000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":1800000000,"form":"10-K","filed":"2026-02-27"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1200000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":2400000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-27"},
                    {"start":"2025-01-01","end":"2025-09-30","val":300000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":700000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":600000000,"form":"10-K","filed":"2026-02-27"},
                    {"start":"2025-01-01","end":"2025-09-30","val":400000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":800000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-27"},
                    {"start":"2025-01-01","end":"2025-09-30","val":70000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":150000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-27"},
                    {"end":"2026-09-30","val":600000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":3057300000,"form":"10-K","filed":"2026-02-27"},
                    {"end":"2026-09-30","val":8948000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "LongTermDebtMaturitiesRepaymentsOfPrincipalInNextTwelveMonths": { "units": { "USD": [
                    {"end":"2026-09-30","val":30000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-12-31","val":5000000000,"form":"10-K","filed":"2026-02-27"},
                    {"end":"2026-09-30","val":7000000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-27"},
                    {"start":"2026-01-01","end":"2026-09-30","val":600000000,"fp":"Q3","form":"10-Q","filed":"2026-11-13"}
                ]}}
            }}
        }"#;
        let qxo_inc = parse_companyfacts_bytes(
            qxo_incomplete_raw,
            "QXO",
            "0001236275",
            "https://sec.test",
            1,
        )
        .unwrap();
        let qxo_ttm = qxo_inc.ttm.unwrap();
        let qxo_debt_prov = qxo_ttm
            .provenance
            .iter()
            .find(|p| p.field == "ttm_total_debt")
            .unwrap();
        assert_eq!(qxo_debt_prov.source_tag, "absent");
        assert_ne!(qxo_ttm.total_debt, Decimal::from(3_057_300_000i64));
    }

    #[test]
    fn adi_lake_cmi_eqix_debt_and_v1_ticker_validation_work() {
        // 1. ADI FY25: CommercialPaper ($446.639M) duplicates ShortTermBorrowings ($446.639M);
        // must NOT double-count ($8.145066B + $0.446639B = $8.591705B, never $9.038344B).
        let adi_raw = br#"{
            "entityName": "ANALOG DEVICES INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":11000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":6800000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":3000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":4200000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2024-11-03","end":"2025-11-01","val":600000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-11-01","val":2000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2025-11-01","val":8145066000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "LongTermDebtCurrent": { "units": { "USD": [
                    {"end":"2025-11-01","val":0,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "CommercialPaper": { "units": { "USD": [
                    {"end":"2025-11-01","val":446639000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "ShortTermBorrowings": { "units": { "USD": [
                    {"end":"2025-11-01","val":446639000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-11-01","val":35000000000,"form":"10-K","filed":"2025-11-25"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2024-11-03","end":"2025-11-01","val":496000000,"form":"10-K","filed":"2025-11-25"}
                ]}}
            }}
        }"#;
        let adi =
            parse_companyfacts_bytes(adi_raw, "ADI", "0000006281", "https://sec.test", 1).unwrap();
        assert_eq!(adi.schema_version, SEC_FACTS_SCHEMA_VERSION);
        assert_eq!(adi.extractor_version, SEC_EXTRACTOR_VERSION);
        assert_eq!(adi.annuals[0].total_debt, Decimal::from(8_591_705_000i64));

        // 2. LAKE FY24: LongTermLoansPayable ($731k) + ShortTermBorrowings ($298k) = $1,029k.
        let lake_raw = br#"{
            "entityName": "LAKELAND INDUSTRIES INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2023-02-01","end":"2024-01-31","val":124000000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2023-02-01","end":"2024-01-31","val":50000000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2023-02-01","end":"2024-01-31","val":8000000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2023-02-01","end":"2024-01-31","val":10000000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2023-02-01","end":"2024-01-31","val":2000000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2024-01-31","val":25000000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "ShortTermBorrowings": { "units": { "USD": [
                    {"end":"2024-01-31","val":298000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "LongTermLoansPayable": { "units": { "USD": [
                    {"end":"2024-01-31","val":731000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2024-01-31","val":120000000,"form":"10-K","filed":"2024-04-11"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2023-02-01","end":"2024-01-31","val":7500000,"form":"10-K","filed":"2024-04-11"}
                ]}}
            }}
        }"#;
        let lake = parse_companyfacts_bytes(lake_raw, "LAKE", "0000798081", "https://sec.test", 1)
            .unwrap();
        assert_eq!(lake.annuals[0].total_debt, Decimal::from(1_029_000i64));

        // 3. CMI FY25 & Q3'26: DebtAndCapitalLeaseObligations == LongTermDebtAndCapitalLeaseObligations + LongTermDebtCurrent,
        // omitting CommercialPaper ($348M) + ShortTermBankLoansAndNotesPayable ($383M); total must reconcile to $7.519B (TTM) / $7.552B (FY25).
        let cmi_raw = br#"{
            "entityName": "CUMMINS INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":34000000000,"form":"10-K","filed":"2026-02-10"},
                    {"start":"2025-01-01","end":"2025-09-30","val":25000000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":26000000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":8500000000,"form":"10-K","filed":"2026-02-10"},
                    {"start":"2025-01-01","end":"2025-09-30","val":6300000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":6600000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":3500000000,"form":"10-K","filed":"2026-02-10"},
                    {"start":"2025-01-01","end":"2025-09-30","val":2600000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":2800000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":3800000000,"form":"10-K","filed":"2026-02-10"},
                    {"start":"2025-01-01","end":"2025-09-30","val":2500000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":2900000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":1100000000,"form":"10-K","filed":"2026-02-10"},
                    {"start":"2025-01-01","end":"2025-09-30","val":700000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":750000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-12-31","val":2200000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":2400000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "DebtAndCapitalLeaseObligations": { "units": { "USD": [
                    {"end":"2025-12-31","val":6886000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":6788000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "LongTermDebtAndCapitalLeaseObligations": { "units": { "USD": [
                    {"end":"2025-12-31","val":6792000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":6634000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "LongTermDebtCurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":94000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":154000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "CommercialPaper": { "units": { "USD": [
                    {"end":"2025-12-31","val":353000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":348000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "ShortTermBankLoansAndNotesPayable": { "units": { "USD": [
                    {"end":"2025-12-31","val":313000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":383000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "DebtInstrumentCarryingAmount": { "units": { "USD": [
                    {"end":"2025-12-31","val":7552000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":7519000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-12-31","val":11000000000,"form":"10-K","filed":"2026-02-10"},
                    {"end":"2026-09-30","val":12000000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-01-01","end":"2025-12-31","val":138000000,"form":"10-K","filed":"2026-02-10"},
                    {"start":"2026-01-01","end":"2026-09-30","val":138000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}}
            }}
        }"#;
        let cmi =
            parse_companyfacts_bytes(cmi_raw, "CMI", "0000026172", "https://sec.test", 1).unwrap();
        assert_eq!(cmi.annuals[0].total_debt, Decimal::from(7_552_000_000i64));
        assert_eq!(cmi.ttm.unwrap().total_debt, Decimal::from(7_519_000_000i64));

        // 4. v1 company_tickers.json validator rejects empty/HTML/non-ticker payloads before rename.
        assert!(validate_company_tickers_v1_json(b"{}").is_err());
        assert!(validate_company_tickers_v1_json(b"<html>bad</html>").is_err());
        assert!(
            validate_company_tickers_v1_json(br#"{"0":{"cik_str":320193,"ticker":"AAPL"}}"#)
                .is_ok()
        );
    }

    #[test]
    fn on_pahc_lcut_tsla_adbe_debt_and_tdg_non_double_count_work() {
        // 1. ON FY25: sub-component DebtAndCapitalLeaseObligations ($0.9M) must be rejected in favor
        // of verified LongTermDebtNoncurrent ($2,980.5M).
        let on_raw = br#"{
            "entityName": "ON SEMICONDUCTOR CORP",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":6000000000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":2500000000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":1100000000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":1400000000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":400000000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-12-31","val":2000000000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "DebtAndCapitalLeaseObligations": { "units": { "USD": [
                    {"end":"2025-12-31","val":900000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":2980500000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "DebtInstrumentCarryingAmount": { "units": { "USD": [
                    {"end":"2025-12-31","val":3004900000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-12-31","val":8500000000,"form":"10-K","filed":"2026-02-09"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-01-01","end":"2025-12-31","val":420000000,"form":"10-K","filed":"2026-02-09"}
                ]}}
            }}
        }"#;
        let on_facts =
            parse_companyfacts_bytes(on_raw, "ON", "0001097864", "https://sec.test", 1).unwrap();
        assert_eq!(
            on_facts.annuals[0].total_debt,
            Decimal::from(2_980_500_000i64)
        );

        // 2. PAHC FY26 (10-K): LongTermLoansPayable ($591.769M) + LongTermLineOfCredit ($116M)
        // + LoansPayableCurrent ($25.025M) = $732.794M; and Q1'27 TTM preserves revolver ($734.877M).
        let pahc_raw = br#"{
            "entityName": "PHIBRO ANIMAL HEALTH CORP",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-07-01","end":"2026-06-30","val":1200000000,"form":"10-K","filed":"2026-08-26"},
                    {"start":"2025-07-01","end":"2025-09-30","val":290000000,"fp":"Q1","form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-07-01","end":"2026-09-30","val":310000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-07-01","end":"2026-06-30","val":400000000,"form":"10-K","filed":"2026-08-26"},
                    {"start":"2025-07-01","end":"2025-09-30","val":95000000,"fp":"Q1","form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-07-01","end":"2026-09-30","val":105000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-07-01","end":"2026-06-30","val":140000000,"form":"10-K","filed":"2026-08-26"},
                    {"start":"2025-07-01","end":"2025-09-30","val":32000000,"fp":"Q1","form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-07-01","end":"2026-09-30","val":38000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-07-01","end":"2026-06-30","val":120000000,"form":"10-K","filed":"2026-08-26"},
                    {"start":"2025-07-01","end":"2025-09-30","val":25000000,"fp":"Q1","form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-07-01","end":"2026-09-30","val":30000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-07-01","end":"2026-06-30","val":40000000,"form":"10-K","filed":"2026-08-26"},
                    {"start":"2025-07-01","end":"2025-09-30","val":9000000,"fp":"Q1","form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-07-01","end":"2026-09-30","val":10000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2026-06-30","val":90000000,"form":"10-K","filed":"2026-08-26"},
                    {"end":"2026-09-30","val":95000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "LongTermLoansPayable": { "units": { "USD": [
                    {"end":"2026-06-30","val":591769000,"form":"10-K","filed":"2026-08-26"},
                    {"end":"2026-09-30","val":585852000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "LongTermDebtNoncurrent": { "units": { "USD": [
                    {"end":"2026-09-30","val":585852000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "LoansPayableCurrent": { "units": { "USD": [
                    {"end":"2026-06-30","val":25025000,"form":"10-K","filed":"2026-08-26"},
                    {"end":"2026-09-30","val":25025000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "LongTermLineOfCredit": { "units": { "USD": [
                    {"end":"2026-06-30","val":116000000,"form":"10-K","filed":"2026-08-26"},
                    {"end":"2026-09-30","val":124000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "DebtInstrumentCarryingAmount": { "units": { "USD": [
                    {"end":"2026-06-30","val":737875000,"form":"10-K","filed":"2026-08-26"},
                    {"end":"2026-09-30","val":615619000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2026-06-30","val":300000000,"form":"10-K","filed":"2026-08-26"},
                    {"end":"2026-09-30","val":310000000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-07-01","end":"2026-06-30","val":40500000,"form":"10-K","filed":"2026-08-26"},
                    {"start":"2026-07-01","end":"2026-09-30","val":40500000,"fp":"Q1","form":"10-Q","filed":"2026-11-04"}
                ]}}
            }}
        }"#;
        let pahc = parse_companyfacts_bytes(pahc_raw, "PAHC", "0001069899", "https://sec.test", 1)
            .unwrap();
        assert_eq!(pahc.annuals[0].total_debt, Decimal::from(732_794_000i64));
        assert_eq!(
            pahc.ttm.as_ref().unwrap().total_debt,
            Decimal::from(734_877_000i64)
        );

        // 3. LCUT Q3'26 TTM: LongTermLoansPayable ($58.351M) + LongTermLineOfCredit ($89.858M) = $148.209M.
        let lcut_raw = br#"{
            "entityName": "LIFETIME BRANDS INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":680000000,"form":"10-K","filed":"2026-03-12"},
                    {"start":"2025-01-01","end":"2025-09-30","val":490000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":500000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":250000000,"form":"10-K","filed":"2026-03-12"},
                    {"start":"2025-01-01","end":"2025-09-30","val":180000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":185000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":40000000,"form":"10-K","filed":"2026-03-12"},
                    {"start":"2025-01-01","end":"2025-09-30","val":25000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":27000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":50000000,"form":"10-K","filed":"2026-03-12"},
                    {"start":"2025-01-01","end":"2025-09-30","val":30000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":32000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-03-12"},
                    {"start":"2025-01-01","end":"2025-09-30","val":7000000,"fp":"Q3","form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":8000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-12-31","val":15000000,"form":"10-K","filed":"2026-03-12"},
                    {"end":"2026-09-30","val":12000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "LongTermLoansPayable": { "units": { "USD": [
                    {"end":"2025-12-31","val":125927000,"form":"10-K","filed":"2026-03-12"},
                    {"end":"2026-09-30","val":58351000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "LongTermLineOfCredit": { "units": { "USD": [
                    {"end":"2025-12-31","val":54105000,"form":"10-K","filed":"2026-03-12"},
                    {"end":"2026-09-30","val":89858000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "LongTermDebtCurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":5022000,"form":"10-K","filed":"2026-03-12"},
                    {"end":"2026-09-30","val":0,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-12-31","val":200000000,"form":"10-K","filed":"2026-03-12"},
                    {"end":"2026-09-30","val":210000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-01-01","end":"2025-12-31","val":22000000,"form":"10-K","filed":"2026-03-12"},
                    {"start":"2026-01-01","end":"2026-09-30","val":22000000,"fp":"Q3","form":"10-Q","filed":"2026-11-05"}
                ]}}
            }}
        }"#;
        let lcut = parse_companyfacts_bytes(lcut_raw, "LCUT", "0000874396", "https://sec.test", 1)
            .unwrap();
        assert_eq!(lcut.annuals[0].total_debt, Decimal::from(185_054_000i64));
        assert_eq!(
            lcut.ttm.as_ref().unwrap().total_debt,
            Decimal::from(148_209_000i64)
        );

        // 4. TSLA Q3'26 TTM & ADBE FY24/Q3'26: LongTermDebt excludes DebtCurrent -> must add DebtCurrent,
        // while TDG combined LongTermDebt > LongTermDebtAndCapitalLeaseObligations must NOT double count current.
        let tsla_raw = br#"{
            "entityName": "TESLA INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":97000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":70000000000,"fp":"Q3","form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":75000000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":17000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":12000000000,"fp":"Q3","form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":13000000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":7000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":5000000000,"fp":"Q3","form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":5500000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":14000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":10000000000,"fp":"Q3","form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":11000000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2025-01-01","end":"2025-12-31","val":9000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":6000000000,"fp":"Q3","form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":6500000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2025-12-31","val":16000000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":18000000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "LongTermDebt": { "units": { "USD": [
                    {"end":"2025-12-31","val":6584000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":7956000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "DebtCurrent": { "units": { "USD": [
                    {"end":"2025-12-31","val":1569000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":1311000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "DebtInstrumentCarryingAmount": { "units": { "USD": [
                    {"end":"2025-12-31","val":8177000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":9284000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2025-12-31","val":73000000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":79000000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2025-01-01","end":"2025-12-31","val":3400000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2026-01-01","end":"2026-09-30","val":3400000000,"fp":"Q3","form":"10-Q","filed":"2026-10-22"}
                ]}}
            }}
        }"#;
        let tsla = parse_companyfacts_bytes(tsla_raw, "TSLA", "0001318605", "https://sec.test", 1)
            .unwrap();
        assert_eq!(tsla.annuals[0].total_debt, Decimal::from(8_153_000_000i64));
        assert_eq!(
            tsla.ttm.as_ref().unwrap().total_debt,
            Decimal::from(9_267_000_000i64)
        );

        let adbe_raw = br#"{
            "entityName": "ADOBE INC",
            "facts": { "us-gaap": {
                "Revenues": { "units": { "USD": [
                    {"start":"2023-12-02","end":"2024-11-29","val":21500000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "GrossProfit": { "units": { "USD": [
                    {"start":"2023-12-02","end":"2024-11-29","val":19000000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "OperatingIncomeLoss": { "units": { "USD": [
                    {"start":"2023-12-02","end":"2024-11-29","val":8000000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                    {"start":"2023-12-02","end":"2024-11-29","val":8000000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                    {"start":"2023-12-02","end":"2024-11-29","val":200000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                    {"end":"2024-11-29","val":7800000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "LongTermDebt": { "units": { "USD": [
                    {"end":"2024-11-29","val":4129000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "DebtCurrent": { "units": { "USD": [
                    {"end":"2024-11-29","val":1499000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "DebtInstrumentCarryingAmount": { "units": { "USD": [
                    {"end":"2024-11-29","val":5650000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "StockholdersEquity": { "units": { "USD": [
                    {"end":"2024-11-29","val":14000000000,"form":"10-K","filed":"2025-01-13"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                    {"start":"2023-12-02","end":"2024-11-29","val":445000000,"form":"10-K","filed":"2025-01-13"}
                ]}}
            }}
        }"#;
        let adbe = parse_companyfacts_bytes(adbe_raw, "ADBE", "0000796343", "https://sec.test", 1)
            .unwrap();
        assert_eq!(adbe.annuals[0].total_debt, Decimal::from(5_628_000_000i64));

        // 5. Direct unit checks: TDG combined LongTermDebt > LongTermDebtAndCapitalLeaseObligations
        // avoids double-counting LongTermDebtCurrent, while >15% carrying shortfall fails closed.
        let tdg_ns: FactNamespace<'_> = serde_json::from_str(
            r#"{
            "LongTermDebt": { "units": { "USD": [
                {"end":"2026-09-30","val":33890000000,"form":"10-Q","filed":"2026-11-05"}
            ]}},
            "LongTermDebtAndCapitalLeaseObligations": { "units": { "USD": [
                {"end":"2026-09-30","val":33743000000,"form":"10-Q","filed":"2026-11-05"}
            ]}},
            "LongTermDebtCurrent": { "units": { "USD": [
                {"end":"2026-09-30","val":147000000,"form":"10-Q","filed":"2026-11-05"}
            ]}},
            "DebtInstrumentCarryingAmount": { "units": { "USD": [
                {"end":"2026-09-30","val":34124000000,"form":"10-Q","filed":"2026-11-05"}
            ]}}
        }"#,
        )
        .unwrap();
        let tdg_debt = extract_total_debt(
            Some(&tdg_ns),
            &["10-Q"],
            parse_date_days("2026-09-30").unwrap(),
            "2026-09-30",
        );
        assert_eq!(tdg_debt.val, Decimal::from(33_890_000_000i64));

        // Contradictory 1.167x carrying shortfall without LongTermDebt must still reject (> 1.15x).
        let short_ns: FactNamespace<'_> = serde_json::from_str(
            r#"{
            "DebtCurrent": { "units": { "USD": [
                {"end":"2026-09-30","val":7956000000,"form":"10-Q","filed":"2026-11-05"}
            ]}},
            "DebtInstrumentCarryingAmount": { "units": { "USD": [
                {"end":"2026-09-30","val":9284000000,"form":"10-Q","filed":"2026-11-05"}
            ]}}
        }"#,
        )
        .unwrap();
        let short_debt = extract_total_debt(
            Some(&short_ns),
            &["10-Q"],
            parse_date_days("2026-09-30").unwrap(),
            "2026-09-30",
        );
        assert_eq!(short_debt.tag, "absent");
    }

    #[test]
    fn skips_incomplete_oldest_annual_without_aborting_complete_years_like_tdg() {
        // Oldest year 2018-09-30 lacks cash_and_st; 2019-09-30 only has DEI shares 48d post-end (>45d);
        // years 2020-09-30..2022-09-30 are complete and must NOT be discarded by older incomplete years.
        let tdg_like_raw = br#"{
            "entityName": "TRANSDIGM GROUP INC",
            "facts": {
                "dei": {
                    "EntityCommonStockSharesOutstanding": { "units": { "shares": [
                        {"end":"2019-11-17","val":53000000,"form":"10-K","filed":"2019-11-19"}
                    ]}}
                },
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2017-10-01","end":"2018-09-30","val":3800000000,"form":"10-K","filed":"2018-11-15"},
                        {"start":"2018-10-01","end":"2019-09-30","val":5200000000,"form":"10-K","filed":"2019-11-19"},
                        {"start":"2019-10-01","end":"2020-09-30","val":5100000000,"form":"10-K","filed":"2020-11-12"},
                        {"start":"2020-10-01","end":"2021-09-30","val":4800000000,"form":"10-K","filed":"2021-11-16"},
                        {"start":"2021-10-01","end":"2022-09-30","val":5400000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2017-10-01","end":"2018-09-30","val":2100000000,"form":"10-K","filed":"2018-11-15"},
                        {"start":"2018-10-01","end":"2019-09-30","val":2900000000,"form":"10-K","filed":"2019-11-19"},
                        {"start":"2019-10-01","end":"2020-09-30","val":2800000000,"form":"10-K","filed":"2020-11-12"},
                        {"start":"2020-10-01","end":"2021-09-30","val":2600000000,"form":"10-K","filed":"2021-11-16"},
                        {"start":"2021-10-01","end":"2022-09-30","val":3100000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2017-10-01","end":"2018-09-30","val":1500000000,"form":"10-K","filed":"2018-11-15"},
                        {"start":"2018-10-01","end":"2019-09-30","val":2000000000,"form":"10-K","filed":"2019-11-19"},
                        {"start":"2019-10-01","end":"2020-09-30","val":1900000000,"form":"10-K","filed":"2020-11-12"},
                        {"start":"2020-10-01","end":"2021-09-30","val":1800000000,"form":"10-K","filed":"2021-11-16"},
                        {"start":"2021-10-01","end":"2022-09-30","val":2200000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2017-10-01","end":"2018-09-30","val":1000000000,"form":"10-K","filed":"2018-11-15"},
                        {"start":"2018-10-01","end":"2019-09-30","val":1100000000,"form":"10-K","filed":"2019-11-19"},
                        {"start":"2019-10-01","end":"2020-09-30","val":1200000000,"form":"10-K","filed":"2020-11-12"},
                        {"start":"2020-10-01","end":"2021-09-30","val":1000000000,"form":"10-K","filed":"2021-11-16"},
                        {"start":"2021-10-01","end":"2022-09-30","val":1400000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2017-10-01","end":"2018-09-30","val":70000000,"form":"10-K","filed":"2018-11-15"},
                        {"start":"2018-10-01","end":"2019-09-30","val":90000000,"form":"10-K","filed":"2019-11-19"},
                        {"start":"2019-10-01","end":"2020-09-30","val":100000000,"form":"10-K","filed":"2020-11-12"},
                        {"start":"2020-10-01","end":"2021-09-30","val":110000000,"form":"10-K","filed":"2021-11-16"},
                        {"start":"2021-10-01","end":"2022-09-30","val":120000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                        {"end":"2019-09-30","val":1400000000,"form":"10-K","filed":"2019-11-19"},
                        {"end":"2020-09-30","val":4700000000,"form":"10-K","filed":"2020-11-12"},
                        {"end":"2021-09-30","val":4780000000,"form":"10-K","filed":"2021-11-16"},
                        {"end":"2022-09-30","val":3000000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "LongTermDebt": { "units": { "USD": [
                        {"end":"2020-09-30","val":19000000000,"form":"10-K","filed":"2020-11-12"},
                        {"end":"2021-09-30","val":19500000000,"form":"10-K","filed":"2021-11-16"},
                        {"end":"2022-09-30","val":19700000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2018-09-30","val":-1000000000,"form":"10-K","filed":"2018-11-15"},
                        {"end":"2019-09-30","val":-2000000000,"form":"10-K","filed":"2019-11-19"},
                        {"end":"2020-09-30","val":-3500000000,"form":"10-K","filed":"2020-11-12"},
                        {"end":"2021-09-30","val":-3800000000,"form":"10-K","filed":"2021-11-16"},
                        {"end":"2022-09-30","val":-3900000000,"form":"10-K","filed":"2022-11-10"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2017-10-01","end":"2018-09-30","val":54000000,"form":"10-K","filed":"2018-11-15"},
                        {"start":"2019-10-01","end":"2020-09-30","val":56000000,"form":"10-K","filed":"2020-11-12"},
                        {"start":"2020-10-01","end":"2021-09-30","val":57000000,"form":"10-K","filed":"2021-11-16"},
                        {"start":"2021-10-01","end":"2022-09-30","val":57500000,"form":"10-K","filed":"2022-11-10"}
                    ]}}
                }
            }
        }"#;
        let parsed =
            parse_companyfacts_bytes(tdg_like_raw, "TDG", "0001260221", "https://sec.test", 1)
                .unwrap();
        assert_eq!(parsed.annuals.len(), 3);
        assert_eq!(parsed.annuals[0].period_end, "2020-09-30");
        assert_eq!(parsed.annuals[2].period_end, "2022-09-30");
        assert_eq!(
            parsed.annuals[2].total_debt,
            Decimal::from(19_700_000_000i64)
        );
    }

    #[test]
    fn xyl_hon_carrying_debt_alsn_noncurrent_marketable_securities_and_period_keys_work() {
        // 1. XYL FY25: DebtLongtermAndShorttermCombinedAmount ($1.942B carrying) < LongTermDebt
        //    ($1.954B principal before $12M unamortized discount) and ShortTermBorrowings ($534M)
        //    already equals 12m maturity -> total_debt must be $1.942B (not $2.488B).
        let xyl_raw = br#"{
            "entityName": "Xylem Inc.",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":8500000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":3200000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":1100000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":1200000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                        {"end":"2025-12-31","val":1479000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "DebtLongtermAndShorttermCombinedAmount": { "units": { "USD": [
                        {"end":"2025-12-31","val":1942000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "LongTermDebt": { "units": { "USD": [
                        {"end":"2025-12-31","val":1954000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "LongTermDebtNoncurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":1408000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "ShortTermBorrowings": { "units": { "USD": [
                        {"end":"2025-12-31","val":534000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "LongTermDebtMaturitiesRepaymentsOfPrincipalInNextTwelveMonths": { "units": { "USD": [
                        {"end":"2025-12-31","val":534000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "DebtInstrumentUnamortizedDiscountPremiumAndDebtIssuanceCostsNet": { "units": { "USD": [
                        {"end":"2025-12-31","val":12000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":10200000000,"form":"10-K","filed":"2026-02-25"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":244000000,"form":"10-K","filed":"2026-02-25"}
                    ]}}
                }
            }
        }"#;
        let xyl =
            parse_companyfacts_bytes(xyl_raw, "XYL", "0001524472", "https://sec.test", 1).unwrap();
        assert_eq!(xyl.annuals[0].total_debt, Decimal::from(1_942_000_000i64));

        // 2. HON FY25: DebtLongtermAndShorttermCombinedAmount ($28.687B = LT $27.141B + cur $1.546B)
        //    is below LongTermDebt ($29.046B footnote) and omits ShortTermBorrowings ($5.893B) ->
        //    total_debt must be $34.580B (not $36.485B).
        let hon_raw = br#"{
            "entityName": "HONEYWELL INTERNATIONAL INC",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":38000000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":14500000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":7800000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":6100000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":1100000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                        {"end":"2025-12-31","val":12487000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "ShortTermInvestments": { "units": { "USD": [
                        {"end":"2025-12-31","val":443000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "DebtLongtermAndShorttermCombinedAmount": { "units": { "USD": [
                        {"end":"2025-12-31","val":28687000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "LongTermDebt": { "units": { "USD": [
                        {"end":"2025-12-31","val":29046000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "LongTermDebtAndCapitalLeaseObligations": { "units": { "USD": [
                        {"end":"2025-12-31","val":27141000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "LongTermDebtAndCapitalLeaseObligationsCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":1546000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "ShortTermBorrowings": { "units": { "USD": [
                        {"end":"2025-12-31","val":5893000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":18000000000,"form":"10-K","filed":"2026-02-17"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":650000000,"form":"10-K","filed":"2026-02-17"}
                    ]}}
                }
            }
        }"#;
        let hon =
            parse_companyfacts_bytes(hon_raw, "HON", "0000773840", "https://sec.test", 1).unwrap();
        assert_eq!(hon.annuals[0].total_debt, Decimal::from(34_580_000_000i64));
        assert_eq!(
            hon.annuals[0].cash_and_st,
            Some(Decimal::from(12_930_000_000i64))
        );

        // 3. ALSN FY25: MarketableSecurities ($24M) is noncurrent (MarketableSecuritiesNoncurrent exists),
        //    so generic MarketableSecurities must NOT be added to CashAndCashEquivalents ($1.495B).
        let alsn_raw = br#"{
            "entityName": "Allison Transmission Holdings Inc",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":3200000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":1500000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":900000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":800000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":150000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                        {"end":"2025-12-31","val":1495000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "MarketableSecurities": { "units": { "USD": [
                        {"end":"2025-12-31","val":24000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "MarketableSecuritiesNoncurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":24000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "AssetsCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":2200000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "LongTermDebtNoncurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":2885000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "LongTermDebtCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":5000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":1800000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":85000000,"form":"10-K","filed":"2026-02-24"}
                    ]}}
                }
            }
        }"#;
        let alsn = parse_companyfacts_bytes(alsn_raw, "ALSN", "0001411207", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            alsn.annuals[0].cash_and_st,
            Some(Decimal::from(1_495_000_000i64))
        );
        assert_eq!(alsn.annuals[0].total_debt, Decimal::from(2_890_000_000i64));

        // 4. Positive current-classification evidence via AssetsCurrent balance sheet identity:
        let ms_evidenced_raw = br#"{
            "entityName": "Current Securities Evidenced Inc.",
            "facts": {
                "us-gaap": {
                    "Revenues": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "GrossProfit": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "OperatingIncomeLoss": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "NetCashProvidedByUsedInOperatingActivities": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":90000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "PaymentsToAcquirePropertyPlantAndEquipment": { "units": { "USD": [
                        {"start":"2025-01-01","end":"2025-12-31","val":20000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "CashAndCashEquivalentsAtCarryingValue": { "units": { "USD": [
                        {"end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "MarketableSecurities": { "units": { "USD": [
                        {"end":"2025-12-31","val":50000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "AccountsReceivableNetCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":40000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "OtherAssetsCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "AssetsCurrent": { "units": { "USD": [
                        {"end":"2025-12-31","val":200000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "StockholdersEquity": { "units": { "USD": [
                        {"end":"2025-12-31","val":400000000,"form":"10-K","filed":"2026-02-24"}
                    ]}},
                    "WeightedAverageNumberOfDilutedSharesOutstanding": { "units": { "shares": [
                        {"start":"2025-01-01","end":"2025-12-31","val":25000000,"form":"10-K","filed":"2026-02-24"}
                    ]}}
                }
            }
        }"#;
        let evidenced = parse_companyfacts_bytes(
            ms_evidenced_raw,
            "CURR",
            "0009999001",
            "https://sec.test",
            1,
        )
        .unwrap();
        assert_eq!(
            evidenced.annuals[0].cash_and_st,
            Some(Decimal::from(150_000_000i64))
        );

        // 5. Intermediate year loss must trigger period-key regression even when count is equal.
        let prev = vec![
            "2021-12-31".to_owned(),
            "2022-12-31".to_owned(),
            "2023-12-31".to_owned(),
            "2024-12-31".to_owned(),
        ];
        let dropped_mid = vec![
            "2021-12-31".to_owned(),
            "2023-12-31".to_owned(),
            "2024-12-31".to_owned(),
            "2025-12-31".to_owned(),
        ];
        assert!(is_annual_period_coverage_regression(
            &prev,
            None,
            &dropped_mid,
            None,
            Some("2025-12-31")
        ));

        // 6. Sliding 8-year window dropping only oldest pre-window year (2018) when adding 2026 is valid.
        let prev_8: Vec<String> = (2018..=2025).map(|y| format!("{y}-12-31")).collect();
        let next_8: Vec<String> = (2019..=2026).map(|y| format!("{y}-12-31")).collect();
        assert!(!is_annual_period_coverage_regression(
            &prev_8,
            None,
            &next_8,
            None,
            Some("2026-12-31")
        ));
    }

    #[test]
    fn khc_ctkb_forr_pays_keel_cash_and_ambiguous_ms_work() {
        // 1. KHC FY25: PrepaidExpenseCurrent + OtherAssetsCurrent + AssetsOfDisposalGroupIncludingDiscontinuedOperationCurrent -> $3.675B
        let khc_raw = br#"{
            "entityName": "Kraft Heinz Co",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-27","val":25000000000,"form":"10-K","filed":"2026-02-20"}]}},
                "GrossProfit": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-27","val":8500000000,"form":"10-K","filed":"2026-02-20"}]}},
                "OperatingIncomeLoss": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-27","val":4500000000,"form":"10-K","filed":"2026-02-20"}]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-27","val":4200000000,"form":"10-K","filed":"2026-02-20"}]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-27","val":1000000000,"form":"10-K","filed":"2026-02-20"}]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[{"end":"2025-12-27","val":2615000000,"form":"10-K","filed":"2026-02-20"}]}},
                "MarketableSecurities": {"units":{"USD":[{"end":"2025-12-27","val":1060000000,"form":"10-K","filed":"2026-02-20"}]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[{"end":"2025-12-27","val":2254000000,"form":"10-K","filed":"2026-02-20"}]}},
                "InventoryNet": {"units":{"USD":[{"end":"2025-12-27","val":3167000000,"form":"10-K","filed":"2026-02-20"}]}},
                "OtherAssetsCurrent": {"units":{"USD":[{"end":"2025-12-27","val":588000000,"form":"10-K","filed":"2026-02-20"}]}},
                "PrepaidExpenseCurrent": {"units":{"USD":[{"end":"2025-12-27","val":291000000,"form":"10-K","filed":"2026-02-20"}]}},
                "AssetsOfDisposalGroupIncludingDiscontinuedOperationCurrent": {"units":{"USD":[{"end":"2025-12-27","val":152000000,"form":"10-K","filed":"2026-02-20"}]}},
                "AssetsCurrent": {"units":{"USD":[{"end":"2025-12-27","val":10127000000,"form":"10-K","filed":"2026-02-20"}]}},
                "StockholdersEquity": {"units":{"USD":[{"end":"2025-12-27","val":41664000000,"form":"10-K","filed":"2026-02-20"}]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[{"start":"2025-01-01","end":"2025-12-27","val":1200000000,"form":"10-K","filed":"2026-02-20"}]}}
            }}
        }"#;
        let khc =
            parse_companyfacts_bytes(khc_raw, "KHC", "0001637459", "https://sec.test", 1).unwrap();
        assert_eq!(
            khc.annuals[0].cash_and_st,
            Some(Decimal::from(3_675_000_000i64))
        );

        // 2. FORR FY25: stale 2013 AvailableForSaleSecuritiesNoncurrent ignored; noncurrent BS identity proves current ($127.656M).
        let forr_raw = br#"{
            "entityName": "Forrester Research Inc",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":400000000,"form":"10-K","filed":"2026-03-01"}]}},
                "GrossProfit": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":230000000,"form":"10-K","filed":"2026-03-01"}]}},
                "OperatingIncomeLoss": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":30000000,"form":"10-K","filed":"2026-03-01"}]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":25000000,"form":"10-K","filed":"2026-03-01"}]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[{"start":"2025-01-01","end":"2025-12-31","val":5000000,"form":"10-K","filed":"2026-03-01"}]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[{"end":"2025-12-31","val":63335000,"form":"10-K","filed":"2026-03-01"}]}},
                "MarketableSecurities": {"units":{"USD":[{"end":"2025-12-31","val":64321000,"form":"10-K","filed":"2026-03-01"}]}},
                "AvailableForSaleSecuritiesNoncurrent": {"units":{"USD":[{"end":"2013-09-30","val":5000000,"form":"10-Q","filed":"2013-11-08"}]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[{"end":"2025-12-31","val":50850000,"form":"10-K","filed":"2026-03-01"}]}},
                "PrepaidExpenseAndOtherAssetsCurrent": {"units":{"USD":[{"end":"2025-12-31","val":12119000,"form":"10-K","filed":"2026-03-01"}]}},
                "AssetsCurrent": {"units":{"USD":[{"end":"2025-12-31","val":212685000,"form":"10-K","filed":"2026-03-01"}]}},
                "Assets": {"units":{"USD":[{"end":"2025-12-31","val":404034000,"form":"10-K","filed":"2026-03-01"}]}},
                "Goodwill": {"units":{"USD":[{"end":"2025-12-31","val":120381000,"form":"10-K","filed":"2026-03-01"}]}},
                "FiniteLivedIntangibleAssetsNet": {"units":{"USD":[{"end":"2025-12-31","val":18730000,"form":"10-K","filed":"2026-03-01"}]}},
                "PropertyPlantAndEquipmentNet": {"units":{"USD":[{"end":"2025-12-31","val":11217000,"form":"10-K","filed":"2026-03-01"}]}},
                "OperatingLeaseRightOfUseAsset": {"units":{"USD":[{"end":"2025-12-31","val":30662000,"form":"10-K","filed":"2026-03-01"}]}},
                "OtherAssetsNoncurrent": {"units":{"USD":[{"end":"2025-12-31","val":10359000,"form":"10-K","filed":"2026-03-01"}]}},
                "StockholdersEquity": {"units":{"USD":[{"end":"2025-12-31","val":126525000,"form":"10-K","filed":"2026-03-01"}]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[{"start":"2025-01-01","end":"2025-12-31","val":19000000,"form":"10-K","filed":"2026-03-01"}]}}
            }}
        }"#;
        let forr = parse_companyfacts_bytes(forr_raw, "FORR", "0001023313", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            forr.annuals[0].cash_and_st,
            Some(Decimal::from(127_656_000i64))
        );

        // 3. PAYS unrestricted "Cash" preferred over combined restricted cash fallback.
        let pays_raw = br#"{
            "entityName": "Paysign Inc",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":60000000,"form":"10-K","filed":"2026-03-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":42000000,"form":"10-Q","filed":"2025-11-10"},
                    {"start":"2026-01-01","end":"2026-09-30","val":55000000,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":35000000,"form":"10-K","filed":"2026-03-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":24000000,"form":"10-Q","filed":"2025-11-10"},
                    {"start":"2026-01-01","end":"2026-09-30","val":32000000,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-03-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":7000000,"form":"10-Q","filed":"2025-11-10"},
                    {"start":"2026-01-01","end":"2026-09-30","val":11000000,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":52000000,"form":"10-K","filed":"2026-03-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":20000000,"form":"10-Q","filed":"2025-11-10"},
                    {"start":"2026-01-01","end":"2026-09-30","val":82000000,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":5000000,"form":"10-K","filed":"2026-03-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":3000000,"form":"10-Q","filed":"2025-11-10"},
                    {"start":"2026-01-01","end":"2026-09-30","val":4000000,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2025-12-31","val":21067651,"form":"10-K","filed":"2026-03-25"},
                    {"end":"2026-09-30","val":34539557,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents": {"units":{"USD":[
                    {"end":"2025-12-31","val":164984711,"form":"10-K","filed":"2026-03-25"},
                    {"end":"2026-09-30","val":236333714,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "RestrictedCash": {"units":{"USD":[
                    {"end":"2025-12-31","val":143917060,"form":"10-K","filed":"2026-03-25"},
                    {"end":"2026-09-30","val":201794157,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":45000000,"form":"10-K","filed":"2026-03-25"},
                    {"end":"2026-09-30","val":52000000,"form":"10-Q","filed":"2026-11-10"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":54000000,"form":"10-K","filed":"2026-03-25"},
                    {"start":"2026-01-01","end":"2026-09-30","val":55000000,"form":"10-Q","filed":"2026-11-10"}
                ]}}
            }}
        }"#;
        let pays = parse_companyfacts_bytes(pays_raw, "PAYS", "0001496443", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            pays.annuals[0].cash_and_st,
            Some(Decimal::from(21_067_651i64))
        );
        assert_eq!(
            pays.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(34_539_557i64))
        );

        // 4. Ambiguous generic MarketableSecurities (unreconciled current & noncurrent) sets cash_and_st = None + absent provenance.
        let ambig_raw = br#"{
            "entityName": "Ambiguous Investments Corp",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":350000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":390000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":210000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":240000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":70000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":85000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":90000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":60000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":75000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":7000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":8000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":110000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "MarketableSecurities": {"units":{"USD":[
                    {"end":"2025-12-31","val":40000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":45000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":320000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "Assets": {"units":{"USD":[
                    {"end":"2025-12-31","val":800000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":850000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":540000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":20000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2026-01-01","end":"2026-09-30","val":20000000,"form":"10-Q","filed":"2026-11-05"}
                ]}}
            }}
        }"#;
        let ambig =
            parse_companyfacts_bytes(ambig_raw, "AMBG", "0009999002", "https://sec.test", 1)
                .unwrap();
        assert_eq!(ambig.annuals[0].cash_and_st, None);
        assert_eq!(ambig.ttm.as_ref().unwrap().cash_and_st, None);
        assert!(
            ambig.annuals[0]
                .provenance
                .iter()
                .any(|p| p.field == "cash_and_st" && p.source_tag == "absent")
        );
        assert!(
            ambig
                .ttm
                .as_ref()
                .unwrap()
                .provenance
                .iter()
                .any(|p| p.field == "ttm_cash_and_st" && p.source_tag == "absent")
        );

        // 5. CTKB FY25 ($261.529M) & Q3'26 ($255.899M): PrepaidTaxes & OtherAssetsCurrent sit inside
        // PrepaidExpenseAndOtherAssetsCurrent and must not double-count against AssetsCurrent.
        let ctkb_raw = br#"{
            "entityName": "Cytek Biosciences, Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":201000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":139000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":141000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":105000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":72000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":73000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":-20000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":-15000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":-14000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":5000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":2000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":4000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":6000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":4000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":3000000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2025-12-31","val":28983000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":21932000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":90853000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":66806000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "RestrictedCashCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":0,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":385000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "MarketableSecurities": {"units":{"USD":[
                    {"end":"2025-12-31","val":170676000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":189093000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":62509000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":59190000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "InventoryNet": {"units":{"USD":[
                    {"end":"2025-12-31","val":48428000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":52371000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "PrepaidExpenseAndOtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":19530000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":11090000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "OtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":2977000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":2693000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "PrepaidTaxes": {"units":{"USD":[
                    {"end":"2025-12-31","val":2789000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":2795000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "IncomeTaxesReceivableCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":1500000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":1500000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":391996000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":378935000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "Assets": {"units":{"USD":[
                    {"end":"2025-12-31","val":461542000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":446145000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":341739000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":321354000,"form":"10-Q","filed":"2026-11-05"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":130000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2026-01-01","end":"2026-09-30","val":128000000,"form":"10-Q","filed":"2026-11-05"}
                ]}}
            }}
        }"#;
        let ctkb = parse_companyfacts_bytes(ctkb_raw, "CTKB", "0001831915", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            ctkb.annuals[0].cash_and_st,
            Some(Decimal::from(261_529_000i64))
        );
        assert_eq!(
            ctkb.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(255_899_000i64))
        );

        // 6. YELP Q3'26 ($119.840M): zero CashCashEquivalentsAndShortTermInvestments falls back to CashAndCashEquivalentsAtCarryingValue.
        let yelp_raw = br#"{
            "entityName": "Yelp Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":1400000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1040000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":1080000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":1280000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":950000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":990000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":180000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":130000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":145000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":220000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":240000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":12000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":8000000,"form":"10-Q","filed":"2025-11-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":9000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":216062000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":119840000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "CashCashEquivalentsAndShortTermInvestments": {"units":{"USD":[
                    {"end":"2026-09-30","val":0,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "MarketableSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":103290000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":0,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":514935000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":313809000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":700000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":680000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":66000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2026-01-01","end":"2026-09-30","val":64000000,"form":"10-Q","filed":"2026-11-06"}
                ]}}
            }}
        }"#;
        let yelp = parse_companyfacts_bytes(yelp_raw, "YELP", "0001345016", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            yelp.annuals[0].cash_and_st,
            Some(Decimal::from(319_352_000i64))
        );
        assert_eq!(
            yelp.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(119_840_000i64))
        );

        // 7. GEN (2026-10-02): disjoint Cash ($420M) + CashEquivalentsAtCarryingValue ($163M) = $583M unrestricted;
        //    ECG Q3'26: RestrictedCashAndCashEquivalents ($19.4M) deducted from combined ($264.191M) = $244.791M.
        let gen_raw = br#"{
            "entityName": "Gen Digital Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-03-29","end":"2026-04-03","val":4100000000,"form":"10-K","filed":"2026-05-21"},
                    {"start":"2025-03-29","end":"2025-09-26","val":2000000000,"form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-04-04","end":"2026-10-02","val":2200000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-03-29","end":"2026-04-03","val":3300000000,"form":"10-K","filed":"2026-05-21"},
                    {"start":"2025-03-29","end":"2025-09-26","val":1600000000,"form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-04-04","end":"2026-10-02","val":1760000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-03-29","end":"2026-04-03","val":1400000000,"form":"10-K","filed":"2026-05-21"},
                    {"start":"2025-03-29","end":"2025-09-26","val":680000000,"form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-04-04","end":"2026-10-02","val":760000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-03-29","end":"2026-04-03","val":1300000000,"form":"10-K","filed":"2026-05-21"},
                    {"start":"2025-03-29","end":"2025-09-26","val":600000000,"form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-04-04","end":"2026-10-02","val":700000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-03-29","end":"2026-04-03","val":20000000,"form":"10-K","filed":"2026-05-21"},
                    {"start":"2025-03-29","end":"2025-09-26","val":10000000,"form":"10-Q","filed":"2025-11-01"},
                    {"start":"2026-04-04","end":"2026-10-02","val":11000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "CashCashEquivalentsAndShortTermInvestments": {"units":{"USD":[
                    {"end":"2026-04-03","val":411000000,"form":"10-K","filed":"2026-05-21"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2026-04-03","val":283000000,"form":"10-K","filed":"2026-05-21"},
                    {"end":"2026-10-02","val":420000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "CashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2026-04-03","val":119000000,"form":"10-K","filed":"2026-05-21"},
                    {"end":"2026-10-02","val":163000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "RestrictedCash": {"units":{"USD":[
                    {"end":"2026-04-03","val":9000000,"form":"10-K","filed":"2026-05-21"},
                    {"end":"2026-10-02","val":6000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents": {"units":{"USD":[
                    {"end":"2026-04-03","val":411000000,"form":"10-K","filed":"2026-05-21"},
                    {"end":"2026-10-02","val":589000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2026-04-03","val":2100000000,"form":"10-K","filed":"2026-05-21"},
                    {"end":"2026-10-02","val":2300000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-03-29","end":"2026-04-03","val":630000000,"form":"10-K","filed":"2026-05-21"},
                    {"start":"2026-04-04","end":"2026-10-02","val":625000000,"form":"10-Q","filed":"2026-10-29"}
                ]}}
            }}
        }"#;
        let gen_rep =
            parse_companyfacts_bytes(gen_raw, "GEN", "0000849399", "https://sec.test", 1).unwrap();
        assert_eq!(
            gen_rep.annuals[0].cash_and_st,
            Some(Decimal::from(402_000_000i64))
        );
        assert_eq!(
            gen_rep.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(583_000_000i64))
        );

        let ecg_raw = br#"{
            "entityName": "Everus Construction Group, Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":3000000000,"form":"10-K","filed":"2026-02-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":2200000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":2600000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":400000000,"form":"10-K","filed":"2026-02-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":290000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":350000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":210000000,"form":"10-K","filed":"2026-02-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":150000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":190000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":220000000,"form":"10-K","filed":"2026-02-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":150000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":180000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":45000000,"form":"10-K","filed":"2026-02-25"},
                    {"start":"2025-01-01","end":"2025-09-30","val":30000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":38000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents": {"units":{"USD":[
                    {"end":"2025-12-31","val":170500000,"form":"10-K","filed":"2026-02-25"},
                    {"end":"2026-09-30","val":264191000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "RestrictedCashAndCashEquivalents": {"units":{"USD":[
                    {"end":"2025-12-31","val":17800000,"form":"10-K","filed":"2026-02-25"},
                    {"end":"2026-09-30","val":19400000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":650000000,"form":"10-K","filed":"2026-02-25"},
                    {"end":"2026-09-30","val":780000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":51000000,"form":"10-K","filed":"2026-02-25"},
                    {"start":"2026-01-01","end":"2026-09-30","val":51200000,"form":"10-Q","filed":"2026-11-06"}
                ]}}
            }}
        }"#;
        let ecg =
            parse_companyfacts_bytes(ecg_raw, "ECG", "0002015845", "https://sec.test", 1).unwrap();
        assert_eq!(
            ecg.annuals[0].cash_and_st,
            Some(Decimal::from(152_700_000i64))
        );
        assert_eq!(
            ecg.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(244_791_000i64))
        );

        // 8. MLSS Q3'26 & SCNX FY24: Cash and CashEquivalentsAtCarryingValue representing the same balance
        //    must NOT be double-counted even when AssetsCurrent has room; scientific raw numbers normalize cleanly.
        let mlss_raw = br#"{
            "entityName": "Milestone Scientific Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":9000000,"form":"10-K","filed":"2026-03-31"},
                    {"start":"2025-01-01","end":"2025-09-30","val":6500000,"form":"10-Q","filed":"2025-11-13"},
                    {"start":"2026-01-01","end":"2026-09-30","val":7200000,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":6000000,"form":"10-K","filed":"2026-03-31"},
                    {"start":"2025-01-01","end":"2025-09-30","val":4300000,"form":"10-Q","filed":"2025-11-13"},
                    {"start":"2026-01-01","end":"2026-09-30","val":4800000,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":-2000000,"form":"10-K","filed":"2026-03-31"},
                    {"start":"2025-01-01","end":"2025-09-30","val":-1500000,"form":"10-Q","filed":"2025-11-13"},
                    {"start":"2026-01-01","end":"2026-09-30","val":-1200000,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":-1800000,"form":"10-K","filed":"2026-03-31"},
                    {"start":"2025-01-01","end":"2025-09-30","val":-1300000,"form":"10-Q","filed":"2025-11-13"},
                    {"start":"2026-01-01","end":"2026-09-30","val":-900000,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":50000,"form":"10-K","filed":"2026-03-31"},
                    {"start":"2025-01-01","end":"2025-09-30","val":30000,"form":"10-Q","filed":"2025-11-13"},
                    {"start":"2026-01-01","end":"2026-09-30","val":20000,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":7477834,"form":"10-K","filed":"2026-03-31"},
                    {"end":"2026-09-30","val":6765191,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2025-12-31","val":1112642,"form":"10-K","filed":"2026-03-31"},
                    {"end":"2026-09-30","val":1315401,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "CashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":1100000,"form":"10-K","filed":"2026-03-31"},
                    {"end":"2026-09-30","val":1300000,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalentsIncludingDisposalGroupAndDiscontinuedOperations": {"units":{"USD":[
                    {"end":"2025-12-31","val":1112642,"form":"10-K","filed":"2026-03-31"},
                    {"end":"2026-09-30","val":1315401,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":4200000,"form":"10-K","filed":"2026-03-31"},
                    {"end":"2026-09-30","val":3656399,"form":"10-Q","filed":"2026-11-13"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":80000000,"form":"10-K","filed":"2026-03-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":82000000,"form":"10-Q","filed":"2026-11-13"}
                ]}}
            }}
        }"#;
        let mlss = parse_companyfacts_bytes(mlss_raw, "MLSS", "0000855683", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            mlss.annuals[0].cash_and_st,
            Some(Decimal::from(1_112_642i64))
        );
        assert_eq!(
            mlss.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(1_315_401i64))
        );

        let scnx_raw = br#"{
            "entityName": "Scienture Holdings, Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-31","val":1200000,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-31","val":400000,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-31","val":-5000000,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-31","val":-4000000,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-31","val":10000,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2024-12-31","val":6305477,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2024-12-31","val":308096,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "CashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2024-12-31","val":308096,"form":"10-K","filed":"2026-03-30"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2024-12-31","val":5000000,"form":"10-K","filed":"2025-03-26"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2024-01-01","end":"2024-12-31","val":10000000,"form":"10-K","filed":"2025-03-26"}
                ]}}
            }}
        }"#;
        let scnx = parse_companyfacts_bytes(scnx_raw, "SCNX", "0001382574", "https://sec.test", 1)
            .unwrap();
        assert_eq!(scnx.annuals[0].cash_and_st, Some(Decimal::from(308_096i64)));

        // Raw XBRL scientific tokens route through exact parse_raw_json_decimal_token cleanly.
        let tiny_raw = serde_json::value::RawValue::from_string(
            "0.00000000000000000000000000001e1".to_owned(),
        )
        .unwrap();
        assert_eq!(parse_raw_decimal(&tiny_raw), Some(Decimal::new(1, 28)));
        let underflow_raw = serde_json::value::RawValue::from_string("1e-29".to_owned()).unwrap();
        assert_eq!(parse_raw_decimal(&underflow_raw), None);

        // 9. Round 12 cash fixes:
        //    - GEV Q3'26 combined+restricted cash without RestrictedCash breakdown emits None/absent
        //      (while FY25 with RestrictedCash subtracts cleanly).
        //    - MBLY FY25 does not double-count MoneyMarketFunds/AFS overlap past AssetsCurrent.
        //    - IPAR FY21 avoids 1000x comparative scale outlier > AssetsCurrent (and emits None when no valid candidate exists).
        let gev_raw = br#"{
            "entityName": "GE Vernova Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":34000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":25000000000,"form":"10-Q","filed":"2025-10-22"},
                    {"start":"2026-01-01","end":"2026-09-30","val":28000000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":6000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":4200000000,"form":"10-Q","filed":"2025-10-22"},
                    {"start":"2026-01-01","end":"2026-09-30","val":5100000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":2000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1300000000,"form":"10-Q","filed":"2025-10-22"},
                    {"start":"2026-01-01","end":"2026-09-30","val":1700000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":2500000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1800000000,"form":"10-Q","filed":"2025-10-22"},
                    {"start":"2026-01-01","end":"2026-09-30","val":2300000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":600000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":400000000,"form":"10-Q","filed":"2025-10-22"},
                    {"start":"2026-01-01","end":"2026-09-30","val":500000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":40216000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":51681000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents": {"units":{"USD":[
                    {"end":"2025-12-31","val":8848000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":16764000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "RestrictedCash": {"units":{"USD":[
                    {"end":"2025-12-31","val":379000000,"form":"10-K","filed":"2026-01-29"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":10000000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":11500000000,"form":"10-Q","filed":"2026-10-28"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":275000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2026-01-01","end":"2026-09-30","val":272000000,"form":"10-Q","filed":"2026-10-28"}
                ]}}
            }}
        }"#;
        let gev =
            parse_companyfacts_bytes(gev_raw, "GEV", "0001996810", "https://sec.test", 1).unwrap();
        assert_eq!(
            gev.annuals[0].cash_and_st,
            Some(Decimal::from(8_469_000_000i64))
        );
        let gev_ttm = gev.ttm.as_ref().unwrap();
        assert_eq!(gev_ttm.cash_and_st, None);
        assert!(
            gev_ttm
                .provenance
                .iter()
                .any(|p| p.field == "ttm_cash_and_st" && p.source_tag == "absent")
        );

        let mbly_raw = br#"{
            "entityName": "Mobileye Global Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-28","val":1654000000,"form":"10-K","filed":"2025-02-20"},
                    {"start":"2025-01-01","end":"2025-12-27","val":1800000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-28","val":810000000,"form":"10-K","filed":"2025-02-20"},
                    {"start":"2025-01-01","end":"2025-12-27","val":900000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-28","val":110000000,"form":"10-K","filed":"2025-02-20"},
                    {"start":"2025-01-01","end":"2025-12-27","val":150000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-28","val":400000000,"form":"10-K","filed":"2025-02-20"},
                    {"start":"2025-01-01","end":"2025-12-27","val":602000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2024-01-01","end":"2024-12-28","val":80000000,"form":"10-K","filed":"2025-02-20"},
                    {"start":"2025-01-01","end":"2025-12-27","val":106000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2024-12-28","val":2174000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":2478000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":2478000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2024-12-28","val":140000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":131000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":131000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "InventoryNet": {"units":{"USD":[
                    {"end":"2024-12-28","val":487000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":327000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":327000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "OtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2024-12-28","val":121000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":184000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":129000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2024-12-28","val":56000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":35000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":35000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "DepositsAssetsCurrent": {"units":{"USD":[
                    {"end":"2024-12-28","val":419000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":785000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2024-12-28","val":1426000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":1836000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":1836000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "MoneyMarketFundsAtCarryingValue": {"units":{"USD":[
                    {"end":"2024-12-28","val":951000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":1016000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":1016000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "AvailableForSaleSecuritiesDebtSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2024-12-28","val":984000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":1071000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalents": {"units":{"USD":[
                    {"end":"2024-12-28","val":1445000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":1860000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":1860000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "RestrictedCash": {"units":{"USD":[
                    {"end":"2024-12-28","val":19000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":24000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2025-12-27","val":24000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2024-12-28","val":11600000000,"form":"10-K","filed":"2025-02-20"},
                    {"end":"2025-12-27","val":11881000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2024-01-01","end":"2024-12-28","val":810000000,"form":"10-K","filed":"2025-02-20"},
                    {"start":"2025-01-01","end":"2025-12-27","val":815000000,"form":"10-K","filed":"2026-02-12"}
                ]}}
            }}
        }"#;
        let mbly = parse_companyfacts_bytes(mbly_raw, "MBLY", "0001910139", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            mbly.annuals[0].cash_and_st,
            Some(Decimal::from(1_459_000_000i64))
        );
        assert_eq!(
            mbly.annuals[1].cash_and_st,
            Some(Decimal::from(1_891_000_000i64))
        );
        assert!(mbly.annuals[1].provenance.iter().any(|p| {
            p.field == "cash_and_st"
                && p.source_tag.contains("derived(")
                && p.source_tag.contains("MoneyMarketFundsAtCarryingValue")
        }));

        // Unverified partial overlap exceeding AssetsCurrent (missing DepositsAssetsCurrent composition proof) must emit None/absent.
        let unverified_split_raw = br#"{
            "entityName": "Unverified Split Corp",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":1800000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":900000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":150000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":602000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":106000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":2478000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "OtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":184000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2025-12-27","val":35000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-27","val":1836000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "MoneyMarketFundsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-27","val":1016000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "AvailableForSaleSecuritiesDebtSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":1071000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-27","val":11881000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-27","val":815000000,"form":"10-K","filed":"2026-02-12"}
                ]}}
            }}
        }"#;
        let unverified = parse_companyfacts_bytes(
            unverified_split_raw,
            "UVSP",
            "0001910140",
            "https://sec.test",
            1,
        )
        .unwrap();
        assert_eq!(unverified.annuals[0].cash_and_st, None);
        assert!(
            unverified.annuals[0]
                .provenance
                .iter()
                .any(|p| p.field == "cash_and_st" && p.source_tag == "absent")
        );

        let ipar_raw = br#"{
            "entityName": "Interparfums, Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":879516000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":556000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":147000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":112000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":4000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2021-12-31","val":709715000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2021-12-31","val":159613000,"form":"10-K","filed":"2023-02-28"},
                    {"end":"2021-12-31","val":159613000000,"form":"10-K","filed":"2024-02-27"}
                ]}},
                "ShortTermInvestments": {"units":{"USD":[
                    {"end":"2021-12-31","val":160014000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "CashCashEquivalentsAndShortTermInvestments": {"units":{"USD":[
                    {"end":"2021-12-31","val":159613000,"form":"10-K","filed":"2022-03-01"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2021-12-31","val":600000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2021-01-01","end":"2021-12-31","val":32000000,"form":"10-K","filed":"2023-02-28"}
                ]}}
            }}
        }"#;
        let ipar = parse_companyfacts_bytes(ipar_raw, "IPAR", "0000822663", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            ipar.annuals[0].cash_and_st,
            Some(Decimal::from(319_627_000i64))
        );

        let over_ac_raw = br#"{
            "entityName": "Over Current Assets Corp",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":879516000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":556000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":147000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":112000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2021-01-01","end":"2021-12-31","val":4000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2021-12-31","val":709715000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2021-12-31","val":159613000000,"form":"10-K","filed":"2024-02-27"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2021-12-31","val":600000000,"form":"10-K","filed":"2023-02-28"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2021-01-01","end":"2021-12-31","val":32000000,"form":"10-K","filed":"2023-02-28"}
                ]}}
            }}
        }"#;
        let over_ac =
            parse_companyfacts_bytes(over_ac_raw, "OAC", "0000822664", "https://sec.test", 1)
                .unwrap();
        assert_eq!(over_ac.annuals[0].cash_and_st, None);
        assert!(
            over_ac.annuals[0]
                .provenance
                .iter()
                .any(|p| p.field == "cash_and_st" && p.source_tag == "absent")
        );

        // 10. Round 13 cash fixes:
        //    - AMZN Q3'26 ($114.226B) & TSLA Q3'26 ($45.447B): overlapping RestrictedCashCurrent /
        //      PrepaidExpenseAndOtherAssetsCurrent must not block explicit ST investments or distort net debt/ROIC.
        //    - IPAR FY25 ($295.184M) & Q3'26 ($240.246M): CashEquivalentsAtCarryingValue == combined_rc
        //      reconciles zero restricted cash so ShortTermInvestments adds cleanly.
        let make_test_company = |ticker: &str, name: &str, cik: &str| crate::model::CompanyEntry {
            ticker: ticker.to_owned(),
            name: name.to_owned(),
            bucket: crate::model::Bucket::Watch,
            source: crate::model::DataSource::SecEdgar,
            cik: Some(cik.to_owned()),
            currency: "USD".to_owned(),
            circle: "Test".to_owned(),
            too_hard_reason: None,
            assumptions: crate::model::ValuationAssumptions {
                multiple_low: Decimal::from(15),
                multiple_high: Decimal::from(20),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        };

        let amzn_raw = br#"{
            "entityName": "Amazon.com, Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":638000000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2025-01-01","end":"2025-09-30","val":450000000000,"form":"10-Q","filed":"2025-10-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":505000000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":312000000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2025-01-01","end":"2025-09-30","val":220000000000,"form":"10-Q","filed":"2025-10-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":250000000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":68593000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2025-01-01","end":"2025-09-30","val":50000000000,"form":"10-Q","filed":"2025-10-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":58407000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":68500000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2025-01-01","end":"2025-09-30","val":49500000000,"form":"10-Q","filed":"2025-10-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":58000000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "IncomeTaxExpenseBenefit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":13700000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2025-01-01","end":"2025-09-30","val":9900000000,"form":"10-Q","filed":"2025-10-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":11600000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":115000000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2025-01-01","end":"2025-09-30","val":82000000000,"form":"10-Q","filed":"2025-10-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":96000000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":83000000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2025-01-01","end":"2025-09-30","val":58000000000,"form":"10-Q","filed":"2025-10-31"},
                    {"start":"2026-01-01","end":"2026-09-30","val":70000000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":190000000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":255100000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":78779000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":55783000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "MarketableSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":22423000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":58443000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":55451000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":98242000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "InventoryNet": {"units":{"USD":[
                    {"end":"2025-12-31","val":34214000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":42632000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "RestrictedCashCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":363000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "PrepaidExpenseAndOtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":6000000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":7500000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "LongTermDebtNoncurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":52623000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":60000000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":240000000000,"form":"10-K","filed":"2026-02-06"},
                    {"end":"2026-09-30","val":260000000000,"form":"10-Q","filed":"2026-10-30"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":10700000000,"form":"10-K","filed":"2026-02-06"},
                    {"start":"2026-01-01","end":"2026-09-30","val":10800000000,"form":"10-Q","filed":"2026-10-30"}
                ]}}
            }}
        }"#;
        let amzn = parse_companyfacts_bytes(amzn_raw, "AMZN", "0001018724", "https://sec.test", 1)
            .unwrap();
        let amzn_ttm = amzn.ttm.as_ref().unwrap();
        assert_eq!(
            amzn_ttm.cash_and_st,
            Some(Decimal::from(114_226_000_000i64))
        );
        let amzn_rep = crate::calculator::build_report(
            &make_test_company("AMZN", "Amazon.com, Inc.", "0001018724"),
            Some(&amzn),
            None,
            0,
        );
        let amzn_ttm_row = amzn_rep.ttm_row.as_ref().unwrap();
        assert_eq!(
            amzn_ttm_row.cash_and_st,
            Some(Decimal::from(114_226_000_000i64))
        );
        assert_eq!(
            amzn_ttm_row.net_debt,
            Some(Decimal::from(-54_226_000_000i64))
        );
        assert_eq!(
            amzn_ttm_row.invested_capital,
            Some(Decimal::from(205_774_000_000i64))
        );
        assert_eq!(
            amzn_ttm_row.roic_pct,
            Some(
                (Decimal::from(61_600_000_000i64) / Decimal::from(205_774_000_000i64))
                    * Decimal::from(100)
            )
        );

        let tsla_raw = br#"{
            "entityName": "Tesla, Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":97690000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":71980000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":75000000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":17450000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":12800000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":13500000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":7000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":5000000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":5500000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "IncomeLossFromContinuingOperationsBeforeIncomeTaxesExtraordinaryItemsNoncontrollingInterest": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":7000000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":5000000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":5500000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "IncomeTaxExpenseBenefit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":1400000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1000000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":1100000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":14900000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":10100000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":11200000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":11300000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2025-01-01","end":"2025-09-30","val":8500000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2026-01-01","end":"2026-09-30","val":8000000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":58360000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":70402000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":16139000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":17853000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "ShortTermInvestments": {"units":{"USD":[
                    {"end":"2025-12-31","val":20424000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":27594000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":4418000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":4636000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "InventoryNet": {"units":{"USD":[
                    {"end":"2025-12-31","val":12017000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":14092000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "PrepaidExpenseAndOtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":5362000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":6227000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "RestrictedCashCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":390000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":489000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "LongTermDebtNoncurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":5757000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":8000000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":72913000000,"form":"10-K","filed":"2026-01-29"},
                    {"end":"2026-09-30","val":75000000000,"form":"10-Q","filed":"2026-10-22"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":3498000000,"form":"10-K","filed":"2026-01-29"},
                    {"start":"2026-01-01","end":"2026-09-30","val":3520000000,"form":"10-Q","filed":"2026-10-22"}
                ]}}
            }}
        }"#;
        let tsla = parse_companyfacts_bytes(tsla_raw, "TSLA", "0001318605", "https://sec.test", 1)
            .unwrap();
        assert_eq!(
            tsla.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(45_447_000_000i64))
        );
        let tsla_rep = crate::calculator::build_report(
            &make_test_company("TSLA", "Tesla, Inc.", "0001318605"),
            Some(&tsla),
            None,
            0,
        );
        let tsla_ttm_row = tsla_rep.ttm_row.as_ref().unwrap();
        assert_eq!(
            tsla_ttm_row.cash_and_st,
            Some(Decimal::from(45_447_000_000i64))
        );
        assert_eq!(
            tsla_ttm_row.net_debt,
            Some(Decimal::from(-37_447_000_000i64))
        );
        assert_eq!(
            tsla_ttm_row.invested_capital,
            Some(Decimal::from(37_553_000_000i64))
        );
        assert_eq!(
            tsla_ttm_row.roic_pct,
            Some(
                (Decimal::from(6_000_000_000i64) / Decimal::from(37_553_000_000i64))
                    * Decimal::from(100)
            )
        );

        let ipar_q3_raw = br#"{
            "entityName": "Interparfums, Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":1490000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":1100000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":1150000000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":950000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":700000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":735000000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":280000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":210000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":225000000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":210000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":130000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":145000000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":8000000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2025-01-01","end":"2025-09-30","val":5000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":6000000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":980000000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":1044277000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "CashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":158091000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":131912000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "CashCashEquivalentsRestrictedCashAndRestrictedCashEquivalentsIncludingDisposalGroupAndDiscontinuedOperations": {"units":{"USD":[
                    {"end":"2025-12-31","val":158091000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":131912000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "ShortTermInvestments": {"units":{"USD":[
                    {"end":"2025-12-31","val":137093000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":108334000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":790000000,"form":"10-K","filed":"2026-02-26"},
                    {"end":"2026-09-30","val":840000000,"form":"10-Q","filed":"2026-11-04"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":32100000,"form":"10-K","filed":"2026-02-26"},
                    {"start":"2026-01-01","end":"2026-09-30","val":32200000,"form":"10-Q","filed":"2026-11-04"}
                ]}}
            }}
        }"#;
        let ipar_q3 =
            parse_companyfacts_bytes(ipar_q3_raw, "IPAR", "0000822663", "https://sec.test", 1)
                .unwrap();
        assert_eq!(
            ipar_q3.annuals[0].cash_and_st,
            Some(Decimal::from(295_184_000i64))
        );
        assert_eq!(
            ipar_q3.ttm.as_ref().unwrap().cash_and_st,
            Some(Decimal::from(240_246_000i64))
        );
    }

    #[test]
    fn round14_st_investment_aggregate_reconciliation_and_mbly_incomplete_cash() {
        // 1. FROG Sep 2026: ShortTermInvestments ($805.701M) must beat partial MarketableSecuritiesCurrent
        //    ($604.753M) so cash_and_st == $75.187M + $805.701M = $880.888M with selected provenance.
        let frog_raw = br#"{
            "entityName": "JFrog Ltd",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":360000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":430000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":400000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":290000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":345000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":40000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":25000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":35000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":110000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":75000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":95000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":5000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2025-01-01","end":"2025-09-30","val":4000000,"form":"10-Q","filed":"2025-11-05"},
                    {"start":"2026-01-01","end":"2026-09-30","val":4500000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":820000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":1078618000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":60000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":75187000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "MarketableSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":604753000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "ShortTermInvestments": {"units":{"USD":[
                    {"end":"2025-12-31","val":620000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":805701000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":110000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":136951000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "PrepaidExpenseAndOtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":30000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":30181000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":850000000,"form":"10-K","filed":"2026-02-20"},
                    {"end":"2026-09-30","val":950000000,"form":"10-Q","filed":"2026-11-06"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":115000000,"form":"10-K","filed":"2026-02-20"},
                    {"start":"2026-01-01","end":"2026-09-30","val":118000000,"form":"10-Q","filed":"2026-11-06"}
                ]}}
            }}
        }"#;
        let frog = parse_companyfacts_bytes(frog_raw, "FROG", "0001800667", "https://sec.test", 1)
            .unwrap();
        let frog_ttm = frog.ttm.as_ref().unwrap();
        assert_eq!(frog_ttm.cash_and_st, Some(Decimal::from(880_888_000i64)));
        assert!(frog_ttm.provenance.iter().any(|p| {
            p.field == "ttm_cash_and_st"
                && p.source_tag == "CashAndCashEquivalentsAtCarryingValue + ShortTermInvestments"
        }));

        // 2. LZB Oct 2026: ShortTermInvestments ($4.275M) must beat partial MarketableSecuritiesCurrent ($30k)
        //    so cash_and_st == $246.696M + $4.275M = $250.971M with ShortTermInvestments provenance.
        let lzb_raw = br#"{
            "entityName": "LA-Z-BOY INC",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-04-27","end":"2026-04-25","val":2100000000,"form":"10-K","filed":"2026-06-16"},
                    {"start":"2025-04-27","end":"2025-10-25","val":1020000000,"form":"10-Q","filed":"2025-11-18"},
                    {"start":"2026-04-26","end":"2026-10-24","val":1060000000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-04-27","end":"2026-04-25","val":900000000,"form":"10-K","filed":"2026-06-16"},
                    {"start":"2025-04-27","end":"2025-10-25","val":435000000,"form":"10-Q","filed":"2025-11-18"},
                    {"start":"2026-04-26","end":"2026-10-24","val":455000000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-04-27","end":"2026-04-25","val":160000000,"form":"10-K","filed":"2026-06-16"},
                    {"start":"2025-04-27","end":"2025-10-25","val":75000000,"form":"10-Q","filed":"2025-11-18"},
                    {"start":"2026-04-26","end":"2026-10-24","val":82000000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-04-27","end":"2026-04-25","val":180000000,"form":"10-K","filed":"2026-06-16"},
                    {"start":"2025-04-27","end":"2025-10-25","val":80000000,"form":"10-Q","filed":"2025-11-18"},
                    {"start":"2026-04-26","end":"2026-10-24","val":90000000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-04-27","end":"2026-04-25","val":60000000,"form":"10-K","filed":"2026-06-16"},
                    {"start":"2025-04-27","end":"2025-10-25","val":30000000,"form":"10-Q","filed":"2025-11-18"},
                    {"start":"2026-04-26","end":"2026-10-24","val":32000000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2026-04-25","val":700000000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":717473000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2026-04-25","val":240000000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":246696000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "MarketableSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2026-04-25","val":30000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":30000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "ShortTermInvestments": {"units":{"USD":[
                    {"end":"2026-04-25","val":4275000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":4275000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "ReceivablesNetCurrent": {"units":{"USD":[
                    {"end":"2026-04-25","val":130000000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":132309000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "InventoryNet": {"units":{"USD":[
                    {"end":"2026-04-25","val":225000000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":229186000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "OtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2026-04-25","val":105000000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":109282000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2026-04-25","val":1000000000,"form":"10-K","filed":"2026-06-16"},
                    {"end":"2026-10-24","val":1050000000,"form":"10-Q","filed":"2026-11-17"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-04-27","end":"2026-04-25","val":41000000,"form":"10-K","filed":"2026-06-16"},
                    {"start":"2026-04-26","end":"2026-10-24","val":40500000,"form":"10-Q","filed":"2026-11-17"}
                ]}}
            }}
        }"#;
        let lzb =
            parse_companyfacts_bytes(lzb_raw, "LZB", "0000057131", "https://sec.test", 1).unwrap();
        let lzb_ttm = lzb.ttm.as_ref().unwrap();
        assert_eq!(lzb_ttm.cash_and_st, Some(Decimal::from(250_971_000i64)));
        assert!(lzb_ttm.provenance.iter().any(|p| {
            p.field == "ttm_cash_and_st"
                && p.source_tag == "CashAndCashEquivalentsAtCarryingValue + ShortTermInvestments"
        }));

        // 3. Disjoint component sum when ShortTermInvestments is absent and AssetsCurrent reconciles.
        let disjoint_raw = br#"{
            "entityName": "Disjoint Components Corp",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":500000000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":300000000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":80000000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":90000000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-31","val":10000000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":1078618000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-31","val":75187000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "MarketableSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":604753000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "OtherShortTermInvestments": {"units":{"USD":[
                    {"end":"2025-12-31","val":200948000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2025-12-31","val":197730000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-31","val":900000000,"form":"10-K","filed":"2026-02-20"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-31","val":100000000,"form":"10-K","filed":"2026-02-20"}
                ]}}
            }}
        }"#;
        let disjoint =
            parse_companyfacts_bytes(disjoint_raw, "DJNT", "0000000901", "https://sec.test", 1)
                .unwrap();
        assert_eq!(
            disjoint.annuals[0].cash_and_st,
            Some(Decimal::from(880_888_000i64))
        );
        assert!(disjoint.annuals[0].provenance.iter().any(|p| {
            p.field == "cash_and_st"
                && p.source_tag
                    == "CashAndCashEquivalentsAtCarryingValue + MarketableSecuritiesCurrent + OtherShortTermInvestments"
        }));

        // 4. MBLY Q3'26: omitted current marketable securities ($153M gap against AssetsCurrent = $2.106B)
        //    with missing standard ST tag + active AFS disclosures/investment flows must mark Q3'26 cash_and_st
        //    absent/None instead of falsely publishing $1.346B alone, while preserving FY25 ($1.891B).
        let mbly_q3_raw = br#"{
            "entityName": "Mobileye Global Inc.",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":1800000000,"form":"10-K","filed":"2026-02-12"},
                    {"start":"2024-12-29","end":"2025-09-27","val":1400000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2025-12-28","end":"2026-09-26","val":1300000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":900000000,"form":"10-K","filed":"2026-02-12"},
                    {"start":"2024-12-29","end":"2025-09-27","val":700000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2025-12-28","end":"2026-09-26","val":660000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":150000000,"form":"10-K","filed":"2026-02-12"},
                    {"start":"2024-12-29","end":"2025-09-27","val":110000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2025-12-28","end":"2026-09-26","val":95000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":602000000,"form":"10-K","filed":"2026-02-12"},
                    {"start":"2024-12-29","end":"2025-09-27","val":420000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2025-12-28","end":"2026-09-26","val":303000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-01-01","end":"2025-12-27","val":106000000,"form":"10-K","filed":"2026-02-12"},
                    {"start":"2024-12-29","end":"2025-09-27","val":70000000,"form":"10-Q","filed":"2025-10-23"},
                    {"start":"2025-12-28","end":"2026-09-26","val":65000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "PaymentsToAcquireInvestments": {"units":{"USD":[
                    {"start":"2025-12-28","end":"2026-09-26","val":206000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":2478000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":2106000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":131000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":202000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "InventoryNet": {"units":{"USD":[
                    {"end":"2025-12-27","val":327000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":295000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "OtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":184000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":110000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "Cash": {"units":{"USD":[
                    {"end":"2025-12-27","val":35000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":50000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "DepositsAssetsCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":785000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":736000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-27","val":1836000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":1346000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "MoneyMarketFundsAtCarryingValue": {"units":{"USD":[
                    {"end":"2025-12-27","val":1016000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "AvailableForSaleSecuritiesDebtSecuritiesCurrent": {"units":{"USD":[
                    {"end":"2025-12-27","val":1071000000,"form":"10-K","filed":"2026-02-12"}
                ]}},
                "AvailableForSaleDebtSecuritiesAccumulatedGrossUnrealizedGainBeforeTax": {"units":{"USD":[
                    {"end":"2026-09-26","val":0,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2025-12-27","val":11881000000,"form":"10-K","filed":"2026-02-12"},
                    {"end":"2026-09-26","val":11500000000,"form":"10-Q","filed":"2026-10-29"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-01-01","end":"2025-12-27","val":815000000,"form":"10-K","filed":"2026-02-12"},
                    {"start":"2025-12-28","end":"2026-09-26","val":818000000,"form":"10-Q","filed":"2026-10-29"}
                ]}}
            }}
        }"#;
        let mbly_q3 =
            parse_companyfacts_bytes(mbly_q3_raw, "MBLY", "0001910139", "https://sec.test", 1)
                .unwrap();
        assert_eq!(
            mbly_q3.annuals[0].cash_and_st,
            Some(Decimal::from(1_891_000_000i64))
        );
        let mbly_ttm = mbly_q3.ttm.as_ref().unwrap();
        assert_eq!(mbly_ttm.cash_and_st, None);
        assert!(mbly_ttm.provenance.iter().any(|p| {
            p.field == "ttm_cash_and_st"
                && p.source_tag == "absent"
                && p.note.as_deref()
                    == Some("missing_current_st_investments_unreconciled_current_assets")
        }));

        // 5. Verified cash-only balance sheet (reconciles AssetsCurrent cleanly) must keep cash_and_st.
        let cash_only_control_raw = br#"{
            "entityName": "Balanced Cash Only Corp",
            "facts": { "us-gaap": {
                "Revenues": {"units":{"USD":[
                    {"start":"2025-10-01","end":"2026-09-30","val":5000000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "GrossProfit": {"units":{"USD":[
                    {"start":"2025-10-01","end":"2026-09-30","val":2800000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "OperatingIncomeLoss": {"units":{"USD":[
                    {"start":"2025-10-01","end":"2026-09-30","val":2000000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "NetCashProvidedByUsedInOperatingActivities": {"units":{"USD":[
                    {"start":"2025-10-01","end":"2026-09-30","val":2187000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "PaymentsToAcquirePropertyPlantAndEquipment": {"units":{"USD":[
                    {"start":"2025-10-01","end":"2026-09-30","val":100000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "AssetsCurrent": {"units":{"USD":[
                    {"end":"2026-09-30","val":7652000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "CashAndCashEquivalentsAtCarryingValue": {"units":{"USD":[
                    {"end":"2026-09-30","val":2279000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "AccountsReceivableNetCurrent": {"units":{"USD":[
                    {"end":"2026-09-30","val":2054000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "InventoryNet": {"units":{"USD":[
                    {"end":"2026-09-30","val":2713000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "PrepaidExpenseAndOtherAssetsCurrent": {"units":{"USD":[
                    {"end":"2026-09-30","val":606000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "StockholdersEquity": {"units":{"USD":[
                    {"end":"2026-09-30","val":3000000000,"form":"10-K","filed":"2026-11-10"}
                ]}},
                "WeightedAverageNumberOfDilutedSharesOutstanding": {"units":{"shares":[
                    {"start":"2025-10-01","end":"2026-09-30","val":56000000,"form":"10-K","filed":"2026-11-10"}
                ]}}
            }}
        }"#;
        let cash_ctrl = parse_companyfacts_bytes(
            cash_only_control_raw,
            "TDGX",
            "0000000902",
            "https://sec.test",
            1,
        )
        .unwrap();
        assert_eq!(
            cash_ctrl.annuals[0].cash_and_st,
            Some(Decimal::from(2_279_000_000i64))
        );

        let v1_norm = br#"{"0":{"cik_str":1166691,"ticker":"CMCSA","title":"COMCAST CORP"},"1":{"cik_str":1166691,"ticker":"CCZ","title":"COMCAST CORP"}}"#;
        let v1_rev = br#"{"0":{"cik_str":1166691,"ticker":"CCZ","title":"COMCAST CORP"},"1":{"cik_str":1166691,"ticker":"CMCSA","title":"COMCAST CORP"}}"#;
        assert_eq!(
            canonical_ticker_for_cik_from_v1_bytes(v1_norm, "0001166691").as_deref(),
            Some("CMCSA")
        );
        assert_eq!(
            canonical_ticker_for_cik_from_v1_bytes(v1_rev, "0001166691").as_deref(),
            Some("CMCSA")
        );
    }
}
