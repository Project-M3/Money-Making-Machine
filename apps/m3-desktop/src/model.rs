use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub mod serde_decimal {
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer, de};
    use std::str::FromStr;

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
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => Decimal::from_str(s.trim()).map_err(de::Error::custom),
            serde_json::Value::Number(n) => {
                Decimal::from_str(&n.to_string()).map_err(de::Error::custom)
            }
            other => Err(de::Error::custom(format!(
                "expected decimal string or number, got {other}"
            ))),
        }
    }
}

pub mod serde_decimal_opt {
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer, de};
    use std::str::FromStr;

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
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Decimal::from_str(trimmed)
                        .map(Some)
                        .map_err(de::Error::custom)
                }
            }
            serde_json::Value::Number(n) => Decimal::from_str(&n.to_string())
                .map(Some)
                .map_err(de::Error::custom),
            other => Err(de::Error::custom(format!(
                "expected optional decimal string or number, got {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Core,
    Watch,
    Out,
    TooHard,
}

impl Bucket {
    pub fn code(&self) -> &'static str {
        match self {
            Bucket::Core => "IN",
            Bucket::Watch => "WATCH",
            Bucket::Out => "OUT",
            Bucket::TooHard => "TOO HARD",
        }
    }

    pub fn short_label(&self) -> &'static str {
        match self {
            Bucket::Core => "核心击球",
            Bucket::Watch => "待审讯",
            Bucket::Out => "看懂否决",
            Bucket::TooHard => "能力圈外",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Bucket::Core => "能力圈内确认的好生意 · 耐心等待击球价重仓",
            Bucket::Watch => "待审讯收件箱 · 完成财报体检与费曼答辩后再分拣进三篮子",
            Bucket::Out => "商业逻辑看懂，但护城河 / 文化 / 资本配置不够好 · 明确说不",
            Bucket::TooHard => "未来 5–10 年现金流无法可靠估算 · 永久入馆防 FOMO",
        }
    }

    pub fn display_order() -> [Bucket; 4] {
        [Bucket::Core, Bucket::Watch, Bucket::Out, Bucket::TooHard]
    }
}

pub fn primary_circle_tag(circle: &str) -> String {
    circle.split('/').next().unwrap_or(circle).trim().to_owned()
}

