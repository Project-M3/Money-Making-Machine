use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub(crate) fn parse_raw_json_decimal_token(raw_token: &str) -> Result<Option<Decimal>, String> {
    let trimmed = raw_token.trim();
    if trimmed == "null" {
        return Ok(None);
    }
    if trimmed.starts_with('"') {
        let s: String =
            serde_json::from_str(trimmed).map_err(|e| format!("invalid decimal string: {e}"))?;
        let inner = s.trim();
        if inner.is_empty() {
            return Ok(None);
        }
        return parse_decimal_token(inner).map(Some);
    }
    parse_decimal_token(trimmed).map(Some)
}

pub(crate) fn parse_decimal_token(token: &str) -> Result<Decimal, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("empty decimal value".to_owned());
    }
    let (negative, unsigned) = if let Some(rest) = trimmed.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        (false, rest)
    } else {
        (false, trimmed)
    };
    if unsigned.is_empty() {
        return Err(format!("invalid decimal {token}: missing digits"));
    }

    let (mantissa_str, exp) = match unsigned.find(['e', 'E']) {
        Some(idx) => {
            let exp_part = &unsigned[idx + 1..];
            let exp_digits = exp_part.strip_prefix(['+', '-']).unwrap_or(exp_part);
            if exp_digits.is_empty() || !exp_digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("invalid scientific exponent in decimal {token}"));
            }
            let parsed_exp = exp_part
                .parse::<i64>()
                .map_err(|e| format!("scientific exponent overflow in decimal {token}: {e}"))?;
            (&unsigned[..idx], parsed_exp)
        }
        None => (unsigned, 0i64),
    };

    let (int_str, frac_str) = match mantissa_str.find('.') {
        Some(dot) => (&mantissa_str[..dot], &mantissa_str[dot + 1..]),
        None => (mantissa_str, ""),
    };
    if (int_str.is_empty() && frac_str.is_empty())
        || !int_str.bytes().all(|b| b.is_ascii_digit())
        || !frac_str.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(format!("invalid decimal {token}"));
    }

    let frac_len = i64::try_from(frac_str.len())
        .map_err(|_| format!("fractional digits overflow in decimal {token}"))?;
    let mut digits = String::with_capacity(int_str.len() + frac_str.len());
    digits.push_str(int_str);
    digits.push_str(frac_str);

    let trimmed_leading = digits.trim_start_matches('0');
    if trimmed_leading.is_empty() {
        return Ok(Decimal::ZERO);
    }
    let sig_digits = trimmed_leading.trim_end_matches('0');
    let stripped_trailing_zeros = trimmed_leading.len() - sig_digits.len();
    let trailing_zeros_i64 = i64::try_from(stripped_trailing_zeros)
        .map_err(|_| format!("exponent overflow in decimal {token}"))?;
    let power10 = exp
        .checked_sub(frac_len)
        .and_then(|v| v.checked_add(trailing_zeros_i64))
        .ok_or_else(|| format!("exponent overflow in decimal {token}"))?;

    if power10 < -28 {
        return Err(format!(
            "decimal {token} exceeds maximum 28-decimal-place exact precision"
        ));
    }

    let fixed = if power10 >= 0 {
        let zeros =
            usize::try_from(power10).map_err(|_| format!("decimal {token} overflows Decimal"))?;
        if sig_digits.len().saturating_add(zeros) > 29 {
            return Err(format!("decimal {token} overflows 96-bit Decimal"));
        }
        let mut s = String::with_capacity(usize::from(negative) + sig_digits.len() + zeros);
        if negative {
            s.push('-');
        }
        s.push_str(sig_digits);
        s.extend(std::iter::repeat_n('0', zeros));
        s
    } else {
        let scale = usize::try_from(-power10)
            .map_err(|_| format!("decimal {token} exceeds scale bounds"))?;
        if sig_digits.len() <= scale {
            let lead_zeros = scale - sig_digits.len();
            let mut s = String::with_capacity(usize::from(negative) + 2 + scale);
            if negative {
                s.push('-');
            }
            s.push_str("0.");
            s.extend(std::iter::repeat_n('0', lead_zeros));
            s.push_str(sig_digits);
            s
        } else {
            let split = sig_digits.len() - scale;
            if sig_digits.len() > 29 {
                return Err(format!("decimal {token} overflows 96-bit Decimal"));
            }
            let mut s = String::with_capacity(usize::from(negative) + sig_digits.len() + 1);
            if negative {
                s.push('-');
            }
            s.push_str(&sig_digits[..split]);
            s.push('.');
            s.push_str(&sig_digits[split..]);
            s
        }
    };

    Decimal::from_str_exact(&fixed).map_err(|e| format!("invalid decimal {token}: {e}"))
}

