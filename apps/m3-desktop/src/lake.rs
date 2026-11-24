use crate::{
    calculator,
    model::{
        AnnualMetricRow, Bucket, FieldProvenance, ValuationAssumptions, WarningFlag, serde_decimal,
        serde_decimal_opt,
    },
    store::{self, WorkspacePaths},
};
use anyhow::{Context, Result, ensure};
use duckdb::{
    AccessMode, Config, Connection, OptionalExt, ToSql, params, params_from_iter, types::ValueRef,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncWatermark {
    pub source_key: String,
    pub cursor: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub synced_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniverseSortBy {
    #[default]
    QualityScore,
    Roic5yAvg,
    StrictFcfYield,
    DistanceToStrike,
    ShareChange3y,
    SbcToOcf,
    LatestPrice,
    StrikePrice,
    WarningsCount,
    Ticker,
    Name,
    Sic,
    StrictOwnerEarnings,
}

impl FromStr for UniverseSortBy {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "quality" | "quality_score" => Ok(Self::QualityScore),
            "roic" | "roic_5y" | "roic_5y_avg" | "roic_5y_avg_pct" => Ok(Self::Roic5yAvg),
            "fcf" | "fcf_yield" | "strict_fcf_yield" | "strict_fcf_yield_pct" => {
                Ok(Self::StrictFcfYield)
            }
            "distance" | "distance_to_strike" | "distance_to_strike_pct" => {
                Ok(Self::DistanceToStrike)
            }
            "share_change" | "share_change_3y" | "share_change_3y_pct" => Ok(Self::ShareChange3y),
            "sbc" | "sbc_ocf" | "sbc_to_ocf" | "sbc_to_ocf_pct" => Ok(Self::SbcToOcf),
            "price" | "latest_price" => Ok(Self::LatestPrice),
            "strike" | "strike_price" => Ok(Self::StrikePrice),
            "warnings" | "warnings_count" => Ok(Self::WarningsCount),
            "ticker" => Ok(Self::Ticker),
            "name" => Ok(Self::Name),
            "sic" => Ok(Self::Sic),
            "oe" | "strict_oe" | "strict_owner_earnings" => Ok(Self::StrictOwnerEarnings),
            other => anyhow::bail!(
                "unknown sort column '{other}'; expected quality|roic|fcf_yield|distance|share_change|sbc_ocf|price|strike|warnings|ticker|name|sic|oe"
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseFilter {
    #[serde(default, with = "serde_decimal_opt")]
    pub min_roic_5y: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub max_sbc_to_ocf: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub max_share_change_3y: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub max_net_debt_to_oe: Option<Decimal>,
    #[serde(default)]
    pub min_years_of_data: Option<usize>,
    pub exclude_financials: bool,
    pub positive_strict_oe_only: bool,
    pub in_price_pool_only: bool,
    pub strike_zone_only: bool,
    #[serde(default)]
    pub query: Option<String>,
    pub sort_by: UniverseSortBy,
    pub sort_asc: bool,
    pub limit: usize,
}

impl Default for UniverseFilter {
    fn default() -> Self {
        Self {
            min_roic_5y: None,
            max_sbc_to_ocf: None,
            max_share_change_3y: None,
            max_net_debt_to_oe: None,
            min_years_of_data: None,
            exclude_financials: true,
            positive_strict_oe_only: false,
            in_price_pool_only: false,
            strike_zone_only: false,
            query: None,
            sort_by: UniverseSortBy::QualityScore,
            sort_asc: false,
            limit: 200,
        }
    }
}

pub(crate) type AssumptionsStateMap = HashMap<String, (String, u64)>;

pub(crate) struct PriceSyncCommit<'a> {
    pub(crate) prices: &'a [LakePriceDailyRow],
    pub(crate) updated_rows: &'a [QuoteSnapshotUpdate],
    pub(crate) expected_assumptions_digests: &'a HashMap<String, String>,
    pub(crate) expected_assumptions_revs: &'a HashMap<String, u64>,
    pub(crate) watermarks: &'a [SyncWatermark],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseRow {
    pub ticker: String,
    pub cik: String,
    pub name: String,
    pub exchange: String,
    pub sic: Option<u32>,
    pub sic_description: Option<String>,
    pub is_financial: bool,
    pub latest_filing_end: Option<String>,
    pub years_of_data: usize,
    #[serde(with = "serde_decimal")]
    pub quality_score: Decimal,
    pub in_price_pool: bool,
    pub in_watchlist: bool,
    pub bucket_opt: Option<Bucket>,
    #[serde(default, with = "serde_decimal_opt")]
    pub latest_price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strike_price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub distance_to_strike_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_fcf_yield_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_5y_avg_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_5y_stddev_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub gross_margin_5y_avg_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub gross_margin_5y_stddev_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub share_change_3y_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub sbc_to_ocf_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub net_debt_to_oe: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_owner_earnings: Option<Decimal>,
    pub warnings_count: usize,
    pub warnings_json: String,
    pub raw_sha256: Option<String>,
}

impl UniverseRow {
    pub fn parsed_warnings(&self) -> Vec<WarningFlag> {
        if self.warnings_json.trim().is_empty() {
            return Vec::new();
        }
        serde_json::from_str(&self.warnings_json).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LakeAnnualRow {
    pub cik: String,
    pub ticker: String,
    pub fiscal_year: i32,
    pub label: String,
    pub period_end: String,
    pub filed: Option<String>,
    #[serde(with = "serde_decimal")]
    pub revenue: Decimal,
    #[serde(with = "serde_decimal")]
    pub gross_profit: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub gross_margin_pct: Option<Decimal>,
    #[serde(with = "serde_decimal")]
    pub ebit: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub nopat: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub invested_capital: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_pct: Option<Decimal>,
    pub roic_note: Option<String>,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_total_capital_pct: Option<Decimal>,
    #[serde(with = "serde_decimal")]
    pub ocf: Decimal,
    #[serde(with = "serde_decimal")]
    pub capex: Decimal,
    #[serde(with = "serde_decimal")]
    pub fcf: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub sbc: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_owner_earnings: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub sbc_to_ocf_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub buybacks: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub cash_and_st: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub total_debt: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub net_debt: Option<Decimal>,
    #[serde(with = "serde_decimal")]
    pub equity: Decimal,
    #[serde(with = "serde_decimal")]
    pub diluted_shares: Decimal,
    #[serde(with = "serde_decimal")]
    pub ending_shares: Decimal,
    pub raw_sha256: String,
    pub provenance_json: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UniverseSummary {
    pub total_companies: usize,
    pub parsed_financial_companies: usize,
    pub non_financial_companies: usize,
    pub quality_pool_count: usize,
    pub priced_count: usize,
    pub strike_zone_count: usize,
    pub last_sec_sync_ms: Option<u64>,
    pub last_price_sync_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerComparison {
    pub target: UniverseRow,
    pub sic_group: String,
    pub peer_count: usize,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_percentile: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub gross_margin_percentile: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_fcf_yield_percentile: Option<Decimal>,
    pub top_peers: Vec<UniverseRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseDrawerDetails {
    pub row: UniverseRow,
    pub annuals: Vec<LakeAnnualRow>,
    pub peers: Option<PeerComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseViewSnapshot {
    pub summary: UniverseSummary,
    pub rows: Vec<UniverseRow>,
    pub selected_details: Option<UniverseDrawerDetails>,
    pub strike_zone_ciks: HashSet<String>,
}

pub(crate) struct PrevSecLakeSnapshot {
    pub all_ciks: HashSet<String>,
    pub parsed_ciks: HashSet<String>,
    pub ttm_ciks: HashSet<String>,
    pub classified_sic_ciks: HashSet<String>,
    pub sic_map: HashMap<String, (Option<u32>, Option<String>)>,
    pub parsed_rows: HashMap<String, UniverseRow>,
    pub extractor_versions: HashMap<String, u32>,
    pub annual_period_ends: HashMap<String, Vec<String>>,
    pub ttm_period_ends: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct LakeCompanyRow {
    pub cik: String,
    pub ticker: String,
    pub name: String,
    pub exchange: String,
    pub sic: Option<u32>,
    pub sic_description: Option<String>,
    pub is_financial: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct LakeTtmRow {
    pub cik: String,
    pub ticker: String,
    pub label: String,
    pub period_end: String,
    pub revenue: Decimal,
    pub gross_profit: Decimal,
    pub gross_margin_pct: Option<Decimal>,
    pub ebit: Decimal,
    pub nopat: Option<Decimal>,
    pub invested_capital: Option<Decimal>,
    pub roic_pct: Option<Decimal>,
    pub roic_note: Option<String>,
    pub roic_total_capital_pct: Option<Decimal>,
    pub ocf: Decimal,
    pub capex: Decimal,
    pub fcf: Decimal,
    pub sbc: Option<Decimal>,
    pub strict_owner_earnings: Option<Decimal>,
    pub sbc_to_ocf_pct: Option<Decimal>,
    pub buybacks: Option<Decimal>,
    pub cash_and_st: Option<Decimal>,
    pub total_debt: Option<Decimal>,
    pub net_debt: Option<Decimal>,
    pub equity: Decimal,
    pub diluted_shares: Decimal,
    pub ytd_period_label: String,
    pub ytd_start: String,
    pub ytd_end: String,
    pub ytd_revenue: Option<Decimal>,
    pub ytd_ocf: Option<Decimal>,
    pub ytd_capex: Option<Decimal>,
    pub ytd_sbc: Option<Decimal>,
    pub ytd_buybacks: Option<Decimal>,
    pub latest_quarter_label: Option<String>,
    pub latest_quarter_ocf: Option<Decimal>,
    pub latest_quarter_capex: Option<Decimal>,
    pub raw_sha256: String,
    pub provenance_json: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LakePriceDailyRow {
    pub symbol: String,
    pub trade_date: String,
    pub price: Decimal,
    pub previous_close: Option<Decimal>,
    pub currency: String,
    pub exchange: String,
    pub market_time_ms: Option<u64>,
    pub fetched_at_ms: u64,
    pub source_url: String,
}

#[derive(Debug, Clone)]
pub(crate) struct QuoteSnapshotUpdate {
    pub cik: String,
    pub expected_ticker: String,
    pub expected_raw_sha256: Option<String>,
    pub expected_latest_filing_end: Option<String>,
    pub expected_basis_period_end: Option<String>,
    pub quote_market_time_ms: Option<u64>,
    pub quote_fetched_at_ms: u64,
    pub latest_price: Option<Decimal>,
    pub strike_price: Option<Decimal>,
    pub distance_to_strike_pct: Option<Decimal>,
    pub strict_fcf_yield_pct: Option<Decimal>,
}

mod lake_decimal {
    use super::*;

    pub fn serialize<S>(value: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.normalize().to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Box::<serde_json::value::RawValue>::deserialize(deserializer)?;
        crate::model::parse_raw_json_decimal_token(raw.get())
            .map_err(de::Error::custom)?
            .ok_or_else(|| de::Error::custom("expected non-null decimal value"))
    }
}

mod lake_decimal_opt {
    use super::*;

    pub fn serialize<S>(value: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(v) => serializer.serialize_str(&v.normalize().to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw_opt = Option::<Box<serde_json::value::RawValue>>::deserialize(deserializer)?;
        let Some(raw) = raw_opt else {
            return Ok(None);
        };
        crate::model::parse_raw_json_decimal_token(raw.get()).map_err(de::Error::custom)
    }
}

#[derive(Serialize, Deserialize)]
struct UniverseExactDecimals {
    #[serde(with = "lake_decimal")]
    quality_score: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    latest_price: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    strike_price: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    distance_to_strike_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    strict_fcf_yield_pct: Option<Decimal>,
    #[serde(default)]
    quote_market_time_ms: Option<u64>,
    #[serde(default, with = "lake_decimal_opt")]
    roic_5y_avg_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    roic_5y_stddev_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    gross_margin_5y_avg_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    gross_margin_5y_stddev_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    share_change_3y_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    sbc_to_ocf_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    net_debt_to_oe: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    strict_owner_earnings: Option<Decimal>,
}

fn push_hex_bytes(out: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_hex_array<const N: usize>(bytes: &[u8]) -> Option<[u8; N]> {
    if bytes.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (dst, pair) in out.iter_mut().zip(bytes.as_chunks::<2>().0) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        *dst = (hi << 4) | lo;
    }
    Some(out)
}

fn encode_universe_exact(ex: &UniverseExactDecimals) -> String {
    let opts = [
        ex.latest_price,
        ex.strike_price,
        ex.distance_to_strike_pct,
        ex.strict_fcf_yield_pct,
        ex.roic_5y_avg_pct,
        ex.roic_5y_stddev_pct,
        ex.gross_margin_5y_avg_pct,
        ex.gross_margin_5y_stddev_pct,
        ex.share_change_3y_pct,
        ex.sbc_to_ocf_pct,
        ex.net_debt_to_oe,
        ex.strict_owner_earnings,
    ];
    let mut mask: u16 = 0;
    for (idx, opt) in opts.iter().enumerate() {
        if opt.is_some() {
            mask |= 1u16 << idx;
        }
    }
    if ex.quote_market_time_ms.is_some() {
        mask |= 1u16 << 12;
    }
    let cap = 3
        + 4
        + 32
        + ((mask & 0x0FFF).count_ones() as usize) * 32
        + if ex.quote_market_time_ms.is_some() {
            16
        } else {
            0
        };
    let mut out = String::with_capacity(cap);
    out.push_str("u1:");
    push_hex_bytes(&mut out, &mask.to_be_bytes());
    push_hex_bytes(&mut out, &ex.quality_score.serialize());
    for dec in opts.into_iter().flatten() {
        push_hex_bytes(&mut out, &dec.serialize());
    }
    if let Some(mkt) = ex.quote_market_time_ms {
        push_hex_bytes(&mut out, &mkt.to_le_bytes());
    }
    out
}

fn decode_universe_exact_bytes(raw: &[u8]) -> Option<UniverseExactDecimals> {
    let trimmed = raw.trim_ascii();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(body) = trimmed.strip_prefix(b"u1:") {
        if body.len() < 4 + 32 {
            return None;
        }
        let mask = u16::from_be_bytes(decode_hex_array::<2>(&body[..4])?);
        let expected_len = 4
            + 32
            + ((mask & 0x0FFF).count_ones() as usize) * 32
            + if (mask & (1 << 12)) != 0 { 16 } else { 0 };
        if body.len() != expected_len {
            return None;
        }
        let quality_score = Decimal::deserialize(decode_hex_array::<16>(&body[4..36])?);
        let mut pos = 36;
        let mut read_opt_dec = |bit: usize| -> Option<Option<Decimal>> {
            if (mask & (1 << bit)) != 0 {
                let d = Decimal::deserialize(decode_hex_array::<16>(&body[pos..pos + 32])?);
                pos += 32;
                Some(Some(d))
            } else {
                Some(None)
            }
        };
        let latest_price = read_opt_dec(0)?;
        let strike_price = read_opt_dec(1)?;
        let distance_to_strike_pct = read_opt_dec(2)?;
        let strict_fcf_yield_pct = read_opt_dec(3)?;
        let roic_5y_avg_pct = read_opt_dec(4)?;
        let roic_5y_stddev_pct = read_opt_dec(5)?;
        let gross_margin_5y_avg_pct = read_opt_dec(6)?;
        let gross_margin_5y_stddev_pct = read_opt_dec(7)?;
        let share_change_3y_pct = read_opt_dec(8)?;
        let sbc_to_ocf_pct = read_opt_dec(9)?;
        let net_debt_to_oe = read_opt_dec(10)?;
        let strict_owner_earnings = read_opt_dec(11)?;
        let quote_market_time_ms = if (mask & (1 << 12)) != 0 {
            Some(u64::from_le_bytes(decode_hex_array::<8>(
                &body[pos..pos + 16],
            )?))
        } else {
            None
        };
        return Some(UniverseExactDecimals {
            quality_score,
            latest_price,
            strike_price,
            distance_to_strike_pct,
            strict_fcf_yield_pct,
            quote_market_time_ms,
            roic_5y_avg_pct,
            roic_5y_stddev_pct,
            gross_margin_5y_avg_pct,
            gross_margin_5y_stddev_pct,
            share_change_3y_pct,
            sbc_to_ocf_pct,
            net_debt_to_oe,
            strict_owner_earnings,
        });
    }
    if trimmed.starts_with(b"{") {
        return std::str::from_utf8(trimmed)
            .ok()
            .and_then(|s| serde_json::from_str::<UniverseExactDecimals>(s).ok());
    }
    None
}

#[derive(Serialize, Deserialize)]
struct AnnualExactDecimals {
    #[serde(with = "lake_decimal")]
    revenue: Decimal,
    #[serde(with = "lake_decimal")]
    gross_profit: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    gross_margin_pct: Option<Decimal>,
    #[serde(with = "lake_decimal")]
    ebit: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    nopat: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    invested_capital: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    roic_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    roic_total_capital_pct: Option<Decimal>,
    #[serde(with = "lake_decimal")]
    ocf: Decimal,
    #[serde(with = "lake_decimal")]
    capex: Decimal,
    #[serde(with = "lake_decimal")]
    fcf: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    sbc: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    strict_owner_earnings: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    sbc_to_ocf_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    buybacks: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    cash_and_st: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    total_debt: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    net_debt: Option<Decimal>,
    #[serde(with = "lake_decimal")]
    equity: Decimal,
    #[serde(with = "lake_decimal")]
    diluted_shares: Decimal,
    #[serde(with = "lake_decimal")]
    ending_shares: Decimal,
}

#[derive(Serialize, Deserialize)]
struct TtmExactDecimals {
    #[serde(with = "lake_decimal")]
    revenue: Decimal,
    #[serde(with = "lake_decimal")]
    gross_profit: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    gross_margin_pct: Option<Decimal>,
    #[serde(with = "lake_decimal")]
    ebit: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    nopat: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    invested_capital: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    roic_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    roic_total_capital_pct: Option<Decimal>,
    #[serde(with = "lake_decimal")]
    ocf: Decimal,
    #[serde(with = "lake_decimal")]
    capex: Decimal,
    #[serde(with = "lake_decimal")]
    fcf: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    sbc: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    strict_owner_earnings: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    sbc_to_ocf_pct: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    buybacks: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    cash_and_st: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    total_debt: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    net_debt: Option<Decimal>,
    #[serde(with = "lake_decimal")]
    equity: Decimal,
    #[serde(with = "lake_decimal")]
    diluted_shares: Decimal,
    #[serde(default, with = "lake_decimal_opt")]
    ytd_revenue: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    ytd_ocf: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    ytd_capex: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    ytd_sbc: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    ytd_buybacks: Option<Decimal>,
    #[serde(default)]
    latest_quarter_label: Option<String>,
    #[serde(default, with = "lake_decimal_opt")]
    latest_quarter_ocf: Option<Decimal>,
    #[serde(default, with = "lake_decimal_opt")]
    latest_quarter_capex: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParquetFileManifestEntry {
    pub file: String,
    #[serde(default)]
    pub rel_path: String,
    pub rows: usize,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParquetExportManifest {
    pub schema_version: u32,
    pub generation_id: String,
    #[serde(default)]
    pub generation_dir: String,
    pub exported_at_ms: u64,
    pub files: Vec<ParquetFileManifestEntry>,
}

static PARQUET_GEN_SEQ: AtomicU64 = AtomicU64::new(1);

const STRIKE_SIGNAL_MAX_AGE_DAYS: i64 = 7;
const STRIKE_SIGNAL_MAX_FUTURE_SKEW_MS: u64 = 36 * 3_600_000;
pub(crate) const WM_SEC_EXTRACTOR_VERSION: &str = "sec_extractor_version";
pub(crate) const WM_SEC_GENERATION: &str = "sec_lake_generation";
pub(crate) const WM_SEC_INPUT_GEN: &str = "sec_input_generation";
pub(crate) const WM_ASSUMPTIONS_REV: &str = "assumptions_revision";
pub(crate) const SEC_LAKE_EXTRACTOR_VERSION: &str = "14";
pub(crate) const SEC_LAKE_EXTRACTOR_VERSION_NUM: u32 = 14;

pub(crate) struct SecGenerationExpectations<'a> {
    pub expected_prev_sec_gen: Option<Option<(u64, u64)>>,
    pub expected_prev_assumptions_rev: Option<u64>,
    pub batch_synced_at_ms: u64,
    pub post_parse_watchlist: Option<Option<&'a crate::model::WatchlistFile>>,
    pub recompute_prices_top: Option<usize>,
    pub refreshed_disk_quotes: &'a HashMap<String, crate::model::PriceSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SecSnapshotMeta {
    pub quote_market_time_ms: Option<u64>,
    pub quote_fetched_at_ms: Option<u64>,
    pub assumptions_digest: Option<String>,
    pub assumptions: Option<ValuationAssumptions>,
    pub extractor_version: u32,
    pub quality_score_override: Option<Decimal>,
}

pub(crate) fn valuation_assumptions_digest(a: &ValuationAssumptions) -> String {
    let ov = a
        .strike_price_override
        .map(|d| d.normalize().to_string())
        .unwrap_or_default();
    let canonical = format!(
        "{}|{}|{}|{}|{}",
        a.multiple_low.normalize(),
        a.multiple_high.normalize(),
        a.target_return_pct.normalize(),
        a.expected_growth_pct.normalize(),
        ov
    );
    crate::edgar::hex_digest(canonical.as_bytes())
}

pub(crate) fn is_issuer_coverage_regression(
    prev_years: usize,
    prev_has_ttm: bool,
    prev_latest_filing_end: Option<&str>,
    new_years: usize,
    new_has_ttm: bool,
    new_latest_filing_end: Option<&str>,
) -> bool {
    if prev_years == 0 {
        return false;
    }
    if new_years == 0 || new_years < prev_years {
        return true;
    }
    if let (Some(prev_end), Some(new_end)) = (prev_latest_filing_end, new_latest_filing_end)
        && new_end < prev_end
    {
        return true;
    }
    if prev_has_ttm
        && !new_has_ttm
        && let (Some(prev_end), Some(new_end)) = (prev_latest_filing_end, new_latest_filing_end)
        && new_end <= prev_end
    {
        return true;
    }
    if prev_has_ttm && !new_has_ttm && new_latest_filing_end.is_none() {
        return true;
    }
    false
}

fn strike_freshness_bounds(as_of_ms: u64) -> (String, u64, u64) {
    let as_of_secs = (as_of_ms / 1000) as i64;
    let cutoff_secs = as_of_secs - (STRIKE_SIGNAL_MAX_AGE_DAYS + 1) * 86_400;
    let cutoff_date = crate::market::civil_iso_from_unix_secs(cutoff_secs);
    let cutoff_mkt_ms = as_of_ms.saturating_sub((STRIKE_SIGNAL_MAX_AGE_DAYS as u64) * 86_400_000);
    let max_mkt_ms = as_of_ms.saturating_add(STRIKE_SIGNAL_MAX_FUTURE_SKEW_MS);
    (cutoff_date, cutoff_mkt_ms, max_mkt_ms)
}

fn strike_zone_fresh_sql_predicate() -> &'static str {
    "in_price_pool = TRUE
     AND latest_price IS NOT NULL
     AND strike_price IS NOT NULL
     AND distance_to_strike_pct IS NOT NULL
     AND distance_to_strike_pct <= ?
     AND quote_market_time_ms IS NOT NULL
     AND quote_market_time_ms >= ?
     AND quote_market_time_ms <= ?
     AND EXISTS (
         SELECT 1 FROM prices_daily pd
         WHERE UPPER(pd.symbol) = UPPER(universe_snapshot.ticker)
           AND pd.market_time_ms = universe_snapshot.quote_market_time_ms
           AND pd.trade_date >= ?
           AND pd.market_time_ms >= ?
           AND pd.market_time_ms <= ?
     )"
}

pub(crate) struct ReadSnapshotTx<'a> {
    conn: &'a Connection,
}

impl<'a> ReadSnapshotTx<'a> {
    pub(crate) fn begin(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch("BEGIN TRANSACTION;")?;
        Ok(Self { conn })
    }
}

impl Drop for ReadSnapshotTx<'_> {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch("ROLLBACK;");
    }
}

pub struct LakeDb {
    pub(crate) conn: Connection,
}

pub(crate) const UNIVERSE_SELECT_COLS: &str = "
    ticker, cik, name, exchange, sic, sic_description, is_financial,
    latest_filing_end, years_of_data, quality_score, in_price_pool, in_watchlist,
    bucket_opt, latest_price, strike_price, distance_to_strike_pct,
    strict_fcf_yield_pct, roic_5y_avg_pct, roic_5y_stddev_pct,
    gross_margin_5y_avg_pct, gross_margin_5y_stddev_pct, share_change_3y_pct,
    sbc_to_ocf_pct, net_debt_to_oe, strict_owner_earnings,
    warnings_count, warnings_json, raw_sha256, exact_decimals_json
";

impl LakeDb {
    pub fn open(paths: &WorkspacePaths) -> Result<Self> {
        let db_path = paths.lake_db_path();
        if let Ok(meta) = std::fs::symlink_metadata(&db_path)
            && meta.file_type().is_symlink()
            && !db_path.exists()
        {
            anyhow::bail!(
                "refusing to open or create DuckDB through dangling symlink at {}",
                db_path.display()
            );
        }
        fs::create_dir_all(paths.lake_dir())?;
        fs::create_dir_all(paths.lake_parquet_dir())?;
        fs::create_dir_all(paths.lake_raw_sec_dir())?;
        let cfg = Config::default().with("threads", "2")?;
        let conn = Connection::open_with_flags(&db_path, cfg)
            .with_context(|| format!("failed to open DuckDB at {}", db_path.display()))?;
        let db = Self { conn };
        db.ensure_schema(paths)?;
        Ok(db)
    }

    pub fn open_read_only_if_exists(paths: &WorkspacePaths) -> Result<Option<Self>> {
        let db_path = paths.lake_db_path();
        match std::fs::symlink_metadata(&db_path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() && !db_path.exists() {
                    anyhow::bail!(
                        "refusing to open DuckDB through dangling symlink at {}",
                        db_path.display()
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to inspect DuckDB at {}", db_path.display()));
            }
        }
        let cfg = Config::default()
            .access_mode(AccessMode::ReadOnly)?
            .with("threads", "2")?;
        let conn = Connection::open_with_flags(&db_path, cfg)
            .with_context(|| format!("failed to open DuckDB read-only at {}", db_path.display()))?;
        Ok(Some(Self { conn }))
    }

    pub(crate) fn needs_schema_migration(&self) -> Result<bool> {
        let required_tables = [
            "sync_watermarks",
            "companies",
            "facts_annual",
            "facts_ttm",
            "prices_daily",
            "universe_snapshot",
        ];
        let mut table_stmt = self
            .conn
            .prepare("SELECT table_name FROM information_schema.tables")?;
        let existing_tables: HashSet<String> = table_stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<HashSet<String>, _>>()?;
        if required_tables
            .iter()
            .any(|t| !existing_tables.contains(*t))
        {
            return Ok(true);
        }

        let required_cols: [(&str, &[&str]); 2] = [
            (
                "facts_ttm",
                &[
                    "exact_decimals_json",
                    "latest_quarter_label",
                    "latest_quarter_ocf",
                    "latest_quarter_capex",
                ],
            ),
            (
                "universe_snapshot",
                &[
                    "exact_decimals_json",
                    "quote_market_time_ms",
                    "extractor_version",
                    "assumptions_digest",
                    "quote_fetched_at_ms",
                    "assumptions_rev",
                ],
            ),
        ];
        let mut col_stmt = self
            .conn
            .prepare("SELECT column_name FROM information_schema.columns WHERE table_name = ?")?;
        for (table, cols) in required_cols {
            let existing_cols: HashSet<String> = col_stmt
                .query_map([table], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<HashSet<String>, _>>()?;
            if cols.iter().any(|c| !existing_cols.contains(*c)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn open_existing_for_pre_sync(paths: &WorkspacePaths) -> Result<Option<Self>> {
        let Some(ro_db) = Self::open_read_only_if_exists(paths)? else {
            return Ok(None);
        };
        if ro_db.needs_schema_migration()? {
            drop(ro_db);
            let db_path = paths.lake_db_path();
            anyhow::ensure!(
                db_path.exists(),
                "existing DuckDB lakehouse {} disappeared before schema migration",
                db_path.display()
            );
            return Ok(Some(Self::open(paths)?));
        }
        Ok(Some(ro_db))
    }

    pub fn ensure_schema(&self, paths: &WorkspacePaths) -> Result<()> {
        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS sync_watermarks (
                source_key VARCHAR PRIMARY KEY,
                cursor VARCHAR,
                etag VARCHAR,
                last_modified VARCHAR,
                synced_at_ms UBIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS companies (
                cik VARCHAR PRIMARY KEY,
                ticker VARCHAR NOT NULL,
                name VARCHAR NOT NULL,
                exchange VARCHAR NOT NULL,
                sic INTEGER,
                sic_description VARCHAR,
                is_financial BOOLEAN NOT NULL,
                is_active BOOLEAN NOT NULL
            );

            CREATE TABLE IF NOT EXISTS facts_annual (
                cik VARCHAR NOT NULL,
                ticker VARCHAR NOT NULL,
                fiscal_year INTEGER NOT NULL,
                label VARCHAR NOT NULL,
                period_end VARCHAR NOT NULL,
                filed VARCHAR,
                revenue DECIMAL(38, 6) NOT NULL,
                gross_profit DECIMAL(38, 6) NOT NULL,
                gross_margin_pct DECIMAL(38, 10),
                ebit DECIMAL(38, 6) NOT NULL,
                nopat DECIMAL(38, 6),
                invested_capital DECIMAL(38, 6),
                roic_pct DECIMAL(38, 10),
                roic_note VARCHAR,
                roic_total_capital_pct DECIMAL(38, 10),
                ocf DECIMAL(38, 6) NOT NULL,
                capex DECIMAL(38, 6) NOT NULL,
                fcf DECIMAL(38, 6) NOT NULL,
                sbc DECIMAL(38, 6),
                strict_owner_earnings DECIMAL(38, 6),
                sbc_to_ocf_pct DECIMAL(38, 10),
                buybacks DECIMAL(38, 6),
                cash_and_st DECIMAL(38, 6),
                total_debt DECIMAL(38, 6),
                net_debt DECIMAL(38, 6),
                equity DECIMAL(38, 6) NOT NULL,
                diluted_shares DECIMAL(38, 6) NOT NULL,
                ending_shares DECIMAL(38, 6) NOT NULL,
                raw_sha256 VARCHAR NOT NULL,
                provenance_json VARCHAR NOT NULL,
                exact_decimals_json VARCHAR NOT NULL,
                PRIMARY KEY (cik, fiscal_year, period_end)
            );

            CREATE TABLE IF NOT EXISTS facts_ttm (
                cik VARCHAR PRIMARY KEY,
                ticker VARCHAR NOT NULL,
                label VARCHAR NOT NULL,
                period_end VARCHAR NOT NULL,
                revenue DECIMAL(38, 6) NOT NULL,
                gross_profit DECIMAL(38, 6) NOT NULL,
                gross_margin_pct DECIMAL(38, 10),
                ebit DECIMAL(38, 6) NOT NULL,
                nopat DECIMAL(38, 6),
                invested_capital DECIMAL(38, 6),
                roic_pct DECIMAL(38, 10),
                roic_note VARCHAR,
                roic_total_capital_pct DECIMAL(38, 10),
                ocf DECIMAL(38, 6) NOT NULL,
                capex DECIMAL(38, 6) NOT NULL,
                fcf DECIMAL(38, 6) NOT NULL,
                sbc DECIMAL(38, 6),
                strict_owner_earnings DECIMAL(38, 6),
                sbc_to_ocf_pct DECIMAL(38, 10),
                buybacks DECIMAL(38, 6),
                cash_and_st DECIMAL(38, 6),
                total_debt DECIMAL(38, 6),
                net_debt DECIMAL(38, 6),
                equity DECIMAL(38, 6) NOT NULL,
                diluted_shares DECIMAL(38, 6) NOT NULL,
                ytd_period_label VARCHAR NOT NULL,
                ytd_start VARCHAR NOT NULL,
                ytd_end VARCHAR NOT NULL,
                ytd_revenue DECIMAL(38, 6),
                ytd_ocf DECIMAL(38, 6),
                ytd_capex DECIMAL(38, 6),
                ytd_sbc DECIMAL(38, 6),
                ytd_buybacks DECIMAL(38, 6),
                raw_sha256 VARCHAR NOT NULL,
                provenance_json VARCHAR NOT NULL,
                exact_decimals_json VARCHAR NOT NULL,
                latest_quarter_label VARCHAR,
                latest_quarter_ocf DECIMAL(38, 6),
                latest_quarter_capex DECIMAL(38, 6)
            );

            ALTER TABLE facts_ttm ADD COLUMN IF NOT EXISTS exact_decimals_json VARCHAR DEFAULT '';
            ALTER TABLE facts_ttm ADD COLUMN IF NOT EXISTS latest_quarter_label VARCHAR;
            ALTER TABLE facts_ttm ADD COLUMN IF NOT EXISTS latest_quarter_ocf DECIMAL(38, 6);
            ALTER TABLE facts_ttm ADD COLUMN IF NOT EXISTS latest_quarter_capex DECIMAL(38, 6);

            CREATE TABLE IF NOT EXISTS prices_daily (
                symbol VARCHAR NOT NULL,
                trade_date VARCHAR NOT NULL,
                price DECIMAL(38, 6) NOT NULL,
                previous_close DECIMAL(38, 6),
                currency VARCHAR NOT NULL,
                exchange VARCHAR NOT NULL,
                market_time_ms UBIGINT,
                fetched_at_ms UBIGINT NOT NULL,
                source_url VARCHAR NOT NULL,
                PRIMARY KEY (symbol, trade_date)
            );

            CREATE TABLE IF NOT EXISTS universe_snapshot (
                ticker VARCHAR NOT NULL,
                cik VARCHAR PRIMARY KEY,
                name VARCHAR NOT NULL,
                exchange VARCHAR NOT NULL,
                sic INTEGER,
                sic_description VARCHAR,
                is_financial BOOLEAN NOT NULL,
                latest_filing_end VARCHAR,
                years_of_data INTEGER NOT NULL,
                quality_score DECIMAL(38, 6) NOT NULL,
                in_price_pool BOOLEAN NOT NULL,
                in_watchlist BOOLEAN NOT NULL,
                bucket_opt VARCHAR,
                latest_price DECIMAL(38, 6),
                strike_price DECIMAL(38, 6),
                distance_to_strike_pct DECIMAL(38, 10),
                strict_fcf_yield_pct DECIMAL(38, 10),
                roic_5y_avg_pct DECIMAL(38, 10),
                roic_5y_stddev_pct DECIMAL(38, 10),
                gross_margin_5y_avg_pct DECIMAL(38, 10),
                gross_margin_5y_stddev_pct DECIMAL(38, 10),
                share_change_3y_pct DECIMAL(38, 10),
                sbc_to_ocf_pct DECIMAL(38, 10),
                net_debt_to_oe DECIMAL(38, 10),
                strict_owner_earnings DECIMAL(38, 6),
                warnings_count INTEGER NOT NULL,
                warnings_json VARCHAR NOT NULL,
                raw_sha256 VARCHAR,
                exact_decimals_json VARCHAR NOT NULL,
                quote_market_time_ms UBIGINT,
                extractor_version INTEGER,
                assumptions_digest VARCHAR,
                quote_fetched_at_ms UBIGINT,
                assumptions_rev UBIGINT
            );
            ",
            )
            .context("failed to initialize DuckDB schema")?;

        self.migrate_nullable_columns()?;
        self.reconcile_prices_daily_trade_dates()?;
        self.refresh_parquet_views(paths)?;
        self.ensure_l1_schema(paths)?;
        self.ensure_l2_schema()?;
        Ok(())
    }

    fn refresh_parquet_views(&self, paths: &WorkspacePaths) -> Result<()> {
        let parquet_dir = paths.lake_parquet_dir();
        let manifest_files: HashMap<String, PathBuf> = Self::read_parquet_manifest(paths)
            .ok()
            .flatten()
            .map(|m| {
                m.files
                    .into_iter()
                    .filter_map(|e| {
                        let rel = if !e.rel_path.is_empty() {
                            parquet_dir.join(&e.rel_path)
                        } else if !m.generation_dir.is_empty() {
                            parquet_dir.join(&m.generation_dir).join(&e.file)
                        } else {
                            parquet_dir.join(&e.file)
                        };
                        rel.exists().then_some((e.file, rel))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let pairs = [
            ("v_companies", "companies.parquet", "companies"),
            ("v_facts_annual", "facts_annual.parquet", "facts_annual"),
            ("v_facts_ttm", "facts_ttm.parquet", "facts_ttm"),
            ("v_prices_daily", "prices_daily.parquet", "prices_daily"),
        ];
        for (view, file_name, table) in pairs {
            let candidate = manifest_files
                .get(file_name)
                .cloned()
                .unwrap_or_else(|| parquet_dir.join(file_name));
            let sql = if candidate.exists() {
                let escaped = escape_sql_literal(&candidate.to_string_lossy());
                format!("CREATE OR REPLACE VIEW {view} AS SELECT * FROM read_parquet('{escaped}');")
            } else {
                format!("CREATE OR REPLACE VIEW {view} AS SELECT * FROM {table};")
            };
            self.conn
                .execute_batch(&sql)
                .with_context(|| format!("failed to refresh view {view}"))?;
        }
        Ok(())
    }

    pub fn read_watermark(&self, source_key: &str) -> Result<Option<SyncWatermark>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_key, cursor, etag, last_modified, synced_at_ms
             FROM sync_watermarks
             WHERE source_key = ?",
        )?;
        stmt.query_row([source_key], |row| {
            Ok(SyncWatermark {
                source_key: row.get(0)?,
                cursor: row.get(1)?,
                etag: row.get(2)?,
                last_modified: row.get(3)?,
                synced_at_ms: row.get(4)?,
            })
        })
        .optional()
        .context("failed to read sync_watermark")
    }

    #[allow(dead_code)]
    pub fn write_watermark(&self, wm: &SyncWatermark) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_watermarks
                 (source_key, cursor, etag, last_modified, synced_at_ms)
                 VALUES (?, ?, ?, ?, ?)",
                params![
                    wm.source_key,
                    wm.cursor,
                    wm.etag,
                    wm.last_modified,
                    wm.synced_at_ms
                ],
            )
            .with_context(|| format!("failed to write sync_watermark '{}'", wm.source_key))?;
        Ok(())
    }

    pub(crate) fn read_sec_input_generation(&self) -> Result<Option<String>> {
        match self
            .conn
            .query_row(
                "SELECT cursor FROM sync_watermarks WHERE source_key = ?",
                [WM_SEC_INPUT_GEN],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .context("read sec_input_generation watermark")?
        {
            None => Ok(None),
            Some(None) => {
                anyhow::bail!("committed DuckDB sec_input_generation watermark row has NULL cursor")
            }
            Some(Some(raw)) => {
                let trimmed = raw.trim();
                anyhow::ensure!(
                    !trimmed.is_empty(),
                    "committed DuckDB sec_input_generation watermark row has blank cursor"
                );
                Ok(Some(trimmed.to_owned()))
            }
        }
    }

    pub(crate) fn lookup_company_ticker_by_cik(&self, cik: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT ticker FROM companies WHERE cik = ? AND is_active = TRUE LIMIT 1",
                [cik],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map(|opt| {
                opt.map(|t| t.trim().to_ascii_uppercase())
                    .filter(|t| !t.is_empty())
            })
            .context("lookup canonical ticker by cik in DuckDB")
    }

    pub fn screen_universe(&self, filter: &UniverseFilter) -> Result<Vec<UniverseRow>> {
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        self.screen_universe_inner(filter, store::now_ms())
    }

    fn screen_universe_inner(
        &self,
        filter: &UniverseFilter,
        as_of_ms: u64,
    ) -> Result<Vec<UniverseRow>> {
        if filter.limit == 0 {
            return Ok(Vec::new());
        }

        let mut clauses: Vec<String> = Vec::new();
        let mut owned_params: Vec<Box<dyn ToSql>> = Vec::new();
        // Widen coarse SQL bounds slightly so SQL rounding never drops boundary rows
        // that pass exact Decimal refinement in Rust before LIMIT is applied.
        let eps = Decimal::new(1, 9);
        let has_eps_range_filter = filter.strike_zone_only
            || filter.positive_strict_oe_only
            || filter.min_roic_5y.is_some()
            || filter.max_sbc_to_ocf.is_some()
            || filter.max_share_change_3y.is_some()
            || filter.max_net_debt_to_oe.is_some();

        if filter.exclude_financials {
            clauses.push("is_financial = FALSE".to_owned());
        }
        if filter.positive_strict_oe_only {
            clauses.push(
                "strict_owner_earnings IS NOT NULL AND strict_owner_earnings >= -0.000001"
                    .to_owned(),
            );
        }
        if filter.in_price_pool_only {
            clauses.push("in_price_pool = TRUE".to_owned());
        }
        if filter.strike_zone_only {
            let (cutoff_date, cutoff_mkt_ms, max_mkt_ms) = strike_freshness_bounds(as_of_ms);
            clauses.push(strike_zone_fresh_sql_predicate().to_owned());
            owned_params.push(Box::new(round_for_sql(eps, 10)));
            owned_params.push(Box::new(cutoff_mkt_ms));
            owned_params.push(Box::new(max_mkt_ms));
            owned_params.push(Box::new(cutoff_date));
            owned_params.push(Box::new(cutoff_mkt_ms));
            owned_params.push(Box::new(max_mkt_ms));
        }
        if let Some(min_years) = filter.min_years_of_data {
            let min_y_i32 = i32::try_from(min_years).unwrap_or(i32::MAX);
            clauses.push("years_of_data >= ?".to_owned());
            owned_params.push(Box::new(min_y_i32));
        }
        if let Some(min_roic) = filter.min_roic_5y {
            clauses.push("roic_5y_avg_pct IS NOT NULL AND roic_5y_avg_pct >= ?".to_owned());
            owned_params.push(Box::new(round_for_sql(min_roic - eps, 10)));
        }
        if let Some(max_sbc) = filter.max_sbc_to_ocf {
            clauses.push("sbc_to_ocf_pct IS NOT NULL AND sbc_to_ocf_pct <= ?".to_owned());
            owned_params.push(Box::new(round_for_sql(max_sbc + eps, 10)));
        }
        if let Some(max_share) = filter.max_share_change_3y {
            clauses.push("share_change_3y_pct IS NOT NULL AND share_change_3y_pct <= ?".to_owned());
            owned_params.push(Box::new(round_for_sql(max_share + eps, 10)));
        }
        if let Some(max_nd) = filter.max_net_debt_to_oe {
            clauses.push("net_debt_to_oe IS NOT NULL AND net_debt_to_oe <= ?".to_owned());
            owned_params.push(Box::new(round_for_sql(max_nd + eps, 10)));
        }
        if let Some(q) = filter
            .query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let needle = q.to_ascii_lowercase();
            clauses.push(
                "(CONTAINS(LOWER(ticker), ?)
                  OR CONTAINS(LOWER(name), ?)
                  OR CONTAINS(LOWER(cik), ?)
                  OR CONTAINS(LOWER(exchange), ?)
                  OR CONTAINS(LOWER(COALESCE(sic_description, '')), ?)
                  OR CONTAINS(COALESCE(CAST(sic AS VARCHAR), ''), ?))"
                    .to_owned(),
            );
            for _ in 0..6 {
                owned_params.push(Box::new(needle.clone()));
            }
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };

        if filter.limit == 0 {
            return Ok(Vec::new());
        }
        let mut out = if !has_eps_range_filter {
            let order_sql = universe_sort_sql(filter.sort_by, filter.sort_asc);
            let sql_limit = i64::try_from(filter.limit.saturating_add(1)).unwrap_or(i64::MAX);
            let mut fast_params: Vec<Box<dyn ToSql>> = owned_params;
            fast_params.push(Box::new(sql_limit));
            let fast_sql = format!(
                "SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot {where_sql} ORDER BY {order_sql} LIMIT ?"
            );
            let mut stmt = self.conn.prepare(&fast_sql)?;
            let refs = fast_params.iter().map(|b| b.as_ref());
            let rows = stmt.query_map(params_from_iter(refs), read_universe_row)?;
            let mut initial = Vec::with_capacity(filter.limit.min(8_192));
            for row in rows {
                let r = row?;
                if row_matches_exact_filter(&r, filter) {
                    initial.push(r);
                }
            }
            if initial.len() > filter.limit
                && let (Some(boundary), Some(next_row)) =
                    (initial.get(filter.limit - 1), initial.get(filter.limit))
                && rows_might_tie_on_sql_primary(boundary, next_row, filter.sort_by)
            {
                let any_clamped = initial
                    .iter()
                    .any(|r| is_sql_primary_clamped(r, filter.sort_by));
                let _ = fast_params.pop();
                if !any_clamped {
                    let first_tie_idx = initial[..filter.limit]
                        .iter()
                        .position(|r| rows_might_tie_on_sql_primary(r, boundary, filter.sort_by))
                        .unwrap_or(filter.limit);
                    let (tie_pred, tie_param_opt) =
                        sql_primary_tie_filter(boundary, filter.sort_by);
                    let tie_where_sql = if clauses.is_empty() {
                        format!("WHERE {tie_pred}")
                    } else {
                        format!("{where_sql} AND {tie_pred}")
                    };
                    if let Some(tie_param) = tie_param_opt {
                        fast_params.push(tie_param);
                    }
                    let tie_sql = format!(
                        "SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot {tie_where_sql}"
                    );
                    let mut tie_stmt = self.conn.prepare(&tie_sql)?;
                    let tie_refs = fast_params.iter().map(|b| b.as_ref());
                    let tie_rows =
                        tie_stmt.query_map(params_from_iter(tie_refs), read_universe_row)?;
                    initial.truncate(first_tie_idx);
                    for row in tie_rows {
                        let r = row?;
                        if row_matches_exact_filter(&r, filter) {
                            initial.push(r);
                        }
                    }
                    initial
                } else {
                    let primary_sql = universe_primary_sort_sql(filter.sort_by, filter.sort_asc);
                    fast_params.push(Box::new(i64::try_from(filter.limit).unwrap_or(i64::MAX)));
                    let tie_sql = format!(
                        "SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot {where_sql}
                         QUALIFY RANK() OVER (ORDER BY {primary_sql}) <= ?
                         ORDER BY {order_sql}"
                    );
                    let mut tie_stmt = self.conn.prepare(&tie_sql)?;
                    let tie_refs = fast_params.iter().map(|b| b.as_ref());
                    let tie_rows =
                        tie_stmt.query_map(params_from_iter(tie_refs), read_universe_row)?;
                    let mut expanded = Vec::new();
                    for row in tie_rows {
                        let r = row?;
                        if row_matches_exact_filter(&r, filter) {
                            expanded.push(r);
                        }
                    }
                    expanded
                }
            } else {
                initial
            }
        } else {
            let full_sql =
                format!("SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot {where_sql}");
            let mut stmt = self.conn.prepare(&full_sql)?;
            let refs = owned_params.iter().map(|b| b.as_ref());
            let rows = stmt.query_map(params_from_iter(refs), read_universe_row)?;
            let mut matched = Vec::new();
            for row in rows {
                let r = row?;
                if row_matches_exact_filter(&r, filter) {
                    matched.push(r);
                }
            }
            matched
        };
        sort_universe_rows(&mut out, filter.sort_by, filter.sort_asc);
        out.truncate(filter.limit);
        Ok(out)
    }

    pub fn query_peers(&self, ticker: &str, limit: usize) -> Result<PeerComparison> {
        let upper = ticker.trim().to_ascii_uppercase();
        ensure!(!upper.is_empty(), "ticker must not be empty");
        let _tx = ReadSnapshotTx::begin(&self.conn)?;

        let target_sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE UPPER(ticker) = ?
             ORDER BY years_of_data DESC, quality_score DESC
             LIMIT 1"
        );
        let target = self
            .conn
            .query_row(&target_sql, [&upper], read_universe_row)
            .optional()?
            .with_context(|| format!("ticker {upper} not found in universe_snapshot"))?;

        self.query_peers_for_target(target, limit)
    }

    fn query_peers_by_cik_inner(&self, cik: &str, limit: usize) -> Result<Option<PeerComparison>> {
        let target_sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE cik = ?"
        );
        let Some(target) = self
            .conn
            .query_row(&target_sql, [cik], read_universe_row)
            .optional()?
        else {
            return Ok(None);
        };
        Ok(Some(self.query_peers_for_target(target, limit)?))
    }

    fn query_peers_for_target(&self, target: UniverseRow, limit: usize) -> Result<PeerComparison> {
        let (extra_where, extra_params, sic_group) = self.select_peer_cohort(&target)?;

        let cohort_sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE years_of_data > 0 AND is_financial = ? AND cik != ? {extra_where}"
        );
        let mut cohort_params: Vec<Box<dyn ToSql>> =
            vec![Box::new(target.is_financial), Box::new(target.cik.clone())];
        for p in &extra_params {
            cohort_params.push(Box::new(*p));
        }
        let mut stmt = self.conn.prepare(&cohort_sql)?;
        let rows = stmt.query_map(
            params_from_iter(cohort_params.iter().map(|b| b.as_ref())),
            read_universe_row,
        )?;
        let mut cohort = Vec::new();
        for r in rows {
            cohort.push(r?);
        }
        let peer_count = cohort.len();

        let roic_percentile = compute_exact_percentile(
            target.roic_5y_avg_pct,
            cohort.iter().filter_map(|r| r.roic_5y_avg_pct),
        );
        let gross_margin_percentile = compute_exact_percentile(
            target.gross_margin_5y_avg_pct,
            cohort.iter().filter_map(|r| r.gross_margin_5y_avg_pct),
        );
        let strict_fcf_yield_percentile = compute_exact_percentile(
            target.strict_fcf_yield_pct,
            cohort.iter().filter_map(|r| r.strict_fcf_yield_pct),
        );

        cohort.sort_by(|a, b| {
            b.quality_score
                .cmp(&a.quality_score)
                .then_with(|| cmp_opt_desc(a.roic_5y_avg_pct, b.roic_5y_avg_pct))
                .then_with(|| a.ticker.cmp(&b.ticker))
        });
        cohort.truncate(limit);

        Ok(PeerComparison {
            target,
            sic_group,
            peer_count,
            roic_percentile,
            gross_margin_percentile,
            strict_fcf_yield_percentile,
            top_peers: cohort,
        })
    }

    fn select_peer_cohort(&self, target: &UniverseRow) -> Result<(String, Vec<i32>, String)> {
        if let Some(sic) = target.sic {
            let sic_i32 = i32::try_from(sic).unwrap_or(0);
            let exact_count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM universe_snapshot
                 WHERE years_of_data > 0 AND is_financial = ? AND cik != ? AND sic = ?",
                params![target.is_financial, target.cik, sic_i32],
                |row| row.get(0),
            )?;
            if exact_count >= 3 {
                let label = match target.sic_description.as_deref().map(str::trim) {
                    Some(desc) if !desc.is_empty() => format!("SIC {sic} · {desc}"),
                    _ => format!("SIC {sic}"),
                };
                return Ok(("AND sic = ?".to_owned(), vec![sic_i32], label));
            }

            let major = sic_i32 / 100;
            let lo = major * 100;
            let hi = lo + 99;
            let major_count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM universe_snapshot
                 WHERE years_of_data > 0 AND is_financial = ? AND cik != ? AND sic BETWEEN ? AND ?",
                params![target.is_financial, target.cik, lo, hi],
                |row| row.get(0),
            )?;
            if major_count >= 3 {
                return Ok((
                    "AND sic BETWEEN ? AND ?".to_owned(),
                    vec![lo, hi],
                    format!("SIC {major:02}xx (expanded from {sic})"),
                ));
            }
        }

        let fallback_label = if target.is_financial {
            "All Financials".to_owned()
        } else {
            "All Non-Financials".to_owned()
        };
        Ok((String::new(), Vec::new(), fallback_label))
    }

    pub fn universe_summary(&self) -> Result<UniverseSummary> {
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        self.universe_summary_inner(store::now_ms())
    }

    fn universe_summary_inner(&self, as_of_ms: u64) -> Result<UniverseSummary> {
        Ok(self.universe_summary_with_strike_ciks_inner(as_of_ms)?.0)
    }

    fn universe_summary_with_strike_ciks_inner(
        &self,
        as_of_ms: u64,
    ) -> Result<(UniverseSummary, HashSet<String>)> {
        let (total, parsed, non_fin, pool, priced): (i64, i64, i64, i64, i64) =
            self.conn.query_row(
                "SELECT
                    COUNT(*),
                    COUNT(*) FILTER (WHERE years_of_data > 0),
                    COUNT(*) FILTER (WHERE is_financial = FALSE),
                    COUNT(*) FILTER (WHERE in_price_pool = TRUE),
                    COUNT(*) FILTER (WHERE latest_price IS NOT NULL)
                 FROM universe_snapshot",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?;

        let eps = Decimal::new(1, 6);
        let (cutoff_date, cutoff_mkt_ms, max_mkt_ms) = strike_freshness_bounds(as_of_ms);
        let strike_sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot WHERE {}",
            strike_zone_fresh_sql_predicate()
        );
        let mut strike_stmt = self.conn.prepare(&strike_sql)?;
        let strike_rows = strike_stmt.query_map(
            params![
                round_for_sql(eps, 10),
                cutoff_mkt_ms,
                max_mkt_ms,
                cutoff_date,
                cutoff_mkt_ms,
                max_mkt_ms
            ],
            read_universe_row,
        )?;
        let mut strike_zone_ciks = HashSet::new();
        for row_res in strike_rows {
            let row = row_res?;
            if row
                .distance_to_strike_pct
                .is_some_and(|d| d <= Decimal::ZERO)
            {
                strike_zone_ciks.insert(row.cik);
            }
        }

        let last_sec_sync_ms: Option<u64> = self
            .conn
            .query_row(
                "SELECT MAX(synced_at_ms) FROM sync_watermarks
                 WHERE source_key IN ('sec_companyfacts_zip', 'sec_sync')",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let last_price_sync_ms: Option<u64> = self
            .conn
            .query_row(
                "SELECT MAX(synced_at_ms) FROM sync_watermarks
                 WHERE source_key = 'yahoo_prices'",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let to_usize = |v: i64| usize::try_from(v.max(0)).unwrap_or(0);
        let summary = UniverseSummary {
            total_companies: to_usize(total),
            parsed_financial_companies: to_usize(parsed),
            non_financial_companies: to_usize(non_fin),
            quality_pool_count: to_usize(pool),
            priced_count: to_usize(priced),
            strike_zone_count: strike_zone_ciks.len(),
            last_sec_sync_ms,
            last_price_sync_ms,
        };
        Ok((summary, strike_zone_ciks))
    }

    #[cfg(test)]
    pub fn get_annual_rows(&self, ticker: &str) -> Result<Vec<LakeAnnualRow>> {
        let upper = ticker.trim().to_ascii_uppercase();
        ensure!(!upper.is_empty(), "ticker must not be empty");
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let mut stmt = self.conn.prepare(
            "SELECT
                cik, ticker, fiscal_year, label, period_end, filed,
                revenue, gross_profit, gross_margin_pct, ebit, nopat, invested_capital,
                roic_pct, roic_note, roic_total_capital_pct, ocf, capex, fcf, sbc,
                strict_owner_earnings, sbc_to_ocf_pct, buybacks, cash_and_st, total_debt,
                net_debt, equity, diluted_shares, ending_shares, raw_sha256,
                provenance_json, exact_decimals_json
             FROM facts_annual
             WHERE UPPER(ticker) = ?
             ORDER BY fiscal_year ASC, period_end ASC",
        )?;
        let rows = stmt.query_map([&upper], read_annual_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn get_annual_rows_by_cik_inner(&self, cik: &str) -> Result<Vec<LakeAnnualRow>> {
        get_annual_rows_by_cik_on_conn(&self.conn, cik)
    }

    pub fn load_universe_snapshot(
        &self,
        filter: &UniverseFilter,
        selected_cik: Option<&str>,
        peer_limit: usize,
    ) -> Result<UniverseViewSnapshot> {
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let as_of_ms = store::now_ms();
        let (summary, strike_zone_ciks) = self.universe_summary_with_strike_ciks_inner(as_of_ms)?;
        let rows = self.screen_universe_inner(filter, as_of_ms)?;
        let target_cik = selected_cik
            .filter(|cik| rows.iter().any(|r| r.cik == *cik))
            .or_else(|| rows.first().map(|r| r.cik.as_str()));
        let selected_details = match target_cik {
            Some(cik) => self.load_drawer_details_by_cik_inner(cik, peer_limit)?,
            None => None,
        };
        Ok(UniverseViewSnapshot {
            summary,
            rows,
            selected_details,
            strike_zone_ciks,
        })
    }

    pub fn load_drawer_details_by_ticker(
        &self,
        ticker: &str,
        peer_limit: usize,
    ) -> Result<Option<UniverseDrawerDetails>> {
        let trimmed = ticker.trim();
        if trimmed.is_empty() || !self.table_exists("universe_snapshot")? {
            return Ok(None);
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let Some(cik) = self
            .conn
            .query_row(
                "SELECT cik
                 FROM universe_snapshot
                 WHERE UPPER(ticker) = UPPER(?)
                 ORDER BY in_watchlist DESC, quality_score DESC, cik ASC
                 LIMIT 1",
                [trimmed],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(None);
        };
        self.load_drawer_details_by_cik_inner(&cik, peer_limit)
    }

    pub fn load_drawer_details_by_cik(
        &self,
        cik: &str,
        peer_limit: usize,
    ) -> Result<Option<UniverseDrawerDetails>> {
        let trimmed = cik.trim();
        ensure!(!trimmed.is_empty(), "cik must not be empty");
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        self.load_drawer_details_by_cik_inner(trimmed, peer_limit)
    }

    fn load_drawer_details_by_cik_inner(
        &self,
        cik: &str,
        peer_limit: usize,
    ) -> Result<Option<UniverseDrawerDetails>> {
        let row_sql = format!("SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot WHERE cik = ?");
        let Some(row) = self
            .conn
            .query_row(&row_sql, [cik], read_universe_row)
            .optional()?
        else {
            return Ok(None);
        };
        let annuals = self.get_annual_rows_by_cik_inner(cik)?;
        let peers = self.query_peers_by_cik_inner(cik, peer_limit)?;
        Ok(Some(UniverseDrawerDetails {
            row,
            annuals,
            peers,
        }))
    }

    pub fn load_universe_rows_by_tickers(&self, tickers: &[String]) -> Result<Vec<UniverseRow>> {
        let mut norm_tickers: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for raw in tickers {
            let up = raw.trim().to_ascii_uppercase();
            if !up.is_empty() && seen.insert(up.clone()) {
                norm_tickers.push(up);
            }
        }
        if norm_tickers.is_empty() || !self.table_exists("universe_snapshot")? {
            return Ok(Vec::new());
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let placeholders = vec!["?"; norm_tickers.len()].join(", ");
        let sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE UPPER(ticker) IN ({placeholders})
             ORDER BY in_watchlist DESC, quality_score DESC, cik ASC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<Box<dyn ToSql>> = norm_tickers
            .iter()
            .map(|t| Box::new(t.clone()) as Box<dyn ToSql>)
            .collect();
        let refs = params.iter().map(|b| b.as_ref());
        let rows = stmt.query_map(params_from_iter(refs), read_universe_row)?;
        let mut by_ticker: HashMap<String, UniverseRow> = HashMap::new();
        for r in rows {
            let row = r?;
            let key = row.ticker.to_ascii_uppercase();
            by_ticker.entry(key).or_insert(row);
        }
        let mut ordered = Vec::with_capacity(by_ticker.len());
        for t in norm_tickers {
            if let Some(row) = by_ticker.remove(&t) {
                ordered.push(row);
            }
        }
        Ok(ordered)
    }

    pub fn load_universe_rows_by_sic(&self, sic: u32, limit: usize) -> Result<Vec<UniverseRow>> {
        if limit == 0 || !self.table_exists("universe_snapshot")? {
            return Ok(Vec::new());
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let sql_limit = i64::try_from(limit).unwrap_or(10);
        let sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE sic = ?
             ORDER BY quality_score DESC, ticker ASC
             LIMIT ?"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![i64::from(sic), sql_limit], read_universe_row)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn search_universe_top_tickers(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UniverseRow>> {
        let trimmed = query.trim();
        if trimmed.is_empty() || limit == 0 || !self.table_exists("universe_snapshot")? {
            return Ok(Vec::new());
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let pattern = format!("%{}%", trimmed.to_ascii_lowercase());
        let sql_limit = i64::try_from(limit).unwrap_or(10);
        let sql = format!(
            "SELECT {UNIVERSE_SELECT_COLS}
             FROM universe_snapshot
             WHERE LOWER(ticker) LIKE ?
                OR LOWER(name) LIKE ?
                OR LOWER(COALESCE(sic_description, '')) LIKE ?
             ORDER BY
                CASE WHEN UPPER(ticker) = UPPER(?) THEN 0 ELSE 1 END ASC,
                quality_score DESC,
                ticker ASC
             LIMIT ?"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![pattern, pattern, pattern, trimmed, sql_limit],
            read_universe_row,
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn count_sec_10k_chunks_by_ticker(
        &self,
        tickers: &[String],
    ) -> Result<HashMap<String, usize>> {
        let norm: Vec<String> = tickers
            .iter()
            .map(|t| t.trim().to_ascii_uppercase())
            .filter(|t| !t.is_empty())
            .collect();
        if norm.is_empty() || !self.table_exists("doc_chunks")? {
            return Ok(HashMap::new());
        }
        let _tx = ReadSnapshotTx::begin(&self.conn)?;
        let placeholders = vec!["?"; norm.len()].join(", ");
        let sql = format!(
            "SELECT UPPER(ticker), COUNT(*)
             FROM doc_chunks
             WHERE UPPER(ticker) IN ({placeholders})
               AND doc_type = 'sec_10k'
             GROUP BY UPPER(ticker)"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<Box<dyn ToSql>> = norm
            .iter()
            .map(|t| Box::new(t.clone()) as Box<dyn ToSql>)
            .collect();
        let refs = params.iter().map(|b| b.as_ref());
        let rows = stmt.query_map(params_from_iter(refs), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut out = HashMap::new();
        for r in rows {
            let (t, cnt) = r?;
            out.insert(t, usize::try_from(cnt.max(0)).unwrap_or(0));
        }
        Ok(out)
    }

    pub(crate) fn load_previous_sec_snapshot(&self) -> Result<PrevSecLakeSnapshot> {
        let mut all_ciks = HashSet::new();
        let mut parsed_ciks = HashSet::new();
        let mut classified_sic_ciks = HashSet::new();
        let mut sic_map = HashMap::new();
        let mut parsed_rows = HashMap::new();
        let mut ttm_ciks = HashSet::new();
        let mut extractor_versions = HashMap::new();
        let mut annual_period_ends: HashMap<String, Vec<String>> = HashMap::new();
        let mut ttm_period_ends: HashMap<String, String> = HashMap::new();

        {
            let mut ann_stmt = self
                .conn
                .prepare("SELECT cik, period_end FROM facts_annual ORDER BY cik, period_end")?;
            let ann_rows =
                ann_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in ann_rows {
                let (cik, period_end) = r?;
                annual_period_ends.entry(cik).or_default().push(period_end);
            }
        }
        {
            let mut ttm_stmt = self.conn.prepare("SELECT cik, period_end FROM facts_ttm")?;
            let ttm_rows =
                ttm_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in ttm_rows {
                let (cik, period_end) = r?;
                ttm_ciks.insert(cik.clone());
                ttm_period_ends.insert(cik, period_end);
            }
        }
        {
            let mut ext_stmt = self.conn.prepare(
                "SELECT cik, COALESCE(extractor_version, 0) FROM universe_snapshot WHERE years_of_data > 0",
            )?;
            let ext_rows = ext_stmt.query_map([], |r| {
                let cik: String = r.get(0)?;
                let v: i32 = r.get(1)?;
                Ok((cik, u32::try_from(v.max(0)).unwrap_or(0)))
            })?;
            for r in ext_rows {
                let (cik, ver) = r?;
                extractor_versions.insert(cik, ver);
            }
        }

        let sql = format!("SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], read_universe_row)?;
        for r in rows {
            let row = r?;
            all_ciks.insert(row.cik.clone());
            if row.sic.is_some() || row.sic_description.is_some() {
                if row.sic.is_some() {
                    classified_sic_ciks.insert(row.cik.clone());
                }
                sic_map.insert(row.cik.clone(), (row.sic, row.sic_description.clone()));
            }
            if row.years_of_data > 0 {
                parsed_ciks.insert(row.cik.clone());
                parsed_rows.insert(row.cik.clone(), row);
            }
        }
        // Rebuild snapshot rows for inactive/delisted issuers whose retained facts survive in DuckDB
        // so relisting in a subsequent sync preserves Scheme 1 scoring even if the incoming parse is partial.
        let now_ms = store::now_ms();
        let mut inactive_stmt = self.conn.prepare(
            "SELECT cik, ticker, name, exchange, sic, sic_description
             FROM companies
             WHERE cik NOT IN (SELECT cik FROM universe_snapshot)
               AND EXISTS (SELECT 1 FROM facts_annual a WHERE a.cik = companies.cik)",
        )?;
        let inactive_rows = inactive_stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i32>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        for r in inactive_rows {
            let (cik, ticker, name, exchange, sic_i32, sic_description) = r?;
            let sic = sic_i32.and_then(|v| u32::try_from(v).ok());
            if sic.is_some() || sic_description.is_some() {
                if sic.is_some() {
                    classified_sic_ciks.insert(cik.clone());
                }
                sic_map
                    .entry(cik.clone())
                    .or_insert((sic, sic_description.clone()));
            }
            if let std::collections::hash_map::Entry::Vacant(e) = parsed_rows.entry(cik.clone()) {
                let target = RetainedRebuildTarget {
                    cik: &cik,
                    ticker: &ticker,
                    name: &name,
                    exchange: &exchange,
                    sic,
                    sic_description: sic_description.as_deref(),
                    in_price_pool: false,
                    wl_entry: None,
                    cached_price: None,
                    is_non_common_listing: false,
                    now_ms,
                };
                if let Some(rebuilt) = rebuild_universe_row_from_retained_conn(&self.conn, &target)?
                    && rebuilt.years_of_data > 0
                {
                    // Inactive retained issuers stay out of parsed_ciks so delistings do not
                    // inflate the active parsed shrinkage denominator, while parsed_rows keeps relist history.
                    e.insert(rebuilt);
                }
            }
        }
        Ok(PrevSecLakeSnapshot {
            all_ciks,
            parsed_ciks,
            ttm_ciks,
            classified_sic_ciks,
            sic_map,
            parsed_rows,
            extractor_versions,
            annual_period_ends,
            ttm_period_ends,
        })
    }

    pub(crate) fn read_sec_generation(&self) -> Result<Option<(u64, u64)>> {
        read_sec_generation_on_conn(&self.conn)
    }

    pub(crate) fn load_assumptions_state(&self) -> Result<(u64, AssumptionsStateMap)> {
        let rev = read_assumptions_revision_on_conn(&self.conn)?;
        let mut map = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT cik, assumptions_digest, COALESCE(assumptions_rev, 0) FROM universe_snapshot",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })?;
        for r in rows {
            let (cik, dig_opt, row_rev) = r?;
            if let Some(dig) = dig_opt {
                map.insert(cik, (dig, row_rev));
            }
        }
        Ok((rev, map))
    }

    #[cfg(test)]
    pub(crate) fn load_parsed_and_classified_ciks(
        &self,
    ) -> Result<(HashSet<String>, HashSet<String>)> {
        let snap = self.load_previous_sec_snapshot()?;
        Ok((snap.parsed_ciks, snap.classified_sic_ciks))
    }

    pub(crate) fn has_missing_exact_decimals(&self) -> Result<bool> {
        let target_ver = i32::try_from(SEC_LAKE_EXTRACTOR_VERSION_NUM).unwrap_or(i32::MAX);
        self.conn
            .query_row(
                "SELECT NOT EXISTS(
                    SELECT 1 FROM sync_watermarks
                    WHERE source_key = ? AND cursor = ?
                 ) OR EXISTS(
                    SELECT 1 FROM universe_snapshot
                    WHERE years_of_data > 0 AND COALESCE(extractor_version, 0) < ?
                 ) OR EXISTS(
                    SELECT 1 FROM facts_ttm
                    WHERE exact_decimals_json IS NULL OR exact_decimals_json = ''
                       OR NOT CONTAINS(provenance_json, '\"field\":\"ytd_buybacks\"')
                       OR (provenance_json LIKE '%\"field\":\"ttm_sbc\",\"source_tag\":\"absent\"%' AND strict_owner_earnings IS NOT NULL)
                       OR (provenance_json LIKE '%\"field\":\"ttm_buybacks\",\"source_tag\":\"absent\"%' AND buybacks IS NOT NULL)
                       OR (provenance_json LIKE '%\"field\":\"ytd_buybacks\",\"source_tag\":\"absent\"%' AND ytd_buybacks IS NOT NULL)
                       OR (provenance_json LIKE '%\"field\":\"ttm_total_debt\",\"source_tag\":\"absent\"%' AND (total_debt IS NOT NULL OR net_debt IS NOT NULL OR invested_capital IS NOT NULL))
                       OR (provenance_json LIKE '%\"field\":\"ttm_cash_and_st\",\"source_tag\":\"absent\"%' AND (cash_and_st IS NOT NULL OR net_debt IS NOT NULL OR invested_capital IS NOT NULL))
                       OR (provenance_json LIKE '%\"field\":\"ttm_tax_expense\",\"source_tag\":\"absent\"%' AND nopat IS NOT NULL)
                 ) OR EXISTS(
                    SELECT 1 FROM facts_annual
                    WHERE (provenance_json LIKE '%\"field\":\"sbc\",\"source_tag\":\"absent\"%' AND strict_owner_earnings IS NOT NULL)
                       OR (provenance_json LIKE '%\"field\":\"buybacks\",\"source_tag\":\"absent\"%' AND buybacks IS NOT NULL)
                       OR (provenance_json LIKE '%\"field\":\"total_debt\",\"source_tag\":\"absent\"%' AND (total_debt IS NOT NULL OR net_debt IS NOT NULL OR invested_capital IS NOT NULL))
                       OR (provenance_json LIKE '%\"field\":\"cash_and_st\",\"source_tag\":\"absent\"%' AND (cash_and_st IS NOT NULL OR net_debt IS NOT NULL OR invested_capital IS NOT NULL))
                       OR (provenance_json LIKE '%\"field\":\"tax_expense\",\"source_tag\":\"absent\"%' AND nopat IS NOT NULL)
                 ) OR EXISTS(
                    SELECT 1 FROM universe_snapshot
                    WHERE exact_decimals_json IS NULL OR exact_decimals_json = ''
                       OR (latest_price IS NOT NULL AND quote_market_time_ms IS NULL)
                 )",
                params![WM_SEC_EXTRACTOR_VERSION, SEC_LAKE_EXTRACTOR_VERSION, target_ver],
                |row| row.get(0),
            )
            .context("check missing exact_decimals_json or extractor version")
    }

    pub fn refresh_watchlist_flags(&self, paths: &WorkspacePaths) -> Result<()> {
        if !paths.watchlist_path().exists() {
            return Ok(());
        }
        let wl = store::load_watchlist(paths).with_context(|| {
            format!(
                "load watchlist at {} (refusing to wipe watchlist flags on unreadable file)",
                paths.watchlist_path().display()
            )
        })?;
        self.conn.execute_batch("BEGIN TRANSACTION;")?;
        let res = refresh_watchlist_flags_on_conn(&self.conn, &wl);
        match res {
            Ok(_) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    }

    pub(crate) fn reconcile_watchlist_and_pool(
        &mut self,
        wl: &crate::model::WatchlistFile,
        prices_top: usize,
        as_of_ms: u64,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        reconcile_watchlist_and_pool_on_conn(&tx, wl, prices_top, as_of_ms)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn sync_watchlist_and_upsert_prices(
        &mut self,
        paths: &WorkspacePaths,
        fetched_watchlist: Option<&crate::model::WatchlistFile>,
        prices_top: usize,
        commit: &PriceSyncCommit<'_>,
    ) -> Result<()> {
        let disk_watchlist = if paths.watchlist_path().exists() {
            Some(store::load_watchlist(paths).with_context(|| {
                format!(
                    "load watchlist at {} during atomic price commit",
                    paths.watchlist_path().display()
                )
            })?)
        } else {
            None
        };
        let tx = self.conn.transaction()?;
        let mut effective_revs = commit.expected_assumptions_revs.clone();
        if let Some(ref wl) = disk_watchlist {
            let bumped =
                reconcile_watchlist_and_pool_on_conn(&tx, wl, prices_top, store::now_ms())?;
            if fetched_watchlist == Some(wl) {
                for (cik, new_rev) in bumped {
                    effective_revs.insert(cik, new_rev);
                }
            } else if !bumped.is_empty() {
                let mut live_digests: HashMap<String, String> = HashMap::new();
                {
                    let mut stmt = tx
                        .prepare("SELECT cik, assumptions_digest FROM universe_snapshot WHERE assumptions_digest IS NOT NULL")?;
                    let rows = stmt
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                    for r in rows {
                        let (cik, dig) = r?;
                        live_digests.insert(cik, dig);
                    }
                }
                for (cik, new_rev) in bumped {
                    if let (Some(live_dig), Some(exp_dig)) = (
                        live_digests.get(&cik),
                        commit.expected_assumptions_digests.get(&cik),
                    ) && live_dig == exp_dig
                    {
                        effective_revs.insert(cik, new_rev);
                    }
                }
            }
        } else {
            let empty_wl = empty_watchlist_file();
            reconcile_watchlist_and_pool_on_conn(&tx, &empty_wl, prices_top, store::now_ms())?;
        }
        upsert_prices_and_update_snapshot_on_conn(
            &tx,
            commit.prices,
            commit.updated_rows,
            commit.expected_assumptions_digests,
            &effective_revs,
            commit.watermarks,
        )?;
        tx.commit()?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn export_parquet_atomic(&self, select_sql: &str, dest_path: &Path) -> Result<()> {
        let tmp_path = self.stage_parquet_tmp(select_sql, dest_path)?;
        if let Err(err) = fs::rename(&tmp_path, dest_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(err)
                .with_context(|| format!("atomic rename failed for {}", dest_path.display()));
        }
        Ok(())
    }

    fn stage_parquet_tmp(&self, select_sql: &str, dest_path: &Path) -> Result<PathBuf> {
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = store::atomic_temp_path(dest_path);
        let escaped_tmp = escape_sql_literal(&tmp_path.to_string_lossy());
        let copy_sql =
            format!("COPY ({select_sql}) TO '{escaped_tmp}' (FORMAT PARQUET, COMPRESSION ZSTD)");
        if let Err(err) = self.conn.execute_batch(&copy_sql) {
            let _ = fs::remove_file(&tmp_path);
            return Err(err).with_context(|| {
                format!("DuckDB COPY PARQUET failed for {}", dest_path.display())
            });
        }
        Ok(tmp_path)
    }

    pub(crate) fn export_all_parquets(&self, paths: &WorkspacePaths) -> Result<()> {
        let dir = paths.lake_parquet_dir();
        fs::create_dir_all(&dir)?;
        let prev_generation = Self::read_parquet_manifest(paths)
            .ok()
            .flatten()
            .map(|m| m.generation_id);
        cleanup_stale_parquet_staging_dirs(&dir);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        let seq = PARQUET_GEN_SEQ.fetch_add(1, AtomicOrdering::Relaxed);
        let generation_id = format!("gen-{}-{now_ms}-{seq}", std::process::id());
        let staging_dir = dir.join(format!(".staging-{generation_id}"));
        let gen_dir = dir.join(&generation_id);
        fs::create_dir_all(&staging_dir)
            .with_context(|| format!("create parquet staging dir {}", staging_dir.display()))?;

        let specs = [
            (
                "companies.parquet",
                "SELECT COUNT(*) FROM companies",
                "SELECT cik, ticker, name, exchange, sic, sic_description, is_financial, is_active FROM companies ORDER BY cik",
            ),
            (
                "facts_annual.parquet",
                "SELECT COUNT(*) FROM facts_annual",
                "SELECT cik, ticker, fiscal_year, label, period_end, filed, revenue, gross_profit, gross_margin_pct, ebit, nopat, invested_capital, roic_pct, roic_note, roic_total_capital_pct, ocf, capex, fcf, sbc, strict_owner_earnings, sbc_to_ocf_pct, buybacks, cash_and_st, total_debt, net_debt, equity, diluted_shares, ending_shares, raw_sha256, provenance_json, exact_decimals_json FROM facts_annual ORDER BY cik, fiscal_year, period_end",
            ),
            (
                "facts_ttm.parquet",
                "SELECT COUNT(*) FROM facts_ttm",
                "SELECT cik, ticker, label, period_end, revenue, gross_profit, gross_margin_pct, ebit, nopat, invested_capital, roic_pct, roic_note, roic_total_capital_pct, ocf, capex, fcf, sbc, strict_owner_earnings, sbc_to_ocf_pct, buybacks, cash_and_st, total_debt, net_debt, equity, diluted_shares, ytd_period_label, ytd_start, ytd_end, ytd_revenue, ytd_ocf, ytd_capex, ytd_sbc, ytd_buybacks, raw_sha256, provenance_json, exact_decimals_json, latest_quarter_label, latest_quarter_ocf, latest_quarter_capex FROM facts_ttm ORDER BY cik",
            ),
            (
                "prices_daily.parquet",
                "SELECT COUNT(*) FROM prices_daily",
                "SELECT symbol, trade_date, price, previous_close, currency, exchange, market_time_ms, fetched_at_ms, source_url FROM prices_daily ORDER BY symbol, trade_date",
            ),
        ];

        // Stage all 4 Parquets into staging_dir, publish immutable gen-<id>/ dir, then atomically swap manifest.json.
        let manifest_res = (|| -> Result<ParquetExportManifest> {
            let mut entries: Vec<ParquetFileManifestEntry> = Vec::with_capacity(specs.len());
            for (file_name, count_sql, select_sql) in specs {
                let stage_path = staging_dir.join(file_name);
                let escaped_stage = escape_sql_literal(&stage_path.to_string_lossy());
                let copy_sql = format!(
                    "COPY ({select_sql}) TO '{escaped_stage}' (FORMAT PARQUET, COMPRESSION ZSTD)"
                );
                self.conn
                    .execute_batch(&copy_sql)
                    .with_context(|| format!("DuckDB COPY PARQUET failed for {file_name}"))?;
                let meta = fs::metadata(&stage_path)
                    .with_context(|| format!("stat staged parquet {}", stage_path.display()))?;
                ensure!(meta.len() > 0, "staged parquet {file_name} is empty");
                let sha256 = file_sha256_hex(&stage_path)?;
                let rows_i64: i64 = self.conn.query_row(count_sql, [], |r| r.get(0))?;
                entries.push(ParquetFileManifestEntry {
                    file: file_name.to_owned(),
                    rel_path: format!("{generation_id}/{file_name}"),
                    rows: usize::try_from(rows_i64.max(0)).unwrap_or(0),
                    bytes: meta.len(),
                    sha256,
                });
            }
            Ok(ParquetExportManifest {
                schema_version: 2,
                generation_id: generation_id.clone(),
                generation_dir: generation_id.clone(),
                exported_at_ms: now_ms,
                files: entries,
            })
        })();
        let manifest = match manifest_res {
            Ok(m) => m,
            Err(err) => {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(err);
            }
        };

        if let Err(err) = fs::rename(&staging_dir, &gen_dir) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(err).with_context(|| {
                format!(
                    "publish immutable parquet generation dir {}",
                    gen_dir.display()
                )
            });
        }

        let manifest_dest = dir.join("manifest.json");
        let manifest_tmp = store::atomic_temp_path(&manifest_dest);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        if let Err(err) = fs::write(&manifest_tmp, manifest_bytes)
            .and_then(|_| fs::rename(&manifest_tmp, &manifest_dest))
        {
            let _ = fs::remove_file(&manifest_tmp);
            let _ = fs::remove_dir_all(&gen_dir);
            return Err(err)
                .with_context(|| format!("atomically update {}", manifest_dest.display()));
        }

        self.refresh_parquet_views(paths)?;
        cleanup_old_parquet_generations(&dir, &generation_id, prev_generation.as_deref());
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn replace_sec_lake_data(
        &mut self,
        companies: &[LakeCompanyRow],
        annuals: &[LakeAnnualRow],
        ttms: &[LakeTtmRow],
        snapshot: &[UniverseRow],
    ) -> Result<()> {
        let empty_meta = HashMap::new();
        let empty_quotes = HashMap::new();
        let exp = SecGenerationExpectations {
            expected_prev_sec_gen: None,
            expected_prev_assumptions_rev: None,
            batch_synced_at_ms: 0,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        self.replace_sec_lake_data_inner(
            companies,
            annuals,
            ttms,
            snapshot,
            &empty_meta,
            &[],
            &exp,
            false,
            false,
            |_| Ok(()),
        )
    }

    #[cfg(test)]
    pub(crate) fn replace_sec_lake_data_with_watermarks(
        &mut self,
        companies: &[LakeCompanyRow],
        annuals: &[LakeAnnualRow],
        ttms: &[LakeTtmRow],
        snapshot: &[UniverseRow],
        watermarks: &[SyncWatermark],
    ) -> Result<()> {
        let empty_meta = HashMap::new();
        let empty_quotes = HashMap::new();
        let exp = SecGenerationExpectations {
            expected_prev_sec_gen: None,
            expected_prev_assumptions_rev: None,
            batch_synced_at_ms: 0,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        self.replace_sec_lake_data_inner(
            companies,
            annuals,
            ttms,
            snapshot,
            &empty_meta,
            watermarks,
            &exp,
            false,
            true,
            |_| Ok(()),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn replace_sec_lake_data_with_quotes_and_watermarks(
        &mut self,
        companies: &[LakeCompanyRow],
        annuals: &[LakeAnnualRow],
        ttms: &[LakeTtmRow],
        snapshot: &[UniverseRow],
        quote_times_by_cik: &HashMap<String, u64>,
        watermarks: &[SyncWatermark],
    ) -> Result<()> {
        let meta: HashMap<String, SecSnapshotMeta> = quote_times_by_cik
            .iter()
            .map(|(cik, &ms)| {
                (
                    cik.clone(),
                    SecSnapshotMeta {
                        quote_market_time_ms: Some(ms),
                        quote_fetched_at_ms: None,
                        assumptions_digest: None,
                        assumptions: None,
                        extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                        quality_score_override: None,
                    },
                )
            })
            .collect();
        let empty_quotes = HashMap::new();
        let exp = SecGenerationExpectations {
            expected_prev_sec_gen: None,
            expected_prev_assumptions_rev: None,
            batch_synced_at_ms: 0,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        self.replace_sec_lake_data_inner(
            companies,
            annuals,
            ttms,
            snapshot,
            &meta,
            watermarks,
            &exp,
            false,
            true,
            |_| Ok(()),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn replace_sec_lake_data_with_meta_and_commit_hook<F>(
        &mut self,
        companies: &[LakeCompanyRow],
        annuals: &[LakeAnnualRow],
        ttms: &[LakeTtmRow],
        snapshot: &[UniverseRow],
        snapshot_meta_by_cik: &HashMap<String, SecSnapshotMeta>,
        watermarks: &[SyncWatermark],
        pre_commit: F,
    ) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        let empty_quotes = HashMap::new();
        let exp = SecGenerationExpectations {
            expected_prev_sec_gen: None,
            expected_prev_assumptions_rev: None,
            batch_synced_at_ms: 0,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        self.replace_sec_lake_data_inner(
            companies,
            annuals,
            ttms,
            snapshot,
            snapshot_meta_by_cik,
            watermarks,
            &exp,
            false,
            true,
            |_| pre_commit(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn replace_sec_lake_generation_with_meta_and_commit_hook<F>(
        &mut self,
        companies: &[LakeCompanyRow],
        annuals: &[LakeAnnualRow],
        ttms: &[LakeTtmRow],
        snapshot: &[UniverseRow],
        snapshot_meta_by_cik: &HashMap<String, SecSnapshotMeta>,
        watermarks: &[SyncWatermark],
        expectations: &SecGenerationExpectations<'_>,
        pre_commit: F,
    ) -> Result<()>
    where
        F: FnOnce(&HashMap<String, Vec<Option<String>>>) -> Result<()>,
    {
        self.replace_sec_lake_data_inner(
            companies,
            annuals,
            ttms,
            snapshot,
            snapshot_meta_by_cik,
            watermarks,
            expectations,
            true,
            true,
            pre_commit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn replace_sec_lake_data_inner<F>(
        &mut self,
        companies: &[LakeCompanyRow],
        annuals: &[LakeAnnualRow],
        ttms: &[LakeTtmRow],
        snapshot: &[UniverseRow],
        snapshot_meta_by_cik: &HashMap<String, SecSnapshotMeta>,
        watermarks: &[SyncWatermark],
        expectations: &SecGenerationExpectations<'_>,
        enforce_batch_time: bool,
        require_companies: bool,
        pre_commit: F,
    ) -> Result<()>
    where
        F: FnOnce(&HashMap<String, Vec<Option<String>>>) -> Result<()>,
    {
        ensure!(
            !snapshot.is_empty() && (!require_companies || !companies.is_empty()),
            "refusing to replace SEC lakehouse tables with an empty companies/snapshot batch"
        );
        for c in companies {
            ensure!(!c.cik.trim().is_empty(), "empty CIK in staged companies");
        }
        for u in snapshot {
            ensure!(!u.cik.trim().is_empty(), "empty CIK in staged snapshot");
        }

        let tx = self.conn.transaction()?;
        let live_sec_gen = read_sec_generation_on_conn(&tx)?;
        if let Some(expected_gen) = expectations.expected_prev_sec_gen {
            ensure!(
                live_sec_gen == expected_gen,
                "SEC lakehouse generation advanced concurrently from {expected_gen:?} to {live_sec_gen:?}; aborting stale SEC sync"
            );
        }
        if enforce_batch_time && let Some((_, live_ms)) = live_sec_gen {
            ensure!(
                expectations.batch_synced_at_ms >= live_ms,
                "refusing to overwrite newer SEC lakehouse generation ({live_ms}) with older sync ({})",
                expectations.batch_synced_at_ms
            );
        }
        let live_assumptions_rev = read_assumptions_revision_on_conn(&tx)?;
        if let Some(exp_rev) = expectations.expected_prev_assumptions_rev {
            ensure!(
                live_assumptions_rev == exp_rev,
                "assumptions revision advanced concurrently from {exp_rev} to {live_assumptions_rev}; aborting stale SEC commit"
            );
        }
        if !watermarks.is_empty() {
            let mut read_wm_stmt = tx
                .prepare("SELECT cursor, synced_at_ms FROM sync_watermarks WHERE source_key = ?")?;
            for wm in watermarks {
                let existing: Option<(Option<String>, u64)> = read_wm_stmt
                    .query_row([&wm.source_key], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()?;
                if let Some((ex_cursor, ex_ms)) = existing {
                    ensure!(
                        wm.synced_at_ms >= ex_ms,
                        "refusing to overwrite newer SEC watermark {} ({ex_ms}) with older sync ({})",
                        wm.source_key,
                        wm.synced_at_ms
                    );
                    if wm.source_key == WM_SEC_EXTRACTOR_VERSION
                        && let (Some(ec), Some(nc)) = (
                            ex_cursor.as_deref().and_then(|s| s.parse::<u32>().ok()),
                            wm.cursor.as_deref().and_then(|s| s.parse::<u32>().ok()),
                        )
                    {
                        ensure!(
                            nc >= ec,
                            "refusing to downgrade SEC extractor watermark cursor {ec} -> {nc}"
                        );
                    }
                }
            }
        }

        let quote_as_of_ms = if expectations.batch_synced_at_ms > 0 {
            expectations.batch_synced_at_ms
        } else {
            store::now_ms()
        };

        struct LiveDbQuoteState {
            ticker: String,
            latest_price: Decimal,
            quote_market_time_ms: Option<u64>,
            quote_fetched_at_ms: Option<u64>,
        }
        let mut live_quotes_by_cik: HashMap<String, LiveDbQuoteState> = HashMap::new();
        {
            let mut stmt = tx.prepare(
                "SELECT cik, ticker, latest_price, quote_market_time_ms, quote_fetched_at_ms, exact_decimals_json
                 FROM universe_snapshot
                 WHERE latest_price IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<Decimal>>(2)?,
                    r.get::<_, Option<u64>>(3)?,
                    r.get::<_, Option<u64>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            for r in rows {
                let (cik, ticker, sql_price, mkt_ms, fetch_ms, exact_json) = r?;
                let exact_opt = decode_universe_exact_bytes(exact_json.as_bytes());
                let price_opt = exact_opt
                    .as_ref()
                    .and_then(|ex| ex.latest_price)
                    .or(sql_price);
                let eff_mkt = exact_opt
                    .as_ref()
                    .and_then(|ex| ex.quote_market_time_ms)
                    .or(mkt_ms);
                if let Some(price) = price_opt
                    && is_valid_quote_candidate(&ticker, price, eff_mkt, fetch_ms, quote_as_of_ms)
                {
                    live_quotes_by_cik.insert(
                        cik,
                        LiveDbQuoteState {
                            ticker,
                            latest_price: price,
                            quote_market_time_ms: eff_mkt,
                            quote_fetched_at_ms: fetch_ms,
                        },
                    );
                }
            }
        }

        let mut live_annual_ends: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt =
                tx.prepare("SELECT cik, period_end FROM facts_annual ORDER BY cik, period_end")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                let (cik, period_end) = r?;
                live_annual_ends.entry(cik).or_default().push(period_end);
            }
        }
        let mut live_ttm_ends: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT cik, period_end FROM facts_ttm")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                let (cik, period_end) = r?;
                live_ttm_ends.insert(cik, period_end);
            }
        }
        let mut live_prev_coverage: HashMap<String, (usize, Option<String>, bool, Option<String>)> =
            HashMap::new();
        {
            let mut stmt = tx.prepare(
                "SELECT u.cik, u.years_of_data, u.latest_filing_end, u.raw_sha256,
                        EXISTS(SELECT 1 FROM facts_ttm t WHERE t.cik = u.cik)
                 FROM universe_snapshot u
                 WHERE u.years_of_data > 0",
            )?;
            let rows = stmt.query_map([], |r| {
                let cik: String = r.get(0)?;
                let yrs_i32: i32 = r.get(1)?;
                let end: Option<String> = r.get(2)?;
                let raw_sha: Option<String> = r.get(3)?;
                let has_ttm: bool = r.get(4)?;
                Ok((
                    cik,
                    usize::try_from(yrs_i32.max(0)).unwrap_or(0),
                    end,
                    raw_sha,
                    has_ttm,
                ))
            })?;
            for r in rows {
                let (cik, yrs, end, raw_sha, has_ttm) = r?;
                live_prev_coverage.insert(cik, (yrs, end, has_ttm, raw_sha));
            }
        }
        let mut live_fact_sha: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = tx.prepare(
                "SELECT cik, raw_sha256 FROM facts_annual
                 QUALIFY ROW_NUMBER() OVER (PARTITION BY cik ORDER BY fiscal_year DESC, period_end DESC) = 1",
            )?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for r in rows {
                let (cik, sha) = r?;
                live_fact_sha.insert(cik, sha);
            }
        }

        let mut new_annual_ends_by_cik: HashMap<&str, Vec<String>> = HashMap::new();
        for a in annuals {
            new_annual_ends_by_cik
                .entry(a.cik.as_str())
                .or_default()
                .push(a.period_end.clone());
        }
        let mut new_ttm_end_by_cik: HashMap<&str, &str> = HashMap::new();
        for t in ttms {
            new_ttm_end_by_cik.insert(t.cik.as_str(), t.period_end.as_str());
        }
        let ttm_ciks_in_batch: HashSet<&str> = new_ttm_end_by_cik.keys().copied().collect();

        // Preserve previously verified facts for any incoming issuer whose replacement failed/skipped
        // OR regressed in annual period keys, TTM presence/date, or latest filing end date.
        let mut preserved_ciks: HashSet<String> = HashSet::new();
        let mut fallback_preserved_sha: HashMap<String, Option<String>> = HashMap::new();
        for u in snapshot {
            let is_unparsed = u.years_of_data == 0 && u.raw_sha256.is_none();
            let regressed_from_annuals = live_annual_ends.get(&u.cik).is_some_and(|prev_ends| {
                let new_ends = new_annual_ends_by_cik
                    .get(u.cik.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                crate::edgar::is_annual_period_coverage_regression(
                    prev_ends,
                    live_ttm_ends.get(&u.cik).map(String::as_str),
                    new_ends,
                    new_ttm_end_by_cik.get(u.cik.as_str()).copied(),
                    u.latest_filing_end.as_deref(),
                )
            });
            let regressed_from_snap = live_prev_coverage.get(&u.cik).is_some_and(
                |(prev_years, prev_end, prev_has_ttm, _)| {
                    is_unparsed
                        || is_issuer_coverage_regression(
                            *prev_years,
                            *prev_has_ttm,
                            prev_end.as_deref(),
                            u.years_of_data,
                            ttm_ciks_in_batch.contains(u.cik.as_str()),
                            u.latest_filing_end.as_deref(),
                        )
                },
            );
            if (is_unparsed
                && (live_prev_coverage.contains_key(&u.cik)
                    || live_annual_ends.contains_key(&u.cik)))
                || regressed_from_annuals
                || regressed_from_snap
            {
                let sha = live_prev_coverage
                    .get(&u.cik)
                    .and_then(|(_, _, _, s)| s.clone())
                    .or_else(|| live_fact_sha.get(&u.cik).cloned());
                preserved_ciks.insert(u.cik.clone());
                fallback_preserved_sha.insert(u.cik.clone(), sha);
            }
        }

        let cur_assumptions_rev = read_assumptions_revision_on_conn(&tx)?.max(1);
        let next_sec_gen = live_sec_gen.map_or(1, |(g, _)| g + 1);
        let sec_gen_ms = if enforce_batch_time {
            expectations.batch_synced_at_ms
        } else {
            store::now_ms()
        };

        // Build _incoming_ciks and _preserve_ciks so inactive issuers retain companies(is_active=FALSE),
        // facts_annual, and facts_ttm while universe_snapshot mirrors the active listed universe.
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS _incoming_ciks (cik VARCHAR PRIMARY KEY);
             DELETE FROM _incoming_ciks;
             CREATE TEMP TABLE IF NOT EXISTS _preserve_ciks (cik VARCHAR PRIMARY KEY);
             DELETE FROM _preserve_ciks;",
        )?;
        let mut incoming_ciks: HashSet<&str> =
            HashSet::with_capacity(companies.len() + snapshot.len());
        {
            let mut ins_in = tx.prepare("INSERT OR IGNORE INTO _incoming_ciks (cik) VALUES (?)")?;
            for c in companies {
                incoming_ciks.insert(c.cik.as_str());
                ins_in.execute([&c.cik])?;
            }
            for u in snapshot {
                incoming_ciks.insert(u.cik.as_str());
                ins_in.execute([&u.cik])?;
            }
            let mut ins_pres = tx.prepare("INSERT INTO _preserve_ciks (cik) VALUES (?)")?;
            for cik in &preserved_ciks {
                ins_pres.execute([cik])?;
            }
        }
        if !companies.is_empty() {
            let prev_active_count: usize = tx
                .query_row(
                    "SELECT COUNT(*) FROM companies WHERE is_active = TRUE",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if prev_active_count >= 10 {
                let retained_active: usize = tx.query_row(
                    "SELECT COUNT(*) FROM companies
                     WHERE is_active = TRUE
                       AND cik IN (SELECT cik FROM _incoming_ciks)",
                    [],
                    |r| r.get(0),
                )?;
                ensure!(
                    retained_active * 100 >= prev_active_count * 85,
                    "SEC lakehouse replacement retained only {retained_active}/{prev_active_count} active companies (<85%); refusing mass deactivation"
                );
            }
        }
        tx.execute_batch(
            "UPDATE companies SET is_active = FALSE WHERE cik NOT IN (SELECT cik FROM _incoming_ciks);
             DELETE FROM companies WHERE cik IN (SELECT cik FROM _incoming_ciks);
             DELETE FROM facts_annual WHERE cik IN (SELECT cik FROM _incoming_ciks) AND cik NOT IN (SELECT cik FROM _preserve_ciks);
             DELETE FROM facts_ttm WHERE cik IN (SELECT cik FROM _incoming_ciks) AND cik NOT IN (SELECT cik FROM _preserve_ciks);
             DELETE FROM universe_snapshot WHERE cik NOT IN (SELECT cik FROM _preserve_ciks);",
        )?;

        {
            let mut app = tx.appender("companies")?;
            let mut seen_ciks = HashSet::with_capacity(companies.len());
            for c in companies {
                if !seen_ciks.insert(c.cik.as_str()) {
                    continue;
                }
                let sic_i32 = c.sic.and_then(|s| i32::try_from(s).ok());
                app.append_row(params![
                    c.cik,
                    c.ticker,
                    c.name,
                    c.exchange,
                    sic_i32,
                    c.sic_description,
                    c.is_financial,
                    c.is_active
                ])?;
            }
            app.flush()?;
        }

        {
            let mut app = tx.appender("facts_annual")?;
            let mut seen_keys = HashSet::with_capacity(annuals.len());
            for a in annuals {
                if !incoming_ciks.contains(a.cik.as_str()) || preserved_ciks.contains(&a.cik) {
                    continue;
                }
                if !seen_keys.insert((a.cik.as_str(), a.fiscal_year, a.period_end.as_str())) {
                    continue;
                }
                let (tax_absent, sbc_absent, buybacks_absent, debt_absent, cash_absent) =
                    if a.provenance_json.contains("\"absent\"") {
                        let prov: Vec<FieldProvenance> =
                            serde_json::from_str(&a.provenance_json).unwrap_or_default();
                        (
                            calculator::has_absent_provenance(&prov, &["tax_expense"]),
                            calculator::has_absent_provenance(&prov, &["sbc"]),
                            calculator::has_absent_provenance(&prov, &["buybacks"]),
                            calculator::has_absent_provenance(&prov, &["total_debt"]),
                            calculator::has_absent_provenance(&prov, &["cash_and_st"]),
                        )
                    } else {
                        (false, false, false, false, false)
                    };
                let roic_avail = !tax_absent && !debt_absent && !cash_absent;
                let tot_cap_roic_avail = !tax_absent && !debt_absent;
                let nopat = (!tax_absent).then_some(a.nopat).flatten();
                let invested_capital = (!debt_absent && !cash_absent)
                    .then_some(a.invested_capital)
                    .flatten();
                let roic_pct = roic_avail.then_some(a.roic_pct).flatten();
                let roic_total_capital_pct = tot_cap_roic_avail
                    .then_some(a.roic_total_capital_pct)
                    .flatten();
                let sbc = (!sbc_absent).then_some(a.sbc).flatten();
                let strict_owner_earnings =
                    (!sbc_absent).then_some(a.strict_owner_earnings).flatten();
                let sbc_to_ocf_pct = (!sbc_absent).then_some(a.sbc_to_ocf_pct).flatten();
                let buybacks = (!buybacks_absent).then_some(a.buybacks).flatten();
                let cash_and_st = (!cash_absent).then_some(a.cash_and_st).flatten();
                let total_debt = (!debt_absent).then_some(a.total_debt).flatten();
                let net_debt = (!debt_absent && !cash_absent)
                    .then_some(a.net_debt)
                    .flatten();
                let exact = AnnualExactDecimals {
                    revenue: a.revenue,
                    gross_profit: a.gross_profit,
                    gross_margin_pct: a.gross_margin_pct,
                    ebit: a.ebit,
                    nopat,
                    invested_capital,
                    roic_pct,
                    roic_total_capital_pct,
                    ocf: a.ocf,
                    capex: a.capex,
                    fcf: a.fcf,
                    sbc,
                    strict_owner_earnings,
                    sbc_to_ocf_pct,
                    buybacks,
                    cash_and_st,
                    total_debt,
                    net_debt,
                    equity: a.equity,
                    diluted_shares: a.diluted_shares,
                    ending_shares: a.ending_shares,
                };
                let exact_json = serde_json::to_string(&exact)?;
                app.append_row(params![
                    a.cik,
                    a.ticker,
                    a.fiscal_year,
                    a.label,
                    a.period_end,
                    a.filed,
                    money_dec(a.revenue),
                    money_dec(a.gross_profit),
                    opt_ratio_dec(a.gross_margin_pct),
                    money_dec(a.ebit),
                    opt_money_dec(nopat),
                    opt_money_dec(invested_capital),
                    opt_ratio_dec(roic_pct),
                    a.roic_note,
                    opt_ratio_dec(roic_total_capital_pct),
                    money_dec(a.ocf),
                    money_dec(a.capex),
                    money_dec(a.fcf),
                    opt_money_dec(sbc),
                    opt_money_dec(strict_owner_earnings),
                    opt_ratio_dec(sbc_to_ocf_pct),
                    opt_money_dec(buybacks),
                    opt_money_dec(cash_and_st),
                    opt_money_dec(total_debt),
                    opt_money_dec(net_debt),
                    money_dec(a.equity),
                    money_dec(a.diluted_shares),
                    money_dec(a.ending_shares),
                    a.raw_sha256,
                    a.provenance_json,
                    exact_json
                ])?;
            }
            app.flush()?;
        }

        {
            let mut app = tx.appender("facts_ttm")?;
            let mut seen_ttm = HashSet::with_capacity(ttms.len());
            for t in ttms {
                if !incoming_ciks.contains(t.cik.as_str()) || preserved_ciks.contains(&t.cik) {
                    continue;
                }
                if !seen_ttm.insert(t.cik.as_str()) {
                    continue;
                }
                let ttm_cash_absent = t.cash_and_st.is_none()
                    || (t.provenance_json.contains("\"absent\"")
                        && serde_json::from_str::<Vec<FieldProvenance>>(&t.provenance_json)
                            .ok()
                            .is_some_and(|prov| {
                                calculator::has_absent_provenance(
                                    &prov,
                                    &["ttm_cash_and_st", "cash_and_st"],
                                )
                            }));
                let cash_and_st = (!ttm_cash_absent).then_some(t.cash_and_st).flatten();
                let invested_capital = (!ttm_cash_absent).then_some(t.invested_capital).flatten();
                let roic_pct = (!ttm_cash_absent).then_some(t.roic_pct).flatten();
                let net_debt = (!ttm_cash_absent).then_some(t.net_debt).flatten();
                let exact_ttm = TtmExactDecimals {
                    revenue: t.revenue,
                    gross_profit: t.gross_profit,
                    gross_margin_pct: t.gross_margin_pct,
                    ebit: t.ebit,
                    nopat: t.nopat,
                    invested_capital,
                    roic_pct,
                    roic_total_capital_pct: t.roic_total_capital_pct,
                    ocf: t.ocf,
                    capex: t.capex,
                    fcf: t.fcf,
                    sbc: t.sbc,
                    strict_owner_earnings: t.strict_owner_earnings,
                    sbc_to_ocf_pct: t.sbc_to_ocf_pct,
                    buybacks: t.buybacks,
                    cash_and_st,
                    total_debt: t.total_debt,
                    net_debt,
                    equity: t.equity,
                    diluted_shares: t.diluted_shares,
                    ytd_revenue: t.ytd_revenue,
                    ytd_ocf: t.ytd_ocf,
                    ytd_capex: t.ytd_capex,
                    ytd_sbc: t.ytd_sbc,
                    ytd_buybacks: t.ytd_buybacks,
                    latest_quarter_label: t.latest_quarter_label.clone(),
                    latest_quarter_ocf: t.latest_quarter_ocf,
                    latest_quarter_capex: t.latest_quarter_capex,
                };
                let exact_json = serde_json::to_string(&exact_ttm)?;
                app.append_row(params![
                    t.cik,
                    t.ticker,
                    t.label,
                    t.period_end,
                    money_dec(t.revenue),
                    money_dec(t.gross_profit),
                    opt_ratio_dec(t.gross_margin_pct),
                    money_dec(t.ebit),
                    opt_money_dec(t.nopat),
                    opt_money_dec(invested_capital),
                    opt_ratio_dec(roic_pct),
                    t.roic_note,
                    opt_ratio_dec(t.roic_total_capital_pct),
                    money_dec(t.ocf),
                    money_dec(t.capex),
                    money_dec(t.fcf),
                    opt_money_dec(t.sbc),
                    opt_money_dec(t.strict_owner_earnings),
                    opt_ratio_dec(t.sbc_to_ocf_pct),
                    opt_money_dec(t.buybacks),
                    opt_money_dec(cash_and_st),
                    opt_money_dec(t.total_debt),
                    opt_money_dec(net_debt),
                    money_dec(t.equity),
                    money_dec(t.diluted_shares),
                    t.ytd_period_label,
                    t.ytd_start,
                    t.ytd_end,
                    opt_money_dec(t.ytd_revenue),
                    opt_money_dec(t.ytd_ocf),
                    opt_money_dec(t.ytd_capex),
                    opt_money_dec(t.ytd_sbc),
                    opt_money_dec(t.ytd_buybacks),
                    t.raw_sha256,
                    t.provenance_json,
                    exact_json,
                    t.latest_quarter_label,
                    opt_money_dec(t.latest_quarter_ocf),
                    opt_money_dec(t.latest_quarter_capex)
                ])?;
            }
            app.flush()?;
        }

        let mut actual_preserved_with_shas: HashMap<String, Vec<Option<String>>> = HashMap::new();
        {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT cik, raw_sha256
                 FROM (
                     SELECT cik, raw_sha256 FROM facts_annual
                     WHERE cik IN (SELECT cik FROM _preserve_ciks)
                        OR cik NOT IN (SELECT cik FROM _incoming_ciks)
                     UNION ALL
                     SELECT cik, raw_sha256 FROM facts_ttm
                     WHERE cik IN (SELECT cik FROM _preserve_ciks)
                        OR cik NOT IN (SELECT cik FROM _incoming_ciks)
                 )",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for r in rows {
                let (cik, sha_opt) = r?;
                let norm = sha_opt.and_then(|s| {
                    let t = s.trim();
                    (!t.is_empty()).then(|| t.to_owned())
                });
                let entry = actual_preserved_with_shas.entry(cik).or_default();
                if !entry.contains(&norm) {
                    entry.push(norm);
                }
            }
        }
        for cik in &preserved_ciks {
            let entry = actual_preserved_with_shas.entry(cik.clone()).or_default();
            if entry.is_empty() {
                let fb = fallback_preserved_sha.get(cik).cloned().flatten();
                entry.push(fb);
            }
        }

        let validate_staged_quotes = expectations.batch_synced_at_ms > 0;
        let pick_winning_quote = |u: &UniverseRow,
                                  meta_opt: Option<&SecSnapshotMeta>,
                                  allow_live_db: bool,
                                  prev_live: Option<(Decimal, Option<u64>, Option<u64>)>,
                                  has_mismatch: bool|
         -> (
            Option<Decimal>,
            Option<u64>,
            Option<u64>,
            bool,
            Option<&crate::model::PriceSnapshot>,
        ) {
            if !u.in_price_pool || has_mismatch {
                return (None, None, None, false, None);
            }
            let staged_price = u.latest_price;
            let staged_mkt =
                staged_price.and_then(|_| meta_opt.and_then(|m| m.quote_market_time_ms));
            let staged_fetch =
                staged_price.and_then(|_| meta_opt.and_then(|m| m.quote_fetched_at_ms));
            let staged_valid = match staged_price {
                Some(sp) if validate_staged_quotes => is_valid_quote_candidate(
                    &u.ticker,
                    sp,
                    staged_mkt,
                    staged_fetch,
                    quote_as_of_ms,
                ),
                _ => true,
            };
            let (mut best_price, mut best_mkt, mut best_fetch, mut changed_vs_staged) =
                if staged_valid {
                    (staged_price, staged_mkt, staged_fetch, false)
                } else {
                    (None, None, None, staged_price.is_some())
                };

            let mut consider = |cand_p: Decimal, cand_mkt: Option<u64>, cand_fetch: Option<u64>| {
                if !is_valid_quote_candidate(
                    &u.ticker,
                    cand_p,
                    cand_mkt,
                    cand_fetch,
                    quote_as_of_ms,
                ) {
                    return;
                }
                let is_better = match best_price {
                    None => true,
                    Some(_) => {
                        (cand_mkt.unwrap_or(0), cand_fetch.unwrap_or(0))
                            > (best_mkt.unwrap_or(0), best_fetch.unwrap_or(0))
                    }
                };
                if is_better {
                    best_price = Some(cand_p);
                    best_mkt = cand_mkt;
                    best_fetch = cand_fetch;
                    changed_vs_staged = true;
                }
            };

            let disk_cand = expectations
                .refreshed_disk_quotes
                .get(&u.cik)
                .filter(|dp| dp.symbol.eq_ignore_ascii_case(&u.ticker));
            if let Some(dp) = disk_cand {
                consider(dp.price, dp.market_time_ms, Some(dp.fetched_at_ms));
            }
            if allow_live_db {
                if let Some((lp, lm, lf)) = prev_live {
                    consider(lp, lm, lf);
                } else if let Some(lq) = live_quotes_by_cik.get(&u.cik)
                    && lq.ticker.eq_ignore_ascii_case(&u.ticker)
                {
                    consider(
                        lq.latest_price,
                        lq.quote_market_time_ms,
                        lq.quote_fetched_at_ms,
                    );
                }
            }
            let adopted_disk = disk_cand.filter(|dp| {
                best_price == Some(dp.price)
                    && best_mkt == dp.market_time_ms
                    && best_fetch == Some(dp.fetched_at_ms)
            });
            (
                best_price,
                best_mkt,
                best_fetch,
                changed_vs_staged,
                adopted_disk,
            )
        };

        let needs_basis_for_fresh = snapshot.iter().any(|u| {
            !preserved_ciks.contains(&u.cik)
                && u.in_price_pool
                && !u.warnings_json.contains("\"SHARE_BASIS_MISMATCH\"")
                && !crate::calculator::is_share_basis_mismatched(&u.ticker, false)
                && pick_winning_quote(u, snapshot_meta_by_cik.get(&u.cik), true, None, false).3
        });
        let basis_map_shared = if needs_basis_for_fresh || !preserved_ciks.is_empty() {
            Some(load_latest_valuation_basis_on_conn(&tx)?)
        } else {
            None
        };
        let default_assump = default_valuation_assumptions();
        let mut adopted_disk_prices: Vec<LakePriceDailyRow> = Vec::new();
        let mut push_adopted_disk_price = |dp: &crate::model::PriceSnapshot| {
            if crate::market::validate_quote_session(dp, quote_as_of_ms).is_ok()
                && let Some(trade_date) = crate::market::trade_date_from_snapshot(dp)
            {
                adopted_disk_prices.push(LakePriceDailyRow {
                    symbol: dp.symbol.clone(),
                    trade_date,
                    price: dp.price,
                    previous_close: dp.previous_close,
                    currency: dp.currency.clone(),
                    exchange: dp.exchange.clone(),
                    market_time_ms: dp.market_time_ms,
                    fetched_at_ms: dp.fetched_at_ms,
                    source_url: dp.source_url.clone(),
                });
            }
        };

        {
            let mut app = tx.appender("universe_snapshot")?;
            let mut seen_snap = HashSet::with_capacity(snapshot.len());
            let mut preserved_updates: Vec<&UniverseRow> = Vec::new();
            for u in snapshot {
                if preserved_ciks.contains(&u.cik) {
                    preserved_updates.push(u);
                    continue;
                }
                if !seen_snap.insert(u.cik.as_str()) {
                    continue;
                }
                let meta_opt = snapshot_meta_by_cik.get(&u.cik);
                let has_mismatch = u.warnings_json.contains("\"SHARE_BASIS_MISMATCH\"")
                    || crate::calculator::is_share_basis_mismatched(&u.ticker, false);
                let (best_price, quote_mkt_ms, quote_fetch_ms, quote_upgraded, adopted_disk) =
                    pick_winning_quote(u, meta_opt, true, None, has_mismatch);
                if let Some(dp) = adopted_disk {
                    push_adopted_disk_price(dp);
                }
                let mut u_owned;
                let row_ref = if quote_upgraded {
                    u_owned = u.clone();
                    u_owned.latest_price = best_price;
                    let assump = meta_opt
                        .and_then(|m| m.assumptions.as_ref())
                        .unwrap_or(&default_assump);
                    let (strike, dist, yld) = recompute_valuation_fields_for_row(
                        &u_owned.ticker,
                        basis_map_shared.as_ref().and_then(|m| m.get(&u_owned.cik)),
                        best_price,
                        quote_mkt_ms,
                        assump,
                        has_mismatch,
                    );
                    u_owned.strike_price = strike;
                    u_owned.distance_to_strike_pct = dist;
                    u_owned.strict_fcf_yield_pct = yld;
                    &u_owned
                } else {
                    u
                };
                let assumptions_dig = meta_opt.and_then(|m| m.assumptions_digest.clone());
                let ext_ver = if row_ref.years_of_data > 0 && row_ref.raw_sha256.is_some() {
                    meta_opt.map_or(SEC_LAKE_EXTRACTOR_VERSION_NUM, |m| {
                        if m.extractor_version == 0 {
                            SEC_LAKE_EXTRACTOR_VERSION_NUM
                        } else {
                            m.extractor_version
                        }
                    })
                } else {
                    0
                };
                let ext_ver_i32 = i32::try_from(ext_ver).unwrap_or(i32::MAX);
                let exact_json = serialize_universe_exact(row_ref, quote_mkt_ms)?;
                let sic_i32 = row_ref.sic.and_then(|s| i32::try_from(s).ok());
                let years_i32 = i32::try_from(row_ref.years_of_data).unwrap_or(i32::MAX);
                let warn_i32 = i32::try_from(row_ref.warnings_count).unwrap_or(i32::MAX);
                let bucket_str = row_ref.bucket_opt.map(bucket_to_str);
                app.append_row(params![
                    row_ref.ticker,
                    row_ref.cik,
                    row_ref.name,
                    row_ref.exchange,
                    sic_i32,
                    row_ref.sic_description,
                    row_ref.is_financial,
                    row_ref.latest_filing_end,
                    years_i32,
                    money_dec(row_ref.quality_score),
                    row_ref.in_price_pool,
                    row_ref.in_watchlist,
                    bucket_str,
                    opt_money_dec(row_ref.latest_price),
                    opt_money_dec(row_ref.strike_price),
                    opt_ratio_dec(row_ref.distance_to_strike_pct),
                    opt_ratio_dec(row_ref.strict_fcf_yield_pct),
                    opt_ratio_dec(row_ref.roic_5y_avg_pct),
                    opt_ratio_dec(row_ref.roic_5y_stddev_pct),
                    opt_ratio_dec(row_ref.gross_margin_5y_avg_pct),
                    opt_ratio_dec(row_ref.gross_margin_5y_stddev_pct),
                    opt_ratio_dec(row_ref.share_change_3y_pct),
                    opt_ratio_dec(row_ref.sbc_to_ocf_pct),
                    opt_ratio_dec(row_ref.net_debt_to_oe),
                    opt_money_dec(row_ref.strict_owner_earnings),
                    warn_i32,
                    row_ref.warnings_json,
                    row_ref.raw_sha256,
                    exact_json,
                    quote_mkt_ms,
                    ext_ver_i32,
                    assumptions_dig,
                    quote_fetch_ms,
                    cur_assumptions_rev
                ])?;
            }
            app.flush()?;
            drop(app);
            if !preserved_updates.is_empty() {
                let mut read_prev_q = tx.prepare(
                    "SELECT ticker, strike_price, exact_decimals_json, quality_score, warnings_json,
                            assumptions_digest, COALESCE(assumptions_rev, 0), latest_price,
                            quote_market_time_ms, quote_fetched_at_ms
                     FROM universe_snapshot
                     WHERE cik = ?",
                )?;
                let mut upd = tx.prepare(
                    "UPDATE universe_snapshot
                     SET ticker = ?,
                         name = ?,
                         exchange = ?,
                         sic = ?,
                         sic_description = ?,
                         is_financial = ?,
                         in_price_pool = ?,
                         in_watchlist = ?,
                         bucket_opt = ?,
                         quality_score = ?,
                         warnings_count = ?,
                         warnings_json = ?,
                         assumptions_digest = COALESCE(?, assumptions_digest),
                         assumptions_rev = ?,
                         latest_price = ?,
                         strike_price = ?,
                         distance_to_strike_pct = ?,
                         strict_fcf_yield_pct = ?,
                         quote_market_time_ms = ?,
                         quote_fetched_at_ms = ?,
                         exact_decimals_json = ?
                     WHERE cik = ?",
                )?;
                let mut ins_relist = tx.prepare(
                    "INSERT INTO universe_snapshot (
                        ticker, cik, name, exchange, sic, sic_description, is_financial,
                        latest_filing_end, years_of_data, quality_score, in_price_pool, in_watchlist,
                        bucket_opt, latest_price, strike_price, distance_to_strike_pct,
                        strict_fcf_yield_pct, roic_5y_avg_pct, roic_5y_stddev_pct,
                        gross_margin_5y_avg_pct, gross_margin_5y_stddev_pct, share_change_3y_pct,
                        sbc_to_ocf_pct, net_debt_to_oe, strict_owner_earnings, warnings_count,
                        warnings_json, raw_sha256, exact_decimals_json, quote_market_time_ms,
                        extractor_version, assumptions_digest, quote_fetched_at_ms, assumptions_rev
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )?;
                let mut upd_ann_ticker =
                    tx.prepare("UPDATE facts_annual SET ticker = ? WHERE cik = ?")?;
                let mut upd_ttm_ticker =
                    tx.prepare("UPDATE facts_ttm SET ticker = ? WHERE cik = ?")?;
                let mut assumptions_rev_bumped = false;
                let bumped_assumptions_rev = cur_assumptions_rev + 1;
                struct PreservedPrevRow {
                    ticker: String,
                    strike_price: Option<Decimal>,
                    exact_decimals_json: String,
                    quality_score: Decimal,
                    warnings_json: String,
                    assumptions_digest: Option<String>,
                    assumptions_rev: u64,
                    latest_price: Option<Decimal>,
                    quote_market_time_ms: Option<u64>,
                    quote_fetched_at_ms: Option<u64>,
                }
                for u in preserved_updates {
                    let meta_opt = snapshot_meta_by_cik.get(&u.cik);
                    let prev_q: Option<PreservedPrevRow> = read_prev_q
                        .query_row([&u.cik], |row| {
                            Ok(PreservedPrevRow {
                                ticker: row.get(0)?,
                                strike_price: row.get(1)?,
                                exact_decimals_json: row.get(2)?,
                                quality_score: row.get(3)?,
                                warnings_json: row.get(4)?,
                                assumptions_digest: row.get(5)?,
                                assumptions_rev: row.get(6)?,
                                latest_price: row.get(7)?,
                                quote_market_time_ms: row.get(8)?,
                                quote_fetched_at_ms: row.get(9)?,
                            })
                        })
                        .optional()?;

                    let Some(prev) = prev_q else {
                        // Relisted issuer returning with partial/regressed data while retained facts survive:
                        // rebuild its full universe_snapshot row from retained facts_annual/facts_ttm and insert.
                        let is_non_common_listing =
                            u.warnings_json.contains("\"SHARE_BASIS_MISMATCH\"")
                                || crate::calculator::is_share_basis_mismatched(&u.ticker, false);
                        let (best_price, quote_mkt_ms, quote_fetch_ms, _, adopted_disk) =
                            pick_winning_quote(u, meta_opt, false, None, is_non_common_listing);
                        if let Some(dp) = adopted_disk {
                            push_adopted_disk_price(dp);
                        }
                        let price_snap_opt = best_price.map(|p| crate::model::PriceSnapshot {
                            schema_version: 1,
                            symbol: u.ticker.clone(),
                            price: p,
                            previous_close: None,
                            currency: "USD".to_owned(),
                            exchange: u.exchange.clone(),
                            market_time_ms: quote_mkt_ms,
                            fetched_at_ms: quote_fetch_ms.or(quote_mkt_ms).unwrap_or(sec_gen_ms),
                            source_url: String::new(),
                        });
                        let assump_for_relist = meta_opt
                            .and_then(|m| m.assumptions.as_ref())
                            .unwrap_or(&default_assump)
                            .clone();
                        let wl_entry_opt = u.in_watchlist.then(|| crate::model::CompanyEntry {
                            ticker: u.ticker.clone(),
                            name: u.name.clone(),
                            bucket: u.bucket_opt.unwrap_or(Bucket::Watch),
                            source: crate::model::DataSource::SecEdgar,
                            cik: Some(u.cik.clone()),
                            currency: "USD".to_owned(),
                            circle: "US Equity".to_owned(),
                            too_hard_reason: None,
                            assumptions: assump_for_relist.clone(),
                        });
                        let target = RetainedRebuildTarget {
                            cik: &u.cik,
                            ticker: &u.ticker,
                            name: &u.name,
                            exchange: &u.exchange,
                            sic: u.sic,
                            sic_description: u.sic_description.as_deref(),
                            in_price_pool: u.in_price_pool,
                            wl_entry: wl_entry_opt.as_ref(),
                            cached_price: price_snap_opt.as_ref(),
                            is_non_common_listing,
                            now_ms: sec_gen_ms,
                        };
                        let mut rebuilt = rebuild_universe_row_from_retained_conn(&tx, &target)?
                            .unwrap_or_else(|| u.clone());
                        rebuilt.in_price_pool = u.in_price_pool;
                        rebuilt.in_watchlist = u.in_watchlist;
                        rebuilt.bucket_opt = u.bucket_opt;
                        if rebuilt.is_financial {
                            rebuilt.quality_score = Decimal::ZERO;
                        }
                        let exact_json = serialize_universe_exact(&rebuilt, quote_mkt_ms)?;
                        let sic_i32 = rebuilt.sic.and_then(|s| i32::try_from(s).ok());
                        let years_i32 = i32::try_from(rebuilt.years_of_data).unwrap_or(i32::MAX);
                        let warn_i32 = i32::try_from(rebuilt.warnings_count).unwrap_or(i32::MAX);
                        let bucket_str = rebuilt.bucket_opt.map(bucket_to_str);
                        let ext_ver = meta_opt.map_or(0, |m| m.extractor_version);
                        let ext_ver_i32 = i32::try_from(ext_ver).unwrap_or(i32::MAX);
                        let dig = meta_opt
                            .and_then(|m| m.assumptions_digest.clone())
                            .unwrap_or_else(|| valuation_assumptions_digest(&default_assump));
                        ins_relist.execute(params![
                            rebuilt.ticker,
                            rebuilt.cik,
                            rebuilt.name,
                            rebuilt.exchange,
                            sic_i32,
                            rebuilt.sic_description,
                            rebuilt.is_financial,
                            rebuilt.latest_filing_end,
                            years_i32,
                            money_dec(rebuilt.quality_score),
                            rebuilt.in_price_pool,
                            rebuilt.in_watchlist,
                            bucket_str,
                            opt_money_dec(rebuilt.latest_price),
                            opt_money_dec(rebuilt.strike_price),
                            opt_ratio_dec(rebuilt.distance_to_strike_pct),
                            opt_ratio_dec(rebuilt.strict_fcf_yield_pct),
                            opt_ratio_dec(rebuilt.roic_5y_avg_pct),
                            opt_ratio_dec(rebuilt.roic_5y_stddev_pct),
                            opt_ratio_dec(rebuilt.gross_margin_5y_avg_pct),
                            opt_ratio_dec(rebuilt.gross_margin_5y_stddev_pct),
                            opt_ratio_dec(rebuilt.share_change_3y_pct),
                            opt_ratio_dec(rebuilt.sbc_to_ocf_pct),
                            opt_ratio_dec(rebuilt.net_debt_to_oe),
                            opt_money_dec(rebuilt.strict_owner_earnings),
                            warn_i32,
                            rebuilt.warnings_json,
                            rebuilt.raw_sha256,
                            exact_json,
                            quote_mkt_ms,
                            ext_ver_i32,
                            dig,
                            quote_fetch_ms,
                            cur_assumptions_rev
                        ])?;
                        upd_ann_ticker.execute(params![rebuilt.ticker, rebuilt.cik])?;
                        upd_ttm_ticker.execute(params![rebuilt.ticker, rebuilt.cik])?;
                        continue;
                    };

                    let sic_i32 = u.sic.and_then(|s| i32::try_from(s).ok());
                    let bucket_str = u.bucket_opt.map(bucket_to_str);
                    let ticker_unchanged = prev.ticker.eq_ignore_ascii_case(&u.ticker);
                    let merged_quality = if u.is_financial {
                        Decimal::ZERO
                    } else if let Some(q) = meta_opt.and_then(|m| m.quality_score_override) {
                        q
                    } else if u.quality_score > Decimal::ZERO {
                        u.quality_score
                    } else {
                        prev.quality_score
                    };
                    let (merged_warn_cnt, merged_warn_json, has_share_mismatch) =
                        reconcile_preserved_warnings_json(
                            &prev.warnings_json,
                            &u.warnings_json,
                            &u.ticker,
                            ticker_unchanged,
                            u.sic.is_none(),
                        );
                    let next_dig = meta_opt.and_then(|m| m.assumptions_digest.as_deref());
                    let assumptions_changed = match (next_dig, prev.assumptions_digest.as_deref()) {
                        (Some(nd), Some(pd)) => nd != pd,
                        (Some(_), None) => true,
                        _ => false,
                    };
                    let next_row_rev = if assumptions_changed {
                        assumptions_rev_bumped = true;
                        bumped_assumptions_rev
                    } else if prev.assumptions_rev > 0 {
                        prev.assumptions_rev
                    } else {
                        cur_assumptions_rev
                    };
                    let keep_quote = u.in_price_pool && ticker_unchanged && !has_share_mismatch;
                    let keep_strike = ticker_unchanged && !has_share_mismatch;
                    let prev_exact_opt =
                        decode_universe_exact_bytes(prev.exact_decimals_json.as_bytes());
                    let raw_prev_pair = prev_exact_opt
                        .as_ref()
                        .and_then(|ex| ex.latest_price)
                        .or(prev.latest_price)
                        .map(|lp| {
                            let lm = prev_exact_opt
                                .as_ref()
                                .and_then(|ex| ex.quote_market_time_ms)
                                .or(prev.quote_market_time_ms);
                            (lp, lm, prev.quote_fetched_at_ms)
                        });
                    let prev_live_pair = raw_prev_pair.filter(|(lp, lm, lf)| {
                        is_valid_quote_candidate(&u.ticker, *lp, *lm, *lf, quote_as_of_ms)
                    });
                    let (win_price, win_mkt_ms, win_fetch_ms, _, adopted_disk) = if keep_quote {
                        pick_winning_quote(u, meta_opt, true, prev_live_pair, has_share_mismatch)
                    } else {
                        (None, None, None, false, None)
                    };
                    if let Some(dp) = adopted_disk {
                        push_adopted_disk_price(dp);
                    }
                    let quote_differs_from_prev = if keep_quote {
                        match (win_price, raw_prev_pair) {
                            (Some(wp), Some((pp, pm, pf))) => {
                                wp != pp || win_mkt_ms != pm || win_fetch_ms != pf
                            }
                            (Some(_), None) | (None, Some(_)) => true,
                            (None, None) => false,
                        }
                    } else {
                        raw_prev_pair.is_some()
                    };
                    let (next_strike, next_dist, next_yield) = if !keep_strike {
                        (None, None, None)
                    } else if assumptions_changed || quote_differs_from_prev {
                        let assump = meta_opt
                            .and_then(|m| m.assumptions.as_ref())
                            .unwrap_or(&default_assump);
                        recompute_valuation_fields_for_row(
                            &u.ticker,
                            basis_map_shared.as_ref().and_then(|m| m.get(&u.cik)),
                            win_price,
                            win_mkt_ms,
                            assump,
                            false,
                        )
                    } else if keep_quote {
                        (
                            prev_exact_opt
                                .as_ref()
                                .and_then(|ex| ex.strike_price)
                                .or(prev.strike_price),
                            prev_exact_opt
                                .as_ref()
                                .and_then(|ex| ex.distance_to_strike_pct),
                            prev_exact_opt
                                .as_ref()
                                .and_then(|ex| ex.strict_fcf_yield_pct),
                        )
                    } else {
                        (
                            prev_exact_opt
                                .as_ref()
                                .and_then(|ex| ex.strike_price)
                                .or(prev.strike_price),
                            None,
                            None,
                        )
                    };
                    let next_exact_json = if let Some(mut ex) = prev_exact_opt {
                        ex.quality_score = merged_quality;
                        ex.latest_price = win_price;
                        ex.quote_market_time_ms = win_mkt_ms;
                        ex.strike_price = next_strike;
                        ex.distance_to_strike_pct = next_dist;
                        ex.strict_fcf_yield_pct = next_yield;
                        encode_universe_exact(&ex)
                    } else {
                        String::new()
                    };
                    let warn_i32 = i32::try_from(merged_warn_cnt).unwrap_or(i32::MAX);
                    upd.execute(params![
                        u.ticker,
                        u.name,
                        u.exchange,
                        sic_i32,
                        u.sic_description,
                        u.is_financial,
                        u.in_price_pool,
                        u.in_watchlist,
                        bucket_str,
                        money_dec(merged_quality),
                        warn_i32,
                        merged_warn_json,
                        next_dig,
                        next_row_rev,
                        opt_money_dec(win_price),
                        opt_money_dec(next_strike),
                        opt_ratio_dec(next_dist),
                        opt_ratio_dec(next_yield),
                        win_mkt_ms,
                        win_fetch_ms,
                        next_exact_json,
                        u.cik
                    ])?;
                    if !ticker_unchanged {
                        upd_ann_ticker.execute(params![u.ticker, u.cik])?;
                        upd_ttm_ticker.execute(params![u.ticker, u.cik])?;
                    }
                }
                if assumptions_rev_bumped {
                    write_assumptions_revision_on_conn(&tx, bumped_assumptions_rev, sec_gen_ms)?;
                }
            }
        }

        if !adopted_disk_prices.is_empty() {
            upsert_prices_daily_on_conn(&tx, &adopted_disk_prices)?;
        }

        if let Some(wl_opt) = expectations.post_parse_watchlist {
            let empty_wl = empty_watchlist_file();
            let wl_ref = wl_opt.unwrap_or(&empty_wl);
            if let Some(top_n) = expectations.recompute_prices_top {
                reconcile_watchlist_and_pool_on_conn(&tx, wl_ref, top_n, quote_as_of_ms)?;
            } else {
                refresh_watchlist_flags_on_conn(&tx, wl_ref)?;
                let mut live_rows = load_all_universe_rows_on_conn(&tx)?;
                let fallback_top = live_rows
                    .iter()
                    .filter(|r| r.in_price_pool && !r.in_watchlist)
                    .count();
                crate::universe::assign_scheme1_quality_pool(
                    &mut live_rows,
                    fallback_top,
                    quote_as_of_ms,
                );
                let pool_ciks: HashSet<String> = live_rows
                    .into_iter()
                    .filter(|r| r.in_price_pool)
                    .map(|r| r.cik)
                    .collect();
                sync_price_pool_flags_on_conn(&tx, &pool_ciks)?;
            }
        }

        if read_assumptions_revision_on_conn(&tx)? == 0 {
            write_assumptions_revision_on_conn(&tx, cur_assumptions_rev, sec_gen_ms)?;
        }

        let mut wm_stmt = tx.prepare(
            "INSERT OR REPLACE INTO sync_watermarks
             (source_key, cursor, etag, last_modified, synced_at_ms)
             VALUES (?, ?, ?, ?, ?)",
        )?;
        for wm in watermarks {
            wm_stmt.execute(params![
                wm.source_key,
                wm.cursor,
                wm.etag,
                wm.last_modified,
                wm.synced_at_ms
            ])?;
        }
        wm_stmt.execute(params![
            WM_SEC_GENERATION,
            format!("{next_sec_gen:020}"),
            Option::<String>::None,
            Option::<String>::None,
            sec_gen_ms
        ])?;
        drop(wm_stmt);

        pre_commit(&actual_preserved_with_shas)?;
        tx.commit()?;
        let _ = self.conn.execute_batch("CHECKPOINT;");
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn upsert_prices_and_update_snapshot(
        &mut self,
        prices: &[LakePriceDailyRow],
        updated_rows: &[QuoteSnapshotUpdate],
        watermarks: &[SyncWatermark],
    ) -> Result<()> {
        let empty = HashMap::new();
        self.upsert_prices_and_update_snapshot_with_assumptions(
            prices,
            updated_rows,
            &empty,
            watermarks,
        )
    }

    #[cfg(test)]
    pub(crate) fn upsert_prices_and_update_snapshot_with_assumptions(
        &mut self,
        prices: &[LakePriceDailyRow],
        updated_rows: &[QuoteSnapshotUpdate],
        expected_assumptions_digests: &HashMap<String, String>,
        watermarks: &[SyncWatermark],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        upsert_prices_and_update_snapshot_on_conn(
            &tx,
            prices,
            updated_rows,
            expected_assumptions_digests,
            &HashMap::new(),
            watermarks,
        )?;
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn sync_price_pool_flags(&mut self, pool_ciks: &HashSet<String>) -> Result<()> {
        self.sync_price_pool_flags_checked(pool_ciks, None)
    }

    pub(crate) fn sync_price_pool_flags_checked(
        &mut self,
        pool_ciks: &HashSet<String>,
        expected_sec_gen: Option<Option<(u64, u64)>>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        if let Some(expected_gen) = expected_sec_gen {
            let live_gen = read_sec_generation_on_conn(&tx)?;
            ensure!(
                live_gen == expected_gen,
                "SEC lakehouse generation advanced concurrently from {expected_gen:?} to {live_gen:?}; aborting stale price pool update"
            );
        }
        sync_price_pool_flags_on_conn(&tx, pool_ciks)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn load_all_universe_rows(&self) -> Result<Vec<UniverseRow>> {
        load_all_universe_rows_on_conn(&self.conn)
    }

    pub(crate) fn load_latest_valuation_basis(
        &self,
    ) -> Result<HashMap<String, (&'static str, AnnualMetricRow, calculator::ValuationGuards)>> {
        load_latest_valuation_basis_on_conn(&self.conn)
    }

    fn migrate_nullable_columns(&self) -> Result<()> {
        let targets: [(&str, &[&str]); 2] = [
            (
                "facts_annual",
                &[
                    "nopat",
                    "invested_capital",
                    "sbc",
                    "strict_owner_earnings",
                    "buybacks",
                    "cash_and_st",
                    "total_debt",
                    "net_debt",
                ],
            ),
            (
                "facts_ttm",
                &[
                    "nopat",
                    "invested_capital",
                    "sbc",
                    "strict_owner_earnings",
                    "buybacks",
                    "cash_and_st",
                    "total_debt",
                    "net_debt",
                    "ytd_revenue",
                    "ytd_ocf",
                    "ytd_capex",
                    "ytd_sbc",
                    "ytd_buybacks",
                ],
            ),
        ];
        for (table, cols) in targets {
            let mut stmt = self.conn.prepare(
                "SELECT column_name
                 FROM information_schema.columns
                 WHERE table_name = ? AND is_nullable = 'NO'",
            )?;
            let not_null_cols: HashSet<String> = stmt
                .query_map([table], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<HashSet<String>, _>>()?;
            for col in cols {
                if not_null_cols.contains(*col) {
                    self.conn
                        .execute_batch(&format!(
                            "ALTER TABLE {table} ALTER COLUMN {col} DROP NOT NULL;"
                        ))
                        .with_context(|| format!("drop NOT NULL on {table}.{col}"))?;
                }
            }
        }
        self.conn
            .execute_batch(
                "ALTER TABLE universe_snapshot ADD COLUMN IF NOT EXISTS quote_market_time_ms UBIGINT;
                 ALTER TABLE universe_snapshot ADD COLUMN IF NOT EXISTS extractor_version INTEGER;
                 ALTER TABLE universe_snapshot ADD COLUMN IF NOT EXISTS assumptions_digest VARCHAR;
                 ALTER TABLE universe_snapshot ADD COLUMN IF NOT EXISTS quote_fetched_at_ms UBIGINT;
                 ALTER TABLE universe_snapshot ADD COLUMN IF NOT EXISTS assumptions_rev UBIGINT;",
            )
            .context("add universe_snapshot migration columns")?;
        Ok(())
    }

    fn reconcile_prices_daily_trade_dates(&self) -> Result<()> {
        struct StoredPriceRow {
            symbol: String,
            trade_date: String,
            price: Decimal,
            previous_close: Option<Decimal>,
            currency: String,
            exchange: String,
            market_time_ms: u64,
            fetched_at_ms: u64,
            source_url: String,
        }

        let mut affected_originals: Vec<(String, String)> = Vec::new();
        let mut staged_winners: HashMap<(String, String), StoredPriceRow> = HashMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT symbol, trade_date, price, previous_close, currency, exchange, market_time_ms, fetched_at_ms, source_url
                 FROM prices_daily
                 WHERE market_time_ms IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(StoredPriceRow {
                    symbol: row.get(0)?,
                    trade_date: row.get(1)?,
                    price: row.get(2)?,
                    previous_close: row.get(3)?,
                    currency: row.get(4)?,
                    exchange: row.get(5)?,
                    market_time_ms: row.get(6)?,
                    fetched_at_ms: row.get(7)?,
                    source_url: row.get(8)?,
                })
            })?;
            for r in rows {
                let row = r?;
                if let Some(canonical_date) = crate::market::trade_date_from_symbol_and_market_time(
                    &row.symbol,
                    Some(row.market_time_ms),
                ) && canonical_date != row.trade_date
                {
                    affected_originals.push((row.symbol.clone(), row.trade_date.clone()));
                    let key = (row.symbol.clone(), canonical_date);
                    match staged_winners.get(&key) {
                        Some(prev)
                            if (prev.market_time_ms, prev.fetched_at_ms)
                                > (row.market_time_ms, row.fetched_at_ms) => {}
                        _ => {
                            staged_winners.insert(key, row);
                        }
                    }
                }
            }
        }
        if affected_originals.is_empty() {
            return Ok(());
        }

        self.conn.execute_batch("BEGIN TRANSACTION;")?;
        let res = (|| -> Result<()> {
            let mut del_stmt = self
                .conn
                .prepare("DELETE FROM prices_daily WHERE symbol = ? AND trade_date = ?")?;
            for (symbol, orig_date) in &affected_originals {
                del_stmt.execute(params![symbol, orig_date])?;
            }

            let mut existing_stmt = self.conn.prepare(
                "SELECT market_time_ms, fetched_at_ms
                 FROM prices_daily
                 WHERE symbol = ? AND trade_date = ?",
            )?;
            let mut upsert_stmt = self.conn.prepare(
                "INSERT OR REPLACE INTO prices_daily
                 (symbol, trade_date, price, previous_close, currency, exchange, market_time_ms, fetched_at_ms, source_url)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for ((symbol, canonical_date), row) in staged_winners {
                let existing: Option<(Option<u64>, u64)> = existing_stmt
                    .query_row(params![symbol, canonical_date], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()?;
                let keep_incoming = match existing {
                    Some((ex_mkt, ex_fetch)) => {
                        (row.market_time_ms, row.fetched_at_ms) >= (ex_mkt.unwrap_or(0), ex_fetch)
                    }
                    None => true,
                };
                if keep_incoming {
                    upsert_stmt.execute(params![
                        symbol,
                        canonical_date,
                        money_dec(row.price),
                        opt_money_dec(row.previous_close),
                        row.currency,
                        row.exchange,
                        row.market_time_ms,
                        row.fetched_at_ms,
                        row.source_url
                    ])?;
                }
            }
            Ok(())
        })();
        match res {
            Ok(()) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    }

    pub(crate) fn read_parquet_manifest(
        paths: &WorkspacePaths,
    ) -> Result<Option<ParquetExportManifest>> {
        let p = paths.lake_parquet_dir().join("manifest.json");
        if !p.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&p).with_context(|| format!("read {}", p.display()))?;
        let parsed: ParquetExportManifest =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", p.display()))?;
        Ok(Some(parsed))
    }
}

fn empty_watchlist_file() -> crate::model::WatchlistFile {
    crate::model::WatchlistFile {
        schema_version: 1,
        benchmark_symbol: "^GSPC".to_owned(),
        default_target_return_pct: Decimal::from(10),
        companies: Vec::new(),
    }
}

fn is_valid_quote_candidate(
    ticker: &str,
    price: Decimal,
    mkt_ms: Option<u64>,
    fetch_ms: Option<u64>,
    as_of_ms: u64,
) -> bool {
    let Some(mkt) = mkt_ms else {
        return false;
    };
    if price <= Decimal::ZERO {
        return false;
    }
    let snap = crate::model::PriceSnapshot {
        schema_version: 1,
        symbol: ticker.to_owned(),
        price,
        previous_close: None,
        currency: "USD".to_owned(),
        exchange: String::new(),
        market_time_ms: Some(mkt),
        fetched_at_ms: fetch_ms.unwrap_or(mkt),
        source_url: String::new(),
    };
    crate::market::validate_quote_session(&snap, as_of_ms).is_ok()
}

fn load_all_universe_rows_on_conn(conn: &Connection) -> Result<Vec<UniverseRow>> {
    let sql = format!(
        "SELECT {UNIVERSE_SELECT_COLS} FROM universe_snapshot ORDER BY quality_score DESC, ticker ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], read_universe_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn sync_price_pool_flags_on_conn(conn: &Connection, pool_ciks: &HashSet<String>) -> Result<()> {
    conn.execute("UPDATE universe_snapshot SET in_price_pool = FALSE", [])?;
    let mut stmt =
        conn.prepare("UPDATE universe_snapshot SET in_price_pool = TRUE WHERE cik = ?")?;
    for cik in pool_ciks {
        stmt.execute([cik])?;
    }

    // Clear stale quote fields on former pool members so strike screening never surfaces stale quotes.
    let mut stale_rows: Vec<(String, Option<Decimal>, String)> = Vec::new();
    {
        let mut select_stale = conn.prepare(
            "SELECT cik, strike_price, exact_decimals_json
             FROM universe_snapshot
             WHERE in_price_pool = FALSE
               AND (latest_price IS NOT NULL
                 OR distance_to_strike_pct IS NOT NULL
                 OR strict_fcf_yield_pct IS NOT NULL
                 OR quote_market_time_ms IS NOT NULL
                 OR quote_fetched_at_ms IS NOT NULL)",
        )?;
        let rows = select_stale.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        for r in rows {
            stale_rows.push(r?);
        }
    }
    if !stale_rows.is_empty() {
        let mut clear_stmt = conn.prepare(
            "UPDATE universe_snapshot
             SET latest_price = NULL,
                 distance_to_strike_pct = NULL,
                 strict_fcf_yield_pct = NULL,
                 quote_market_time_ms = NULL,
                 quote_fetched_at_ms = NULL,
                 exact_decimals_json = ?
             WHERE cik = ?",
        )?;
        for (cik, strike_sql, exact_json) in stale_rows {
            let strike_val = decode_universe_exact_bytes(exact_json.as_bytes())
                .and_then(|ex| ex.strike_price)
                .or(strike_sql);
            let patched =
                patch_universe_quote_exact(&exact_json, None, strike_val, None, None, None)?;
            clear_stmt.execute(params![patched, cik])?;
        }
    }
    Ok(())
}

fn reconcile_watchlist_and_pool_on_conn(
    conn: &Connection,
    wl: &crate::model::WatchlistFile,
    prices_top: usize,
    as_of_ms: u64,
) -> Result<HashMap<String, u64>> {
    let bumped = refresh_watchlist_flags_on_conn(conn, wl)?;
    let mut rows = load_all_universe_rows_on_conn(conn)?;
    crate::universe::assign_scheme1_quality_pool(&mut rows, prices_top, as_of_ms);
    let pool_ciks: HashSet<String> = rows
        .into_iter()
        .filter(|r| r.in_price_pool)
        .map(|r| r.cik)
        .collect();
    sync_price_pool_flags_on_conn(conn, &pool_ciks)?;
    Ok(bumped)
}

fn default_valuation_assumptions() -> ValuationAssumptions {
    ValuationAssumptions {
        multiple_low: Decimal::from(18),
        multiple_high: Decimal::from(22),
        target_return_pct: Decimal::from(10),
        expected_growth_pct: Decimal::from(8),
        strike_price_override: None,
    }
}

fn read_sec_generation_on_conn(conn: &Connection) -> Result<Option<(u64, u64)>> {
    let row: Option<(Option<String>, u64)> = conn
        .query_row(
            "SELECT cursor, synced_at_ms FROM sync_watermarks WHERE source_key = ?",
            [WM_SEC_GENERATION],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((cursor_opt, synced_at_ms)) = row else {
        return Ok(None);
    };
    let seq = cursor_opt
        .as_deref()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Ok(Some((seq, synced_at_ms)))
}

fn read_assumptions_revision_on_conn(conn: &Connection) -> Result<u64> {
    let cursor_opt: Option<Option<String>> = conn
        .query_row(
            "SELECT cursor FROM sync_watermarks WHERE source_key = ?",
            [WM_ASSUMPTIONS_REV],
            |r| r.get(0),
        )
        .optional()?;
    Ok(cursor_opt
        .flatten()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0))
}

fn write_assumptions_revision_on_conn(
    conn: &Connection,
    rev: u64,
    synced_at_ms: u64,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sync_watermarks
         (source_key, cursor, etag, last_modified, synced_at_ms)
         VALUES (?, ?, NULL, NULL, ?)",
        params![WM_ASSUMPTIONS_REV, rev.to_string(), synced_at_ms],
    )?;
    Ok(())
}

fn get_annual_rows_by_cik_on_conn(conn: &Connection, cik: &str) -> Result<Vec<LakeAnnualRow>> {
    let mut stmt = conn.prepare(
        "SELECT
            cik, ticker, fiscal_year, label, period_end, filed,
            revenue, gross_profit, gross_margin_pct, ebit, nopat, invested_capital,
            roic_pct, roic_note, roic_total_capital_pct, ocf, capex, fcf, sbc,
            strict_owner_earnings, sbc_to_ocf_pct, buybacks, cash_and_st, total_debt,
            net_debt, equity, diluted_shares, ending_shares, raw_sha256,
            provenance_json, exact_decimals_json
         FROM facts_annual
         WHERE cik = ?
         ORDER BY fiscal_year ASC, period_end ASC",
    )?;
    let rows = stmt.query_map([cik], read_annual_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

struct RetainedRebuildTarget<'a> {
    cik: &'a str,
    ticker: &'a str,
    name: &'a str,
    exchange: &'a str,
    sic: Option<u32>,
    sic_description: Option<&'a str>,
    in_price_pool: bool,
    wl_entry: Option<&'a crate::model::CompanyEntry>,
    cached_price: Option<&'a crate::model::PriceSnapshot>,
    is_non_common_listing: bool,
    now_ms: u64,
}

fn rebuild_universe_row_from_retained_conn(
    conn: &Connection,
    target: &RetainedRebuildTarget<'_>,
) -> Result<Option<UniverseRow>> {
    let annuals = get_annual_rows_by_cik_on_conn(conn, target.cik)?;
    if annuals.is_empty() {
        return Ok(None);
    }
    let annual_bases: Vec<calculator::AnnualReportBasis> = annuals
        .iter()
        .map(|a| {
            let provenance: Vec<FieldProvenance> =
                serde_json::from_str(&a.provenance_json).unwrap_or_default();
            let ending_shares = a.ending_shares;
            calculator::AnnualReportBasis {
                row: annual_to_metric_row(a.clone()),
                ending_shares,
                provenance,
            }
        })
        .collect();

    let mut ttm_stmt = conn.prepare(
        "SELECT
            cik, label, period_end,
            revenue, gross_profit, gross_margin_pct, ebit, nopat, invested_capital,
            roic_pct, roic_note, roic_total_capital_pct, ocf, capex, fcf, sbc,
            strict_owner_earnings, sbc_to_ocf_pct, buybacks, cash_and_st, total_debt,
            net_debt, equity, diluted_shares, exact_decimals_json, provenance_json,
            ytd_period_label, ytd_start, ytd_end, ytd_revenue, ytd_ocf, ytd_capex,
            ytd_sbc, ytd_buybacks, raw_sha256,
            latest_quarter_label, latest_quarter_ocf, latest_quarter_capex
         FROM facts_ttm
         WHERE cik = ?",
    )?;
    let mut ttm_raw_sha: Option<String> = None;
    let ttm_basis: Option<calculator::TtmReportBasis> = ttm_stmt
        .query_row([target.cik], |row| {
            let (_, metric, provenance_json) = read_ttm_metric_row(row)?;
            let provenance: Vec<FieldProvenance> =
                serde_json::from_str(&provenance_json).unwrap_or_default();
            let exact_json: String = row.get(24)?;
            let exact_opt: Option<TtmExactDecimals> = (!exact_json.is_empty())
                .then(|| serde_json::from_str(&exact_json).ok())
                .flatten();
            let sql_ytd_revenue: Option<Decimal> = row.get(29)?;
            let sql_ytd_ocf: Option<Decimal> = row.get(30)?;
            let sql_ytd_capex: Option<Decimal> = row.get(31)?;
            let sql_ytd_sbc: Option<Decimal> = row.get(32)?;
            let sql_ytd_buybacks: Option<Decimal> = row.get(33)?;
            let sql_q_label: Option<String> = row.get(35)?;
            let sql_q_ocf: Option<Decimal> = row.get(36)?;
            let sql_q_capex: Option<Decimal> = row.get(37)?;
            let ytd = crate::model::YtdPeriodFinancials {
                period_label: row.get(26)?,
                start: row.get(27)?,
                end: row.get(28)?,
                filed: None,
                revenue: exact_opt
                    .as_ref()
                    .and_then(|ex| ex.ytd_revenue)
                    .or(sql_ytd_revenue),
                ocf: exact_opt.as_ref().and_then(|ex| ex.ytd_ocf).or(sql_ytd_ocf),
                capex: exact_opt
                    .as_ref()
                    .and_then(|ex| ex.ytd_capex)
                    .or(sql_ytd_capex),
                sbc: exact_opt.as_ref().and_then(|ex| ex.ytd_sbc).or(sql_ytd_sbc),
                buybacks: exact_opt
                    .as_ref()
                    .and_then(|ex| ex.ytd_buybacks)
                    .or(sql_ytd_buybacks),
                latest_quarter_label: exact_opt
                    .as_ref()
                    .and_then(|ex| ex.latest_quarter_label.clone())
                    .or(sql_q_label),
                latest_quarter_ocf: exact_opt
                    .as_ref()
                    .and_then(|ex| ex.latest_quarter_ocf)
                    .or(sql_q_ocf),
                latest_quarter_capex: exact_opt
                    .as_ref()
                    .and_then(|ex| ex.latest_quarter_capex)
                    .or(sql_q_capex),
                latest_quarter_sbc: None,
                latest_quarter_buybacks: None,
            };
            let sha: String = row.get(34)?;
            Ok((
                calculator::TtmReportBasis {
                    row: metric,
                    ending_shares: None,
                    provenance,
                    latest_ytd: ytd,
                },
                sha,
            ))
        })
        .optional()?
        .map(|(basis, sha)| {
            ttm_raw_sha = Some(sha);
            basis
        });

    let raw_sha256 = ttm_raw_sha.or_else(|| annuals.last().map(|a| a.raw_sha256.clone()));
    let inputs = calculator::MetricReportInputs {
        raw_sha256,
        annuals: annual_bases,
        ttm: ttm_basis,
    };

    let company = match target.wl_entry {
        Some(wl) => {
            let mut c = wl.clone();
            c.ticker = target.ticker.to_owned();
            c.name = target.name.to_owned();
            c.cik = Some(target.cik.to_owned());
            c
        }
        None => crate::model::CompanyEntry {
            ticker: target.ticker.to_owned(),
            name: target.name.to_owned(),
            bucket: Bucket::Watch,
            source: crate::model::DataSource::SecEdgar,
            cik: Some(target.cik.to_owned()),
            currency: "USD".to_owned(),
            circle: "US Equity".to_owned(),
            too_hard_reason: None,
            assumptions: default_valuation_assumptions(),
        },
    };
    let report = calculator::build_report_from_metrics(
        &company,
        &inputs,
        target.cached_price,
        target.now_ms,
    );
    let latest_prov: &[FieldProvenance] = inputs
        .ttm
        .as_ref()
        .map(|t| t.provenance.as_slice())
        .or_else(|| inputs.annuals.last().map(|a| a.provenance.as_slice()))
        .unwrap_or(&[]);
    let listed = crate::universe::ListedCompany {
        cik: target.cik.to_owned(),
        ticker: target.ticker.to_owned(),
        name: target.name.to_owned(),
        exchange: target.exchange.to_owned(),
        is_non_common_listing: target.is_non_common_listing,
    };
    let sic_info = (target.sic.is_some() || target.sic_description.is_some()).then(|| {
        crate::universe::SicInfo {
            sic: target.sic,
            sic_description: target.sic_description.map(str::to_owned),
        }
    });
    let mut row = crate::universe::build_parsed_universe_row_with_latest_prov(
        &listed,
        sic_info.as_ref(),
        target.wl_entry,
        latest_prov,
        &report,
        target.now_ms,
    );
    row.in_price_pool = target.in_price_pool;
    Ok(Some(row))
}

fn load_latest_valuation_basis_on_conn(
    conn: &Connection,
) -> Result<HashMap<String, (&'static str, AnnualMetricRow, calculator::ValuationGuards)>> {
    let mut map: HashMap<String, (&'static str, AnnualMetricRow, calculator::ValuationGuards)> =
        HashMap::new();

    let mut ann_stmt = conn.prepare(
        "SELECT
            cik, ticker, fiscal_year, label, period_end, filed,
            revenue, gross_profit, gross_margin_pct, ebit, nopat, invested_capital,
            roic_pct, roic_note, roic_total_capital_pct, ocf, capex, fcf, sbc,
            strict_owner_earnings, sbc_to_ocf_pct, buybacks, cash_and_st, total_debt,
            net_debt, equity, diluted_shares, ending_shares, raw_sha256,
            provenance_json, exact_decimals_json
         FROM facts_annual
         QUALIFY ROW_NUMBER() OVER (PARTITION BY cik ORDER BY fiscal_year DESC, period_end DESC) = 1",
    )?;
    let ann_rows = ann_stmt.query_map([], read_annual_row)?;
    for r in ann_rows {
        let a = r?;
        let prov: Vec<FieldProvenance> =
            serde_json::from_str(&a.provenance_json).unwrap_or_default();
        let ending = a.ending_shares;
        let cik = a.cik.clone();
        let ticker = a.ticker.clone();
        let metric = annual_to_metric_row(a);
        let mut guards =
            calculator::guards_from_provenance(&ticker, &prov, metric.diluted_shares, Some(ending));
        guards.cash_available &= metric.cash_and_st.is_some();
        map.insert(cik, ("FY", metric, guards));
    }

    let mut ttm_stmt = conn.prepare(
        "SELECT
            cik, label, period_end,
            revenue, gross_profit, gross_margin_pct, ebit, nopat, invested_capital,
            roic_pct, roic_note, roic_total_capital_pct, ocf, capex, fcf, sbc,
            strict_owner_earnings, sbc_to_ocf_pct, buybacks, cash_and_st, total_debt,
            net_debt, equity, diluted_shares, exact_decimals_json, provenance_json, ticker
         FROM facts_ttm",
    )?;
    let ttm_rows = ttm_stmt.query_map([], |row| {
        let (cik, metric, prov_json) = read_ttm_metric_row(row)?;
        let ticker: String = row.get(26)?;
        Ok((cik, ticker, metric, prov_json))
    })?;
    for r in ttm_rows {
        let (cik, ticker, metric, prov_json) = r?;
        let prov: Vec<FieldProvenance> = serde_json::from_str(&prov_json).unwrap_or_default();
        let mut guards =
            calculator::guards_from_provenance(&ticker, &prov, metric.diluted_shares, None);
        guards.cash_available &= metric.cash_and_st.is_some();
        map.insert(cik, ("TTM", metric, guards));
    }

    Ok(map)
}

fn recompute_valuation_fields_for_row(
    ticker: &str,
    basis_opt: Option<&(&'static str, AnnualMetricRow, calculator::ValuationGuards)>,
    latest_price: Option<Decimal>,
    quote_market_time_ms: Option<u64>,
    assumptions: &ValuationAssumptions,
    suppress_per_share: bool,
) -> (Option<Decimal>, Option<Decimal>, Option<Decimal>) {
    if suppress_per_share {
        return (None, None, None);
    }
    let Some((basis_label, metric, guards)) = basis_opt else {
        return (None, None, None);
    };
    let snap_opt = latest_price
        .zip(quote_market_time_ms)
        .map(|(price, mkt_ms)| crate::model::PriceSnapshot {
            schema_version: 1,
            symbol: ticker.to_owned(),
            price,
            previous_close: None,
            currency: "USD".to_owned(),
            exchange: String::new(),
            market_time_ms: Some(mkt_ms),
            fetched_at_ms: mkt_ms,
            source_url: String::new(),
        });
    let val =
        calculator::compute_valuation(basis_label, metric, snap_opt.as_ref(), assumptions, *guards);
    (
        val.strike_price,
        val.distance_to_strike_pct,
        val.strict_fcf_yield_mcap_pct,
    )
}

fn refresh_watchlist_flags_on_conn(
    conn: &Connection,
    wl: &crate::model::WatchlistFile,
) -> Result<HashMap<String, u64>> {
    let default_assump = default_valuation_assumptions();
    let default_digest = valuation_assumptions_digest(&default_assump);
    let mut wl_by_cik: HashMap<String, (&crate::model::CompanyEntry, String)> = HashMap::new();
    let mut wl_by_ticker: HashMap<String, (&crate::model::CompanyEntry, String)> = HashMap::new();
    for co in &wl.companies {
        let dig = valuation_assumptions_digest(&co.assumptions);
        if let Some(cik) = co
            .cik
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|c| crate::edgar::normalize_cik(c).ok())
            && (!crate::calculator::is_supplemental_security_ticker(&co.ticker, "")
                || !wl_by_cik.contains_key(&cik))
        {
            wl_by_cik.insert(cik, (co, dig.clone()));
        }
        wl_by_ticker.insert(co.ticker.trim().to_ascii_uppercase(), (co, dig));
    }

    struct LiveWlState {
        cik: String,
        ticker: String,
        in_watchlist: bool,
        bucket_opt: Option<String>,
        assumptions_digest: Option<String>,
        assumptions_rev: u64,
        latest_price: Option<Decimal>,
        quote_market_time_ms: Option<u64>,
        warnings_json: String,
        exact_decimals_json: String,
    }
    let mut current_rows = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT cik, ticker, in_watchlist, bucket_opt, assumptions_digest,
                    COALESCE(assumptions_rev, 0), latest_price, quote_market_time_ms,
                    warnings_json, exact_decimals_json
             FROM universe_snapshot",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(LiveWlState {
                cik: row.get(0)?,
                ticker: row.get(1)?,
                in_watchlist: row.get(2)?,
                bucket_opt: row.get(3)?,
                assumptions_digest: row.get(4)?,
                assumptions_rev: row.get(5)?,
                latest_price: row.get(6)?,
                quote_market_time_ms: row.get(7)?,
                warnings_json: row.get(8)?,
                exact_decimals_json: row.get(9)?,
            })
        })?;
        for r in rows {
            current_rows.push(r?);
        }
    }

    let cur_rev = read_assumptions_revision_on_conn(conn)?;
    let next_rev = cur_rev + 1;
    let mut any_assumptions_changed = false;
    let mut needs_basis_load = false;
    struct WatchlistSyncTarget<'a> {
        row: LiveWlState,
        in_watchlist: bool,
        bucket: Option<&'static str>,
        assumptions: &'a ValuationAssumptions,
        digest: String,
        digest_changed: bool,
    }
    let mut targets: Vec<WatchlistSyncTarget<'_>> = Vec::with_capacity(current_rows.len());

    for row in current_rows {
        let matched = wl_by_ticker
            .get(&row.ticker.to_ascii_uppercase())
            .or_else(|| {
                wl_by_cik.get(&row.cik).filter(|(co, _)| {
                    !crate::calculator::is_supplemental_security_ticker(&co.ticker, &row.ticker)
                })
            });
        let (target_in_wl, target_bucket, target_assump, target_digest) =
            if let Some((co, dig)) = matched {
                (
                    true,
                    Some(bucket_to_str(co.bucket)),
                    &co.assumptions,
                    dig.clone(),
                )
            } else {
                (false, None, &default_assump, default_digest.clone())
            };
        let digest_changed = row.assumptions_digest.as_deref() != Some(target_digest.as_str());
        if digest_changed {
            needs_basis_load = true;
            any_assumptions_changed = true;
        }
        let meta_changed = row.in_watchlist != target_in_wl
            || row.bucket_opt.as_deref() != target_bucket
            || row.assumptions_rev == 0
            || digest_changed;
        if meta_changed {
            targets.push(WatchlistSyncTarget {
                row,
                in_watchlist: target_in_wl,
                bucket: target_bucket,
                assumptions: target_assump,
                digest: target_digest,
                digest_changed,
            });
        }
    }

    let basis_map_opt = if needs_basis_load {
        Some(load_latest_valuation_basis_on_conn(conn)?)
    } else {
        None
    };

    let mut bumped_revs: HashMap<String, u64> = HashMap::new();
    let mut upd_full = conn.prepare(
        "UPDATE universe_snapshot
         SET in_watchlist = ?,
             bucket_opt = ?,
             assumptions_digest = ?,
             assumptions_rev = ?,
             strike_price = ?,
             distance_to_strike_pct = ?,
             strict_fcf_yield_pct = ?,
             exact_decimals_json = ?
         WHERE cik = ?",
    )?;
    let mut upd_flags_only = conn.prepare(
        "UPDATE universe_snapshot
         SET in_watchlist = ?,
             bucket_opt = ?,
             assumptions_digest = COALESCE(assumptions_digest, ?),
             assumptions_rev = CASE WHEN COALESCE(assumptions_rev, 0) = 0 THEN ? ELSE assumptions_rev END
         WHERE cik = ?",
    )?;

    let init_rev = cur_rev.max(1);
    for t in targets {
        if t.digest_changed {
            let exact_opt = decode_universe_exact_bytes(t.row.exact_decimals_json.as_bytes());
            let live_price = exact_opt
                .as_ref()
                .and_then(|ex| ex.latest_price)
                .or(t.row.latest_price);
            let live_mkt = exact_opt
                .as_ref()
                .and_then(|ex| ex.quote_market_time_ms)
                .or(t.row.quote_market_time_ms);
            let suppress_per_share = t.row.warnings_json.contains("\"SHARE_BASIS_MISMATCH\"")
                || crate::calculator::is_share_basis_mismatched(&t.row.ticker, false);
            let basis_opt = basis_map_opt.as_ref().and_then(|m| m.get(&t.row.cik));
            let (next_strike, next_dist, next_yield) = recompute_valuation_fields_for_row(
                &t.row.ticker,
                basis_opt,
                live_price,
                live_mkt,
                t.assumptions,
                suppress_per_share,
            );
            let patched_exact = patch_universe_quote_exact(
                &t.row.exact_decimals_json,
                live_price,
                next_strike,
                next_dist,
                next_yield,
                live_mkt,
            )?;
            upd_full.execute(params![
                t.in_watchlist,
                t.bucket,
                t.digest,
                next_rev,
                opt_money_dec(next_strike),
                opt_ratio_dec(next_dist),
                opt_ratio_dec(next_yield),
                patched_exact,
                t.row.cik
            ])?;
            bumped_revs.insert(t.row.cik, next_rev);
        } else {
            upd_flags_only.execute(params![
                t.in_watchlist,
                t.bucket,
                t.digest,
                init_rev,
                t.row.cik
            ])?;
        }
    }

    if any_assumptions_changed {
        write_assumptions_revision_on_conn(conn, next_rev, store::now_ms())?;
    } else if cur_rev == 0 {
        write_assumptions_revision_on_conn(conn, init_rev, store::now_ms())?;
    }
    Ok(bumped_revs)
}

fn upsert_prices_daily_on_conn(tx: &Connection, prices: &[LakePriceDailyRow]) -> Result<()> {
    if prices.is_empty() {
        return Ok(());
    }
    let mut latest_existing_stmt = tx.prepare(
        "SELECT COALESCE(market_time_ms, 0), fetched_at_ms
         FROM prices_daily
         WHERE UPPER(symbol) = UPPER(?)
         ORDER BY COALESCE(market_time_ms, 0) DESC, fetched_at_ms DESC, trade_date DESC
         LIMIT 1",
    )?;
    let mut exact_date_stmt = tx.prepare(
        "SELECT market_time_ms, fetched_at_ms
         FROM prices_daily
         WHERE symbol = ? AND trade_date = ?",
    )?;
    let mut stale_date_stmt = tx.prepare(
        "DELETE FROM prices_daily
         WHERE symbol = ? AND market_time_ms IS NOT NULL AND market_time_ms = ? AND trade_date != ?",
    )?;
    let mut price_stmt = tx.prepare(
        "INSERT OR REPLACE INTO prices_daily
         (symbol, trade_date, price, previous_close, currency, exchange, market_time_ms, fetched_at_ms, source_url)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;

    for p in prices {
        let incoming_ts = (p.market_time_ms.unwrap_or(0), p.fetched_at_ms);
        let latest_existing: (u64, u64) = latest_existing_stmt
            .query_row([&p.symbol], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?
            .unwrap_or((0, 0));
        if p.market_time_ms.is_none() && latest_existing.0 > 0 {
            continue;
        }

        let existing_on_date: Option<(Option<u64>, u64)> = exact_date_stmt
            .query_row(params![p.symbol, p.trade_date], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        if let Some((ex_mkt, ex_fetch)) = existing_on_date
            && incoming_ts < (ex_mkt.unwrap_or(0), ex_fetch)
        {
            continue;
        }

        if let Some(mkt_ms) = p.market_time_ms {
            stale_date_stmt.execute(params![p.symbol, mkt_ms, p.trade_date])?;
        }
        price_stmt.execute(params![
            p.symbol,
            p.trade_date,
            money_dec(p.price),
            opt_money_dec(p.previous_close),
            p.currency,
            p.exchange,
            p.market_time_ms,
            p.fetched_at_ms,
            p.source_url
        ])?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn upsert_prices_and_update_snapshot_on_conn(
    tx: &Connection,
    prices: &[LakePriceDailyRow],
    updated_rows: &[QuoteSnapshotUpdate],
    expected_assumptions_digests: &HashMap<String, String>,
    expected_assumptions_revs: &HashMap<String, u64>,
    watermarks: &[SyncWatermark],
) -> Result<()> {
    upsert_prices_daily_on_conn(tx, prices)?;
    {
        let mut read_live_stmt = tx.prepare(
            "SELECT raw_sha256, exact_decimals_json, ticker, in_price_pool, latest_filing_end,
                    quote_market_time_ms, warnings_json, quote_fetched_at_ms, assumptions_digest,
                    assumptions_rev
             FROM universe_snapshot
             WHERE cik = ?",
        )?;
        let mut read_basis_stmt = tx.prepare(
            "SELECT COALESCE(
                (SELECT period_end FROM facts_ttm WHERE cik = ?),
                (SELECT period_end FROM facts_annual WHERE cik = ? ORDER BY fiscal_year DESC, period_end DESC LIMIT 1)
             )",
        )?;
        let mut read_latest_quote_stmt = tx.prepare(
            "SELECT COALESCE(market_time_ms, 0), fetched_at_ms
             FROM prices_daily
             WHERE UPPER(symbol) = UPPER(?)
             ORDER BY COALESCE(market_time_ms, 0) DESC, fetched_at_ms DESC, trade_date DESC
             LIMIT 1",
        )?;
        let mut snap_stmt = tx.prepare(
            "UPDATE universe_snapshot
             SET latest_price = ?,
                 strike_price = ?,
                 distance_to_strike_pct = ?,
                 strict_fcf_yield_pct = ?,
                 quote_market_time_ms = ?,
                 quote_fetched_at_ms = ?,
                 exact_decimals_json = ?
             WHERE cik = ?",
        )?;
        for u in updated_rows {
            if u.latest_price.is_some() && u.quote_market_time_ms.is_none() {
                continue;
            }
            struct LiveSnapshotGuardRow {
                raw_sha256: Option<String>,
                exact_decimals_json: String,
                ticker: String,
                in_price_pool: bool,
                latest_filing_end: Option<String>,
                quote_market_time_ms: Option<u64>,
                warnings_json: String,
                quote_fetched_at_ms: Option<u64>,
                assumptions_digest: Option<String>,
                assumptions_rev: Option<u64>,
            }
            let live_opt: Option<LiveSnapshotGuardRow> = read_live_stmt
                .query_row([&u.cik], |row| {
                    Ok(LiveSnapshotGuardRow {
                        raw_sha256: row.get(0)?,
                        exact_decimals_json: row.get(1)?,
                        ticker: row.get(2)?,
                        in_price_pool: row.get(3)?,
                        latest_filing_end: row.get(4)?,
                        quote_market_time_ms: row.get(5)?,
                        warnings_json: row.get(6)?,
                        quote_fetched_at_ms: row.get(7)?,
                        assumptions_digest: row.get(8)?,
                        assumptions_rev: row.get(9)?,
                    })
                })
                .optional()?;
            let Some(LiveSnapshotGuardRow {
                raw_sha256: live_sha,
                exact_decimals_json: live_exact_json,
                ticker: live_ticker,
                in_price_pool: live_in_price_pool,
                latest_filing_end: live_filing_end,
                quote_market_time_ms: live_quote_mkt_ms,
                warnings_json: live_warnings_json,
                quote_fetched_at_ms: live_quote_fetch_ms,
                assumptions_digest: live_assumptions_digest,
                assumptions_rev: live_assumptions_rev,
            }) = live_opt
            else {
                continue;
            };
            if !live_in_price_pool {
                continue;
            }
            if u.expected_ticker.trim().is_empty()
                || !live_ticker.eq_ignore_ascii_case(u.expected_ticker.trim())
            {
                continue;
            }
            let live_fetch_ms = live_quote_fetch_ms.unwrap_or(0);
            if let (Some(u_mkt), Some(live_mkt)) = (u.quote_market_time_ms, live_quote_mkt_ms)
                && (u_mkt, u.quote_fetched_at_ms) < (live_mkt, live_fetch_ms)
            {
                continue;
            }
            let latest_stored_ts: (u64, u64) = read_latest_quote_stmt
                .query_row([&live_ticker], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?
                .unwrap_or((0, 0));
            if latest_stored_ts > (0, 0)
                && (u.quote_market_time_ms.unwrap_or(0), u.quote_fetched_at_ms) < latest_stored_ts
            {
                continue;
            }
            if live_sha != u.expected_raw_sha256 {
                continue;
            }
            if u.expected_latest_filing_end.is_some()
                && live_filing_end != u.expected_latest_filing_end
            {
                continue;
            }
            if u.expected_basis_period_end.is_some() {
                let live_basis_end: Option<String> = read_basis_stmt
                    .query_row(params![u.cik, u.cik], |row| row.get(0))
                    .optional()?
                    .flatten();
                if live_basis_end != u.expected_basis_period_end {
                    continue;
                }
            }
            if let Some(exp) = expected_assumptions_digests.get(&u.cik)
                && live_assumptions_digest.as_deref() != Some(exp.as_str())
            {
                continue;
            }
            if let Some(&exp_rev) = expected_assumptions_revs.get(&u.cik)
                && exp_rev > 0
                && live_assumptions_rev != Some(exp_rev)
            {
                continue;
            }

            let suppress_per_share = live_warnings_json.contains("\"SHARE_BASIS_MISMATCH\"")
                || crate::calculator::is_share_basis_mismatched(&live_ticker, false);
            let strike_out = (!suppress_per_share).then_some(u.strike_price).flatten();
            let dist_out = (!suppress_per_share)
                .then_some(u.distance_to_strike_pct)
                .flatten();
            let yield_out = (!suppress_per_share)
                .then_some(u.strict_fcf_yield_pct)
                .flatten();

            let persisted_quote_mkt_ms = u.latest_price.and(u.quote_market_time_ms);
            let persisted_quote_fetch_ms = u.latest_price.map(|_| u.quote_fetched_at_ms);
            let patched_exact_json = patch_universe_quote_exact(
                &live_exact_json,
                u.latest_price,
                strike_out,
                dist_out,
                yield_out,
                persisted_quote_mkt_ms,
            )?;
            snap_stmt.execute(params![
                opt_money_dec(u.latest_price),
                opt_money_dec(strike_out),
                opt_ratio_dec(dist_out),
                opt_ratio_dec(yield_out),
                persisted_quote_mkt_ms,
                persisted_quote_fetch_ms,
                patched_exact_json,
                u.cik
            ])?;
        }
    }
    if !watermarks.is_empty() {
        let mut read_wm_stmt =
            tx.prepare("SELECT cursor, synced_at_ms FROM sync_watermarks WHERE source_key = ?")?;
        let mut wm_stmt = tx.prepare(
            "INSERT OR REPLACE INTO sync_watermarks
             (source_key, cursor, etag, last_modified, synced_at_ms)
             VALUES (?, ?, ?, ?, ?)",
        )?;
        for wm in watermarks {
            let existing: Option<(Option<String>, u64)> = read_wm_stmt
                .query_row([&wm.source_key], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            if let Some((ex_cursor, ex_ms)) = existing {
                let regresses = match (ex_cursor.as_deref(), wm.cursor.as_deref()) {
                    (Some(ec), Some(nc)) => nc < ec || (nc == ec && wm.synced_at_ms < ex_ms),
                    (Some(_), None) => true,
                    _ => wm.synced_at_ms < ex_ms,
                };
                if regresses {
                    continue;
                }
            }
            wm_stmt.execute(params![
                wm.source_key,
                wm.cursor,
                wm.etag,
                wm.last_modified,
                wm.synced_at_ms
            ])?;
        }
    }
    Ok(())
}

pub(crate) fn reconcile_preserved_warnings_json(
    prev_warnings_json: &str,
    incoming_warnings_json: &str,
    ticker: &str,
    ticker_unchanged: bool,
    sic_missing: bool,
) -> (usize, String, bool) {
    let mut warnings: Vec<WarningFlag> =
        serde_json::from_str(prev_warnings_json).unwrap_or_default();
    let incoming: Vec<WarningFlag> =
        serde_json::from_str(incoming_warnings_json).unwrap_or_default();
    let incoming_mismatch_flag = incoming
        .iter()
        .find(|w| w.code == "SHARE_BASIS_MISMATCH")
        .cloned();
    let has_mismatch = incoming_mismatch_flag.is_some()
        || crate::calculator::is_share_basis_mismatched(ticker, false);
    if has_mismatch {
        let replacement = incoming_mismatch_flag.unwrap_or_else(|| WarningFlag {
            code: "SHARE_BASIS_MISMATCH".to_owned(),
            severity: crate::model::WarningSeverity::Amber,
            title: "股本口径与报价类别不一致".to_owned(),
            detail: format!(
                "{ticker} 属于非普通股或与财报稀释股本口径不一致（如 BRK-B），已抑制每股估值"
            ),
        });
        warnings.retain(|w| w.code != "SHARE_BASIS_MISMATCH");
        warnings.push(replacement);
    } else if !ticker_unchanged {
        warnings.retain(|w| w.code != "SHARE_BASIS_MISMATCH");
    }

    if sic_missing {
        if !warnings.iter().any(|w| w.code == "UNCLASSIFIED_SIC")
            && let Some(flag) = incoming
                .iter()
                .find(|w| w.code == "UNCLASSIFIED_SIC")
                .cloned()
        {
            warnings.push(flag);
        }
    } else {
        warnings.retain(|w| w.code != "UNCLASSIFIED_SIC");
    }

    let final_mismatch = warnings.iter().any(|w| w.code == "SHARE_BASIS_MISMATCH");
    let count = warnings.len();
    let json = serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".to_owned());
    (count, json, final_mismatch)
}

fn patch_universe_quote_exact(
    existing_json: &str,
    latest_price: Option<Decimal>,
    strike_price: Option<Decimal>,
    distance_to_strike_pct: Option<Decimal>,
    strict_fcf_yield_pct: Option<Decimal>,
    quote_market_time_ms: Option<u64>,
) -> Result<String> {
    if existing_json.trim().is_empty() {
        return Ok(String::new());
    }
    let mut exact: UniverseExactDecimals = decode_universe_exact_bytes(existing_json.as_bytes())
        .context("parse live UniverseExactDecimals")?;
    exact.latest_price = latest_price;
    exact.strike_price = strike_price;
    exact.distance_to_strike_pct = distance_to_strike_pct;
    exact.strict_fcf_yield_pct = strict_fcf_yield_pct;
    exact.quote_market_time_ms = quote_market_time_ms;
    Ok(encode_universe_exact(&exact))
}

fn universe_primary_sort_sql(sort_by: UniverseSortBy, sort_asc: bool) -> String {
    let dir = if sort_asc { "ASC" } else { "DESC" };
    match sort_by {
        UniverseSortBy::QualityScore => format!("quality_score {dir} NULLS LAST"),
        UniverseSortBy::Roic5yAvg => format!("roic_5y_avg_pct {dir} NULLS LAST"),
        UniverseSortBy::StrictFcfYield => format!("strict_fcf_yield_pct {dir} NULLS LAST"),
        UniverseSortBy::DistanceToStrike => format!("distance_to_strike_pct {dir} NULLS LAST"),
        UniverseSortBy::ShareChange3y => format!("share_change_3y_pct {dir} NULLS LAST"),
        UniverseSortBy::SbcToOcf => format!("sbc_to_ocf_pct {dir} NULLS LAST"),
        UniverseSortBy::LatestPrice => format!("latest_price {dir} NULLS LAST"),
        UniverseSortBy::StrikePrice => format!("strike_price {dir} NULLS LAST"),
        UniverseSortBy::WarningsCount => format!("warnings_count {dir} NULLS LAST"),
        UniverseSortBy::Ticker => format!("ticker {dir}"),
        UniverseSortBy::Name => format!("name {dir}"),
        UniverseSortBy::Sic => format!("sic {dir} NULLS LAST"),
        UniverseSortBy::StrictOwnerEarnings => {
            format!("strict_owner_earnings {dir} NULLS LAST")
        }
    }
}

fn universe_sort_sql(sort_by: UniverseSortBy, sort_asc: bool) -> String {
    let primary = universe_primary_sort_sql(sort_by, sort_asc);
    format!("{primary}, quality_score DESC, ticker ASC")
}

#[derive(PartialEq, Eq)]
enum SqlPrimaryTieKey<'a> {
    Dec(&'static str, Option<Decimal>),
    Int(&'static str, Option<i32>),
    Str(&'static str, &'a str),
}

fn sql_primary_tie_key(row: &UniverseRow, sort_by: UniverseSortBy) -> SqlPrimaryTieKey<'_> {
    match sort_by {
        UniverseSortBy::QualityScore => {
            SqlPrimaryTieKey::Dec("quality_score", duckdb_dec(row.quality_score, 6, 30))
        }
        UniverseSortBy::Roic5yAvg => {
            SqlPrimaryTieKey::Dec("roic_5y_avg_pct", opt_ratio_dec(row.roic_5y_avg_pct))
        }
        UniverseSortBy::StrictFcfYield => SqlPrimaryTieKey::Dec(
            "strict_fcf_yield_pct",
            opt_ratio_dec(row.strict_fcf_yield_pct),
        ),
        UniverseSortBy::DistanceToStrike => SqlPrimaryTieKey::Dec(
            "distance_to_strike_pct",
            opt_ratio_dec(row.distance_to_strike_pct),
        ),
        UniverseSortBy::ShareChange3y => SqlPrimaryTieKey::Dec(
            "share_change_3y_pct",
            opt_ratio_dec(row.share_change_3y_pct),
        ),
        UniverseSortBy::SbcToOcf => {
            SqlPrimaryTieKey::Dec("sbc_to_ocf_pct", opt_ratio_dec(row.sbc_to_ocf_pct))
        }
        UniverseSortBy::LatestPrice => {
            SqlPrimaryTieKey::Dec("latest_price", opt_money_dec(row.latest_price))
        }
        UniverseSortBy::StrikePrice => {
            SqlPrimaryTieKey::Dec("strike_price", opt_money_dec(row.strike_price))
        }
        UniverseSortBy::WarningsCount => SqlPrimaryTieKey::Int(
            "warnings_count",
            Some(i32::try_from(row.warnings_count).unwrap_or(i32::MAX)),
        ),
        UniverseSortBy::Ticker => SqlPrimaryTieKey::Str("ticker", &row.ticker),
        UniverseSortBy::Name => SqlPrimaryTieKey::Str("name", &row.name),
        UniverseSortBy::Sic => {
            SqlPrimaryTieKey::Int("sic", row.sic.and_then(|v| i32::try_from(v).ok()))
        }
        UniverseSortBy::StrictOwnerEarnings => SqlPrimaryTieKey::Dec(
            "strict_owner_earnings",
            opt_money_dec(row.strict_owner_earnings),
        ),
    }
}

fn is_sql_primary_clamped(row: &UniverseRow, sort_by: UniverseSortBy) -> bool {
    match sort_by {
        UniverseSortBy::QualityScore => duckdb_dec(row.quality_score, 6, 30).is_none(),
        UniverseSortBy::Roic5yAvg => {
            row.roic_5y_avg_pct.is_some() && opt_ratio_dec(row.roic_5y_avg_pct).is_none()
        }
        UniverseSortBy::StrictFcfYield => {
            row.strict_fcf_yield_pct.is_some() && opt_ratio_dec(row.strict_fcf_yield_pct).is_none()
        }
        UniverseSortBy::DistanceToStrike => {
            row.distance_to_strike_pct.is_some()
                && opt_ratio_dec(row.distance_to_strike_pct).is_none()
        }
        UniverseSortBy::ShareChange3y => {
            row.share_change_3y_pct.is_some() && opt_ratio_dec(row.share_change_3y_pct).is_none()
        }
        UniverseSortBy::SbcToOcf => {
            row.sbc_to_ocf_pct.is_some() && opt_ratio_dec(row.sbc_to_ocf_pct).is_none()
        }
        UniverseSortBy::LatestPrice => {
            row.latest_price.is_some() && opt_money_dec(row.latest_price).is_none()
        }
        UniverseSortBy::StrikePrice => {
            row.strike_price.is_some() && opt_money_dec(row.strike_price).is_none()
        }
        UniverseSortBy::StrictOwnerEarnings => {
            row.strict_owner_earnings.is_some()
                && opt_money_dec(row.strict_owner_earnings).is_none()
        }
        UniverseSortBy::WarningsCount
        | UniverseSortBy::Ticker
        | UniverseSortBy::Name
        | UniverseSortBy::Sic => false,
    }
}

fn sql_primary_tie_filter(
    row: &UniverseRow,
    sort_by: UniverseSortBy,
) -> (String, Option<Box<dyn ToSql>>) {
    match sql_primary_tie_key(row, sort_by) {
        SqlPrimaryTieKey::Dec(col, Some(val)) => (format!("{col} = ?"), Some(Box::new(val))),
        SqlPrimaryTieKey::Dec(col, None) => (format!("{col} IS NULL"), None),
        SqlPrimaryTieKey::Int(col, Some(val)) => (format!("{col} = ?"), Some(Box::new(val))),
        SqlPrimaryTieKey::Int(col, None) => (format!("{col} IS NULL"), None),
        SqlPrimaryTieKey::Str(col, s) => (format!("{col} = ?"), Some(Box::new(s.to_owned()))),
    }
}

fn rows_might_tie_on_sql_primary(
    a: &UniverseRow,
    b: &UniverseRow,
    sort_by: UniverseSortBy,
) -> bool {
    sql_primary_tie_key(a, sort_by) == sql_primary_tie_key(b, sort_by)
}

fn row_matches_exact_filter(row: &UniverseRow, filter: &UniverseFilter) -> bool {
    if filter.exclude_financials && row.is_financial {
        return false;
    }
    if filter.positive_strict_oe_only
        && !row
            .strict_owner_earnings
            .is_some_and(|oe| oe > Decimal::ZERO)
    {
        return false;
    }
    if filter.in_price_pool_only && !row.in_price_pool {
        return false;
    }
    if filter.strike_zone_only
        && (!row.in_price_pool
            || row.latest_price.is_none()
            || row.strike_price.is_none()
            || !row
                .distance_to_strike_pct
                .is_some_and(|d| d <= Decimal::ZERO))
    {
        return false;
    }
    if let Some(min_years) = filter.min_years_of_data
        && row.years_of_data < min_years
    {
        return false;
    }
    if let Some(min_roic) = filter.min_roic_5y
        && !row.roic_5y_avg_pct.is_some_and(|v| v >= min_roic)
    {
        return false;
    }
    if let Some(max_sbc) = filter.max_sbc_to_ocf
        && !row.sbc_to_ocf_pct.is_some_and(|v| v <= max_sbc)
    {
        return false;
    }
    if let Some(max_share) = filter.max_share_change_3y
        && !row.share_change_3y_pct.is_some_and(|v| v <= max_share)
    {
        return false;
    }
    if let Some(max_nd) = filter.max_net_debt_to_oe
        && !row.net_debt_to_oe.is_some_and(|v| v <= max_nd)
    {
        return false;
    }
    true
}

fn cmp_opt_ord<T: Ord>(a: Option<T>, b: Option<T>, asc: bool) -> Ordering {
    match (a, b) {
        (Some(va), Some(vb)) => {
            if asc {
                va.cmp(&vb)
            } else {
                vb.cmp(&va)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn cmp_opt_desc<T: Ord>(a: Option<T>, b: Option<T>) -> Ordering {
    cmp_opt_ord(a, b, false)
}

fn sort_universe_rows(rows: &mut [UniverseRow], sort_by: UniverseSortBy, sort_asc: bool) {
    rows.sort_by(|a, b| {
        let primary = match sort_by {
            UniverseSortBy::QualityScore => {
                if sort_asc {
                    a.quality_score.cmp(&b.quality_score)
                } else {
                    b.quality_score.cmp(&a.quality_score)
                }
            }
            UniverseSortBy::Roic5yAvg => {
                cmp_opt_ord(a.roic_5y_avg_pct, b.roic_5y_avg_pct, sort_asc)
            }
            UniverseSortBy::StrictFcfYield => {
                cmp_opt_ord(a.strict_fcf_yield_pct, b.strict_fcf_yield_pct, sort_asc)
            }
            UniverseSortBy::DistanceToStrike => {
                cmp_opt_ord(a.distance_to_strike_pct, b.distance_to_strike_pct, sort_asc)
            }
            UniverseSortBy::ShareChange3y => {
                cmp_opt_ord(a.share_change_3y_pct, b.share_change_3y_pct, sort_asc)
            }
            UniverseSortBy::SbcToOcf => cmp_opt_ord(a.sbc_to_ocf_pct, b.sbc_to_ocf_pct, sort_asc),
            UniverseSortBy::LatestPrice => cmp_opt_ord(a.latest_price, b.latest_price, sort_asc),
            UniverseSortBy::StrikePrice => cmp_opt_ord(a.strike_price, b.strike_price, sort_asc),
            UniverseSortBy::WarningsCount => {
                if sort_asc {
                    a.warnings_count.cmp(&b.warnings_count)
                } else {
                    b.warnings_count.cmp(&a.warnings_count)
                }
            }
            UniverseSortBy::Ticker => {
                if sort_asc {
                    a.ticker.cmp(&b.ticker)
                } else {
                    b.ticker.cmp(&a.ticker)
                }
            }
            UniverseSortBy::Name => {
                if sort_asc {
                    a.name.cmp(&b.name)
                } else {
                    b.name.cmp(&a.name)
                }
            }
            UniverseSortBy::Sic => cmp_opt_ord(a.sic, b.sic, sort_asc),
            UniverseSortBy::StrictOwnerEarnings => {
                cmp_opt_ord(a.strict_owner_earnings, b.strict_owner_earnings, sort_asc)
            }
        };
        primary
            .then_with(|| b.quality_score.cmp(&a.quality_score))
            .then_with(|| a.ticker.cmp(&b.ticker))
    });
}

fn compute_exact_percentile<I: Iterator<Item = Decimal>>(
    target: Option<Decimal>,
    peers: I,
) -> Option<Decimal> {
    let target_val = target?;
    let mut le_count = 0usize;
    let mut total = 0usize;
    for v in peers {
        total += 1;
        if v <= target_val {
            le_count += 1;
        }
    }
    if total == 0 {
        return None;
    }
    let pct = (Decimal::from(le_count) * Decimal::from(100)) / Decimal::from(total);
    Some(pct.round_dp(2))
}

fn annual_to_metric_row(a: LakeAnnualRow) -> AnnualMetricRow {
    AnnualMetricRow {
        label: a.label,
        period_end: a.period_end,
        revenue: a.revenue,
        gross_profit: a.gross_profit,
        gross_margin_pct: a.gross_margin_pct,
        ebit: a.ebit,
        nopat: a.nopat,
        invested_capital: a.invested_capital,
        roic_pct: a.roic_pct,
        roic_note: a.roic_note,
        roic_total_capital_pct: a.roic_total_capital_pct,
        ocf: a.ocf,
        capex: a.capex,
        fcf: a.fcf,
        sbc: a.sbc,
        strict_owner_earnings: a.strict_owner_earnings,
        sbc_to_ocf_pct: a.sbc_to_ocf_pct,
        buybacks: a.buybacks,
        cash_and_st: a.cash_and_st,
        total_debt: a.total_debt,
        net_debt: a.net_debt,
        equity: a.equity,
        diluted_shares: a.diluted_shares,
    }
}

fn read_ttm_metric_row(row: &duckdb::Row<'_>) -> duckdb::Result<(String, AnnualMetricRow, String)> {
    let cik: String = row.get(0)?;
    let label: String = row.get(1)?;
    let period_end: String = row.get(2)?;
    let exact_json: String = row.get(24)?;
    let provenance_json: String = row.get(25)?;
    let exact_opt: Option<TtmExactDecimals> = (!exact_json.is_empty())
        .then(|| serde_json::from_str(&exact_json).ok())
        .flatten();

    let metric = if let Some(ex) = exact_opt {
        AnnualMetricRow {
            label,
            period_end,
            revenue: ex.revenue,
            gross_profit: ex.gross_profit,
            gross_margin_pct: ex.gross_margin_pct,
            ebit: ex.ebit,
            nopat: ex.nopat,
            invested_capital: ex.invested_capital,
            roic_pct: ex.roic_pct,
            roic_note: row.get(10)?,
            roic_total_capital_pct: ex.roic_total_capital_pct,
            ocf: ex.ocf,
            capex: ex.capex,
            fcf: ex.fcf,
            sbc: ex.sbc,
            strict_owner_earnings: ex.strict_owner_earnings,
            sbc_to_ocf_pct: ex.sbc_to_ocf_pct,
            buybacks: ex.buybacks,
            cash_and_st: ex.cash_and_st,
            total_debt: ex.total_debt,
            net_debt: ex.net_debt,
            equity: ex.equity,
            diluted_shares: ex.diluted_shares,
        }
    } else {
        let nopat: Option<Decimal> = row.get(7)?;
        let invested_capital: Option<Decimal> = row.get(8)?;
        let sbc: Option<Decimal> = row.get(15)?;
        let strict_owner_earnings: Option<Decimal> = row.get(16)?;
        let buybacks: Option<Decimal> = row.get(18)?;
        let total_debt: Option<Decimal> = row.get(20)?;
        let net_debt: Option<Decimal> = row.get(21)?;
        AnnualMetricRow {
            label,
            period_end,
            revenue: row.get(3)?,
            gross_profit: row.get(4)?,
            gross_margin_pct: row.get(5)?,
            ebit: row.get(6)?,
            nopat,
            invested_capital,
            roic_pct: row.get(9)?,
            roic_note: row.get(10)?,
            roic_total_capital_pct: row.get(11)?,
            ocf: row.get(12)?,
            capex: row.get(13)?,
            fcf: row.get(14)?,
            sbc,
            strict_owner_earnings,
            sbc_to_ocf_pct: row.get(17)?,
            buybacks,
            cash_and_st: row.get(19)?,
            total_debt,
            net_debt,
            equity: row.get(22)?,
            diluted_shares: row.get(23)?,
        }
    };
    Ok((cik, metric, provenance_json))
}

fn serialize_universe_exact(u: &UniverseRow, quote_market_time_ms: Option<u64>) -> Result<String> {
    let exact = UniverseExactDecimals {
        quality_score: u.quality_score,
        latest_price: u.latest_price,
        strike_price: u.strike_price,
        distance_to_strike_pct: u.distance_to_strike_pct,
        strict_fcf_yield_pct: u.strict_fcf_yield_pct,
        quote_market_time_ms,
        roic_5y_avg_pct: u.roic_5y_avg_pct,
        roic_5y_stddev_pct: u.roic_5y_stddev_pct,
        gross_margin_5y_avg_pct: u.gross_margin_5y_avg_pct,
        gross_margin_5y_stddev_pct: u.gross_margin_5y_stddev_pct,
        share_change_3y_pct: u.share_change_3y_pct,
        sbc_to_ocf_pct: u.sbc_to_ocf_pct,
        net_debt_to_oe: u.net_debt_to_oe,
        strict_owner_earnings: u.strict_owner_earnings,
    };
    Ok(encode_universe_exact(&exact))
}

pub(crate) fn read_universe_row(row: &duckdb::Row<'_>) -> duckdb::Result<UniverseRow> {
    let sic_opt: Option<i32> = row.get(4)?;
    let years_i32: i32 = row.get(8)?;
    let bucket_opt = match row.get_ref(12)? {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes).ok().and_then(str_to_bucket),
        _ => None,
    };
    let warn_i32: i32 = row.get(25)?;
    let exact_opt: Option<UniverseExactDecimals> = match row.get_ref(28)? {
        ValueRef::Text(bytes) if !bytes.is_empty() => decode_universe_exact_bytes(bytes),
        _ => None,
    };

    let (
        quality_score,
        latest_price,
        strike_price,
        distance_to_strike_pct,
        strict_fcf_yield_pct,
        roic_5y_avg_pct,
        roic_5y_stddev_pct,
        gross_margin_5y_avg_pct,
        gross_margin_5y_stddev_pct,
        share_change_3y_pct,
        sbc_to_ocf_pct,
        net_debt_to_oe,
        strict_owner_earnings,
    ) = if let Some(ex) = exact_opt {
        (
            ex.quality_score,
            ex.latest_price,
            ex.strike_price,
            ex.distance_to_strike_pct,
            ex.strict_fcf_yield_pct,
            ex.roic_5y_avg_pct,
            ex.roic_5y_stddev_pct,
            ex.gross_margin_5y_avg_pct,
            ex.gross_margin_5y_stddev_pct,
            ex.share_change_3y_pct,
            ex.sbc_to_ocf_pct,
            ex.net_debt_to_oe,
            ex.strict_owner_earnings,
        )
    } else {
        (
            row.get(9)?,
            row.get(13)?,
            row.get(14)?,
            row.get(15)?,
            row.get(16)?,
            row.get(17)?,
            row.get(18)?,
            row.get(19)?,
            row.get(20)?,
            row.get(21)?,
            row.get(22)?,
            row.get(23)?,
            row.get(24)?,
        )
    };

    Ok(UniverseRow {
        ticker: row.get(0)?,
        cik: row.get(1)?,
        name: row.get(2)?,
        exchange: row.get(3)?,
        sic: sic_opt.and_then(|v| u32::try_from(v).ok()),
        sic_description: row.get(5)?,
        is_financial: row.get(6)?,
        latest_filing_end: row.get(7)?,
        years_of_data: usize::try_from(years_i32.max(0)).unwrap_or(0),
        quality_score,
        in_price_pool: row.get(10)?,
        in_watchlist: row.get(11)?,
        bucket_opt,
        latest_price,
        strike_price,
        distance_to_strike_pct,
        strict_fcf_yield_pct,
        roic_5y_avg_pct,
        roic_5y_stddev_pct,
        gross_margin_5y_avg_pct,
        gross_margin_5y_stddev_pct,
        share_change_3y_pct,
        sbc_to_ocf_pct,
        net_debt_to_oe,
        strict_owner_earnings,
        warnings_count: usize::try_from(warn_i32.max(0)).unwrap_or(0),
        warnings_json: row.get(26)?,
        raw_sha256: row.get(27)?,
    })
}

fn read_annual_row(row: &duckdb::Row<'_>) -> duckdb::Result<LakeAnnualRow> {
    let exact_json: String = row.get(30)?;
    let exact_opt: Option<AnnualExactDecimals> = (!exact_json.is_empty())
        .then(|| serde_json::from_str(&exact_json).ok())
        .flatten();

    let (
        revenue,
        gross_profit,
        gross_margin_pct,
        ebit,
        nopat,
        invested_capital,
        roic_pct,
        roic_total_capital_pct,
        ocf,
        capex,
        fcf,
        sbc,
        strict_owner_earnings,
        sbc_to_ocf_pct,
        buybacks,
        cash_and_st,
        total_debt,
        net_debt,
        equity,
        diluted_shares,
        ending_shares,
    ) = if let Some(ex) = exact_opt {
        (
            ex.revenue,
            ex.gross_profit,
            ex.gross_margin_pct,
            ex.ebit,
            ex.nopat,
            ex.invested_capital,
            ex.roic_pct,
            ex.roic_total_capital_pct,
            ex.ocf,
            ex.capex,
            ex.fcf,
            ex.sbc,
            ex.strict_owner_earnings,
            ex.sbc_to_ocf_pct,
            ex.buybacks,
            ex.cash_and_st,
            ex.total_debt,
            ex.net_debt,
            ex.equity,
            ex.diluted_shares,
            ex.ending_shares,
        )
    } else {
        let nopat_sql: Option<Decimal> = row.get(10)?;
        let ic_sql: Option<Decimal> = row.get(11)?;
        let sbc_sql: Option<Decimal> = row.get(18)?;
        let oe_sql: Option<Decimal> = row.get(19)?;
        let buybacks_sql: Option<Decimal> = row.get(21)?;
        let debt_sql: Option<Decimal> = row.get(23)?;
        let net_debt_sql: Option<Decimal> = row.get(24)?;
        (
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            nopat_sql,
            ic_sql,
            row.get(12)?,
            row.get(14)?,
            row.get(15)?,
            row.get(16)?,
            row.get(17)?,
            sbc_sql,
            oe_sql,
            row.get(20)?,
            buybacks_sql,
            row.get(22)?,
            debt_sql,
            net_debt_sql,
            row.get(25)?,
            row.get(26)?,
            row.get(27)?,
        )
    };

    Ok(LakeAnnualRow {
        cik: row.get(0)?,
        ticker: row.get(1)?,
        fiscal_year: row.get(2)?,
        label: row.get(3)?,
        period_end: row.get(4)?,
        filed: row.get(5)?,
        revenue,
        gross_profit,
        gross_margin_pct,
        ebit,
        nopat,
        invested_capital,
        roic_pct,
        roic_note: row.get(13)?,
        roic_total_capital_pct,
        ocf,
        capex,
        fcf,
        sbc,
        strict_owner_earnings,
        sbc_to_ocf_pct,
        buybacks,
        cash_and_st,
        total_debt,
        net_debt,
        equity,
        diluted_shares,
        ending_shares,
        raw_sha256: row.get(28)?,
        provenance_json: row.get(29)?,
    })
}

fn bucket_to_str(b: Bucket) -> &'static str {
    match b {
        Bucket::Core => "core",
        Bucket::Watch => "watch",
        Bucket::Out => "out",
        Bucket::TooHard => "too_hard",
    }
}

fn str_to_bucket(s: &str) -> Option<Bucket> {
    match s.trim().to_ascii_lowercase().as_str() {
        "core" | "in" => Some(Bucket::Core),
        "watch" => Some(Bucket::Watch),
        "out" => Some(Bucket::Out),
        "too_hard" => Some(Bucket::TooHard),
        _ => None,
    }
}

pub(crate) fn escape_sql_literal(s: &str) -> String {
    s.replace('\'', "''")
}

fn cleanup_stale_parquet_staging_dirs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if (name_str.starts_with(".staging-") || name_str.starts_with(".manifest.json.tmp"))
            && entry.path().is_dir()
        {
            let _ = fs::remove_dir_all(entry.path());
        } else if name_str.starts_with(".manifest.") && entry.path().is_file() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn cleanup_old_parquet_generations(dir: &Path, current_gen: &str, prev_gen: Option<&str>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() && name_str.starts_with("gen-") {
            if name_str != current_gen && prev_gen != Some(name_str.as_ref()) {
                let _ = fs::remove_dir_all(&path);
            }
        } else if path.is_file()
            && matches!(
                name_str.as_ref(),
                "companies.parquet"
                    | "facts_annual.parquet"
                    | "facts_ttm.parquet"
                    | "prices_daily.parquet"
            )
        {
            // Remove legacy flat files once immutable generation + manifest.json is live.
            let _ = fs::remove_file(&path);
        }
    }
}

fn file_sha256_hex(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn duckdb_dec(d: Decimal, scale: u32, max_int_digits: u32) -> Option<Decimal> {
    let rounded = d.round_dp(scale);
    if let Some(pow) = 10i128.checked_pow(max_int_digits)
        && let Ok(limit) = Decimal::try_from_i128_with_scale(pow, 0)
        && rounded.abs() >= limit
    {
        return None;
    }
    Some(rounded)
}

pub(crate) fn round_for_sql(d: Decimal, scale: u32) -> Decimal {
    d.round_dp(scale)
}

fn money_dec(d: Decimal) -> Decimal {
    duckdb_dec(d, 6, 30).unwrap_or(Decimal::ZERO)
}

fn opt_money_dec(d: Option<Decimal>) -> Option<Decimal> {
    d.and_then(|v| duckdb_dec(v, 6, 30))
}

fn opt_ratio_dec(d: Option<Decimal>) -> Option<Decimal> {
    d.and_then(|v| duckdb_dec(v, 10, 26))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CompanyEntry, DataSource, ValuationAssumptions, WarningSeverity, WatchlistFile,
    };
    use std::time::Instant;

    fn temp_paths(tag: &str) -> WorkspacePaths {
        let dir = std::env::temp_dir().join(format!(
            "m3-lake-test-{tag}-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workbench")).unwrap();
        WorkspacePaths::from_root(dir)
    }

    struct TestRowSpec<'a> {
        ticker: &'a str,
        cik: &'a str,
        sic: Option<u32>,
        is_financial: bool,
        roic: Option<Decimal>,
        gm: Option<Decimal>,
        fcf_yield: Option<Decimal>,
        sbc_ocf: Option<Decimal>,
        share_3y: Option<Decimal>,
        dist: Option<Decimal>,
        quality: Decimal,
    }

    fn make_universe_row(spec: TestRowSpec<'_>) -> UniverseRow {
        UniverseRow {
            ticker: spec.ticker.to_owned(),
            cik: spec.cik.to_owned(),
            name: format!("{} Corp.", spec.ticker),
            exchange: "Nasdaq".to_owned(),
            sic: spec.sic,
            sic_description: spec.sic.map(|s| format!("Industry {s}")),
            is_financial: spec.is_financial,
            latest_filing_end: Some("2025-12-31".to_owned()),
            years_of_data: 6,
            quality_score: spec.quality,
            in_price_pool: true,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: Some(Decimal::from(100)),
            strike_price: Some(Decimal::from(110)),
            distance_to_strike_pct: spec.dist,
            strict_fcf_yield_pct: spec.fcf_yield,
            roic_5y_avg_pct: spec.roic,
            roic_5y_stddev_pct: Some(Decimal::new(15, 1)),
            gross_margin_5y_avg_pct: spec.gm,
            gross_margin_5y_stddev_pct: Some(Decimal::new(8, 1)),
            share_change_3y_pct: spec.share_3y,
            sbc_to_ocf_pct: spec.sbc_ocf,
            net_debt_to_oe: Some(Decimal::new(-5, 1)),
            strict_owner_earnings: Some(Decimal::from(1_000_000_000)),
            warnings_count: 1,
            warnings_json: serde_json::to_string(&vec![WarningFlag {
                code: "TEST_WARN".to_owned(),
                severity: WarningSeverity::Info,
                title: "Info".to_owned(),
                detail: "Test".to_owned(),
            }])
            .unwrap(),
            raw_sha256: Some("deadbeef".to_owned()),
        }
    }

    #[test]
    fn watermarks_parquet_roundtrip_screening_and_peers_work_under_100ms() {
        let paths = temp_paths("roundtrip");
        let mut db = LakeDb::open(&paths).unwrap();

        let wm = SyncWatermark {
            source_key: "sec_companyfacts_zip".to_owned(),
            cursor: Some("2026-11-23".to_owned()),
            etag: Some("\"etag-123\"".to_owned()),
            last_modified: Some("Mon, 23 Nov 2026 00:00:00 GMT".to_owned()),
            synced_at_ms: 1_795_400_000_000,
        };
        db.write_watermark(&wm).unwrap();
        assert_eq!(db.read_watermark("sec_companyfacts_zip").unwrap(), Some(wm));

        let companies = vec![
            LakeCompanyRow {
                cik: "0000769397".into(),
                ticker: "ADSK".into(),
                name: "Autodesk, Inc.".into(),
                exchange: "Nasdaq".into(),
                sic: Some(7372),
                sic_description: Some("Prepackaged Software".into()),
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000857005".into(),
                ticker: "PTC".into(),
                name: "PTC Inc.".into(),
                exchange: "Nasdaq".into(),
                sic: Some(7372),
                sic_description: Some("Prepackaged Software".into()),
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000883241".into(),
                ticker: "SNPS".into(),
                name: "Synopsys, Inc.".into(),
                exchange: "Nasdaq".into(),
                sic: Some(7372),
                sic_description: Some("Prepackaged Software".into()),
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000796343".into(),
                ticker: "ADBE".into(),
                name: "Adobe Inc.".into(),
                exchange: "Nasdaq".into(),
                sic: Some(7372),
                sic_description: Some("Prepackaged Software".into()),
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000070858".into(),
                ticker: "BAC".into(),
                name: "Bank of America".into(),
                exchange: "NYSE".into(),
                sic: Some(6021),
                sic_description: Some("National Commercial Banks".into()),
                is_financial: true,
                is_active: true,
            },
        ];

        // Preserve exact 20+ digit decimal precision across Parquet roundtrip.
        let exact_roic = Decimal::from_str("28.123456789012345678901234").unwrap();
        let annuals = vec![LakeAnnualRow {
            cik: "0000769397".into(),
            ticker: "ADSK".into(),
            fiscal_year: 2025,
            label: "FY2025".into(),
            period_end: "2025-01-31".into(),
            filed: Some("2025-03-14".into()),
            revenue: Decimal::from(6_131_000_000i64),
            gross_profit: Decimal::from(5_600_000_000i64),
            gross_margin_pct: Some(Decimal::from_str("91.339096395367802968520632").unwrap()),
            ebit: Decimal::from(1_350_000_000i64),
            nopat: Some(Decimal::from(1_080_000_000i64)),
            invested_capital: Some(Decimal::from(3_840_000_000i64)),
            roic_pct: Some(exact_roic),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(24)),
            ocf: Decimal::from(1_600_000_000i64),
            capex: Decimal::from(40_000_000i64),
            fcf: Decimal::from(1_560_000_000i64),
            sbc: Some(Decimal::from(680_000_000i64)),
            strict_owner_earnings: Some(Decimal::from(880_000_000i64)),
            sbc_to_ocf_pct: Some(Decimal::new(425, 1)),
            buybacks: Some(Decimal::from(850_000_000i64)),
            cash_and_st: Some(Decimal::from(2_000_000_000i64)),
            total_debt: Some(Decimal::from(2_300_000_000i64)),
            net_debt: Some(Decimal::from(300_000_000i64)),
            equity: Decimal::from(2_500_000_000i64),
            diluted_shares: Decimal::from(216_000_000i64),
            ending_shares: Decimal::from(214_000_000i64),
            raw_sha256: "abc123".into(),
            provenance_json: "[]".into(),
        }];

        let mut snapshot = vec![
            make_universe_row(TestRowSpec {
                ticker: "ADSK",
                cik: "0000769397",
                sic: Some(7372),
                is_financial: false,
                roic: Some(exact_roic),
                gm: Some(Decimal::from(91)),
                fcf_yield: Some(Decimal::new(420, 2)),
                sbc_ocf: Some(Decimal::from(42)),
                share_3y: Some(Decimal::from(-2)),
                dist: Some(Decimal::from(12)),
                quality: Decimal::from(82),
            }),
            make_universe_row(TestRowSpec {
                ticker: "PTC",
                cik: "0000857005",
                sic: Some(7372),
                is_financial: false,
                roic: Some(Decimal::from(22)),
                gm: Some(Decimal::from(80)),
                fcf_yield: Some(Decimal::new(510, 2)),
                sbc_ocf: Some(Decimal::from(18)),
                share_3y: Some(Decimal::from(1)),
                dist: Some(Decimal::from(-5)),
                quality: Decimal::from(86),
            }),
            make_universe_row(TestRowSpec {
                ticker: "SNPS",
                cik: "0000883241",
                sic: Some(7372),
                is_financial: false,
                roic: Some(Decimal::from(25)),
                gm: Some(Decimal::from(79)),
                fcf_yield: Some(Decimal::new(310, 2)),
                sbc_ocf: Some(Decimal::from(24)),
                share_3y: Some(Decimal::from(0)),
                dist: Some(Decimal::from(20)),
                quality: Decimal::from(84),
            }),
            make_universe_row(TestRowSpec {
                ticker: "ADBE",
                cik: "0000796343",
                sic: Some(7372),
                is_financial: false,
                roic: Some(Decimal::from(35)),
                gm: Some(Decimal::from(88)),
                fcf_yield: Some(Decimal::new(480, 2)),
                sbc_ocf: Some(Decimal::from(19)),
                share_3y: Some(Decimal::from(-5)),
                dist: Some(Decimal::from(-8)),
                quality: Decimal::from(93),
            }),
            make_universe_row(TestRowSpec {
                ticker: "BAC",
                cik: "0000070858",
                sic: Some(6021),
                is_financial: true,
                roic: Some(Decimal::from(40)),
                gm: Some(Decimal::from(95)),
                fcf_yield: Some(Decimal::new(800, 2)),
                sbc_ocf: Some(Decimal::from(5)),
                share_3y: Some(Decimal::from(-3)),
                dist: Some(Decimal::from(-10)),
                quality: Decimal::from(0),
            }),
        ];

        // Scale-test 5,000 synthetic companies for <100ms query latency verification.
        for i in 0..5_000u32 {
            let t = format!("SYN{i:04}");
            let cik = format!("{:010}", 9_000_000 + i);
            snapshot.push(make_universe_row(TestRowSpec {
                ticker: &t,
                cik: &cik,
                sic: Some(3571),
                is_financial: false,
                roic: Some(Decimal::from(i % 20)),
                gm: Some(Decimal::from(50)),
                fcf_yield: Some(Decimal::new((i % 9) as i64 + 1, 0)),
                sbc_ocf: Some(Decimal::from(i % 30)),
                share_3y: Some(Decimal::from(0)),
                dist: Some(Decimal::from((i as i64 % 30) - 10)),
                quality: Decimal::from(i % 100),
            }));
        }

        let fresh_now_ms = store::now_ms();
        let fresh_mkt_ms = fresh_now_ms.saturating_sub(86_400_000);
        let fresh_date = crate::market::civil_iso_from_unix_secs((fresh_mkt_ms / 1000) as i64);
        let quote_times: HashMap<String, u64> = snapshot
            .iter()
            .take(5)
            .map(|r| (r.cik.clone(), fresh_mkt_ms))
            .collect();
        db.replace_sec_lake_data_with_quotes_and_watermarks(
            &companies,
            &annuals,
            &[],
            &snapshot,
            &quote_times,
            &[],
        )
        .unwrap();
        let fresh_prices: Vec<LakePriceDailyRow> = ["ADSK", "PTC", "SNPS", "ADBE", "BAC"]
            .into_iter()
            .map(|sym| LakePriceDailyRow {
                symbol: sym.into(),
                trade_date: fresh_date.clone(),
                price: Decimal::from(100),
                previous_close: None,
                currency: "USD".into(),
                exchange: "Nasdaq".into(),
                market_time_ms: Some(fresh_mkt_ms),
                fetched_at_ms: fresh_now_ms,
                source_url: "https://price.test".into(),
            })
            .collect();
        db.upsert_prices_and_update_snapshot(&fresh_prices, &[], &[])
            .unwrap();
        db.export_all_parquets(&paths).unwrap();

        let manifest = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        assert_eq!(manifest.files.len(), 4);
        assert!(!manifest.generation_dir.is_empty());
        for entry in &manifest.files {
            assert!(paths.lake_parquet_dir().join(&entry.rel_path).exists());
        }
        assert!(!paths.lake_parquet_dir().join("companies.parquet").exists());

        // Exact decimal parity via Parquet view
        let back_annuals = db.get_annual_rows("ADSK").unwrap();
        assert_eq!(back_annuals.len(), 1);
        assert_eq!(back_annuals[0].roic_pct, Some(exact_roic));
        assert_eq!(
            back_annuals[0].strict_owner_earnings,
            Some(Decimal::from(880_000_000i64))
        );

        // Exact screening boundary check: SQL rounds exact_roic (28.123456789012345...) to 10dp (...1234567890),
        // while exact threshold > exact_roic must NOT return ADSK!
        let slightly_above_adsk = Decimal::from_str("28.123456789012345678901235").unwrap();
        let exact_bound_hits = db
            .screen_universe(&UniverseFilter {
                min_roic_5y: Some(slightly_above_adsk),
                exclude_financials: true,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(!exact_bound_hits.iter().any(|r| r.ticker == "ADSK"));
        assert!(exact_bound_hits.iter().any(|r| r.ticker == "ADBE"));

        // Screen excludes financials by default, respects filters, and runs <100ms on 5,005 rows
        let t0 = Instant::now();
        let screened = db
            .screen_universe(&UniverseFilter {
                min_roic_5y: Some(Decimal::from(20)),
                max_sbc_to_ocf: Some(Decimal::from(25)),
                exclude_financials: true,
                strike_zone_only: true,
                sort_by: UniverseSortBy::Roic5yAvg,
                sort_asc: false,
                limit: 50,
                ..Default::default()
            })
            .unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() < 100,
            "screen took too long: {elapsed:?}"
        );
        assert!(!screened.iter().any(|r| r.ticker == "BAC"));
        assert_eq!(screened[0].ticker, "ADBE");
        assert!(screened.iter().any(|r| r.ticker == "PTC"));

        // Verify broad 5,000-row query timing and coherent Parquet manifest.
        let t5000 = Instant::now();
        let broad = db
            .screen_universe(&UniverseFilter {
                exclude_financials: true,
                limit: 5_000,
                ..Default::default()
            })
            .unwrap();
        let elapsed_5000 = t5000.elapsed();
        assert_eq!(broad.len(), 5_000);
        #[cfg(not(debug_assertions))]
        assert!(
            elapsed_5000.as_millis() < 100,
            "limit=5000 release query took too long: {elapsed_5000:?}"
        );
        #[cfg(debug_assertions)]
        let _ = elapsed_5000;

        let manifest = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        assert_eq!(manifest.files.len(), 4);
        for entry in &manifest.files {
            assert!(entry.bytes > 0, "empty manifest entry {}", entry.file);
            assert_eq!(entry.sha256.len(), 64);
        }
        let leftover_staging: Vec<_> = fs::read_dir(paths.lake_parquet_dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
            .collect();
        assert!(leftover_staging.is_empty());

        // Summary counts
        let summary = db.universe_summary().unwrap();
        assert_eq!(summary.total_companies, 5_005);
        assert_eq!(summary.non_financial_companies, 5_004);
        assert_eq!(summary.parsed_financial_companies, 5_005);

        // Peer percentiles on SIC 7372
        let peers = db.query_peers("ADSK", 10).unwrap();
        assert_eq!(peers.target.roic_5y_avg_pct, Some(exact_roic));
        assert_eq!(peers.peer_count, 3); // PTC, SNPS, ADBE
        assert!(peers.sic_group.contains("7372"));
        // ADSK ROIC (28.12%) > PTC (22%) and SNPS (25%), < ADBE (35%) -> 2/3 = 66.67%
        assert_eq!(peers.roic_percentile, Some(Decimal::new(6667, 2)));

        // Watchlist refresh updates flags cleanly
        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![CompanyEntry {
                    ticker: "PTC".into(),
                    name: "PTC Inc.".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0000857005".into()),
                    currency: "USD".into(),
                    circle: "CAD".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(18),
                        multiple_high: Decimal::from(22),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(9),
                        strike_price_override: None,
                    },
                }],
            },
        )
        .unwrap();
        db.refresh_watchlist_flags(&paths).unwrap();
        let ptc = db.query_peers("PTC", 5).unwrap().target;
        assert!(ptc.in_watchlist);
        assert_eq!(ptc.bucket_opt, Some(Bucket::Core));

        let t_broad = Instant::now();
        let broad_one = db
            .screen_universe(&UniverseFilter {
                exclude_financials: false,
                limit: 1,
                ..Default::default()
            })
            .unwrap();
        let broad_elapsed = t_broad.elapsed();
        assert_eq!(broad_one.len(), 1);
        assert!(
            broad_elapsed.as_millis() < 100,
            "broad query took too long: {broad_elapsed:?}"
        );

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn sec_replace_rollback_quote_safety_and_pool_clearing_work() {
        let paths = temp_paths("safety");
        let mut db = LakeDb::open(&paths).unwrap();
        let comp = vec![LakeCompanyRow {
            cik: "0000320193".into(),
            ticker: "AAPL".into(),
            name: "Apple Inc.".into(),
            exchange: "Nasdaq".into(),
            sic: Some(3571),
            sic_description: Some("Computers".into()),
            is_financial: false,
            is_active: true,
        }];
        let snap_row = UniverseRow {
            ticker: "AAPL".into(),
            cik: "0000320193".into(),
            name: "Apple Inc.".into(),
            exchange: "Nasdaq".into(),
            sic: Some(3571),
            sic_description: Some("Computers".into()),
            is_financial: false,
            latest_filing_end: Some("2025-12-31".into()),
            years_of_data: 6,
            quality_score: Decimal::from(92),
            in_price_pool: true,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: Some(Decimal::from(100)),
            strike_price: Some(Decimal::from(110)),
            distance_to_strike_pct: Some(Decimal::from(-5)),
            strict_fcf_yield_pct: Some(Decimal::from(4)),
            roic_5y_avg_pct: Some(Decimal::from(30)),
            roic_5y_stddev_pct: Some(Decimal::new(15, 1)),
            gross_margin_5y_avg_pct: Some(Decimal::from(45)),
            gross_margin_5y_stddev_pct: Some(Decimal::new(8, 1)),
            share_change_3y_pct: Some(Decimal::from(-3)),
            sbc_to_ocf_pct: Some(Decimal::from(10)),
            net_debt_to_oe: Some(Decimal::new(-5, 1)),
            strict_owner_earnings: Some(Decimal::from(1_000_000_000)),
            warnings_count: 0,
            warnings_json: "[]".into(),
            raw_sha256: Some("sha_v1".into()),
        };
        db.replace_sec_lake_data_with_watermarks(&comp, &[], &[], &[snap_row], &[])
            .unwrap();

        // Empty batch must fail and leave AAPL untouched.
        assert!(
            db.replace_sec_lake_data_with_watermarks(&[], &[], &[], &[], &[])
                .is_err()
        );
        assert_eq!(db.universe_summary().unwrap().total_companies, 1);

        // Stale raw_sha256 quote update must NOT overwrite newer financials.
        db.upsert_prices_and_update_snapshot(
            &[LakePriceDailyRow {
                symbol: "AAPL".into(),
                trade_date: "2026-11-23".into(), // old bogus execution date
                price: Decimal::from(99),
                previous_close: None,
                currency: "USD".into(),
                exchange: "NMS".into(),
                market_time_ms: Some(1_795_208_402_000),
                fetched_at_ms: 1_795_413_110_691,
                source_url: "https://price.test".into(),
            }],
            &[QuoteSnapshotUpdate {
                cik: "0000320193".into(),
                expected_ticker: "AAPL".into(),
                expected_raw_sha256: Some("sha_stale".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: None,
                quote_market_time_ms: Some(1_795_208_402_000),
                quote_fetched_at_ms: 1_795_413_110_691,
                latest_price: Some(Decimal::from(99)),
                strike_price: Some(Decimal::from(99)),
                distance_to_strike_pct: Some(Decimal::ZERO),
                strict_fcf_yield_pct: Some(Decimal::from(1)),
            }],
            &[],
        )
        .unwrap();
        let after_stale = db.query_peers("AAPL", 1).unwrap().target;
        assert_eq!(after_stale.latest_price, Some(Decimal::from(100)));

        // Re-upserting with true Nov 20 session date deletes the bogus Nov 23 row for same market_time_ms.
        db.upsert_prices_and_update_snapshot(
            &[LakePriceDailyRow {
                symbol: "AAPL".into(),
                trade_date: "2026-11-20".into(),
                price: Decimal::from(105),
                previous_close: None,
                currency: "USD".into(),
                exchange: "NMS".into(),
                market_time_ms: Some(1_795_208_402_000),
                fetched_at_ms: 1_795_413_110_691,
                source_url: "https://price.test".into(),
            }],
            &[],
            &[],
        )
        .unwrap();
        let bogus_cnt: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM prices_daily WHERE symbol = 'AAPL' AND trade_date = '2026-11-23'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bogus_cnt, 0);

        // Removing AAPL from price pool clears stale quote fields & strike_zone screening.
        db.sync_price_pool_flags(&HashSet::new()).unwrap();
        let dropped = db.query_peers("AAPL", 1).unwrap().target;
        assert!(!dropped.in_price_pool);
        assert!(dropped.latest_price.is_none());
        assert!(dropped.distance_to_strike_pct.is_none());
        assert!(dropped.strict_fcf_yield_pct.is_none());
        let strike_hits = db
            .screen_universe(&UniverseFilter {
                strike_zone_only: true,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(strike_hits.is_empty());

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn duckdb_decimal_clamping_handles_normal_and_huge_values() {
        let normal = Decimal::from_str("1234567890123.123456789012345").unwrap();
        let rounded = duckdb_dec(normal, 10, 26).unwrap();
        assert_eq!(rounded.scale(), 10);

        let huge = Decimal::from_str("999999999999999999999999999").unwrap();
        assert!(duckdb_dec(huge, 10, 26).is_none());
        assert!(duckdb_dec(-huge, 10, 26).is_none());
    }

    #[test]
    fn absent_ttm_and_annual_inputs_stay_null_in_parquet_and_migrate_legacy_db() {
        let paths = temp_paths("nullable-parquet");
        fs::create_dir_all(paths.lake_dir()).unwrap();
        fs::create_dir_all(paths.lake_parquet_dir()).unwrap();
        fs::create_dir_all(paths.lake_raw_sec_dir()).unwrap();

        // Seed a legacy DuckDB schema with old NOT NULL constraints and a fake AIT strict_oe row.
        {
            let conn = Connection::open(paths.lake_db_path()).unwrap();
            conn.execute_batch(
                "CREATE TABLE facts_annual (
                    cik VARCHAR NOT NULL, ticker VARCHAR NOT NULL, fiscal_year INTEGER NOT NULL,
                    label VARCHAR NOT NULL, period_end VARCHAR NOT NULL, filed VARCHAR,
                    revenue DECIMAL(38,6) NOT NULL, gross_profit DECIMAL(38,6) NOT NULL,
                    gross_margin_pct DECIMAL(38,10), ebit DECIMAL(38,6) NOT NULL,
                    nopat DECIMAL(38,6) NOT NULL, invested_capital DECIMAL(38,6) NOT NULL,
                    roic_pct DECIMAL(38,10), roic_note VARCHAR, roic_total_capital_pct DECIMAL(38,10),
                    ocf DECIMAL(38,6) NOT NULL, capex DECIMAL(38,6) NOT NULL, fcf DECIMAL(38,6) NOT NULL,
                    sbc DECIMAL(38,6) NOT NULL, strict_owner_earnings DECIMAL(38,6) NOT NULL,
                    sbc_to_ocf_pct DECIMAL(38,10), buybacks DECIMAL(38,6) NOT NULL,
                    cash_and_st DECIMAL(38,6) NOT NULL, total_debt DECIMAL(38,6) NOT NULL,
                    net_debt DECIMAL(38,6) NOT NULL, equity DECIMAL(38,6) NOT NULL,
                    diluted_shares DECIMAL(38,6) NOT NULL, ending_shares DECIMAL(38,6) NOT NULL,
                    raw_sha256 VARCHAR NOT NULL, provenance_json VARCHAR NOT NULL,
                    exact_decimals_json VARCHAR NOT NULL,
                    PRIMARY KEY (cik, fiscal_year, period_end)
                );
                CREATE TABLE facts_ttm (
                    cik VARCHAR PRIMARY KEY, ticker VARCHAR NOT NULL, label VARCHAR NOT NULL,
                    period_end VARCHAR NOT NULL, revenue DECIMAL(38,6) NOT NULL,
                    gross_profit DECIMAL(38,6) NOT NULL, gross_margin_pct DECIMAL(38,10),
                    ebit DECIMAL(38,6) NOT NULL, nopat DECIMAL(38,6) NOT NULL,
                    invested_capital DECIMAL(38,6) NOT NULL, roic_pct DECIMAL(38,10),
                    roic_note VARCHAR, roic_total_capital_pct DECIMAL(38,10),
                    ocf DECIMAL(38,6) NOT NULL, capex DECIMAL(38,6) NOT NULL, fcf DECIMAL(38,6) NOT NULL,
                    sbc DECIMAL(38,6) NOT NULL, strict_owner_earnings DECIMAL(38,6) NOT NULL,
                    sbc_to_ocf_pct DECIMAL(38,10), buybacks DECIMAL(38,6) NOT NULL,
                    cash_and_st DECIMAL(38,6) NOT NULL, total_debt DECIMAL(38,6) NOT NULL,
                    net_debt DECIMAL(38,6) NOT NULL, equity DECIMAL(38,6) NOT NULL,
                    diluted_shares DECIMAL(38,6) NOT NULL, ytd_period_label VARCHAR NOT NULL,
                    ytd_start VARCHAR NOT NULL, ytd_end VARCHAR NOT NULL,
                    ytd_revenue DECIMAL(38,6) NOT NULL, ytd_ocf DECIMAL(38,6) NOT NULL,
                    ytd_capex DECIMAL(38,6) NOT NULL, ytd_sbc DECIMAL(38,6) NOT NULL,
                    ytd_buybacks DECIMAL(38,6) NOT NULL, raw_sha256 VARCHAR NOT NULL,
                    provenance_json VARCHAR NOT NULL, exact_decimals_json VARCHAR NOT NULL
                );
                INSERT INTO facts_ttm VALUES (
                    '0000109563', 'AIT', 'TTM (2026-09-30)', '2026-09-30', 5000, 1500, 30,
                    600, 480, 1000, 48, NULL, 40, 450, 41, 408.96,
                    0, 408.96, NULL, 0, 300, 282, -18, 1200, 38,
                    '2026 Q1 YTD', '2026-07-01', '2026-09-30', 1200, 110, 10, 0, 0,
                    'sha_old', '[{\"field\":\"ttm_sbc\",\"source_tag\":\"absent\"}]', '{\"dummy\":1}'
                );",
            )
            .unwrap();
        }

        let mut db = LakeDb::open(&paths).unwrap();
        assert!(db.has_missing_exact_decimals().unwrap());

        let companies = vec![
            LakeCompanyRow {
                cik: "0000109563".into(),
                ticker: "AIT".into(),
                name: "Applied Industrial".into(),
                exchange: "NYSE".into(),
                sic: Some(5080),
                sic_description: Some("Machinery".into()),
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0001575515".into(),
                ticker: "SFM".into(),
                name: "Sprouts".into(),
                exchange: "Nasdaq".into(),
                sic: Some(5411),
                sic_description: Some("Grocery".into()),
                is_financial: false,
                is_active: true,
            },
        ];
        let annuals = vec![LakeAnnualRow {
            cik: "0001575515".into(),
            ticker: "SFM".into(),
            fiscal_year: 2025,
            label: "FY2025".into(),
            period_end: "2025-12-28".into(),
            filed: Some("2026-02-20".into()),
            revenue: Decimal::from(8_000_000_000i64),
            gross_profit: Decimal::from(3_000_000_000i64),
            gross_margin_pct: Some(Decimal::new(375, 1)),
            ebit: Decimal::from(600_000_000i64),
            nopat: Some(Decimal::from(450_000_000i64)),
            invested_capital: Some(Decimal::from(1_000_000_000i64)),
            roic_pct: None,
            roic_note: Some("total_debt_unavailable".into()),
            roic_total_capital_pct: None,
            ocf: Decimal::from(700_000_000i64),
            capex: Decimal::from(200_000_000i64),
            fcf: Decimal::from(500_000_000i64),
            sbc: Some(Decimal::from(25_000_000i64)),
            strict_owner_earnings: Some(Decimal::from(475_000_000i64)),
            sbc_to_ocf_pct: Some(Decimal::new(357, 2)),
            buybacks: Some(Decimal::from(200_000_000i64)),
            cash_and_st: None,
            total_debt: None,
            net_debt: None,
            equity: Decimal::from(1_228_659_000i64),
            diluted_shares: Decimal::from(100_000_000i64),
            ending_shares: Decimal::from(100_000_000i64),
            raw_sha256: "sfm_sha".into(),
            provenance_json: "[{\"field\":\"cash_and_st\",\"source_tag\":\"absent\",\"period_end\":\"2025-12-28\"},{\"field\":\"total_debt\",\"source_tag\":\"absent\",\"filed\":null,\"period_end\":\"2025-12-28\",\"note\":\"absent_in_xbrl_assumed_zero\"}]".into(),
        }];
        let ttms = vec![
            LakeTtmRow {
                cik: "0000109563".into(),
                ticker: "AIT".into(),
                label: "TTM (2026-09-30)".into(),
                period_end: "2026-09-30".into(),
                revenue: Decimal::from(4_600_000_000i64),
                gross_profit: Decimal::from(1_400_000_000i64),
                gross_margin_pct: Some(Decimal::from(30)),
                ebit: Decimal::from(500_000_000i64),
                nopat: Some(Decimal::from(380_000_000i64)),
                invested_capital: Some(Decimal::from(1_500_000_000i64)),
                roic_pct: Some(Decimal::from(25)),
                roic_note: None,
                roic_total_capital_pct: Some(Decimal::from(24)),
                ocf: Decimal::from(450_000_000i64),
                capex: Decimal::from(41_040_000i64),
                fcf: Decimal::from(408_960_000i64),
                sbc: None,
                strict_owner_earnings: None,
                sbc_to_ocf_pct: None,
                buybacks: None,
                cash_and_st: Some(Decimal::from(400_000_000i64)),
                total_debt: Some(Decimal::from(282_300_000i64)),
                net_debt: Some(Decimal::from(-117_700_000i64)),
                equity: Decimal::from(1_600_000_000i64),
                diluted_shares: Decimal::from(38_000_000i64),
                ytd_period_label: "2026 Q1 YTD".into(),
                ytd_start: "2026-07-01".into(),
                ytd_end: "2026-09-30".into(),
                ytd_revenue: Some(Decimal::from(1_200_000_000i64)),
                ytd_ocf: Some(Decimal::from(110_000_000i64)),
                ytd_capex: Some(Decimal::from(10_000_000i64)),
                ytd_sbc: Some(Decimal::from(15_700_000i64)),
                ytd_buybacks: None,
                latest_quarter_label: None,
                latest_quarter_ocf: None,
                latest_quarter_capex: None,
                raw_sha256: "ait_sha".into(),
                provenance_json: "[{\"field\":\"ttm_sbc\",\"source_tag\":\"absent\"},{\"field\":\"ttm_buybacks\",\"source_tag\":\"absent\"},{\"field\":\"ytd_buybacks\",\"source_tag\":\"absent\"}]".into(),
            },
            LakeTtmRow {
                cik: "0001575515".into(),
                ticker: "SFM".into(),
                label: "TTM (2026-09-27)".into(),
                period_end: "2026-09-27".into(),
                revenue: Decimal::from(8_500_000_000i64),
                gross_profit: Decimal::from(3_200_000_000i64),
                gross_margin_pct: Some(Decimal::from(37)),
                ebit: Decimal::from(650_000_000i64),
                nopat: Some(Decimal::from(490_000_000i64)),
                invested_capital: None,
                roic_pct: None,
                roic_note: Some("total_debt_unavailable".into()),
                roic_total_capital_pct: None,
                ocf: Decimal::from(720_000_000i64),
                capex: Decimal::from(210_000_000i64),
                fcf: Decimal::from(510_000_000i64),
                sbc: Some(Decimal::from(26_000_000i64)),
                strict_owner_earnings: Some(Decimal::from(484_000_000i64)),
                sbc_to_ocf_pct: Some(Decimal::new(361, 2)),
                buybacks: Some(Decimal::from(210_000_000i64)),
                cash_and_st: None,
                total_debt: None,
                net_debt: None,
                equity: Decimal::from(1_300_000_000i64),
                diluted_shares: Decimal::from(98_000_000i64),
                ytd_period_label: "2026 Q3 YTD".into(),
                ytd_start: "2025-12-29".into(),
                ytd_end: "2026-09-27".into(),
                ytd_revenue: Some(Decimal::from(6_400_000_000i64)),
                ytd_ocf: Some(Decimal::from(540_000_000i64)),
                ytd_capex: Some(Decimal::from(150_000_000i64)),
                ytd_sbc: Some(Decimal::from(19_000_000i64)),
                ytd_buybacks: Some(Decimal::from(160_000_000i64)),
                latest_quarter_label: None,
                latest_quarter_ocf: None,
                latest_quarter_capex: None,
                raw_sha256: "sfm_sha".into(),
                provenance_json: "[{\"field\":\"ttm_cash_and_st\",\"source_tag\":\"absent\",\"period_end\":\"2026-09-27\"},{\"field\":\"ttm_total_debt\",\"source_tag\":\"absent\",\"period_end\":\"2026-09-27\"},{\"field\":\"ytd_buybacks\",\"source_tag\":\"us-gaap:PaymentsForRepurchaseOfCommonStock\",\"period_end\":\"2026-09-27\"}]"
                    .into(),
            },
        ];
        let snap = vec![
            make_universe_row(TestRowSpec {
                ticker: "AIT",
                cik: "0000109563",
                sic: Some(5080),
                is_financial: false,
                roic: Some(Decimal::from(20)),
                gm: Some(Decimal::from(30)),
                fcf_yield: None,
                sbc_ocf: None,
                share_3y: Some(Decimal::ZERO),
                dist: None,
                quality: Decimal::from(60),
            }),
            make_universe_row(TestRowSpec {
                ticker: "SFM",
                cik: "0001575515",
                sic: Some(5411),
                is_financial: false,
                roic: None,
                gm: Some(Decimal::from(37)),
                fcf_yield: Some(Decimal::from(5)),
                sbc_ocf: Some(Decimal::from(4)),
                share_3y: Some(Decimal::from(-4)),
                dist: Some(Decimal::from(10)),
                quality: Decimal::from(70),
            }),
        ];
        let now_ms = store::now_ms();
        let quote_times = HashMap::from([
            ("0000109563".to_owned(), now_ms),
            ("0001575515".to_owned(), now_ms),
        ]);
        let wms = vec![SyncWatermark {
            source_key: WM_SEC_EXTRACTOR_VERSION.to_owned(),
            cursor: Some(SEC_LAKE_EXTRACTOR_VERSION.to_owned()),
            etag: None,
            last_modified: None,
            synced_at_ms: now_ms,
        }];
        db.replace_sec_lake_data_with_quotes_and_watermarks(
            &companies,
            &annuals,
            &ttms,
            &snap,
            &quote_times,
            &wms,
        )
        .unwrap();
        db.export_all_parquets(&paths).unwrap();
        assert!(!db.has_missing_exact_decimals().unwrap());

        db.write_watermark(&SyncWatermark {
            source_key: WM_SEC_EXTRACTOR_VERSION.to_owned(),
            cursor: Some("1".to_owned()),
            etag: None,
            last_modified: None,
            synced_at_ms: now_ms,
        })
        .unwrap();
        assert!(db.has_missing_exact_decimals().unwrap());

        let manifest = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        let ttm_entry = manifest
            .files
            .iter()
            .find(|f| f.file == "facts_ttm.parquet")
            .unwrap();
        let ann_entry = manifest
            .files
            .iter()
            .find(|f| f.file == "facts_annual.parquet")
            .unwrap();
        let ttm_pq = paths.lake_parquet_dir().join(&ttm_entry.rel_path);
        let (ait_oe_is_null, ait_sbc_is_null, ait_buybacks_is_null, ait_ytd_bb_is_null, ait_ytd_sbc, ait_json): (
            bool,
            bool,
            bool,
            bool,
            Option<Decimal>,
            String,
        ) = db
            .conn
            .query_row(
                &format!(
                    "SELECT strict_owner_earnings IS NULL, sbc IS NULL, buybacks IS NULL, ytd_buybacks IS NULL, ytd_sbc, exact_decimals_json
                     FROM read_parquet('{}') WHERE ticker = 'AIT'",
                    escape_sql_literal(&ttm_pq.to_string_lossy())
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert!(ait_oe_is_null && ait_sbc_is_null && ait_buybacks_is_null && ait_ytd_bb_is_null);
        assert_eq!(ait_ytd_sbc, Some(Decimal::from(15_700_000i64)));
        assert!(ait_json.contains("\"strict_owner_earnings\":null"));
        assert!(ait_json.contains("\"buybacks\":null"));
        assert!(ait_json.contains("\"ytd_buybacks\":null"));
        assert!(ait_json.contains("\"ytd_sbc\":\"15700000\""));

        let (sfm_cash_null, sfm_debt_null, sfm_nd_null, sfm_ic_null, sfm_json): (
            bool,
            bool,
            bool,
            bool,
            String,
        ) = db
            .conn
            .query_row(
                &format!(
                    "SELECT cash_and_st IS NULL, total_debt IS NULL, net_debt IS NULL, invested_capital IS NULL, exact_decimals_json
                     FROM read_parquet('{}') WHERE ticker = 'SFM'",
                    escape_sql_literal(&ttm_pq.to_string_lossy())
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert!(sfm_cash_null && sfm_debt_null && sfm_nd_null && sfm_ic_null);
        assert!(sfm_json.contains("\"cash_and_st\":null"));
        assert!(sfm_json.contains("\"total_debt\":null"));
        assert!(sfm_json.contains("\"net_debt\":null"));
        assert!(sfm_json.contains("\"invested_capital\":null"));

        let ann_pq = paths.lake_parquet_dir().join(&ann_entry.rel_path);
        let (ann_cash_null, ann_debt_null, ann_nd_null, ann_ic_null): (bool, bool, bool, bool) = db
            .conn
            .query_row(
                &format!(
                    "SELECT cash_and_st IS NULL, total_debt IS NULL, net_debt IS NULL, invested_capital IS NULL
                     FROM read_parquet('{}') WHERE ticker = 'SFM' AND fiscal_year = 2025",
                    escape_sql_literal(&ann_pq.to_string_lossy())
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert!(ann_cash_null && ann_debt_null && ann_nd_null && ann_ic_null);
        let sfm_annuals = db.get_annual_rows("SFM").unwrap();
        assert_eq!(sfm_annuals.len(), 1);
        assert_eq!(sfm_annuals[0].cash_and_st, None);
        let sfm_sql_cash_null: bool = db
            .conn
            .query_row(
                "SELECT cash_and_st IS NULL FROM facts_ttm WHERE ticker = 'SFM'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sfm_sql_cash_null);

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn screen_universe_handles_large_tie_groups_and_reconciles_legacy_prices_daily() {
        let paths = temp_paths("ties-and-prices");
        let mut db = LakeDb::open(&paths).unwrap();

        let mut companies = Vec::new();
        let mut snapshot = Vec::new();
        // 150 rows sharing the exact same primary quality_score = 88 -> exceeds old +64 cutoff!
        for i in 0..150u32 {
            let cik = format!("{:010}", 5_000_000 + i);
            let ticker = format!("TIE{i:03}");
            companies.push(LakeCompanyRow {
                cik: cik.clone(),
                ticker: ticker.clone(),
                name: ticker.clone(),
                exchange: "Nasdaq".into(),
                sic: Some(7372),
                sic_description: None,
                is_financial: false,
                is_active: true,
            });
            // Vary ticker / 11th decimal ROIC so ties resolve deterministically without cutoff loss.
            snapshot.push(make_universe_row(TestRowSpec {
                ticker: &ticker,
                cik: &cik,
                sic: Some(7372),
                is_financial: false,
                roic: Some(Decimal::from_str(&format!("25.1234567890{i:03}")).unwrap()),
                gm: Some(Decimal::from(60)),
                fcf_yield: Some(Decimal::from(5)),
                sbc_ocf: Some(Decimal::from(5)),
                share_3y: Some(Decimal::ZERO),
                dist: Some(Decimal::ZERO),
                quality: Decimal::from(88),
            }));
        }
        // Reverse order on primary tie key via QualityScore DESC (quality_score equal, ticker ASC):
        // Also test Roic5yAvg DESC where all 150 tie to 10dp (`25.1234567890`)!
        db.replace_sec_lake_data_with_watermarks(&companies, &[], &[], &snapshot, &[])
            .unwrap();

        let top_roic = db
            .screen_universe(&UniverseFilter {
                sort_by: UniverseSortBy::Roic5yAvg,
                sort_asc: false,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(top_roic.len(), 10);
        // Since SQL stores DECIMAL(38,10), all 150 rows tie in SQL at 25.1234567890;
        // exact 28-digit ROIC sorting must select TIE149 down to TIE140!
        assert_eq!(top_roic[0].ticker, "TIE149");
        assert_eq!(top_roic[9].ticker, "TIE140");

        // Seed legacy bogus '2026-11-23' rows for a former-pool ticker OLDPOOL (never re-fetched)
        // plus TIE000, whose market_time_ms is actually Nov 20 ET (1795208402000).
        db.conn
            .execute_batch(
                "INSERT INTO prices_daily VALUES
                    ('OLDPOOL', '2026-11-23', 42.0, NULL, 'USD', 'NYSE', 1795208402000, 1795413110691, 'https://p'),
                    ('TIE000', '2026-11-23', 100.0, NULL, 'USD', 'NMS', 1795208402000, 1795413110691, 'https://p');",
            )
            .unwrap();
        drop(db);

        // Re-opening LakeDb reconciles ALL legacy prices_daily trade dates across the whole table.
        let mut db = LakeDb::open(&paths).unwrap();
        let remaining_bogus: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM prices_daily WHERE trade_date = '2026-11-23'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining_bogus, 0);
        let oldpool_fixed: String = db
            .conn
            .query_row(
                "SELECT trade_date FROM prices_daily WHERE symbol = 'OLDPOOL'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(oldpool_fixed, "2026-11-20");

        // Monotonic timestamp guard: an older quote must NOT overwrite newer prices_daily or universe_snapshot.
        db.upsert_prices_and_update_snapshot(
            &[LakePriceDailyRow {
                symbol: "TIE000".into(),
                trade_date: "2026-11-19".into(),
                price: Decimal::from(11),
                previous_close: None,
                currency: "USD".into(),
                exchange: "NMS".into(),
                market_time_ms: Some(1_795_122_000_000), // older than 1_795_208_402_000
                fetched_at_ms: 1_795_413_110_692,
                source_url: "https://stale.test".into(),
            }],
            &[QuoteSnapshotUpdate {
                cik: "0005000000".into(),
                expected_ticker: "TIE000".into(),
                expected_raw_sha256: Some("deadbeef".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: None,
                quote_market_time_ms: Some(1_795_122_000_000),
                quote_fetched_at_ms: 1_795_413_110_692,
                latest_price: Some(Decimal::from(11)),
                strike_price: Some(Decimal::from(150)),
                distance_to_strike_pct: Some(Decimal::from(-92)),
                strict_fcf_yield_pct: Some(Decimal::from(10)),
            }],
            &[],
        )
        .unwrap();
        let snap_tie0 = db.query_peers("TIE000", 1).unwrap().target;
        assert_eq!(snap_tie0.latest_price, Some(Decimal::from(100)));

        // Also test same trade_date stale overwrite rejection.
        db.upsert_prices_and_update_snapshot(
            &[LakePriceDailyRow {
                symbol: "OLDPOOL".into(),
                trade_date: "2026-11-20".into(),
                price: Decimal::from(5),
                previous_close: None,
                currency: "USD".into(),
                exchange: "NYSE".into(),
                market_time_ms: Some(1_795_200_000_000),
                fetched_at_ms: 1_795_413_000_000,
                source_url: "https://stale.test".into(),
            }],
            &[],
            &[],
        )
        .unwrap();
        let oldpool_price: Decimal = db
            .conn
            .query_row(
                "SELECT price FROM prices_daily WHERE symbol = 'OLDPOOL' AND trade_date = '2026-11-20'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(oldpool_price, Decimal::from(42));

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn replace_sec_preserves_previously_parsed_facts_on_replacement_parse_failure() {
        let paths = temp_paths("preserve-failed-issuer");
        let mut db = LakeDb::open(&paths).unwrap();

        let companies_v1 = vec![
            LakeCompanyRow {
                cik: "0000111111".into(),
                ticker: "KEEP".into(),
                name: "Keep Corp".into(),
                exchange: "NYSE".into(),
                sic: Some(3571),
                sic_description: Some("Tech".into()),
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000222222".into(),
                ticker: "UPDT".into(),
                name: "Update Corp".into(),
                exchange: "Nasdaq".into(),
                sic: Some(7372),
                sic_description: Some("Software".into()),
                is_financial: false,
                is_active: true,
            },
        ];
        let keep_annual = LakeAnnualRow {
            cik: "0000111111".into(),
            ticker: "KEEP".into(),
            fiscal_year: 2025,
            label: "FY2025".into(),
            period_end: "2025-12-31".into(),
            filed: Some("2026-02-20".into()),
            revenue: Decimal::from(1_000_000_000i64),
            gross_profit: Decimal::from(400_000_000i64),
            gross_margin_pct: Some(Decimal::from(40)),
            ebit: Decimal::from(200_000_000i64),
            nopat: Some(Decimal::from(150_000_000i64)),
            invested_capital: Some(Decimal::from(600_000_000i64)),
            roic_pct: Some(Decimal::from(25)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(24)),
            ocf: Decimal::from(220_000_000i64),
            capex: Decimal::from(40_000_000i64),
            fcf: Decimal::from(180_000_000i64),
            sbc: Some(Decimal::from(10_000_000i64)),
            strict_owner_earnings: Some(Decimal::from(170_000_000i64)),
            sbc_to_ocf_pct: Some(Decimal::new(455, 2)),
            buybacks: Some(Decimal::from(50_000_000i64)),
            cash_and_st: Some(Decimal::from(200_000_000i64)),
            total_debt: Some(Decimal::from(100_000_000i64)),
            net_debt: Some(Decimal::from(-100_000_000i64)),
            equity: Decimal::from(700_000_000i64),
            diluted_shares: Decimal::from(50_000_000i64),
            ending_shares: Decimal::from(50_000_000i64),
            raw_sha256: "keep_sha".into(),
            provenance_json: "[]".into(),
        };
        let mut keep_snap = make_universe_row(TestRowSpec {
            ticker: "KEEP",
            cik: "0000111111",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(40)),
            fcf_yield: Some(Decimal::from(4)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-2)),
            quality: Decimal::from(80),
        });
        keep_snap.raw_sha256 = Some("keep_sha".into());
        let updt_snap_v1 = make_universe_row(TestRowSpec {
            ticker: "UPDT",
            cik: "0000222222",
            sic: Some(7372),
            is_financial: false,
            roic: Some(Decimal::from(22)),
            gm: Some(Decimal::from(50)),
            fcf_yield: None,
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: None,
            quality: Decimal::from(75),
        });
        db.replace_sec_lake_data_with_watermarks(
            &companies_v1,
            &[keep_annual],
            &[],
            &[keep_snap, updt_snap_v1],
            &[],
        )
        .unwrap();

        let (parsed_ciks, classified_ciks) = db.load_parsed_and_classified_ciks().unwrap();
        assert!(parsed_ciks.contains("0000111111"));
        assert!(classified_ciks.contains("0000111111"));

        // Second sync: KEEP failed/skipped parsing (`years_of_data = 0, raw_sha256 = None`)
        // while gaining watchlist metadata and losing price-pool eligibility; UPDT succeeded with higher quality score.
        let mut keep_failed = make_universe_row(TestRowSpec {
            ticker: "KEEP",
            cik: "0000111111",
            sic: Some(3571),
            is_financial: false,
            roic: None,
            gm: None,
            fcf_yield: None,
            sbc_ocf: None,
            share_3y: None,
            dist: None,
            quality: Decimal::ZERO,
        });
        keep_failed.years_of_data = 0;
        keep_failed.raw_sha256 = None;
        keep_failed.in_price_pool = false;
        keep_failed.in_watchlist = true;
        keep_failed.bucket_opt = Some(Bucket::Core);
        let updt_snap_v2 = make_universe_row(TestRowSpec {
            ticker: "UPDT",
            cik: "0000222222",
            sic: Some(7372),
            is_financial: false,
            roic: Some(Decimal::from(28)),
            gm: Some(Decimal::from(55)),
            fcf_yield: None,
            sbc_ocf: Some(Decimal::from(4)),
            share_3y: Some(Decimal::ZERO),
            dist: None,
            quality: Decimal::from(95),
        });
        db.replace_sec_lake_data_with_watermarks(
            &companies_v1,
            &[],
            &[],
            &[keep_failed, updt_snap_v2],
            &[],
        )
        .unwrap();

        let keep_after = db.query_peers("KEEP", 1).unwrap().target;
        assert_eq!(keep_after.years_of_data, 6);
        assert_eq!(keep_after.raw_sha256.as_deref(), Some("keep_sha"));
        assert!(keep_after.in_watchlist);
        assert!(!keep_after.in_price_pool);
        assert!(keep_after.latest_price.is_none());
        assert!(keep_after.distance_to_strike_pct.is_none());
        assert!(keep_after.strict_fcf_yield_pct.is_none());
        assert_eq!(keep_after.bucket_opt, Some(Bucket::Core));
        assert_eq!(db.get_annual_rows("KEEP").unwrap().len(), 1);
        assert_eq!(
            db.query_peers("UPDT", 1).unwrap().target.quality_score,
            Decimal::from(95)
        );

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn parquet_generations_are_immutable_and_orphaned_staging_does_not_corrupt_views() {
        let paths = temp_paths("parquet-gen-immutable");
        let mut db = LakeDb::open(&paths).unwrap();
        let comp = vec![LakeCompanyRow {
            cik: "0000320193".into(),
            ticker: "AAPL".into(),
            name: "Apple Inc.".into(),
            exchange: "Nasdaq".into(),
            sic: Some(3571),
            sic_description: Some("Tech".into()),
            is_financial: false,
            is_active: true,
        }];
        let snap = vec![make_universe_row(TestRowSpec {
            ticker: "AAPL",
            cik: "0000320193",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(35)),
            gm: Some(Decimal::from(45)),
            fcf_yield: None,
            sbc_ocf: Some(Decimal::from(8)),
            share_3y: Some(Decimal::from(-3)),
            dist: None,
            quality: Decimal::from(92),
        })];
        db.replace_sec_lake_data_with_watermarks(&comp, &[], &[], &snap, &[])
            .unwrap();

        db.export_all_parquets(&paths).unwrap();
        let m1 = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        db.export_all_parquets(&paths).unwrap();
        let m2 = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        db.export_all_parquets(&paths).unwrap();
        let m3 = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        assert_ne!(m1.generation_dir, m2.generation_dir);
        assert_ne!(m2.generation_dir, m3.generation_dir);
        assert!(!paths.lake_parquet_dir().join(&m1.generation_dir).exists());
        assert!(paths.lake_parquet_dir().join(&m2.generation_dir).exists());
        assert!(paths.lake_parquet_dir().join(&m3.generation_dir).exists());

        // Plant orphaned staging & unreferenced gen dirs; active manifest & views stay intact.
        let crash_staging = paths.lake_parquet_dir().join(".staging-crash");
        fs::create_dir_all(&crash_staging).unwrap();
        fs::write(crash_staging.join("companies.parquet"), b"not parquet").unwrap();
        drop(db);

        let reopened = LakeDb::open(&paths).unwrap();
        let active = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        assert_eq!(active.generation_dir, m3.generation_dir);
        let cnt: i64 = reopened
            .conn
            .query_row("SELECT COUNT(*) FROM v_companies", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 1);

        drop(reopened);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn reconcile_prices_daily_resolves_chained_colliding_trade_dates() {
        let paths = temp_paths("reconcile-collisions");
        let db = LakeDb::open(&paths).unwrap();
        db.conn
            .execute_batch(
                "INSERT INTO prices_daily VALUES
                    ('AAPL', '2026-11-23', 200.0, NULL, 'USD', 'NMS', 1795208402000, 100, 'https://p1'),
                    ('AAPL', '2026-11-20', 190.0, NULL, 'USD', 'NMS', 1795122002000, 90, 'https://p2');",
            )
            .unwrap();
        drop(db);

        let reopened = LakeDb::open(&paths).unwrap();
        let count_23: i64 = reopened
            .conn
            .query_row(
                "SELECT COUNT(*) FROM prices_daily WHERE symbol = 'AAPL' AND trade_date = '2026-11-23'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count_23, 0);
        let (p20, mkt20): (Decimal, u64) = reopened
            .conn
            .query_row(
                "SELECT price, market_time_ms FROM prices_daily WHERE symbol = 'AAPL' AND trade_date = '2026-11-20'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let (p19, mkt19): (Decimal, u64) = reopened
            .conn
            .query_row(
                "SELECT price, market_time_ms FROM prices_daily WHERE symbol = 'AAPL' AND trade_date = '2026-11-19'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(p20, Decimal::from(200));
        assert_eq!(mkt20, 1_795_208_402_000);
        assert_eq!(p19, Decimal::from(190));
        assert_eq!(mkt19, 1_795_122_002_000);

        drop(reopened);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn quote_commit_enforces_ticker_pool_basis_and_monotonic_watermark() {
        let paths = temp_paths("quote-invariants");
        let mut db = LakeDb::open(&paths).unwrap();
        let comp = vec![LakeCompanyRow {
            cik: "0000001001".into(),
            ticker: "AAA".into(),
            name: "AAA Corp".into(),
            exchange: "NYSE".into(),
            sic: Some(3571),
            sic_description: None,
            is_financial: false,
            is_active: true,
        }];
        let ann = vec![LakeAnnualRow {
            cik: "0000001001".into(),
            ticker: "AAA".into(),
            fiscal_year: 2025,
            label: "FY2025".into(),
            period_end: "2025-12-31".into(),
            filed: Some("2026-02-15".into()),
            revenue: Decimal::from(1_000_000),
            gross_profit: Decimal::from(500_000),
            gross_margin_pct: Some(Decimal::from(50)),
            ebit: Decimal::from(200_000),
            nopat: Some(Decimal::from(150_000)),
            invested_capital: Some(Decimal::from(500_000)),
            roic_pct: Some(Decimal::from(30)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(28)),
            ocf: Decimal::from(220_000),
            capex: Decimal::from(20_000),
            fcf: Decimal::from(200_000),
            sbc: Some(Decimal::from(10_000)),
            strict_owner_earnings: Some(Decimal::from(190_000)),
            sbc_to_ocf_pct: Some(Decimal::new(455, 2)),
            buybacks: Some(Decimal::from(50_000)),
            cash_and_st: Some(Decimal::from(100_000)),
            total_debt: Some(Decimal::ZERO),
            net_debt: Some(Decimal::from(-100_000)),
            equity: Decimal::from(600_000),
            diluted_shares: Decimal::from(100_000),
            ending_shares: Decimal::from(100_000),
            raw_sha256: "sha1".into(),
            provenance_json: "[]".into(),
        }];
        let mut snap = make_universe_row(TestRowSpec {
            ticker: "AAA",
            cik: "0000001001",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(30)),
            gm: Some(Decimal::from(50)),
            fcf_yield: Some(Decimal::from(4)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-5)),
            quality: Decimal::from(85),
        });
        snap.raw_sha256 = Some("sha1".into());
        db.replace_sec_lake_data_with_watermarks(&comp, &ann, &[], &[snap], &[])
            .unwrap();

        let bad_updates = vec![
            QuoteSnapshotUpdate {
                cik: "0000001001".into(),
                expected_ticker: "BBB".into(),
                expected_raw_sha256: Some("sha1".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: Some("2025-12-31".into()),
                quote_market_time_ms: Some(1_795_208_402_000),
                quote_fetched_at_ms: 2_000,
                latest_price: Some(Decimal::from(111)),
                strike_price: Some(Decimal::from(110)),
                distance_to_strike_pct: Some(Decimal::from(1)),
                strict_fcf_yield_pct: Some(Decimal::from(3)),
            },
            QuoteSnapshotUpdate {
                cik: "0000001001".into(),
                expected_ticker: "AAA".into(),
                expected_raw_sha256: Some("sha1".into()),
                expected_latest_filing_end: Some("2024-12-31".into()),
                expected_basis_period_end: Some("2025-12-31".into()),
                quote_market_time_ms: Some(1_795_208_402_000),
                quote_fetched_at_ms: 2_000,
                latest_price: Some(Decimal::from(112)),
                strike_price: Some(Decimal::from(110)),
                distance_to_strike_pct: Some(Decimal::from(2)),
                strict_fcf_yield_pct: Some(Decimal::from(3)),
            },
            QuoteSnapshotUpdate {
                cik: "0000001001".into(),
                expected_ticker: "AAA".into(),
                expected_raw_sha256: Some("sha1".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: Some("2026-09-30".into()),
                quote_market_time_ms: Some(1_795_208_402_000),
                quote_fetched_at_ms: 2_000,
                latest_price: Some(Decimal::from(113)),
                strike_price: Some(Decimal::from(110)),
                distance_to_strike_pct: Some(Decimal::from(3)),
                strict_fcf_yield_pct: Some(Decimal::from(3)),
            },
        ];
        db.upsert_prices_and_update_snapshot(&[], &bad_updates, &[])
            .unwrap();
        assert_eq!(
            db.query_peers("AAA", 1).unwrap().target.latest_price,
            Some(Decimal::from(100))
        );

        let good_wm = SyncWatermark {
            source_key: "yahoo_prices".into(),
            cursor: Some("2026-11-20".into()),
            etag: None,
            last_modified: None,
            synced_at_ms: 2_000,
        };
        let older_wm = SyncWatermark {
            source_key: "yahoo_prices".into(),
            cursor: Some("2026-11-19".into()),
            etag: None,
            last_modified: None,
            synced_at_ms: 9_999,
        };
        db.upsert_prices_and_update_snapshot(
            &[LakePriceDailyRow {
                symbol: "AAA".into(),
                trade_date: "2026-11-20".into(),
                price: Decimal::from(120),
                previous_close: None,
                currency: "USD".into(),
                exchange: "NYSE".into(),
                market_time_ms: Some(1_795_208_402_000),
                fetched_at_ms: 2_000,
                source_url: "https://p".into(),
            }],
            &[QuoteSnapshotUpdate {
                cik: "0000001001".into(),
                expected_ticker: "AAA".into(),
                expected_raw_sha256: Some("sha1".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: Some("2025-12-31".into()),
                quote_market_time_ms: Some(1_795_208_402_000),
                quote_fetched_at_ms: 2_000,
                latest_price: Some(Decimal::from(120)),
                strike_price: Some(Decimal::from(110)),
                distance_to_strike_pct: Some(Decimal::from(9)),
                strict_fcf_yield_pct: Some(Decimal::from(3)),
            }],
            &[good_wm],
        )
        .unwrap();
        assert_eq!(
            db.query_peers("AAA", 1).unwrap().target.latest_price,
            Some(Decimal::from(120))
        );

        // Regressing watermark cursor must be ignored even if synced_at_ms is higher.
        db.upsert_prices_and_update_snapshot(&[], &[], &[older_wm])
            .unwrap();
        assert_eq!(
            db.read_watermark("yahoo_prices")
                .unwrap()
                .unwrap()
                .cursor
                .as_deref(),
            Some("2026-11-20")
        );

        // Dropping from price pool prevents quote commit from writing latest_price back.
        db.sync_price_pool_flags(&HashSet::new()).unwrap();
        db.upsert_prices_and_update_snapshot(
            &[],
            &[QuoteSnapshotUpdate {
                cik: "0000001001".into(),
                expected_ticker: "AAA".into(),
                expected_raw_sha256: Some("sha1".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: Some("2025-12-31".into()),
                quote_market_time_ms: Some(1_795_208_402_000),
                quote_fetched_at_ms: 2_500,
                latest_price: Some(Decimal::from(130)),
                strike_price: Some(Decimal::from(110)),
                distance_to_strike_pct: Some(Decimal::from(18)),
                strict_fcf_yield_pct: Some(Decimal::from(3)),
            }],
            &[],
        )
        .unwrap();
        assert!(
            db.query_peers("AAA", 1)
                .unwrap()
                .target
                .latest_price
                .is_none()
        );

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn screen_universe_handles_rounded_zero_oe_and_stale_quote_trade_date() {
        let paths = temp_paths("oe-boundary-stale-date");
        let mut db = LakeDb::open(&paths).unwrap();
        let comp = vec![
            LakeCompanyRow {
                cik: "0000000001".into(),
                ticker: "TINYPOS".into(),
                name: "Tiny Pos".into(),
                exchange: "NYSE".into(),
                sic: Some(3571),
                sic_description: None,
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000000002".into(),
                ticker: "TINYNEG".into(),
                name: "Tiny Neg".into(),
                exchange: "NYSE".into(),
                sic: Some(3571),
                sic_description: None,
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000000003".into(),
                ticker: "STALEQ".into(),
                name: "Stale Quote".into(),
                exchange: "NYSE".into(),
                sic: Some(3571),
                sic_description: None,
                is_financial: false,
                is_active: true,
            },
        ];
        let mut tiny_pos = make_universe_row(TestRowSpec {
            ticker: "TINYPOS",
            cik: "0000000001",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(40)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-10)),
            quality: Decimal::from(90),
        });
        tiny_pos.strict_owner_earnings = Some(Decimal::from_str("0.0000004").unwrap());

        let mut tiny_neg = make_universe_row(TestRowSpec {
            ticker: "TINYNEG",
            cik: "0000000002",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(40)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-10)),
            quality: Decimal::from(89),
        });
        tiny_neg.strict_owner_earnings = Some(Decimal::from_str("-0.0000004").unwrap());

        let stale_q = make_universe_row(TestRowSpec {
            ticker: "STALEQ",
            cik: "0000000003",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(40)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-15)),
            quality: Decimal::from(95),
        });
        let now_ms = store::now_ms();
        let fresh_mkt_ms = now_ms.saturating_sub(86_400_000);
        let fresh_date = crate::market::civil_iso_from_unix_secs((fresh_mkt_ms / 1000) as i64);
        let stale_mkt_ms = now_ms.saturating_sub(45 * 86_400_000);
        let stale_date = crate::market::civil_iso_from_unix_secs((stale_mkt_ms / 1000) as i64);
        let quote_times = HashMap::from([
            ("0000000001".to_owned(), fresh_mkt_ms),
            ("0000000002".to_owned(), fresh_mkt_ms),
            ("0000000003".to_owned(), stale_mkt_ms),
        ]);
        db.replace_sec_lake_data_with_quotes_and_watermarks(
            &comp,
            &[],
            &[],
            &[tiny_pos, tiny_neg, stale_q],
            &quote_times,
            &[],
        )
        .unwrap();
        db.upsert_prices_and_update_snapshot(
            &[
                LakePriceDailyRow {
                    symbol: "TINYPOS".into(),
                    trade_date: fresh_date,
                    price: Decimal::from(100),
                    previous_close: None,
                    currency: "USD".into(),
                    exchange: "NYSE".into(),
                    market_time_ms: Some(fresh_mkt_ms),
                    fetched_at_ms: now_ms,
                    source_url: "https://p".into(),
                },
                LakePriceDailyRow {
                    symbol: "STALEQ".into(),
                    trade_date: stale_date,
                    price: Decimal::from(95),
                    previous_close: None,
                    currency: "USD".into(),
                    exchange: "NYSE".into(),
                    market_time_ms: Some(stale_mkt_ms),
                    fetched_at_ms: now_ms, // refreshed today, but quote is 45 days old
                    source_url: "https://p".into(),
                },
            ],
            &[],
            &[],
        )
        .unwrap();

        let pos_oe_hits = db
            .screen_universe(&UniverseFilter {
                positive_strict_oe_only: true,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(pos_oe_hits.iter().any(|r| r.ticker == "TINYPOS"));
        assert!(!pos_oe_hits.iter().any(|r| r.ticker == "TINYNEG"));

        let strike_hits = db
            .screen_universe(&UniverseFilter {
                strike_zone_only: true,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(strike_hits.iter().any(|r| r.ticker == "TINYPOS"));
        assert!(!strike_hits.iter().any(|r| r.ticker == "STALEQ"));
        assert_eq!(db.universe_summary().unwrap().strike_zone_count, 1);

        // Removing the only fresh row leaves DB MAX(trade_date) at 45d ago; BOTH summary and screen must yield 0.
        db.conn
            .execute("DELETE FROM prices_daily WHERE symbol = 'TINYPOS'", [])
            .unwrap();
        assert_eq!(db.universe_summary().unwrap().strike_zone_count, 0);
        assert!(
            db.screen_universe(&UniverseFilter {
                strike_zone_only: true,
                limit: 10,
                ..Default::default()
            })
            .unwrap()
            .is_empty()
        );

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn drawer_and_universe_snapshot_read_consistent_duckdb_state_even_before_parquet_refresh() {
        let paths = temp_paths("drawer-snapshot-consistency");
        let mut db = LakeDb::open(&paths).unwrap();
        let comp = vec![LakeCompanyRow {
            cik: "0000320193".into(),
            ticker: "AAPL".into(),
            name: "Apple Inc.".into(),
            exchange: "Nasdaq".into(),
            sic: Some(3571),
            sic_description: Some("Computers".into()),
            is_financial: false,
            is_active: true,
        }];
        let ann_v1 = LakeAnnualRow {
            cik: "0000320193".into(),
            ticker: "AAPL".into(),
            fiscal_year: 2025,
            label: "FY2025".into(),
            period_end: "2025-09-27".into(),
            filed: Some("2025-11-01".into()),
            revenue: Decimal::from(100),
            gross_profit: Decimal::from(50),
            gross_margin_pct: Some(Decimal::from(50)),
            ebit: Decimal::from(30),
            nopat: Some(Decimal::from(24)),
            invested_capital: Some(Decimal::from(80)),
            roic_pct: Some(Decimal::from(30)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(25)),
            ocf: Decimal::from(35),
            capex: Decimal::from(5),
            fcf: Decimal::from(30),
            sbc: Some(Decimal::from(3)),
            strict_owner_earnings: Some(Decimal::from(27)),
            sbc_to_ocf_pct: Some(Decimal::from(8)),
            buybacks: Some(Decimal::from(20)),
            cash_and_st: Some(Decimal::from(20)),
            total_debt: Some(Decimal::from(40)),
            net_debt: Some(Decimal::from(20)),
            equity: Decimal::from(60),
            diluted_shares: Decimal::from(10),
            ending_shares: Decimal::from(10),
            raw_sha256: "sha_v1".into(),
            provenance_json: "[]".into(),
        };
        let snap_v1 = make_universe_row(TestRowSpec {
            ticker: "AAPL",
            cik: "0000320193",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(30)),
            gm: Some(Decimal::from(50)),
            fcf_yield: Some(Decimal::from(4)),
            sbc_ocf: Some(Decimal::from(8)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-5)),
            quality: Decimal::from(90),
        });
        db.replace_sec_lake_data_with_watermarks(
            &comp,
            std::slice::from_ref(&ann_v1),
            &[],
            &[snap_v1],
            &[],
        )
        .unwrap();
        db.export_all_parquets(&paths).unwrap();

        // Update DuckDB tables to FY2025 + FY2026 WITHOUT regenerating Parquet views yet (simulating mid-sync window).
        let mut ann_v2 = ann_v1.clone();
        ann_v2.fiscal_year = 2026;
        ann_v2.label = "FY2026".into();
        ann_v2.period_end = "2026-09-26".into();
        let snap_v2 = make_universe_row(TestRowSpec {
            ticker: "AAPL",
            cik: "0000320193",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(32)),
            gm: Some(Decimal::from(52)),
            fcf_yield: Some(Decimal::from(4)),
            sbc_ocf: Some(Decimal::from(8)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-5)),
            quality: Decimal::from(92),
        });
        db.replace_sec_lake_data_with_watermarks(&comp, &[ann_v1, ann_v2], &[], &[snap_v2], &[])
            .unwrap();
        drop(db);

        let ro_db = LakeDb::open_read_only_if_exists(&paths).unwrap().unwrap();
        let view_snap = ro_db
            .load_universe_snapshot(&UniverseFilter::default(), Some("0000320193"), 5)
            .unwrap();
        assert_eq!(view_snap.rows[0].quality_score, Decimal::from(92));
        let details = view_snap.selected_details.expect("drawer details");
        assert_eq!(details.row.quality_score, Decimal::from(92));
        assert_eq!(details.annuals.len(), 2);
        assert_eq!(details.annuals[1].label, "FY2026");

        let direct_details = ro_db
            .load_drawer_details_by_cik("0000320193", 5)
            .unwrap()
            .unwrap();
        assert_eq!(direct_details.annuals.len(), 2);
        assert_eq!(ro_db.get_annual_rows("AAPL").unwrap().len(), 2);

        drop(ro_db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn quote_ordering_strike_freshness_parity_and_share_class_invalidation_work() {
        let paths = temp_paths("quote-strike-sync");
        let mut db = LakeDb::open(&paths).unwrap();
        let now_ms = store::now_ms();
        let day_ms = 86_400_000u64;
        let stale_20d = now_ms.saturating_sub(20 * day_ms);
        let fresh_2d = now_ms.saturating_sub(2 * day_ms);
        let fresh_1d = now_ms.saturating_sub(day_ms);

        let comp = vec![
            LakeCompanyRow {
                cik: "0000001111".into(),
                ticker: "FOXA".into(),
                name: "Fox Class A".into(),
                exchange: "Nasdaq".into(),
                sic: Some(4833),
                sic_description: Some("TV".into()),
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000002222".into(),
                ticker: "TINYPLUS".into(),
                name: "Tiny Boundary".into(),
                exchange: "NYSE".into(),
                sic: Some(3571),
                sic_description: Some("Tech".into()),
                is_financial: false,
                is_active: true,
            },
        ];
        let ann = vec![LakeAnnualRow {
            cik: "0000001111".into(),
            ticker: "FOXA".into(),
            fiscal_year: 2025,
            label: "FY2025".into(),
            period_end: "2025-06-30".into(),
            filed: Some("2025-08-15".into()),
            revenue: Decimal::from(1_000),
            gross_profit: Decimal::from(500),
            gross_margin_pct: Some(Decimal::from(50)),
            ebit: Decimal::from(200),
            nopat: Some(Decimal::from(150)),
            invested_capital: Some(Decimal::from(500)),
            roic_pct: Some(Decimal::from(30)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(30)),
            ocf: Decimal::from(200),
            capex: Decimal::from(20),
            fcf: Decimal::from(180),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(170)),
            sbc_to_ocf_pct: Some(Decimal::from(5)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::ZERO),
            net_debt: Some(Decimal::from(-100)),
            equity: Decimal::from(500),
            diluted_shares: Decimal::from(100),
            ending_shares: Decimal::from(100),
            raw_sha256: "sha_fox".into(),
            provenance_json: "[]".into(),
        }];
        let mut foxa = make_universe_row(TestRowSpec {
            ticker: "FOXA",
            cik: "0000001111",
            sic: Some(4833),
            is_financial: false,
            roic: Some(Decimal::from(30)),
            gm: Some(Decimal::from(50)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-10)),
            quality: Decimal::from(90),
        });
        foxa.raw_sha256 = Some("sha_fox".into());

        // TINYPLUS has exact distance +1e-18 (above zero; must NOT enter strike zone in either summary or screen).
        let mut tiny_plus = make_universe_row(TestRowSpec {
            ticker: "TINYPLUS",
            cik: "0000002222",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(50)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from_str("0.000000000000000001").unwrap()),
            quality: Decimal::from(88),
        });
        tiny_plus.raw_sha256 = Some("sha_tiny".into());

        // 1. FOXA starts with 20d-stale strike timestamp; TINYPLUS has 1d-fresh timestamp.
        let quote_times = HashMap::from([
            ("0000001111".to_owned(), stale_20d),
            ("0000002222".to_owned(), fresh_1d),
        ]);
        db.replace_sec_lake_data_with_quotes_and_watermarks(
            &comp,
            &ann,
            &[],
            &[foxa.clone(), tiny_plus],
            &quote_times,
            &[],
        )
        .unwrap();

        let date_20d = crate::market::civil_iso_from_unix_secs((stale_20d / 1000) as i64);
        let date_2d = crate::market::civil_iso_from_unix_secs((fresh_2d / 1000) as i64);
        let date_1d = crate::market::civil_iso_from_unix_secs((fresh_1d / 1000) as i64);

        // Seed mixed-column timestamps on FOXA in prices_daily (older market_time with higher fetched_at_ms=9999,
        // and fresh_2d with fetched_at_ms=2000), while FOXA snapshot stays at stale_20d (not updated due to wrong sha).
        db.upsert_prices_and_update_snapshot(
            &[
                LakePriceDailyRow {
                    symbol: "FOXA".into(),
                    trade_date: date_20d,
                    price: Decimal::from(90),
                    previous_close: None,
                    currency: "USD".into(),
                    exchange: "Nasdaq".into(),
                    market_time_ms: Some(stale_20d),
                    fetched_at_ms: 9_999,
                    source_url: "https://p".into(),
                },
                LakePriceDailyRow {
                    symbol: "FOXA".into(),
                    trade_date: date_2d.clone(),
                    price: Decimal::from(95),
                    previous_close: None,
                    currency: "USD".into(),
                    exchange: "Nasdaq".into(),
                    market_time_ms: Some(fresh_2d),
                    fetched_at_ms: 2_000,
                    source_url: "https://p".into(),
                },
                LakePriceDailyRow {
                    symbol: "TINYPLUS".into(),
                    trade_date: date_1d,
                    price: Decimal::from(100),
                    previous_close: None,
                    currency: "USD".into(),
                    exchange: "NYSE".into(),
                    market_time_ms: Some(fresh_1d),
                    fetched_at_ms: 2_000,
                    source_url: "https://p".into(),
                },
            ],
            &[QuoteSnapshotUpdate {
                cik: "0000001111".into(),
                expected_ticker: "FOXA".into(),
                expected_raw_sha256: Some("wrong_sha".into()),
                expected_latest_filing_end: None,
                expected_basis_period_end: None,
                quote_market_time_ms: Some(fresh_2d),
                quote_fetched_at_ms: 2_000,
                latest_price: Some(Decimal::from(95)),
                strike_price: Some(Decimal::from(100)),
                distance_to_strike_pct: Some(Decimal::from(-5)),
                strict_fcf_yield_pct: Some(Decimal::from(5)),
            }],
            &[],
        )
        .unwrap();

        // FOXA has a fresh prices_daily row, YET its strike itself was calculated from 20d-stale quote -> excluded!
        // TINYPLUS has +1e-18 distance -> excluded identically from BOTH summary and screen!
        assert_eq!(db.universe_summary().unwrap().strike_zone_count, 0);
        assert!(
            db.screen_universe(&UniverseFilter {
                strike_zone_only: true,
                limit: 20,
                ..Default::default()
            })
            .unwrap()
            .is_empty()
        );

        // 2. Column-mixing order check: update FOXA with (fresh_2d, fetched=3000); must NOT be blocked
        // by stale_20d's fetched=9999!
        db.upsert_prices_and_update_snapshot(
            &[LakePriceDailyRow {
                symbol: "FOXA".into(),
                trade_date: date_2d,
                price: Decimal::from(92),
                previous_close: None,
                currency: "USD".into(),
                exchange: "Nasdaq".into(),
                market_time_ms: Some(fresh_2d),
                fetched_at_ms: 3_000,
                source_url: "https://p".into(),
            }],
            &[QuoteSnapshotUpdate {
                cik: "0000001111".into(),
                expected_ticker: "FOXA".into(),
                expected_raw_sha256: Some("sha_fox".into()),
                expected_latest_filing_end: None,
                expected_basis_period_end: None,
                quote_market_time_ms: Some(fresh_2d),
                quote_fetched_at_ms: 3_000,
                latest_price: Some(Decimal::from(92)),
                strike_price: Some(Decimal::from(100)),
                distance_to_strike_pct: Some(Decimal::from(-8)),
                strict_fcf_yield_pct: Some(Decimal::from(5)),
            }],
            &[],
        )
        .unwrap();
        assert_eq!(db.universe_summary().unwrap().strike_zone_count, 1);
        assert_eq!(
            db.screen_universe(&UniverseFilter {
                strike_zone_only: true,
                limit: 20,
                ..Default::default()
            })
            .unwrap()
            .len(),
            1
        );

        // 3. Share-class / ticker switch on a preserved failed issuer invalidates stale quote & strike!
        let mut fox_comp = comp[0].clone();
        fox_comp.ticker = "FOX".into();
        let failed_fox = UniverseRow {
            ticker: "FOX".into(),
            cik: "0000001111".into(),
            name: "Fox Class B".into(),
            exchange: "Nasdaq".into(),
            sic: Some(4833),
            sic_description: Some("TV".into()),
            is_financial: false,
            latest_filing_end: None,
            years_of_data: 0,
            quality_score: Decimal::ZERO,
            in_price_pool: true,
            in_watchlist: false,
            bucket_opt: None,
            latest_price: None,
            strike_price: None,
            distance_to_strike_pct: None,
            strict_fcf_yield_pct: None,
            roic_5y_avg_pct: None,
            roic_5y_stddev_pct: None,
            gross_margin_5y_avg_pct: None,
            gross_margin_5y_stddev_pct: None,
            share_change_3y_pct: None,
            sbc_to_ocf_pct: None,
            net_debt_to_oe: None,
            strict_owner_earnings: None,
            warnings_count: 0,
            warnings_json: "[]".into(),
            raw_sha256: None,
        };
        db.replace_sec_lake_data_with_watermarks(&[fox_comp], &[], &[], &[failed_fox], &[])
            .unwrap();
        let fox_details = db
            .load_drawer_details_by_cik("0000001111", 5)
            .unwrap()
            .unwrap();
        assert_eq!(fox_details.row.ticker, "FOX");
        assert!(fox_details.row.latest_price.is_none());
        assert!(fox_details.row.strike_price.is_none());
        assert!(fox_details.row.distance_to_strike_pct.is_none());
        assert_eq!(fox_details.annuals[0].ticker, "FOX");

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn sec_generation_guard_coverage_regression_share_switch_and_assumptions_guard_work() {
        let paths = temp_paths("sec-cov-share-assump");
        let mut db = LakeDb::open(&paths).unwrap();
        let now_ms = store::now_ms();
        let day_ms = 86_400_000u64;
        let fresh_ts = now_ms.saturating_sub(day_ms);

        let comp = vec![LakeCompanyRow {
            cik: "0000005555".into(),
            ticker: "FOXA".into(),
            name: "Fox Corp".into(),
            exchange: "Nasdaq".into(),
            sic: Some(4833),
            sic_description: Some("TV".into()),
            is_financial: false,
            is_active: true,
        }];
        let make_ann = |fy: i32, end: &str| LakeAnnualRow {
            cik: "0000005555".into(),
            ticker: "FOXA".into(),
            fiscal_year: fy,
            label: format!("FY{fy}"),
            period_end: end.into(),
            filed: Some("2026-08-01".into()),
            revenue: Decimal::from(1000),
            gross_profit: Decimal::from(500),
            gross_margin_pct: Some(Decimal::from(50)),
            ebit: Decimal::from(200),
            nopat: Some(Decimal::from(150)),
            invested_capital: Some(Decimal::from(600)),
            roic_pct: Some(Decimal::from(25)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(25)),
            ocf: Decimal::from(220),
            capex: Decimal::from(20),
            fcf: Decimal::from(200),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(190)),
            sbc_to_ocf_pct: Some(Decimal::new(45, 1)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(600),
            diluted_shares: Decimal::from(10),
            ending_shares: Decimal::from(10),
            raw_sha256: "sha_5555".into(),
            provenance_json: "[]".into(),
        };
        let anns_5y = vec![
            make_ann(2022, "2022-06-30"),
            make_ann(2023, "2023-06-30"),
            make_ann(2024, "2024-06-30"),
            make_ann(2025, "2025-06-30"),
            make_ann(2026, "2026-06-30"),
        ];
        let ttm = vec![LakeTtmRow {
            cik: "0000005555".into(),
            ticker: "FOXA".into(),
            label: "TTM Q1".into(),
            period_end: "2026-09-30".into(),
            revenue: Decimal::from(1100),
            gross_profit: Decimal::from(550),
            gross_margin_pct: Some(Decimal::from(50)),
            ebit: Decimal::from(210),
            nopat: Some(Decimal::from(160)),
            invested_capital: Some(Decimal::from(600)),
            roic_pct: Some(Decimal::from(26)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(26)),
            ocf: Decimal::from(230),
            capex: Decimal::from(20),
            fcf: Decimal::from(210),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(200)),
            sbc_to_ocf_pct: Some(Decimal::new(43, 1)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(110)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-60)),
            equity: Decimal::from(620),
            diluted_shares: Decimal::from(10),
            ytd_period_label: "Q1".into(),
            ytd_start: "2026-07-01".into(),
            ytd_end: "2026-09-30".into(),
            ytd_revenue: Some(Decimal::from(260)),
            ytd_ocf: Some(Decimal::from(55)),
            ytd_capex: Some(Decimal::from(5)),
            ytd_sbc: Some(Decimal::from(2)),
            ytd_buybacks: Some(Decimal::ZERO),
            latest_quarter_label: None,
            latest_quarter_ocf: None,
            latest_quarter_capex: None,
            raw_sha256: "sha_5555".into(),
            provenance_json: "[]".into(),
        }];
        let mut foxa = make_universe_row(TestRowSpec {
            ticker: "FOXA",
            cik: "0000005555",
            sic: Some(4833),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(50)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(4)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-5)),
            quality: Decimal::from(88),
        });
        foxa.years_of_data = 5;
        foxa.latest_filing_end = Some("2026-09-30".into());
        foxa.raw_sha256 = Some("sha_5555".into());

        let old_assump_dig = valuation_assumptions_digest(&ValuationAssumptions {
            multiple_low: Decimal::from(18),
            multiple_high: Decimal::from(22),
            target_return_pct: Decimal::from(10),
            expected_growth_pct: Decimal::from(8),
            strike_price_override: None,
        });
        let meta_init = HashMap::from([(
            "0000005555".to_owned(),
            SecSnapshotMeta {
                quote_market_time_ms: Some(fresh_ts),
                quote_fetched_at_ms: Some(now_ms),
                assumptions_digest: Some(old_assump_dig.clone()),
                assumptions: None,
                extractor_version: 2,
                quality_score_override: None,
            },
        )]);
        let wm_v3 = vec![SyncWatermark {
            source_key: WM_SEC_EXTRACTOR_VERSION.into(),
            cursor: Some(SEC_LAKE_EXTRACTOR_VERSION.into()),
            etag: None,
            last_modified: None,
            synced_at_ms: now_ms,
        }];
        db.replace_sec_lake_data_with_meta_and_commit_hook(
            &comp,
            &anns_5y,
            &ttm,
            &[foxa],
            &meta_init,
            &wm_v3,
            || Ok(()),
        )
        .unwrap();
        // Per-issuer extractor_version=2 (<3) must STILL trigger backfill despite global watermark=3!
        assert!(db.has_missing_exact_decimals().unwrap());
        assert_eq!(
            db.load_previous_sec_snapshot()
                .unwrap()
                .extractor_versions
                .get("0000005555")
                .copied(),
            Some(2)
        );

        // 1. Pre-commit hook failure rolls back transaction atomically leaving 5y+TTM intact.
        let mut partial_2y_row = make_universe_row(TestRowSpec {
            ticker: "BAD",
            cik: "0000005555",
            sic: Some(4833),
            is_financial: false,
            roic: Some(Decimal::from(10)),
            gm: Some(Decimal::from(40)),
            fcf_yield: None,
            sbc_ocf: None,
            share_3y: None,
            dist: None,
            quality: Decimal::from(30),
        });
        partial_2y_row.years_of_data = 2;
        assert!(
            db.replace_sec_lake_data_with_meta_and_commit_hook(
                &comp,
                &anns_5y[3..],
                &[],
                &[partial_2y_row.clone()],
                &HashMap::new(),
                &[],
                || Err(anyhow::anyhow!("simulated zip promotion failure")),
            )
            .is_err()
        );
        assert_eq!(db.get_annual_rows("FOXA").unwrap().len(), 5);

        // 2. Partial parse regression (5y+TTM -> 2y no TTM) + warrant ticker switch (FOXA -> FOXA-WT):
        // preserves 5y+TTM, keeps extractor_version=2, persists SHARE_BASIS_MISMATCH, suppresses strike!
        let mut warrant_comp = comp[0].clone();
        warrant_comp.ticker = "FOXA-WT".into();
        partial_2y_row.ticker = "FOXA-WT".into();
        db.replace_sec_lake_data_with_meta_and_commit_hook(
            &[warrant_comp],
            &anns_5y[3..],
            &[],
            &[partial_2y_row],
            &HashMap::new(),
            &[],
            || Ok(()),
        )
        .unwrap();
        let after_partial = db
            .load_drawer_details_by_cik("0000005555", 5)
            .unwrap()
            .unwrap();
        assert_eq!(after_partial.annuals.len(), 5);
        assert_eq!(after_partial.row.years_of_data, 5);
        assert_eq!(after_partial.row.ticker, "FOXA-WT");
        assert!(
            after_partial
                .row
                .warnings_json
                .contains("\"SHARE_BASIS_MISMATCH\"")
        );
        assert!(after_partial.row.warnings_count >= 1);
        assert!(after_partial.row.strike_price.is_none());
        assert!(after_partial.row.distance_to_strike_pct.is_none());
        assert!(after_partial.row.strict_fcf_yield_pct.is_none());
        assert_eq!(
            db.load_previous_sec_snapshot()
                .unwrap()
                .extractor_versions
                .get("0000005555")
                .copied(),
            Some(2)
        );
        assert!(
            db.load_previous_sec_snapshot()
                .unwrap()
                .ttm_ciks
                .contains("0000005555")
        );

        // 3. Stale expected_assumptions_digest & older fetch timestamp tie-break reject snapshot update.
        let mut comp2 = comp[0].clone();
        comp2.cik = "0000006666".into();
        comp2.ticker = "OKCO".into();
        let mut ok_row = make_universe_row(TestRowSpec {
            ticker: "OKCO",
            cik: "0000006666",
            sic: Some(4833),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(50)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(4)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-5)),
            quality: Decimal::from(88),
        });
        ok_row.latest_filing_end = Some("2026-09-30".into());
        ok_row.raw_sha256 = Some("sha_6666".into());
        let new_assump_dig = valuation_assumptions_digest(&ValuationAssumptions {
            multiple_low: Decimal::from(25),
            multiple_high: Decimal::from(28),
            target_return_pct: Decimal::from(10),
            expected_growth_pct: Decimal::from(8),
            strike_price_override: None,
        });
        let meta_ok = HashMap::from([(
            "0000006666".to_owned(),
            SecSnapshotMeta {
                quote_market_time_ms: Some(fresh_ts),
                quote_fetched_at_ms: Some(now_ms + 5_000),
                assumptions_digest: Some(new_assump_dig.clone()),
                assumptions: None,
                extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                quality_score_override: None,
            },
        )]);
        db.replace_sec_lake_data_with_meta_and_commit_hook(
            &[comp2],
            &[],
            &[],
            &[ok_row],
            &meta_ok,
            &[],
            || Ok(()),
        )
        .unwrap();

        let trade_date = crate::market::civil_iso_from_unix_secs((fresh_ts / 1000) as i64);
        let p_row = LakePriceDailyRow {
            symbol: "OKCO".into(),
            trade_date,
            price: Decimal::from(99),
            previous_close: None,
            currency: "USD".into(),
            exchange: "Nasdaq".into(),
            market_time_ms: Some(fresh_ts),
            fetched_at_ms: now_ms + 6_000,
            source_url: "https://p".into(),
        };
        let stale_assump_upd = QuoteSnapshotUpdate {
            cik: "0000006666".into(),
            expected_ticker: "OKCO".into(),
            expected_raw_sha256: Some("sha_6666".into()),
            expected_latest_filing_end: Some("2026-09-30".into()),
            expected_basis_period_end: None,
            quote_market_time_ms: Some(fresh_ts),
            quote_fetched_at_ms: now_ms + 6_000,
            latest_price: Some(Decimal::from(99)),
            strike_price: Some(Decimal::from(777)),
            distance_to_strike_pct: Some(Decimal::from(-10)),
            strict_fcf_yield_pct: Some(Decimal::from(6)),
        };
        db.upsert_prices_and_update_snapshot_with_assumptions(
            std::slice::from_ref(&p_row),
            &[stale_assump_upd],
            &HashMap::from([("0000006666".to_owned(), old_assump_dig)]),
            &[],
        )
        .unwrap();
        // Mismatched assumption revision must NOT overwrite strike=110 -> 777.
        assert_eq!(
            db.query_peers("OKCO", 1).unwrap().target.strike_price,
            Some(Decimal::from(110))
        );

        // Older fetched_at_ms on same market_time_ms must also be ignored even with matching digest.
        let older_fetch_upd = QuoteSnapshotUpdate {
            cik: "0000006666".into(),
            expected_ticker: "OKCO".into(),
            expected_raw_sha256: Some("sha_6666".into()),
            expected_latest_filing_end: Some("2026-09-30".into()),
            expected_basis_period_end: None,
            quote_market_time_ms: Some(fresh_ts),
            quote_fetched_at_ms: now_ms + 1_000,
            latest_price: Some(Decimal::from(88)),
            strike_price: Some(Decimal::from(888)),
            distance_to_strike_pct: Some(Decimal::from(-12)),
            strict_fcf_yield_pct: Some(Decimal::from(7)),
        };
        db.upsert_prices_and_update_snapshot_with_assumptions(
            &[p_row],
            &[older_fetch_upd],
            &HashMap::from([("0000006666".to_owned(), new_assump_dig)]),
            &[],
        )
        .unwrap();
        assert_eq!(
            db.query_peers("OKCO", 1).unwrap().target.strike_price,
            Some(Decimal::from(110))
        );

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn sec_generation_inactive_facts_retention_watchlist_recompute_and_raw_decimal_work() {
        // 1. Legacy bare high-precision JSON numeric tokens must preserve >f64 precision.
        let raw_exact_json = r#"{
            "quality_score": 88.123456789012345678901234,
            "latest_price": null,
            "roic_5y_avg_pct": 28.987654321098765432109876
        }"#;
        let decoded: UniverseExactDecimals = serde_json::from_str(raw_exact_json).unwrap();
        assert_eq!(
            decoded.quality_score,
            Decimal::from_str("88.123456789012345678901234").unwrap()
        );
        assert_eq!(
            decoded.roic_5y_avg_pct,
            Some(Decimal::from_str("28.987654321098765432109876").unwrap())
        );

        let paths = temp_paths("r7-invariants");
        let mut db = LakeDb::open(&paths).unwrap();
        let now_ms = store::now_ms();
        let make_ann = |cik: &str, ticker: &str, yr: i32| LakeAnnualRow {
            cik: cik.into(),
            ticker: ticker.into(),
            fiscal_year: yr,
            label: format!("FY{yr}"),
            period_end: format!("{yr}-12-31"),
            filed: Some(format!("{}-02-15", yr + 1)),
            revenue: Decimal::from(1_000_000_000i64),
            gross_profit: Decimal::from(600_000_000i64),
            gross_margin_pct: Some(Decimal::from(60)),
            ebit: Decimal::from(250_000_000i64),
            nopat: Some(Decimal::from(200_000_000i64)),
            invested_capital: Some(Decimal::from(800_000_000i64)),
            roic_pct: Some(Decimal::from(25)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(25)),
            ocf: Decimal::from(220_000_000i64),
            capex: Decimal::from(20_000_000i64),
            fcf: Decimal::from(200_000_000i64),
            sbc: Some(Decimal::from(10_000_000i64)),
            strict_owner_earnings: Some(Decimal::from(190_000_000i64)),
            sbc_to_ocf_pct: Some(Decimal::new(455, 2)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100_000_000i64)),
            total_debt: Some(Decimal::from(50_000_000i64)),
            net_debt: Some(Decimal::from(-50_000_000i64)),
            equity: Decimal::from(750_000_000i64),
            diluted_shares: Decimal::from(10_000_000i64),
            ending_shares: Decimal::from(10_000_000i64),
            raw_sha256: "sha_r7".into(),
            provenance_json: "[]".into(),
        };

        let comps_v1 = vec![
            LakeCompanyRow {
                cik: "0000007771".into(),
                ticker: "ACTV".into(),
                name: "Active Corp".into(),
                exchange: "Nasdaq".into(),
                sic: Some(7372),
                sic_description: None,
                is_financial: false,
                is_active: true,
            },
            LakeCompanyRow {
                cik: "0000007772".into(),
                ticker: "GONE".into(),
                name: "Delisted Corp".into(),
                exchange: "NYSE".into(),
                sic: Some(3571),
                sic_description: None,
                is_financial: false,
                is_active: true,
            },
        ];
        let anns_v1 = vec![
            make_ann("0000007771", "ACTV", 2021),
            make_ann("0000007771", "ACTV", 2022),
            make_ann("0000007771", "ACTV", 2023),
            make_ann("0000007771", "ACTV", 2024),
            make_ann("0000007772", "GONE", 2024),
        ];
        let mut actv = make_universe_row(TestRowSpec {
            ticker: "ACTV",
            cik: "0000007771",
            sic: Some(7372),
            is_financial: false,
            roic: Some(Decimal::from(25)),
            gm: Some(Decimal::from(60)),
            fcf_yield: Some(Decimal::from(5)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(-10)),
            quality: Decimal::from(90),
        });
        actv.latest_filing_end = Some("2024-12-31".into());
        actv.years_of_data = 4;
        actv.raw_sha256 = Some("sha_r7".into());
        let mut gone = make_universe_row(TestRowSpec {
            ticker: "GONE",
            cik: "0000007772",
            sic: Some(3571),
            is_financial: false,
            roic: Some(Decimal::from(20)),
            gm: Some(Decimal::from(55)),
            fcf_yield: Some(Decimal::from(4)),
            sbc_ocf: Some(Decimal::from(5)),
            share_3y: Some(Decimal::ZERO),
            dist: Some(Decimal::from(5)),
            quality: Decimal::from(80),
        });
        gone.latest_filing_end = Some("2024-12-31".into());
        gone.years_of_data = 1;
        gone.raw_sha256 = Some("sha_gone".into());

        let def_dig = valuation_assumptions_digest(&default_valuation_assumptions());
        let mut meta_v1 = HashMap::new();
        meta_v1.insert(
            "0000007771".to_owned(),
            SecSnapshotMeta {
                quote_market_time_ms: Some(now_ms - 60_000),
                quote_fetched_at_ms: Some(now_ms),
                assumptions_digest: Some(def_dig.clone()),
                assumptions: None,
                extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                quality_score_override: None,
            },
        );
        let empty_quotes = HashMap::new();
        let gen0_exp = SecGenerationExpectations {
            expected_prev_sec_gen: Some(None),
            expected_prev_assumptions_rev: None,
            batch_synced_at_ms: now_ms,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_v1,
            &anns_v1,
            &[],
            &[actv.clone(), gone],
            &meta_v1,
            &[],
            &gen0_exp,
            |_| Ok(()),
        )
        .unwrap();
        let gen1 = db.read_sec_generation().unwrap().unwrap();
        let (assump_rev1, _) = db.load_assumptions_state().unwrap();

        // Concurrent/stale generation mismatch must fail cleanly without touching DB.
        let stale_gen_exp = SecGenerationExpectations {
            expected_prev_sec_gen: Some(None), // stale caller thought DB was empty
            expected_prev_assumptions_rev: None,
            batch_synced_at_ms: now_ms + 1_000,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        assert!(
            db.replace_sec_lake_generation_with_meta_and_commit_hook(
                &comps_v1[..1],
                &anns_v1[..4],
                &[],
                std::slice::from_ref(&actv),
                &meta_v1,
                &[],
                &stale_gen_exp,
                |_| Ok(()),
            )
            .is_err()
        );
        // Older timestamp than gen1 must also fail.
        let older_ts_exp = SecGenerationExpectations {
            expected_prev_sec_gen: Some(Some(gen1)),
            expected_prev_assumptions_rev: None,
            batch_synced_at_ms: now_ms - 1,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        assert!(
            db.replace_sec_lake_generation_with_meta_and_commit_hook(
                &comps_v1[..1],
                &anns_v1[..4],
                &[],
                std::slice::from_ref(&actv),
                &meta_v1,
                &[],
                &older_ts_exp,
                |_| Ok(()),
            )
            .is_err()
        );

        // Replacement dropping intermediate year 2022 while adding 2025 (count=4 preserved!) AND
        // dropping GONE from active listing: ACTV must preserve 2021..2024; GONE must remain
        // inactive in `companies` with retained `facts_annual`, and BOTH surviving non-new-zip
        // issuers must appear in the raw preservation map.
        let mut actv_dropped_mid = actv.clone();
        actv_dropped_mid.latest_filing_end = Some("2025-12-31".into());
        let anns_dropped_mid = vec![
            make_ann("0000007771", "ACTV", 2021),
            make_ann("0000007771", "ACTV", 2023),
            make_ann("0000007771", "ACTV", 2024),
            make_ann("0000007771", "ACTV", 2025),
        ];
        let mut preserved_seen: Vec<String> = Vec::new();
        let gen1_exp = SecGenerationExpectations {
            expected_prev_sec_gen: Some(Some(gen1)),
            expected_prev_assumptions_rev: Some(assump_rev1),
            batch_synced_at_ms: now_ms + 2_000,
            post_parse_watchlist: None,
            recompute_prices_top: None,
            refreshed_disk_quotes: &empty_quotes,
        };
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_v1[..1],
            &anns_dropped_mid,
            &[],
            &[actv_dropped_mid],
            &meta_v1,
            &[],
            &gen1_exp,
            |pres| {
                let mut keys: Vec<String> = pres.keys().cloned().collect();
                keys.sort();
                preserved_seen.extend(keys);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            preserved_seen,
            vec!["0000007771".to_owned(), "0000007772".to_owned()]
        );
        assert_eq!(db.get_annual_rows("ACTV").unwrap().len(), 4);
        assert_eq!(db.get_annual_rows("GONE").unwrap().len(), 1);
        let gone_active: bool = db
            .conn
            .query_row(
                "SELECT is_active FROM companies WHERE cik = '0000007772'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!gone_active);

        // Watchlist assumption edit triggers in-tx strike recompute + revision bump.
        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![CompanyEntry {
                    ticker: "ACTV".into(),
                    name: "Active Corp".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("0000007771".into()),
                    currency: "USD".into(),
                    circle: "Software".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(18),
                        multiple_high: Decimal::from(22),
                        target_return_pct: Decimal::from(15),
                        expected_growth_pct: Decimal::from(10),
                        strike_price_override: Some(Decimal::from(333)),
                    },
                }],
            },
        )
        .unwrap();
        db.refresh_watchlist_flags(&paths).unwrap();
        let refreshed = db.query_peers("ACTV", 1).unwrap().target;
        assert_eq!(refreshed.strike_price, Some(Decimal::from(333)));
        let (global_rev, assump_state) = db.load_assumptions_state().unwrap();
        assert!(global_rev >= 2);
        let (actv_dig, actv_rev) = assump_state["0000007771"].clone();
        assert!(actv_rev >= 2);
        assert_ne!(actv_dig, def_dig);

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn relist_rebuild_stale_quote_assumptions_guard_and_unpadded_watchlist_cik_work() {
        let dir = std::env::temp_dir().join(format!("m3-lake-round8-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let mut db = LakeDb::open(&paths).unwrap();
        let now_ms = store::now_ms();
        let def_dig = valuation_assumptions_digest(&default_valuation_assumptions());
        let make_comp = |cik: &str, ticker: &str| LakeCompanyRow {
            cik: cik.into(),
            ticker: ticker.into(),
            name: format!("{ticker} Corp"),
            exchange: "NASDAQ".into(),
            sic: Some(7372),
            sic_description: Some("Software".into()),
            is_financial: false,
            is_active: true,
        };
        let make_ann_y = |cik: &str, ticker: &str, fy: i32, sha: &str| LakeAnnualRow {
            cik: cik.into(),
            ticker: ticker.into(),
            fiscal_year: fy,
            label: format!("FY{fy}"),
            period_end: format!("{fy}-12-31"),
            filed: Some(format!("{}-02-15", fy + 1)),
            revenue: Decimal::from(500),
            gross_profit: Decimal::from(350),
            gross_margin_pct: Some(Decimal::from(70)),
            ebit: Decimal::from(150),
            nopat: Some(Decimal::from(120)),
            invested_capital: Some(Decimal::from(400)),
            roic_pct: Some(Decimal::from(30)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(24)),
            ocf: Decimal::from(160),
            capex: Decimal::from(40),
            fcf: Decimal::from(120),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(110)),
            sbc_to_ocf_pct: Some(Decimal::new(625, 2)),
            buybacks: Some(Decimal::from(20)),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(450),
            diluted_shares: Decimal::from(10),
            ending_shares: Decimal::from(10),
            raw_sha256: sha.into(),
            provenance_json: "[]".into(),
        };
        let make_snap =
            |cik: &str, ticker: &str, yrs: usize, sha: Option<&str>, px: Option<i64>| UniverseRow {
                ticker: ticker.into(),
                cik: cik.into(),
                name: format!("{ticker} Corp"),
                exchange: "NASDAQ".into(),
                sic: Some(7372),
                sic_description: Some("Software".into()),
                is_financial: false,
                latest_filing_end: (yrs > 0).then(|| "2025-12-31".into()),
                years_of_data: yrs,
                quality_score: if yrs >= 5 {
                    Decimal::from(88)
                } else {
                    Decimal::ZERO
                },
                in_price_pool: true,
                in_watchlist: false,
                bucket_opt: None,
                latest_price: px.map(Decimal::from),
                strike_price: px.map(|_| Decimal::from(198)),
                distance_to_strike_pct: px.map(|v| Decimal::from(v - 198)),
                strict_fcf_yield_pct: px.map(|v| {
                    if v > 0 {
                        Decimal::from(11)
                    } else {
                        Decimal::ZERO
                    }
                }),
                roic_5y_avg_pct: (yrs >= 5).then(|| Decimal::from(30)),
                roic_5y_stddev_pct: (yrs >= 5).then_some(Decimal::ZERO),
                gross_margin_5y_avg_pct: (yrs >= 5).then(|| Decimal::from(70)),
                gross_margin_5y_stddev_pct: (yrs >= 5).then_some(Decimal::ZERO),
                share_change_3y_pct: (yrs >= 4).then_some(Decimal::ZERO),
                sbc_to_ocf_pct: (yrs > 0).then(|| Decimal::new(625, 2)),
                net_debt_to_oe: None,
                strict_owner_earnings: (yrs > 0).then(|| Decimal::from(110)),
                warnings_count: 0,
                warnings_json: "[]".into(),
                raw_sha256: sha.map(str::to_owned),
            };

        // Gen 1: ACTV (0000008881) & RLST (0000008882) both parsed with 5Y.
        let mut anns_g1 = Vec::new();
        for fy in 2021..=2025 {
            anns_g1.push(make_ann_y("0000008881", "ACTV", fy, "sha-actv-1"));
            anns_g1.push(make_ann_y("0000008882", "RLST", fy, "sha-rlst-1"));
        }
        let comps_g1 = vec![
            make_comp("0000008881", "ACTV"),
            make_comp("0000008882", "RLST"),
        ];
        let mut meta_g1 = HashMap::new();
        meta_g1.insert(
            "0000008881".into(),
            SecSnapshotMeta {
                quote_market_time_ms: Some(now_ms - 120_000),
                quote_fetched_at_ms: Some(now_ms - 100_000),
                assumptions_digest: Some(def_dig.clone()),
                assumptions: None,
                extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                quality_score_override: None,
            },
        );
        let empty_q = HashMap::new();
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_g1,
            &anns_g1,
            &[],
            &[
                make_snap("0000008881", "ACTV", 5, Some("sha-actv-1"), Some(150)),
                make_snap("0000008882", "RLST", 5, Some("sha-rlst-1"), Some(120)),
            ],
            &meta_g1,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(None),
                expected_prev_assumptions_rev: Some(0),
                batch_synced_at_ms: now_ms,
                post_parse_watchlist: None,
                recompute_prices_top: None,
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();

        // Concurrent quote write moves ACTV forward to $175 at newer market_time_ms, and unpadded CIK
        // "8881" in watchlist normalizes cleanly to "0000008881" and bumps assumptions_rev.
        store::save_watchlist(
            &paths,
            &WatchlistFile {
                schema_version: 1,
                benchmark_symbol: "^GSPC".into(),
                default_target_return_pct: Decimal::from(10),
                companies: vec![CompanyEntry {
                    ticker: "ACTV-NEW".into(), // mismatch on ticker -> must still match via normalized "8881"!
                    name: "Active Corp".into(),
                    bucket: Bucket::Core,
                    source: DataSource::SecEdgar,
                    cik: Some("8881".into()),
                    currency: "USD".into(),
                    circle: "Software".into(),
                    too_hard_reason: None,
                    assumptions: ValuationAssumptions {
                        multiple_low: Decimal::from(18),
                        multiple_high: Decimal::from(22),
                        target_return_pct: Decimal::from(10),
                        expected_growth_pct: Decimal::from(8),
                        strike_price_override: Some(Decimal::from(250)),
                    },
                }],
            },
        )
        .unwrap();
        let wl_live = store::load_watchlist(&paths).unwrap();
        let wl_dig = valuation_assumptions_digest(&wl_live.companies[0].assumptions);
        let (_, pre_revs) = db.load_assumptions_state().unwrap();
        let actv_rev_before = pre_revs["0000008881"].1;
        let commit = PriceSyncCommit {
            prices: &[],
            updated_rows: &[QuoteSnapshotUpdate {
                cik: "0000008881".into(),
                expected_ticker: "ACTV".into(),
                expected_raw_sha256: Some("sha-actv-1".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: Some("2025-12-31".into()),
                quote_market_time_ms: Some(now_ms - 10_000),
                quote_fetched_at_ms: now_ms - 5_000,
                latest_price: Some(Decimal::from(175)),
                strike_price: Some(Decimal::from(250)),
                distance_to_strike_pct: Some(Decimal::from(-30)),
                strict_fcf_yield_pct: Some(Decimal::from(6)),
            }],
            expected_assumptions_digests: &HashMap::from([("0000008881".to_owned(), wl_dig)]),
            expected_assumptions_revs: &HashMap::from([("0000008881".to_owned(), actv_rev_before)]),
            watermarks: &[],
        };
        db.sync_watchlist_and_upsert_prices(&paths, Some(&wl_live), 50, &commit)
            .unwrap();
        let actv_after_wl = db.query_peers("ACTV", 1).unwrap().target;
        assert!(actv_after_wl.in_watchlist);
        assert_eq!(actv_after_wl.strike_price, Some(Decimal::from(250)));

        let actv_anns: Vec<_> = anns_g1
            .iter()
            .filter(|a| a.cik == "0000008881")
            .cloned()
            .collect();
        // Gen 2 with stale assumptions_rev MUST fail cleanly.
        let gen1 = db.read_sec_generation().unwrap().unwrap();
        let (live_rev, _) = db.load_assumptions_state().unwrap();
        assert!(
            db.replace_sec_lake_generation_with_meta_and_commit_hook(
                &comps_g1[..1],
                &actv_anns,
                &[],
                &[make_snap(
                    "0000008881",
                    "ACTV",
                    5,
                    Some("sha-actv-1"),
                    Some(150)
                )],
                &meta_g1,
                &[],
                &SecGenerationExpectations {
                    expected_prev_sec_gen: Some(Some(gen1)),
                    expected_prev_assumptions_rev: Some(1), // stale!
                    batch_synced_at_ms: now_ms + 1_000,
                    post_parse_watchlist: None,
                    recompute_prices_top: None,
                    refreshed_disk_quotes: &empty_q,
                },
                |_| Ok(()),
            )
            .is_err()
        );

        // Gen 2 with matching live_rev BUT older staged quote ($150) & delisting RLST:
        // must preserve newer $175 quote on ACTV AND retain RLST in companies/facts_annual AND
        // keep RLST's raw sha in preserved callback.
        let mut pres_g2 = Vec::new();
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_g1[..1],
            &actv_anns,
            &[],
            &[make_snap(
                "0000008881",
                "ACTV",
                5,
                Some("sha-actv-1"),
                Some(150),
            )],
            &meta_g1,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(Some(gen1)),
                expected_prev_assumptions_rev: Some(live_rev),
                batch_synced_at_ms: now_ms + 2_000,
                post_parse_watchlist: None,
                recompute_prices_top: None,
                refreshed_disk_quotes: &empty_q,
            },
            |pres| {
                pres_g2 = pres.get("0000008882").cloned().unwrap_or_default();
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(pres_g2, vec![Some("sha-rlst-1".to_owned())]);
        let actv_preserved_q = db.query_peers("ACTV", 1).unwrap().target;
        assert_eq!(actv_preserved_q.latest_price, Some(Decimal::from(175)));

        // Pre-sync load_previous_sec_snapshot MUST also rebuild RLST's 5Y metrics from retained facts!
        let prev_snap = db.load_previous_sec_snapshot().unwrap();
        assert!(prev_snap.parsed_rows["0000008882"].years_of_data == 5);
        assert!(prev_snap.parsed_rows["0000008882"].quality_score > Decimal::ZERO);

        // Gen 3: RLST relists with partial unparsed (years=0, sha=None) incoming snapshot;
        // universe_snapshot MUST rebuild RLST's full 5Y snapshot row instead of updating 0 rows!
        let gen2 = db.read_sec_generation().unwrap().unwrap();
        let (rev2, _) = db.load_assumptions_state().unwrap();
        let mut rlst_partial = make_snap("0000008882", "RLST", 0, None, None);
        rlst_partial.latest_filing_end = None;
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_g1,
            &actv_anns,
            &[],
            &[
                make_snap("0000008881", "ACTV", 5, Some("sha-actv-1"), Some(175)),
                rlst_partial,
            ],
            &meta_g1,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(Some(gen2)),
                expected_prev_assumptions_rev: Some(rev2),
                batch_synced_at_ms: now_ms + 4_000,
                post_parse_watchlist: None,
                recompute_prices_top: None,
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();
        let rlst_relisted = db.query_peers("RLST", 1).unwrap().target;
        assert_eq!(rlst_relisted.years_of_data, 5);
        assert_eq!(rlst_relisted.roic_5y_avg_pct, Some(Decimal::from(30)));
        assert_eq!(rlst_relisted.raw_sha256.as_deref(), Some("sha-rlst-1"));
        assert!(rlst_relisted.quality_score > Decimal::ZERO);

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn round10_relisted_latest_quarter_capex_warning_and_score_preserved() {
        let paths = temp_paths("relist-quarter");
        let mut db = LakeDb::open(&paths).unwrap();
        let now_ms = store::now_ms();
        let empty_q = HashMap::new();
        let make_comp = |cik: &str, ticker: &str| LakeCompanyRow {
            cik: cik.into(),
            ticker: ticker.into(),
            name: format!("{ticker} Corp"),
            exchange: "NASDAQ".into(),
            sic: Some(7372),
            sic_description: Some("Software".into()),
            is_financial: false,
            is_active: true,
        };
        let make_ann_y = |cik: &str, ticker: &str, fy: i32| LakeAnnualRow {
            cik: cik.into(),
            ticker: ticker.into(),
            fiscal_year: fy,
            label: format!("FY{fy}"),
            period_end: format!("{fy}-12-31"),
            filed: Some(format!("{}-02-15", fy + 1)),
            revenue: Decimal::from(500),
            gross_profit: Decimal::from(350),
            gross_margin_pct: Some(Decimal::from(70)),
            ebit: Decimal::from(150),
            nopat: Some(Decimal::from(120)),
            invested_capital: Some(Decimal::from(400)),
            roic_pct: Some(Decimal::from(30)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(28)),
            ocf: Decimal::from(140),
            capex: Decimal::from(20),
            fcf: Decimal::from(120),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(110)),
            sbc_to_ocf_pct: Some(Decimal::new(714, 2)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(450),
            diluted_shares: Decimal::from(10),
            ending_shares: Decimal::from(10),
            raw_sha256: format!("sha-{ticker}-{fy}"),
            provenance_json: "[]".into(),
        };
        let make_ttm = |cik: &str, ticker: &str, q_ocf: i64, q_capex: i64| LakeTtmRow {
            cik: cik.into(),
            ticker: ticker.into(),
            label: "TTM (2026-09-30)".into(),
            period_end: "2026-09-30".into(),
            revenue: Decimal::from(520),
            gross_profit: Decimal::from(360),
            gross_margin_pct: Some(Decimal::from(69)),
            ebit: Decimal::from(155),
            nopat: Some(Decimal::from(124)),
            invested_capital: Some(Decimal::from(400)),
            roic_pct: Some(Decimal::from(31)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(29)),
            ocf: Decimal::from(145),
            capex: Decimal::from(20),
            fcf: Decimal::from(125),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(115)),
            sbc_to_ocf_pct: Some(Decimal::new(690, 2)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(460),
            diluted_shares: Decimal::from(10),
            ytd_period_label: "2026 Q3 YTD".into(),
            ytd_start: "2026-01-01".into(),
            ytd_end: "2026-09-30".into(),
            ytd_revenue: Some(Decimal::from(390)),
            ytd_ocf: Some(Decimal::from(100)),
            ytd_capex: Some(Decimal::from(15)),
            ytd_sbc: Some(Decimal::from(7)),
            ytd_buybacks: Some(Decimal::ZERO),
            latest_quarter_label: Some("2026 Q3".into()),
            latest_quarter_ocf: Some(Decimal::from(q_ocf)),
            latest_quarter_capex: Some(Decimal::from(q_capex)),
            raw_sha256: format!("sha-{ticker}-ttm"),
            provenance_json: "[]".into(),
        };
        let make_snap =
            |cik: &str, ticker: &str, yrs: usize, sha: Option<&str>, q: i64| UniverseRow {
                ticker: ticker.into(),
                cik: cik.into(),
                name: format!("{ticker} Corp"),
                exchange: "NASDAQ".into(),
                sic: Some(7372),
                sic_description: Some("Software".into()),
                is_financial: false,
                latest_filing_end: (yrs > 0).then(|| "2026-09-30".into()),
                years_of_data: yrs,
                quality_score: Decimal::from(q),
                in_price_pool: false,
                in_watchlist: false,
                bucket_opt: None,
                latest_price: None,
                strike_price: None,
                distance_to_strike_pct: None,
                strict_fcf_yield_pct: None,
                roic_5y_avg_pct: (yrs >= 5).then(|| Decimal::from(30)),
                roic_5y_stddev_pct: (yrs >= 5).then_some(Decimal::ZERO),
                gross_margin_5y_avg_pct: (yrs >= 5).then(|| Decimal::from(70)),
                gross_margin_5y_stddev_pct: (yrs >= 5).then_some(Decimal::ZERO),
                share_change_3y_pct: (yrs >= 4).then_some(Decimal::ZERO),
                sbc_to_ocf_pct: (yrs > 0).then(|| Decimal::new(714, 2)),
                net_debt_to_oe: None,
                strict_owner_earnings: (yrs > 0).then(|| Decimal::from(115)),
                warnings_count: 0,
                warnings_json: "[]".into(),
                raw_sha256: sha.map(str::to_owned),
            };

        let comps_g1 = vec![
            make_comp("0000008900", "ACTV"),
            make_comp("0000008901", "CLEAN"),
            make_comp("0000008902", "RELQ"),
        ];
        let mut anns_g1 = Vec::new();
        for fy in 2021..=2025 {
            anns_g1.push(make_ann_y("0000008900", "ACTV", fy));
            anns_g1.push(make_ann_y("0000008901", "CLEAN", fy));
            anns_g1.push(make_ann_y("0000008902", "RELQ", fy));
        }
        let ttms_g1 = vec![
            make_ttm("0000008900", "ACTV", 50, 10),
            make_ttm("0000008901", "CLEAN", 50, 10),
            make_ttm("0000008902", "RELQ", 10, 25),
        ];
        let snap_g1 = vec![
            make_snap("0000008900", "ACTV", 5, Some("sha-actv"), 88),
            make_snap("0000008901", "CLEAN", 5, Some("sha-clean"), 88),
            make_snap("0000008902", "RELQ", 5, Some("sha-relq"), 73),
        ];
        let meta_g1: HashMap<String, SecSnapshotMeta> = comps_g1
            .iter()
            .map(|c| {
                (
                    c.cik.clone(),
                    SecSnapshotMeta {
                        quote_market_time_ms: None,
                        quote_fetched_at_ms: None,
                        assumptions_digest: None,
                        assumptions: None,
                        extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                        quality_score_override: None,
                    },
                )
            })
            .collect();
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_g1,
            &anns_g1,
            &ttms_g1,
            &snap_g1,
            &meta_g1,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(None),
                expected_prev_assumptions_rev: None,
                batch_synced_at_ms: now_ms,
                post_parse_watchlist: None,
                recompute_prices_top: None,
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();
        db.export_all_parquets(&paths).unwrap();
        let manifest = LakeDb::read_parquet_manifest(&paths).unwrap().unwrap();
        let ttm_rel = manifest
            .files
            .iter()
            .find(|f| f.file == "facts_ttm.parquet")
            .unwrap()
            .rel_path
            .clone();
        let ttm_pq = paths.lake_parquet_dir().join(ttm_rel);
        let (qlab, qocf, qcapex): (Option<String>, Option<Decimal>, Option<Decimal>) = db
            .conn
            .query_row(
                &format!(
                    "SELECT latest_quarter_label, latest_quarter_ocf, latest_quarter_capex
                     FROM read_parquet('{}') WHERE ticker = 'RELQ'",
                    escape_sql_literal(&ttm_pq.to_string_lossy())
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(qlab.as_deref(), Some("2026 Q3"));
        assert_eq!(qocf, Some(Decimal::from(10)));
        assert_eq!(qcapex, Some(Decimal::from(25)));

        // Gen 2: CLEAN and RELQ temporarily delist.
        let gen1 = db.read_sec_generation().unwrap().unwrap();
        let (rev1, _) = db.load_assumptions_state().unwrap();
        let actv_anns: Vec<_> = anns_g1
            .iter()
            .filter(|r| r.ticker == "ACTV")
            .cloned()
            .collect();
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_g1[..1],
            &actv_anns,
            &ttms_g1[..1],
            &snap_g1[..1],
            &HashMap::from([(comps_g1[0].cik.clone(), meta_g1[&comps_g1[0].cik].clone())]),
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(Some(gen1)),
                expected_prev_assumptions_rev: Some(rev1),
                batch_synced_at_ms: now_ms + 1_000,
                post_parse_watchlist: None,
                recompute_prices_top: None,
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();

        // Gen 3: CLEAN and RELQ relist unparsed -> rebuilt rows must retain quarter operands & -15 penalty!
        let gen2 = db.read_sec_generation().unwrap().unwrap();
        let (rev2, _) = db.load_assumptions_state().unwrap();
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps_g1,
            &actv_anns,
            &ttms_g1[..1],
            &[
                snap_g1[0].clone(),
                make_snap("0000008901", "CLEAN", 0, None, 0),
                make_snap("0000008902", "RELQ", 0, None, 0),
            ],
            &meta_g1,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(Some(gen2)),
                expected_prev_assumptions_rev: Some(rev2),
                batch_synced_at_ms: now_ms + 2_000,
                post_parse_watchlist: None,
                recompute_prices_top: None,
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();
        let clean_relisted = db.query_peers("CLEAN", 1).unwrap().target;
        let relq_relisted = db.query_peers("RELQ", 1).unwrap().target;
        assert!(
            !clean_relisted
                .warnings_json
                .contains("LATEST_Q_CAPEX_EXCEEDS_OCF")
        );
        assert!(
            relq_relisted
                .warnings_json
                .contains("LATEST_Q_CAPEX_EXCEEDS_OCF")
        );
        assert_eq!(
            relq_relisted.quality_score,
            clean_relisted.quality_score - Decimal::from(15)
        );

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn round10_post_parse_watchlist_eviction_stale_db_quote_rejection_and_disk_prices_sync() {
        let paths = temp_paths("wl-evict-stale-disk");
        let mut db = LakeDb::open(&paths).unwrap();
        let now_ms = store::now_ms();
        let stale_ms = now_ms.saturating_sub(15 * 86_400_000);
        let fresh_ms = now_ms.saturating_sub(60_000);
        let empty_q = HashMap::new();

        let make_comp = |cik: &str, ticker: &str| LakeCompanyRow {
            cik: cik.into(),
            ticker: ticker.into(),
            name: format!("{ticker} Corp"),
            exchange: "NASDAQ".into(),
            sic: Some(7372),
            sic_description: Some("Software".into()),
            is_financial: false,
            is_active: true,
        };
        let make_ann = |cik: &str, ticker: &str, fy: i32| LakeAnnualRow {
            cik: cik.into(),
            ticker: ticker.into(),
            fiscal_year: fy,
            label: format!("FY{fy}"),
            period_end: format!("{fy}-12-31"),
            filed: Some(format!("{}-02-15", fy + 1)),
            revenue: Decimal::from(500),
            gross_profit: Decimal::from(350),
            gross_margin_pct: Some(Decimal::from(70)),
            ebit: Decimal::from(150),
            nopat: Some(Decimal::from(120)),
            invested_capital: Some(Decimal::from(400)),
            roic_pct: Some(Decimal::from(30)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(28)),
            ocf: Decimal::from(140),
            capex: Decimal::from(20),
            fcf: Decimal::from(120),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(110)),
            sbc_to_ocf_pct: Some(Decimal::new(714, 2)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(450),
            diluted_shares: Decimal::from(10),
            ending_shares: Decimal::from(10),
            raw_sha256: format!("sha-{ticker}-{fy}"),
            provenance_json: "[]".into(),
        };
        let make_snap =
            |cik: &str, ticker: &str, q: i64, in_wl: bool, in_pool: bool, px: Option<i64>| {
                UniverseRow {
                    ticker: ticker.into(),
                    cik: cik.into(),
                    name: format!("{ticker} Corp"),
                    exchange: "NASDAQ".into(),
                    sic: Some(7372),
                    sic_description: Some("Software".into()),
                    is_financial: false,
                    latest_filing_end: Some("2025-12-31".into()),
                    years_of_data: 5,
                    quality_score: Decimal::from(q),
                    in_price_pool: in_pool,
                    in_watchlist: in_wl,
                    bucket_opt: in_wl.then_some(crate::model::Bucket::Watch),
                    latest_price: px.map(Decimal::from),
                    strike_price: px.map(|_| Decimal::from(198)),
                    distance_to_strike_pct: px.map(|v| Decimal::from(v - 198)),
                    strict_fcf_yield_pct: px.map(|_| Decimal::from(11)),
                    roic_5y_avg_pct: Some(Decimal::from(30)),
                    roic_5y_stddev_pct: Some(Decimal::ZERO),
                    gross_margin_5y_avg_pct: Some(Decimal::from(70)),
                    gross_margin_5y_stddev_pct: Some(Decimal::ZERO),
                    share_change_3y_pct: Some(Decimal::ZERO),
                    sbc_to_ocf_pct: Some(Decimal::new(714, 2)),
                    net_debt_to_oe: None,
                    strict_owner_earnings: Some(Decimal::from(110)),
                    warnings_count: 0,
                    warnings_json: "[]".into(),
                    raw_sha256: Some(format!("sha-{ticker}")),
                }
            };

        let comps = vec![
            make_comp("0000009101", "TOPA"),
            make_comp("0000009102", "STAL"),
            make_comp("0000009103", "DSKQ"),
            make_comp("0000009104", "MIDB"),
            make_comp("0000009105", "EVWL"),
        ];
        let mut anns = Vec::new();
        for c in &comps {
            for fy in 2021..=2025 {
                anns.push(make_ann(&c.cik, &c.ticker, fy));
            }
        }
        let snap_g1 = vec![
            make_snap("0000009101", "TOPA", 90, false, true, Some(100)),
            make_snap("0000009102", "STAL", 88, false, true, Some(99)),
            make_snap("0000009103", "DSKQ", 85, false, true, None),
            make_snap("0000009104", "MIDB", 80, false, false, None),
            make_snap("0000009105", "EVWL", 10, true, true, Some(42)),
        ];
        let meta_g1 = HashMap::from([
            (
                "0000009101".to_owned(),
                SecSnapshotMeta {
                    quote_market_time_ms: Some(fresh_ms),
                    quote_fetched_at_ms: Some(fresh_ms),
                    assumptions_digest: None,
                    assumptions: None,
                    extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                    quality_score_override: None,
                },
            ),
            (
                "0000009102".to_owned(),
                SecSnapshotMeta {
                    quote_market_time_ms: Some(stale_ms),
                    quote_fetched_at_ms: Some(stale_ms),
                    assumptions_digest: None,
                    assumptions: None,
                    extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                    quality_score_override: None,
                },
            ),
            (
                "0000009105".to_owned(),
                SecSnapshotMeta {
                    quote_market_time_ms: Some(fresh_ms),
                    quote_fetched_at_ms: Some(fresh_ms),
                    assumptions_digest: None,
                    assumptions: None,
                    extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                    quality_score_override: None,
                },
            ),
        ]);
        // Gen 1 uses batch_synced_at_ms = 0 so legacy stale DB quote on STAL ($99 at stale_ms) can seed.
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps,
            &anns,
            &[],
            &snap_g1,
            &meta_g1,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(None),
                expected_prev_assumptions_rev: None,
                batch_synced_at_ms: 0,
                post_parse_watchlist: None,
                recompute_prices_top: None,
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();

        // Gen 2: post-parse watchlist removes EVWL & adds MIDB; Top-3 pool is TOPA/STAL/DSKQ + MIDB.
        // Incoming snapshot has EVWL marked as initially in_watchlist=true & px=$42, STAL px=None, DSKQ px=None.
        let gen1 = db.read_sec_generation().unwrap().unwrap();
        let (rev1, _) = db.load_assumptions_state().unwrap();
        let wl_updated = crate::model::WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![crate::model::CompanyEntry {
                ticker: "MIDB".into(),
                name: "MIDB Corp".into(),
                bucket: crate::model::Bucket::Core,
                source: crate::model::DataSource::SecEdgar,
                cik: Some("0000009104".into()),
                currency: "USD".into(),
                circle: "Tech".into(),
                too_hard_reason: None,
                assumptions: crate::model::ValuationAssumptions {
                    multiple_low: Decimal::from(15),
                    multiple_high: Decimal::from(20),
                    target_return_pct: Decimal::from(10),
                    expected_growth_pct: Decimal::from(10),
                    strike_price_override: None,
                },
            }],
        };
        let disk_quotes = HashMap::from([(
            "0000009103".to_owned(),
            crate::model::PriceSnapshot {
                schema_version: 1,
                symbol: "DSKQ".into(),
                price: Decimal::from(77),
                previous_close: Some(Decimal::from(76)),
                currency: "USD".into(),
                exchange: "NASDAQ".into(),
                market_time_ms: Some(now_ms.saturating_sub(1_000)),
                fetched_at_ms: now_ms,
                source_url: "https://disk.test/DSKQ".into(),
            },
        )]);
        let snap_g2 = vec![
            make_snap("0000009101", "TOPA", 90, false, true, None),
            make_snap("0000009102", "STAL", 88, false, true, None),
            make_snap("0000009103", "DSKQ", 85, false, true, None),
            make_snap("0000009104", "MIDB", 80, false, false, None),
            make_snap("0000009105", "EVWL", 10, true, true, Some(42)),
        ];
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps,
            &anns,
            &[],
            &snap_g2,
            &HashMap::new(),
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(Some(gen1)),
                expected_prev_assumptions_rev: Some(rev1),
                batch_synced_at_ms: now_ms,
                post_parse_watchlist: Some(Some(&wl_updated)),
                recompute_prices_top: Some(3),
                refreshed_disk_quotes: &disk_quotes,
            },
            |_| Ok(()),
        )
        .unwrap();

        let evwl = db.query_peers("EVWL", 1).unwrap().target;
        assert!(!evwl.in_watchlist);
        assert!(!evwl.in_price_pool);
        assert_eq!(evwl.bucket_opt, None);
        assert_eq!(evwl.latest_price, None);
        assert_eq!(evwl.strict_fcf_yield_pct, None);
        assert_eq!(evwl.strike_price, Some(Decimal::from(198)));
        assert_eq!(evwl.distance_to_strike_pct, None);
        let (evwl_qm, evwl_qf): (Option<i64>, Option<i64>) = db
            .conn
            .query_row(
                "SELECT quote_market_time_ms, quote_fetched_at_ms FROM universe_snapshot WHERE ticker = 'EVWL'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((evwl_qm, evwl_qf), (None, None));

        let midb = db.query_peers("MIDB", 1).unwrap().target;
        assert!(midb.in_watchlist);
        assert!(midb.in_price_pool);

        let stal = db.query_peers("STAL", 1).unwrap().target;
        assert!(stal.in_price_pool);
        assert_eq!(stal.latest_price, None);
        let stal_qm: Option<i64> = db
            .conn
            .query_row(
                "SELECT quote_market_time_ms FROM universe_snapshot WHERE ticker = 'STAL'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stal_qm, None);

        let dskq = db.query_peers("DSKQ", 1).unwrap().target;
        assert_eq!(dskq.latest_price, Some(Decimal::from(77)));
        let (dskq_pd_count, dskq_pd_px): (i64, Option<i64>) = db
            .conn
            .query_row(
                "SELECT COUNT(*), CAST(MAX(price) AS BIGINT) FROM prices_daily WHERE symbol = 'DSKQ'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(dskq_pd_count, 1);
        assert_eq!(dskq_pd_px, Some(77));

        let topa = db.query_peers("TOPA", 1).unwrap().target;
        assert_eq!(topa.latest_price, Some(Decimal::from(100)));

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn round11_assumptions_edit_during_parse_and_watchlist_edit_during_quotes_sync_atomically() {
        let paths = temp_paths("round11-races");
        let mut db = LakeDb::open(&paths).unwrap();
        let now_ms = store::now_ms();
        let empty_q = HashMap::new();

        let make_comp = |cik: &str, ticker: &str| LakeCompanyRow {
            cik: cik.into(),
            ticker: ticker.into(),
            name: format!("{ticker} Corp"),
            exchange: "NASDAQ".into(),
            sic: Some(7372),
            sic_description: Some("Software".into()),
            is_financial: false,
            is_active: true,
        };
        let make_ann = |cik: &str, ticker: &str, fy: i32| LakeAnnualRow {
            cik: cik.into(),
            ticker: ticker.into(),
            fiscal_year: fy,
            label: format!("FY{fy}"),
            period_end: format!("{fy}-12-31"),
            filed: Some(format!("{}-02-15", fy + 1)),
            revenue: Decimal::from(500),
            gross_profit: Decimal::from(350),
            gross_margin_pct: Some(Decimal::from(70)),
            ebit: Decimal::from(150),
            nopat: Some(Decimal::from(120)),
            invested_capital: Some(Decimal::from(400)),
            roic_pct: Some(Decimal::from(30)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(28)),
            ocf: Decimal::from(140),
            capex: Decimal::from(20),
            fcf: Decimal::from(120),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(110)), // OE/sh = 11
            sbc_to_ocf_pct: Some(Decimal::new(714, 2)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(450),
            diluted_shares: Decimal::from(10),
            ending_shares: Decimal::from(10),
            raw_sha256: format!("sha-{ticker}-{fy}"),
            provenance_json: "[]".into(),
        };
        let make_snap = |cik: &str,
                         ticker: &str,
                         q: i64,
                         in_wl: bool,
                         in_pool: bool,
                         strike: i64,
                         px: Option<i64>| UniverseRow {
            ticker: ticker.into(),
            cik: cik.into(),
            name: format!("{ticker} Corp"),
            exchange: "NASDAQ".into(),
            sic: Some(7372),
            sic_description: Some("Software".into()),
            is_financial: false,
            latest_filing_end: Some("2025-12-31".into()),
            years_of_data: 5,
            quality_score: Decimal::from(q),
            in_price_pool: in_pool,
            in_watchlist: in_wl,
            bucket_opt: in_wl.then_some(crate::model::Bucket::Core),
            latest_price: px.map(Decimal::from),
            strike_price: Some(Decimal::from(strike)),
            distance_to_strike_pct: px.map(|p| Decimal::from(p - strike)),
            strict_fcf_yield_pct: px.map(|_| Decimal::from(11)),
            roic_5y_avg_pct: Some(Decimal::from(30)),
            roic_5y_stddev_pct: Some(Decimal::ZERO),
            gross_margin_5y_avg_pct: Some(Decimal::from(70)),
            gross_margin_5y_stddev_pct: Some(Decimal::ZERO),
            share_change_3y_pct: Some(Decimal::ZERO),
            sbc_to_ocf_pct: Some(Decimal::new(714, 2)),
            net_debt_to_oe: None,
            strict_owner_earnings: Some(Decimal::from(110)),
            warnings_count: 0,
            warnings_json: "[]".into(),
            raw_sha256: Some(format!("sha-{ticker}")),
        };
        let make_wl_co = |cik: &str, ticker: &str, mult_low: i64| crate::model::CompanyEntry {
            ticker: ticker.into(),
            name: format!("{ticker} Corp"),
            bucket: crate::model::Bucket::Core,
            source: crate::model::DataSource::SecEdgar,
            cik: Some(cik.into()),
            currency: "USD".into(),
            circle: "Tech".into(),
            too_hard_reason: None,
            assumptions: crate::model::ValuationAssumptions {
                multiple_low: Decimal::from(mult_low),
                multiple_high: Decimal::from(mult_low + 4),
                target_return_pct: Decimal::from(10),
                expected_growth_pct: Decimal::from(8),
                strike_price_override: None,
            },
        };

        let comps = vec![
            make_comp("0000009201", "TOP1"),
            make_comp("0000009202", "OLDW"),
            make_comp("0000009203", "NEWW"),
        ];
        let mut anns = Vec::new();
        for c in &comps {
            for fy in 2021..=2025 {
                anns.push(make_ann(&c.cik, &c.ticker, fy));
            }
        }

        // 1. Assumptions edit during SEC parsing: snapshot & meta carry parsed t0 assumptions (18x -> strike=198),
        //    while post_parse_watchlist carries edited t1 assumptions (12x -> strike=132 for TOP1, and retains OLDW).
        let wl_t0 = crate::model::WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![
                make_wl_co("0000009201", "TOP1", 18),
                make_wl_co("0000009202", "OLDW", 18),
            ],
        };
        let wl_t1 = crate::model::WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![
                make_wl_co("0000009201", "TOP1", 12),
                make_wl_co("0000009202", "OLDW", 18),
            ],
        };
        let dig_18 = valuation_assumptions_digest(&wl_t0.companies[0].assumptions);
        let dig_12 = valuation_assumptions_digest(&wl_t1.companies[0].assumptions);
        let meta_t0 = HashMap::from([
            (
                "0000009201".to_owned(),
                SecSnapshotMeta {
                    quote_market_time_ms: None,
                    quote_fetched_at_ms: None,
                    assumptions_digest: Some(dig_18.clone()),
                    assumptions: Some(wl_t0.companies[0].assumptions.clone()),
                    extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                    quality_score_override: None,
                },
            ),
            (
                "0000009202".to_owned(),
                SecSnapshotMeta {
                    quote_market_time_ms: Some(now_ms - 5_000),
                    quote_fetched_at_ms: Some(now_ms - 5_000),
                    assumptions_digest: Some(dig_18.clone()),
                    assumptions: Some(wl_t0.companies[1].assumptions.clone()),
                    extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                    quality_score_override: None,
                },
            ),
            (
                "0000009203".to_owned(),
                SecSnapshotMeta {
                    quote_market_time_ms: None,
                    quote_fetched_at_ms: None,
                    assumptions_digest: Some(valuation_assumptions_digest(
                        &default_valuation_assumptions(),
                    )),
                    assumptions: Some(default_valuation_assumptions()),
                    extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                    quality_score_override: None,
                },
            ),
        ]);
        let snap_t0 = vec![
            make_snap("0000009201", "TOP1", 95, true, true, 198, None),
            make_snap("0000009202", "OLDW", 10, true, true, 198, Some(42)),
            make_snap("0000009203", "NEWW", 5, false, false, 198, None),
        ];
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps,
            &anns,
            &[],
            &snap_t0,
            &meta_t0,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: Some(None),
                expected_prev_assumptions_rev: None,
                batch_synced_at_ms: now_ms,
                post_parse_watchlist: Some(Some(&wl_t1)),
                recompute_prices_top: Some(1),
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();

        let top1 = db.query_peers("TOP1", 1).unwrap().target;
        assert_eq!(top1.strike_price, Some(Decimal::from(132)));
        let stored_top1_dig: Option<String> = db
            .conn
            .query_row(
                "SELECT assumptions_digest FROM universe_snapshot WHERE cik = '0000009201'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored_top1_dig.as_deref(), Some(dig_12.as_str()));

        // 2. Watchlist edited during Yahoo quote fetching: OLDW removed, NEWW added (both fetched_watchlist
        //    and disk_watchlist match wl_t2!). sync_watchlist_and_upsert_prices MUST still rerank pool,
        //    evict OLDW's quote, admit NEWW, and attach NEWW's newly fetched quote ($77)!
        let wl_t2 = crate::model::WatchlistFile {
            schema_version: 1,
            benchmark_symbol: "^GSPC".into(),
            default_target_return_pct: Decimal::from(10),
            companies: vec![
                make_wl_co("0000009201", "TOP1", 12),
                make_wl_co("0000009203", "NEWW", 15),
            ],
        };
        store::save_watchlist(&paths, &wl_t2).unwrap();
        let dig_neww_15 = valuation_assumptions_digest(&wl_t2.companies[1].assumptions);
        let mut expected_digests = HashMap::new();
        expected_digests.insert("0000009201".to_owned(), dig_12.clone());
        expected_digests.insert("0000009203".to_owned(), dig_neww_15);
        let (_, assump_state) = db.load_assumptions_state().unwrap();
        let expected_revs: HashMap<String, u64> = assump_state
            .into_iter()
            .map(|(cik, (_, rev))| (cik, rev))
            .collect();
        let p_neww = LakePriceDailyRow {
            symbol: "NEWW".into(),
            trade_date: "2026-11-23".into(),
            price: Decimal::from(77),
            previous_close: Some(Decimal::from(76)),
            currency: "USD".into(),
            exchange: "NASDAQ".into(),
            market_time_ms: Some(now_ms),
            fetched_at_ms: now_ms,
            source_url: "https://yahoo.test/NEWW".into(),
        };
        let updates = vec![
            QuoteSnapshotUpdate {
                cik: "0000009202".into(),
                expected_ticker: "OLDW".into(),
                expected_raw_sha256: Some("sha-OLDW".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: Some("2025-12-31".into()),
                quote_market_time_ms: Some(now_ms),
                quote_fetched_at_ms: now_ms,
                latest_price: Some(Decimal::from(43)),
                strike_price: Some(Decimal::from(198)),
                distance_to_strike_pct: Some(Decimal::from(-78)),
                strict_fcf_yield_pct: Some(Decimal::from(25)),
            },
            QuoteSnapshotUpdate {
                cik: "0000009203".into(),
                expected_ticker: "NEWW".into(),
                expected_raw_sha256: Some("sha-NEWW".into()),
                expected_latest_filing_end: Some("2025-12-31".into()),
                expected_basis_period_end: Some("2025-12-31".into()),
                quote_market_time_ms: Some(now_ms),
                quote_fetched_at_ms: now_ms,
                latest_price: Some(Decimal::from(77)),
                strike_price: Some(Decimal::from(165)),
                distance_to_strike_pct: Some(Decimal::from(-53)),
                strict_fcf_yield_pct: Some(Decimal::from(14)),
            },
        ];
        let commit = PriceSyncCommit {
            prices: &[p_neww],
            updated_rows: &updates,
            expected_assumptions_digests: &expected_digests,
            expected_assumptions_revs: &expected_revs,
            watermarks: &[],
        };
        db.sync_watchlist_and_upsert_prices(&paths, Some(&wl_t2), 1, &commit)
            .unwrap();

        let oldw = db.query_peers("OLDW", 1).unwrap().target;
        assert!(!oldw.in_watchlist);
        assert!(!oldw.in_price_pool);
        assert_eq!(oldw.latest_price, None);

        let neww = db.query_peers("NEWW", 1).unwrap().target;
        assert!(neww.in_watchlist);
        assert!(neww.in_price_pool);
        assert_eq!(neww.latest_price, Some(Decimal::from(77)));
        assert_eq!(neww.strike_price, Some(Decimal::from(165)));

        // 3. Round 12 preferred vs common ticker guard: watchlist entry "TOP1-PA" sharing TOP1's CIK
        //    must NOT hijack TOP1's watchlist/assumptions or attach TOP1-PA quotes onto TOP1 common stock.
        let mut wl_pref = wl_t2.clone();
        wl_pref
            .companies
            .push(make_wl_co("0000009201", "TOP1-PA", 50));
        store::save_watchlist(&paths, &wl_pref).unwrap();
        let pref_bad_upd = QuoteSnapshotUpdate {
            cik: "0000009201".into(),
            expected_ticker: "TOP1-PA".into(),
            expected_raw_sha256: Some("sha-TOP1".into()),
            expected_latest_filing_end: Some("2025-12-31".into()),
            expected_basis_period_end: Some("2025-12-31".into()),
            quote_market_time_ms: Some(now_ms + 1_000),
            quote_fetched_at_ms: now_ms + 1_000,
            latest_price: Some(Decimal::from(48)),
            strike_price: Some(Decimal::from(550)),
            distance_to_strike_pct: Some(Decimal::from(-91)),
            strict_fcf_yield_pct: Some(Decimal::from(23)),
        };
        let empty_upd_commit = PriceSyncCommit {
            prices: &[],
            updated_rows: &[pref_bad_upd],
            expected_assumptions_digests: &expected_digests,
            expected_assumptions_revs: &HashMap::new(),
            watermarks: &[],
        };
        db.sync_watchlist_and_upsert_prices(&paths, Some(&wl_pref), 1, &empty_upd_commit)
            .unwrap();
        let top1_after_pref = db.query_peers("TOP1", 1).unwrap().target;
        assert_eq!(top1_after_pref.strike_price, Some(Decimal::from(132)));
        assert_ne!(top1_after_pref.latest_price, Some(Decimal::from(48)));

        drop(db);
        let _ = fs::remove_dir_all(&paths.repo_root);
    }

    #[test]
    fn round12_replace_sec_rejects_mass_company_deactivation() {
        let dir = std::env::temp_dir().join(format!("m3-lake-r12-shrink-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let mut db = LakeDb::open(&paths).unwrap();
        let comps: Vec<LakeCompanyRow> = (1..=12)
            .map(|i| LakeCompanyRow {
                cik: format!("{i:010}"),
                ticker: format!("CO{i:02}"),
                name: format!("Company {i}"),
                exchange: "NYSE".into(),
                sic: Some(3571),
                sic_description: Some("Tech".into()),
                is_financial: false,
                is_active: true,
            })
            .collect();
        let snaps: Vec<UniverseRow> = comps
            .iter()
            .map(|c| UniverseRow {
                ticker: c.ticker.clone(),
                cik: c.cik.clone(),
                name: c.name.clone(),
                exchange: c.exchange.clone(),
                sic: c.sic,
                sic_description: c.sic_description.clone(),
                is_financial: false,
                latest_filing_end: Some("2025-12-31".into()),
                years_of_data: 5,
                quality_score: Decimal::from(50),
                in_price_pool: false,
                in_watchlist: false,
                bucket_opt: None,
                latest_price: None,
                strike_price: None,
                distance_to_strike_pct: None,
                strict_fcf_yield_pct: None,
                roic_5y_avg_pct: Some(Decimal::from(15)),
                roic_5y_stddev_pct: Some(Decimal::from(2)),
                gross_margin_5y_avg_pct: Some(Decimal::from(50)),
                gross_margin_5y_stddev_pct: Some(Decimal::from(2)),
                share_change_3y_pct: Some(Decimal::ZERO),
                sbc_to_ocf_pct: Some(Decimal::from(5)),
                net_debt_to_oe: Some(Decimal::ZERO),
                strict_owner_earnings: Some(Decimal::from(100)),
                warnings_count: 0,
                warnings_json: "[]".into(),
                raw_sha256: Some(format!("sha-{}", c.cik)),
            })
            .collect();
        db.replace_sec_lake_data(&comps, &[], &[], &snaps).unwrap();

        let err = db
            .replace_sec_lake_data(&comps[..2], &[], &[], &snaps[..2])
            .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("refusing mass deactivation"),
            "unexpected error: {msg}"
        );

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round13_retained_delistings_exact_ytd_and_strike_zone_ciks() {
        let dir = std::env::temp_dir().join(format!("m3-lake-r13-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = WorkspacePaths::from_root(dir.clone());
        let mut db = LakeDb::open(&paths).unwrap();
        let fresh_ms = store::now_ms();
        let comps: Vec<LakeCompanyRow> = (1..=10)
            .map(|i| LakeCompanyRow {
                cik: format!("{i:010}"),
                ticker: format!("CO{i:02}"),
                name: format!("Company {i}"),
                exchange: "NASDAQ".into(),
                sic: Some(7372),
                sic_description: Some("Software".into()),
                is_financial: false,
                is_active: true,
            })
            .collect();
        let mut anns = Vec::new();
        for c in &comps {
            anns.push(LakeAnnualRow {
                cik: c.cik.clone(),
                ticker: c.ticker.clone(),
                fiscal_year: 2025,
                label: "FY2025".into(),
                period_end: "2025-12-31".into(),
                filed: Some("2026-02-15".into()),
                revenue: Decimal::from(500),
                gross_profit: Decimal::from(350),
                gross_margin_pct: Some(Decimal::from(70)),
                ebit: Decimal::from(150),
                nopat: Some(Decimal::from(120)),
                invested_capital: Some(Decimal::from(400)),
                roic_pct: Some(Decimal::from(30)),
                roic_note: None,
                roic_total_capital_pct: Some(Decimal::from(28)),
                ocf: Decimal::from(140),
                capex: Decimal::from(20),
                fcf: Decimal::from(120),
                sbc: Some(Decimal::from(10)),
                strict_owner_earnings: Some(Decimal::from(110)),
                sbc_to_ocf_pct: Some(Decimal::new(714, 2)),
                buybacks: Some(Decimal::ZERO),
                cash_and_st: Some(Decimal::from(100)),
                total_debt: Some(Decimal::from(50)),
                net_debt: Some(Decimal::from(-50)),
                equity: Decimal::from(450),
                diluted_shares: Decimal::from(10),
                ending_shares: Decimal::from(10),
                raw_sha256: format!("sha-{}", c.cik),
                provenance_json: "[]".into(),
            });
        }
        let ttm_c10 = LakeTtmRow {
            cik: "0000000010".into(),
            ticker: "CO10".into(),
            label: "TTM (2026-09-30)".into(),
            period_end: "2026-09-30".into(),
            revenue: Decimal::from(520),
            gross_profit: Decimal::from(364),
            gross_margin_pct: Some(Decimal::from(70)),
            ebit: Decimal::from(160),
            nopat: Some(Decimal::from(128)),
            invested_capital: Some(Decimal::from(400)),
            roic_pct: Some(Decimal::from(32)),
            roic_note: None,
            roic_total_capital_pct: Some(Decimal::from(30)),
            ocf: Decimal::from(145),
            capex: Decimal::from(20),
            fcf: Decimal::from(125),
            sbc: Some(Decimal::from(10)),
            strict_owner_earnings: Some(Decimal::from(115)),
            sbc_to_ocf_pct: Some(Decimal::new(690, 2)),
            buybacks: Some(Decimal::ZERO),
            cash_and_st: Some(Decimal::from(100)),
            total_debt: Some(Decimal::from(50)),
            net_debt: Some(Decimal::from(-50)),
            equity: Decimal::from(450),
            diluted_shares: Decimal::from(10),
            ytd_period_label: "2026 Q3 YTD".into(),
            ytd_start: "2026-01-01".into(),
            ytd_end: "2026-09-30".into(),
            ytd_revenue: Some(Decimal::new(400_005, 3)),
            ytd_ocf: Some(Decimal::new(100_005, 3)),
            ytd_capex: Some(Decimal::new(100_001, 3)),
            ytd_sbc: Some(Decimal::from(5)),
            ytd_buybacks: Some(Decimal::ZERO),
            latest_quarter_label: None,
            latest_quarter_ocf: None,
            latest_quarter_capex: None,
            raw_sha256: "sha-ttm-10".into(),
            provenance_json: "[]".into(),
        };
        let mut snaps: Vec<UniverseRow> = comps
            .iter()
            .enumerate()
            .map(|(idx, c)| UniverseRow {
                ticker: c.ticker.clone(),
                cik: c.cik.clone(),
                name: c.name.clone(),
                exchange: c.exchange.clone(),
                sic: c.sic,
                sic_description: c.sic_description.clone(),
                is_financial: false,
                latest_filing_end: Some("2025-12-31".into()),
                years_of_data: if idx == 0 { 5 } else { 1 },
                quality_score: Decimal::from(if idx == 0 { 90 } else { 50 }),
                in_price_pool: idx == 0,
                in_watchlist: false,
                bucket_opt: None,
                latest_price: (idx == 0).then_some(Decimal::from(80)),
                strike_price: (idx == 0).then_some(Decimal::from(100)),
                distance_to_strike_pct: (idx == 0).then_some(Decimal::from(-20)),
                strict_fcf_yield_pct: (idx == 0).then_some(Decimal::from(14)),
                roic_5y_avg_pct: Some(Decimal::from(15)),
                roic_5y_stddev_pct: Some(Decimal::from(2)),
                gross_margin_5y_avg_pct: Some(Decimal::from(50)),
                gross_margin_5y_stddev_pct: Some(Decimal::from(2)),
                share_change_3y_pct: Some(Decimal::ZERO),
                sbc_to_ocf_pct: Some(Decimal::from(5)),
                net_debt_to_oe: Some(Decimal::ZERO),
                strict_owner_earnings: Some(Decimal::from(100)),
                warnings_count: 0,
                warnings_json: "[]".into(),
                raw_sha256: Some(format!("sha-{}", c.cik)),
            })
            .collect();
        let mut meta = HashMap::from([(
            "0000000001".to_owned(),
            SecSnapshotMeta {
                quote_market_time_ms: Some(fresh_ms),
                quote_fetched_at_ms: Some(fresh_ms),
                assumptions_digest: None,
                assumptions: None,
                extractor_version: SEC_LAKE_EXTRACTOR_VERSION_NUM,
                quality_score_override: None,
            },
        )]);
        let empty_q = HashMap::new();
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps,
            &anns,
            &[ttm_c10],
            &snaps,
            &meta,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: None,
                expected_prev_assumptions_rev: None,
                batch_synced_at_ms: 0,
                post_parse_watchlist: None,
                recompute_prices_top: Some(1),
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();
        let fresh_trade_date = crate::market::civil_iso_from_unix_secs((fresh_ms / 1000) as i64);
        let p_co1 = LakePriceDailyRow {
            symbol: "CO01".into(),
            trade_date: fresh_trade_date,
            price: Decimal::from(80),
            previous_close: Some(Decimal::from(80)),
            currency: "USD".into(),
            exchange: "NASDAQ".into(),
            market_time_ms: Some(fresh_ms),
            fetched_at_ms: fresh_ms,
            source_url: "https://yahoo.test/CO01".into(),
        };
        let u_co1 = QuoteSnapshotUpdate {
            cik: "0000000001".into(),
            expected_ticker: "CO01".into(),
            expected_raw_sha256: Some("sha-0000000001".into()),
            expected_latest_filing_end: Some("2025-12-31".into()),
            expected_basis_period_end: Some("2025-12-31".into()),
            quote_market_time_ms: Some(fresh_ms),
            quote_fetched_at_ms: fresh_ms,
            latest_price: Some(Decimal::from(80)),
            strike_price: Some(Decimal::from(100)),
            distance_to_strike_pct: Some(Decimal::from(-20)),
            strict_fcf_yield_pct: Some(Decimal::from(14)),
        };
        let (_, assump_state) = db.load_assumptions_state().unwrap();
        let expected_revs: HashMap<String, u64> = assump_state
            .into_iter()
            .map(|(cik, (_, rev))| (cik, rev))
            .collect();
        let commit = PriceSyncCommit {
            prices: &[p_co1],
            updated_rows: &[u_co1],
            expected_assumptions_digests: &HashMap::new(),
            expected_assumptions_revs: &expected_revs,
            watermarks: &[],
        };
        db.sync_watchlist_and_upsert_prices(&paths, None, 1, &commit)
            .unwrap();

        let view = db
            .load_universe_snapshot(&UniverseFilter::default(), Some("0000000001"), 3)
            .unwrap();
        assert_eq!(view.summary.strike_zone_count, 1);
        assert_eq!(
            view.strike_zone_ciks,
            HashSet::from(["0000000001".to_owned()])
        );

        // Clobber SQL ytd_ocf / ytd_capex for CO10 while exact_decimals_json preserves 100.005 > 100.001,
        // and deactivate CO10 (>= 90% active passes guard).
        db.conn
            .execute(
                "UPDATE facts_ttm SET ytd_ocf = 100.00, ytd_capex = 101.00 WHERE cik = '0000000010'",
                [],
            )
            .unwrap();
        snaps.pop();
        meta.remove("0000000010");
        db.replace_sec_lake_generation_with_meta_and_commit_hook(
            &comps[..9],
            &anns[..9],
            &[],
            &snaps,
            &meta,
            &[],
            &SecGenerationExpectations {
                expected_prev_sec_gen: None,
                expected_prev_assumptions_rev: None,
                batch_synced_at_ms: fresh_ms,
                post_parse_watchlist: None,
                recompute_prices_top: Some(1),
                refreshed_disk_quotes: &empty_q,
            },
            |_| Ok(()),
        )
        .unwrap();

        let prev = db.load_previous_sec_snapshot().unwrap();
        assert_eq!(prev.parsed_ciks.len(), 9);
        assert!(!prev.parsed_ciks.contains("0000000010"));
        let retained_c10 = prev.parsed_rows.get("0000000010").unwrap();
        assert!(
            !retained_c10.warnings_json.contains("YTD_CAPEX_EXCEEDS_OCF"),
            "expected exact_decimals_json YTD operands to prevent false YTD warning: {}",
            retained_c10.warnings_json
        );

        // Present WM_SEC_INPUT_GEN row with NULL or blank cursor must fail loudly instead of Ok(None).
        db.conn
            .execute(
                "INSERT OR REPLACE INTO sync_watermarks (source_key, cursor, etag, last_modified, synced_at_ms) VALUES (?, NULL, NULL, NULL, 1)",
                [WM_SEC_INPUT_GEN],
            )
            .unwrap();
        assert!(db.read_sec_input_generation().is_err());
        db.conn
            .execute(
                "INSERT OR REPLACE INTO sync_watermarks (source_key, cursor, etag, last_modified, synced_at_ms) VALUES (?, '   ', NULL, NULL, 1)",
                [WM_SEC_INPUT_GEN],
            )
            .unwrap();
        assert!(db.read_sec_input_generation().is_err());

        drop(db);
        let _ = fs::remove_dir_all(&dir);
    }
}