pub fn secondary_circle_detail(circle: &str) -> Option<String> {
    let mut parts = circle.splitn(2, '/');
    let _ = parts.next();
    parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    SecEdgar,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationAssumptions {
    #[serde(with = "serde_decimal")]
    pub multiple_low: Decimal,
    #[serde(with = "serde_decimal")]
    pub multiple_high: Decimal,
    #[serde(with = "serde_decimal")]
    pub target_return_pct: Decimal,
    #[serde(with = "serde_decimal")]
    pub expected_growth_pct: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub strike_price_override: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyEntry {
    pub ticker: String,
    pub name: String,
    pub bucket: Bucket,
    pub source: DataSource,
    #[serde(default)]
    pub cik: Option<String>,
    pub currency: String,
    pub circle: String,
    #[serde(default)]
    pub too_hard_reason: Option<String>,
    pub assumptions: ValuationAssumptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchlistFile {
    pub schema_version: u32,
    pub benchmark_symbol: String,
    #[serde(with = "serde_decimal")]
    pub default_target_return_pct: Decimal,
    pub companies: Vec<CompanyEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldProvenance {
    pub field: String,
    pub source_tag: String,
    #[serde(default)]
    pub filed: Option<String>,
    pub period_end: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnualFinancials {
    pub fiscal_year: i32,
    pub period_end: String,
    #[serde(default)]
    pub filed: Option<String>,
    #[serde(with = "serde_decimal")]
    pub revenue: Decimal,
    #[serde(with = "serde_decimal")]
    pub gross_profit: Decimal,
    #[serde(with = "serde_decimal")]
    pub ebit: Decimal,
    #[serde(with = "serde_decimal")]
    pub tax_expense: Decimal,
    #[serde(with = "serde_decimal")]
    pub pretax_income: Decimal,
    #[serde(with = "serde_decimal")]
    pub ocf: Decimal,
    #[serde(with = "serde_decimal")]
    pub capex: Decimal,
    #[serde(with = "serde_decimal")]
    pub sbc: Decimal,
    #[serde(with = "serde_decimal")]
    pub buybacks: Decimal,
    #[serde(with = "serde_decimal")]
    pub cash_and_st: Decimal,
    #[serde(with = "serde_decimal")]
    pub total_debt: Decimal,
    #[serde(with = "serde_decimal")]
    pub equity: Decimal,
    #[serde(with = "serde_decimal")]
    pub diluted_shares: Decimal,
    #[serde(with = "serde_decimal")]
    pub ending_shares: Decimal,
    #[serde(default)]
    pub provenance: Vec<FieldProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtdPeriodFinancials {
    pub period_label: String,
    pub start: String,
    pub end: String,
    #[serde(default)]
    pub filed: Option<String>,
    #[serde(with = "serde_decimal")]
    pub revenue: Decimal,
    #[serde(with = "serde_decimal")]
    pub ocf: Decimal,
    #[serde(with = "serde_decimal")]
    pub capex: Decimal,
    #[serde(with = "serde_decimal")]
    pub sbc: Decimal,
    #[serde(with = "serde_decimal")]
    pub buybacks: Decimal,
    #[serde(default)]
    pub latest_quarter_label: Option<String>,
    #[serde(default, with = "serde_decimal_opt")]
    pub latest_quarter_ocf: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub latest_quarter_capex: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub latest_quarter_sbc: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub latest_quarter_buybacks: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtmFinancials {
    pub as_of_end: String,
    pub label: String,
    #[serde(with = "serde_decimal")]
    pub revenue: Decimal,
    #[serde(with = "serde_decimal")]
    pub gross_profit: Decimal,
    #[serde(with = "serde_decimal")]
    pub ebit: Decimal,
    #[serde(with = "serde_decimal")]
    pub tax_expense: Decimal,
    #[serde(with = "serde_decimal")]
    pub pretax_income: Decimal,
    #[serde(with = "serde_decimal")]
    pub ocf: Decimal,
    #[serde(with = "serde_decimal")]
    pub capex: Decimal,
    #[serde(with = "serde_decimal")]
    pub sbc: Decimal,
    #[serde(with = "serde_decimal")]
    pub buybacks: Decimal,
    #[serde(with = "serde_decimal")]
    pub cash_and_st: Decimal,
    #[serde(with = "serde_decimal")]
    pub total_debt: Decimal,
    #[serde(with = "serde_decimal")]
    pub equity: Decimal,
    #[serde(with = "serde_decimal")]
    pub diluted_shares: Decimal,
    #[serde(with = "serde_decimal")]
    pub ending_shares: Decimal,
    pub latest_ytd: YtdPeriodFinancials,
    #[serde(default)]
    pub provenance: Vec<FieldProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCompanyFactsSummary {
    pub schema_version: u32,
    pub ticker: String,
    pub entity_name: String,
    pub cik: String,
    pub source_url: String,
    pub raw_sha256: String,
    pub fetched_at_ms: u64,
    pub annuals: Vec<AnnualFinancials>,
    #[serde(default)]
    pub ttm: Option<TtmFinancials>,
    #[serde(default)]
    pub manual_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub schema_version: u32,
    pub symbol: String,
    #[serde(with = "serde_decimal")]
    pub price: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub previous_close: Option<Decimal>,
    pub currency: String,
    pub exchange: String,
    #[serde(default)]
    pub market_time_ms: Option<u64>,
    pub fetched_at_ms: u64,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnualMetricRow {
    pub label: String,
    pub period_end: String,
    #[serde(with = "serde_decimal")]
    pub revenue: Decimal,
    #[serde(with = "serde_decimal")]
    pub gross_profit: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub gross_margin_pct: Option<Decimal>,
    #[serde(with = "serde_decimal")]
    pub ebit: Decimal,
    #[serde(with = "serde_decimal")]
    pub nopat: Decimal,
    #[serde(with = "serde_decimal")]
    pub invested_capital: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_pct: Option<Decimal>,
    #[serde(default)]
    pub roic_note: Option<String>,
    #[serde(default, with = "serde_decimal_opt")]
    pub roic_total_capital_pct: Option<Decimal>,
    #[serde(with = "serde_decimal")]
    pub ocf: Decimal,
    #[serde(with = "serde_decimal")]
    pub capex: Decimal,
    #[serde(with = "serde_decimal")]
    pub fcf: Decimal,
    #[serde(with = "serde_decimal")]
    pub sbc: Decimal,
    #[serde(with = "serde_decimal")]
    pub strict_owner_earnings: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub sbc_to_ocf_pct: Option<Decimal>,
    #[serde(with = "serde_decimal")]
    pub buybacks: Decimal,
    #[serde(with = "serde_decimal")]
    pub cash_and_st: Decimal,
    #[serde(with = "serde_decimal")]
    pub total_debt: Decimal,
    #[serde(with = "serde_decimal")]
    pub net_debt: Decimal,
    #[serde(with = "serde_decimal")]
    pub equity: Decimal,
    #[serde(with = "serde_decimal")]
    pub diluted_shares: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Red,
    Amber,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarningFlag {
    pub code: String,
    pub severity: WarningSeverity,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrikeZoneStatus {
    InStrikeZone,
    NearStrikeZone,
    Wait,
    WarningBlocked,
    Out,
    TooHard,
    NoFinancials,
}

impl StrikeZoneStatus {
    pub fn label(&self) -> &'static str {
        match self {
            StrikeZoneStatus::InStrikeZone => "击球区",
            StrikeZoneStatus::NearStrikeZone => "接近击球",
            StrikeZoneStatus::Wait => "耐心等待",
            StrikeZoneStatus::WarningBlocked => "财报预警",
            StrikeZoneStatus::Out => "已否决",
            StrikeZoneStatus::TooHard => "Too Hard",
            StrikeZoneStatus::NoFinancials => "待录财报",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationSnapshot {
    pub basis_label: String,
    pub period_end: String,
    #[serde(with = "serde_decimal")]
    pub strict_owner_earnings: Decimal,
    #[serde(with = "serde_decimal")]
    pub shares_basis: Decimal,
    #[serde(default, with = "serde_decimal_opt")]
    pub owner_earnings_per_share: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub market_cap: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub enterprise_value: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_fcf_yield_mcap_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_fcf_yield_ev_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub fair_value_low: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub fair_value_high: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strike_price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub distance_to_strike_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub implied_growth_for_target_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub expected_total_return_pct: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    #[serde(default, with = "serde_decimal_opt")]
    pub fair_value: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub distance_to_strike_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_fcf_yield_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub implied_growth_for_target_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub expected_total_return_pct: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MungerReport {
    pub schema_version: u32,
    pub computed_at_ms: u64,
    pub ticker: String,
    pub name: String,
    pub bucket: Bucket,
    pub source: DataSource,
    #[serde(default)]
    pub cik: Option<String>,
    pub currency: String,
    pub circle: String,
    #[serde(default)]
    pub too_hard_reason: Option<String>,
    pub assumptions: ValuationAssumptions,
    #[serde(default)]
    pub price: Option<PriceSnapshot>,
    #[serde(default)]
    pub raw_sha256: Option<String>,
    #[serde(default)]
    pub latest_filing_end: Option<String>,
    pub annual_rows: Vec<AnnualMetricRow>,
    #[serde(default)]
    pub ttm_row: Option<AnnualMetricRow>,
    #[serde(default)]
    pub latest_ytd: Option<YtdPeriodFinancials>,
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
    pub share_change_5y_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub net_debt: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub net_debt_to_owner_earnings: Option<Decimal>,
    #[serde(default)]
    pub fy_valuation: Option<ValuationSnapshot>,
    #[serde(default)]
    pub ttm_valuation: Option<ValuationSnapshot>,
    pub warnings: Vec<WarningFlag>,
    pub status: StrikeZoneStatus,
    pub provenance_summary: Vec<FieldProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Latticework {
    pub microeconomics: String,
    pub ecology: String,
    pub psychology_incentives: String,
    pub capital_allocation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanJudgment {
    pub pending_human_review: bool,
    #[serde(default)]
    pub my_stance: Option<String>,
    #[serde(default)]
    pub weight_rationale: Option<String>,
    #[serde(default)]
    pub falsification_metrics: Vec<String>,
    #[serde(default)]
    pub feynman_checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dossier {
    pub schema_version: u32,
    pub ticker: String,
    pub name: String,
    pub updated_at: String,
    pub draft_source: String,
    pub three_sentence_business: String,
    pub latticework: Latticework,
    pub premortem: Vec<String>,
    pub open_questions: Vec<String>,
    pub human_judgment: HumanJudgment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPosition {
    pub ticker: String,
    #[serde(with = "serde_decimal")]
    pub shares: Decimal,
    #[serde(with = "serde_decimal")]
    pub avg_cost: Decimal,
    pub opened_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JournalAction {
    Wait,
    Out,
    TooHard,
    PaperBuy,
    PaperSell,
}

impl JournalAction {
    pub fn label(&self) -> &'static str {
        match self {
            JournalAction::Wait => "WAIT",
            JournalAction::Out => "OUT",
            JournalAction::TooHard => "TOO_HARD",
            JournalAction::PaperBuy => "PAPER_BUY",
            JournalAction::PaperSell => "PAPER_SELL",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub recorded_at: String,
    pub ticker: String,
    pub action: JournalAction,
    #[serde(default, with = "serde_decimal_opt")]
    pub price: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub shares: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_fcf_yield_pct: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub benchmark_price: Option<Decimal>,
    pub rationale: String,
    pub falsification_trigger: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionJournal {
    pub schema_version: u32,
    pub account_note: String,
    pub currency: String,
    #[serde(with = "serde_decimal")]
    pub starting_cash: Decimal,
    #[serde(with = "serde_decimal")]
    pub cash_balance: Decimal,
    pub benchmark_symbol: String,
    #[serde(default, with = "serde_decimal_opt")]
    pub benchmark_start_price: Option<Decimal>,
    pub positions: Vec<PaperPosition>,
    pub entries: Vec<JournalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncTickerStatus {
    pub ticker: String,
    pub financials_ok: bool,
    pub price_ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusFile {
    pub schema_version: u32,
    pub synced_at_ms: u64,
    pub benchmark_ok: bool,
    #[serde(default)]
    pub benchmark_error: Option<String>,
    pub tickers: Vec<SyncTickerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OhlcBar {
    pub ts_ms: u64,
    #[serde(with = "serde_decimal")]
    pub open: Decimal,
    #[serde(with = "serde_decimal")]
    pub high: Decimal,
    #[serde(with = "serde_decimal")]
    pub low: Decimal,
    #[serde(with = "serde_decimal")]
    pub close: Decimal,
    pub volume: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceHistorySnapshot {
    pub schema_version: u32,
    pub symbol: String,
    pub currency: String,
    pub interval: String,
    pub range: String,
    pub fetched_at_ms: u64,
    pub source_url: String,
    pub bars: Vec<OhlcBar>,
}