pub mod serde_decimal {
    use super::parse_raw_json_decimal_token;
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer, de};

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
        parse_raw_json_decimal_token(raw.get())
            .map_err(de::Error::custom)?
            .ok_or_else(|| de::Error::custom("expected decimal string or number, got null/empty"))
    }
}

pub mod serde_decimal_opt {
    use super::parse_raw_json_decimal_token;
    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serializer, de};

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
        parse_raw_json_decimal_token(raw.get()).map_err(de::Error::custom)
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

    pub fn short_label_for(&self, lang: crate::i18n::UiLang) -> &'static str {
        match (self, lang) {
            (Bucket::Core, crate::i18n::UiLang::Zh) => "核心击球",
            (Bucket::Core, crate::i18n::UiLang::En) => "Core Strike",
            (Bucket::Watch, crate::i18n::UiLang::Zh) => "待审讯",
            (Bucket::Watch, crate::i18n::UiLang::En) => "Watch",
            (Bucket::Out, crate::i18n::UiLang::Zh) => "看懂否决",
            (Bucket::Out, crate::i18n::UiLang::En) => "Pass / Out",
            (Bucket::TooHard, crate::i18n::UiLang::Zh) => "能力圈外",
            (Bucket::TooHard, crate::i18n::UiLang::En) => "Too Hard",
        }
    }

    pub fn description_for(&self, lang: crate::i18n::UiLang) -> &'static str {
        match (self, lang) {
            (Bucket::Core, crate::i18n::UiLang::Zh) => "能力圈内确认的好生意 · 耐心等待击球价重仓",
            (Bucket::Core, crate::i18n::UiLang::En) => {
                "Verified compounders inside your Circle · wait patiently for strike price"
            }
            (Bucket::Watch, crate::i18n::UiLang::Zh) => {
                "待审讯收件箱 · 完成财报体检与费曼答辩后再分拣进能力圈"
            }
            (Bucket::Watch, crate::i18n::UiLang::En) => {
                "Under study · complete accounting triage and Alan interrogation before sorting"
            }
            (Bucket::Out, crate::i18n::UiLang::Zh) => {
                "商业逻辑看懂，但护城河 / 文化 / 资本配置不够好 · 明确说不"
            }
            (Bucket::Out, crate::i18n::UiLang::En) => {
                "Understood clearly, rejected on moat, culture, or capital allocation"
            }
            (Bucket::TooHard, crate::i18n::UiLang::Zh) => {
                "未来 5–10 年现金流无法可靠估算 · 永久入馆防 FOMO"
            }
            (Bucket::TooHard, crate::i18n::UiLang::En) => {
                "5–10Y cash flows unforecastable · archived permanently against FOMO"
            }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
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
    #[serde(default, with = "serde_decimal_opt")]
    pub cash_and_st: Option<Decimal>,
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
    #[serde(default, with = "serde_decimal_opt")]
    pub revenue: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub ocf: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub capex: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub sbc: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub buybacks: Option<Decimal>,
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
    #[serde(default, with = "serde_decimal_opt")]
    pub cash_and_st: Option<Decimal>,
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
    #[serde(default)]
    pub extractor_version: u32,
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
    #[serde(default, with = "serde_decimal_opt")]
    pub nopat: Option<Decimal>,
    #[serde(default, with = "serde_decimal_opt")]
    pub invested_capital: Option<Decimal>,
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
    pub fn label_for(&self, lang: crate::i18n::UiLang) -> &'static str {
        match (self, lang) {
            (StrikeZoneStatus::InStrikeZone, crate::i18n::UiLang::Zh) => "击球区",
            (StrikeZoneStatus::InStrikeZone, crate::i18n::UiLang::En) => "Strike Zone",
            (StrikeZoneStatus::NearStrikeZone, crate::i18n::UiLang::Zh) => "接近击球",
            (StrikeZoneStatus::NearStrikeZone, crate::i18n::UiLang::En) => "Near Strike",
            (StrikeZoneStatus::Wait, crate::i18n::UiLang::Zh) => "耐心等待",
            (StrikeZoneStatus::Wait, crate::i18n::UiLang::En) => "Wait",
            (StrikeZoneStatus::WarningBlocked, crate::i18n::UiLang::Zh) => "财报预警",
            (StrikeZoneStatus::WarningBlocked, crate::i18n::UiLang::En) => "Warning",
            (StrikeZoneStatus::Out, crate::i18n::UiLang::Zh) => "已否决",
            (StrikeZoneStatus::Out, crate::i18n::UiLang::En) => "Out",
            (StrikeZoneStatus::TooHard, _) => "Too Hard",
            (StrikeZoneStatus::NoFinancials, crate::i18n::UiLang::Zh) => "待录财报",
            (StrikeZoneStatus::NoFinancials, crate::i18n::UiLang::En) => "No Financials",
        }
    }

    pub fn label(&self) -> &'static str {
        self.label_for(crate::i18n::UiLang::Zh)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationSnapshot {
    pub basis_label: String,
    pub period_end: String,
    #[serde(default, with = "serde_decimal_opt")]
    pub strict_owner_earnings: Option<Decimal>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct SampleDec {
        #[serde(with = "serde_decimal")]
        req: Decimal,
        #[serde(default, with = "serde_decimal_opt")]
        opt: Option<Decimal>,
    }

    #[test]
    fn raw_value_decimal_serde_preserves_bare_json_number_precision_without_f64() {
        let json = r#"{
            "req": 123456789012345678901.23456789,
            "opt": 0.1234567890123456789012345
        }"#;
        let parsed: SampleDec = serde_json::from_str(json).expect("parse exact JSON numbers");
        assert_eq!(
            parsed.req.to_string(),
            "123456789012345678901.23456789",
            "must preserve 29 digits beyond IEEE-754 f64 15-17 digit limit"
        );
        assert_eq!(
            parsed.opt.expect("opt").to_string(),
            "0.1234567890123456789012345"
        );

        let sci: SampleDec = serde_json::from_str(r#"{"req": 1.234567890123456789e6, "opt": ""}"#)
            .expect("parse scientific token and empty string");
        assert_eq!(sci.req.to_string(), "1234567.890123456789");
        assert_eq!(sci.opt, None);
    }

    #[test]
    fn parse_decimal_token_normalizes_scientific_exponents_and_rejects_excess_precision() {
        assert_eq!(
            parse_decimal_token("0.00000000000000000000000000001e1").unwrap(),
            Decimal::new(1, 28)
        );
        assert_eq!(parse_decimal_token("1.25e2").unwrap(), Decimal::from(125));
        assert_eq!(parse_decimal_token("-0.0000e-40").unwrap(), Decimal::ZERO);
        assert!(parse_decimal_token("0.00000000000000000000000000001").is_err());
        assert!(parse_decimal_token("1e-29").is_err());
        assert!(parse_decimal_token("1e30").is_err());
        assert!(parse_decimal_token("NaN").is_err());
    }
}
