use crate::model::{
    AnnualFinancials, AnnualMetricRow, Bucket, CompanyEntry, MungerReport, PriceSnapshot,
    RawCompanyFactsSummary, SandboxResult, StrikeZoneStatus, TtmFinancials, ValuationAssumptions,
    ValuationSnapshot, WarningFlag, WarningSeverity,
};
use rust_decimal::Decimal;

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
    cash_and_st: Decimal,
    total_debt: Decimal,
    equity: Decimal,
    diluted_shares: Decimal,
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

    let nopat = inp.ebit * (Decimal::ONE - effective_tax_rate);
    let invested_capital = inp.total_debt + inp.equity - inp.cash_and_st;
    let (roic_pct, roic_note) = if invested_capital > Decimal::ZERO {
        (Some((nopat / invested_capital) * hundred()), None)
    } else {
        (None, Some("net_cash_exceeds_book_capital".to_owned()))
    };

    let total_capital = inp.total_debt + inp.equity;
    let roic_total_capital_pct = if total_capital > Decimal::ZERO {
        Some((nopat / total_capital) * hundred())
    } else {
        None
    };

    let fcf = inp.ocf - inp.capex;
    let strict_owner_earnings = inp.ocf - inp.capex - inp.sbc;
    let sbc_to_ocf_pct = if inp.ocf > Decimal::ZERO {
        Some((inp.sbc / inp.ocf) * hundred())
    } else {
        None
    };
    let net_debt = inp.total_debt - inp.cash_and_st;

    AnnualMetricRow {
        label: inp.label,
        period_end: inp.period_end,
        revenue: inp.revenue,
        gross_profit: inp.gross_profit,
        gross_margin_pct,
        ebit: inp.ebit,
        nopat,
        invested_capital,
        roic_pct,
        roic_note,
        roic_total_capital_pct,
        ocf: inp.ocf,
        capex: inp.capex,
        fcf,
        sbc: inp.sbc,
        strict_owner_earnings,
        sbc_to_ocf_pct,
        buybacks: inp.buybacks,
        cash_and_st: inp.cash_and_st,
        total_debt: inp.total_debt,
        net_debt,
        equity: inp.equity,
        diluted_shares: inp.diluted_shares,
    }
}

fn from_annual(a: &AnnualFinancials) -> AnnualMetricRow {
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
        cash_and_st: a.cash_and_st,
        total_debt: a.total_debt,
        equity: a.equity,
        diluted_shares: a.diluted_shares,
    })
}

fn from_ttm(t: &TtmFinancials) -> AnnualMetricRow {
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
        cash_and_st: t.cash_and_st,
        total_debt: t.total_debt,
        equity: t.equity,
        diluted_shares: t.diluted_shares,
    })
}

fn compute_valuation(
    basis_label: &str,
    row: &AnnualMetricRow,
    price_opt: Option<&PriceSnapshot>,
    assumptions: &ValuationAssumptions,
) -> ValuationSnapshot {
    let shares = row.diluted_shares;
    let oe = row.strict_owner_earnings;
    let oe_per_share = if shares > Decimal::ZERO {
        Some(oe / shares)
    } else {
        None
    };

    let fair_value_low = oe_per_share
        .filter(|v| *v > Decimal::ZERO)
        .map(|v| v * assumptions.multiple_low);
    let fair_value_high = oe_per_share
        .filter(|v| *v > Decimal::ZERO)
        .map(|v| v * assumptions.multiple_high);
    let strike_price = assumptions.strike_price_override.or(fair_value_low);

    let (market_cap, enterprise_value, yield_mcap, yield_ev, distance, implied_growth, expected_ret) =
        if let Some(p) = price_opt {
            let mcap = if shares > Decimal::ZERO {
                Some(p.price * shares)
            } else {
                None
            };
            let ev = mcap.map(|m| m + row.net_debt);
            let ym = mcap
                .filter(|m| *m > Decimal::ZERO)
                .map(|m| (oe / m) * hundred());
            let ye = ev
                .filter(|e| *e > Decimal::ZERO)
                .map(|e| (oe / e) * hundred());
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
        strict_owner_earnings: oe,
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

pub fn build_report(
    company: &CompanyEntry,
    facts: Option<&RawCompanyFactsSummary>,
    price: Option<&PriceSnapshot>,
    computed_at_ms: u64,
) -> MungerReport {
    let annual_rows: Vec<AnnualMetricRow> = facts
        .map(|f| f.annuals.iter().map(from_annual).collect())
        .unwrap_or_default();
    let ttm_row = facts.and_then(|f| f.ttm.as_ref()).map(from_ttm);
    let latest_ytd = facts
        .and_then(|f| f.ttm.as_ref())
        .map(|t| t.latest_ytd.clone());

    let last5: Vec<&AnnualMetricRow> = annual_rows.iter().rev().take(5).collect();
    let roic_vals: Vec<Decimal> = last5.iter().filter_map(|r| r.roic_pct).collect();
    let (roic_5y_avg_pct, roic_5y_stddev_pct) = mean_and_stddev(&roic_vals)
        .map(|(m, s)| (Some(m), Some(s)))
        .unwrap_or((None, None));

    let gm_vals: Vec<Decimal> = last5.iter().filter_map(|r| r.gross_margin_pct).collect();
    let (gross_margin_5y_avg_pct, gross_margin_5y_stddev_pct) = mean_and_stddev(&gm_vals)
        .map(|(m, s)| (Some(m), Some(s)))
        .unwrap_or((None, None));

    let share_change_3y_pct = if annual_rows.len() >= 4 {
        let latest = &annual_rows[annual_rows.len() - 1];
        let base = &annual_rows[annual_rows.len() - 4];
        if base.diluted_shares > Decimal::ZERO {
            Some(((latest.diluted_shares - base.diluted_shares) / base.diluted_shares) * hundred())
        } else {
            None
        }
    } else {
        None
    };

    let share_change_5y_pct = if annual_rows.len() >= 6 {
        let latest = &annual_rows[annual_rows.len() - 1];
        let base = &annual_rows[annual_rows.len() - 6];
        if base.diluted_shares > Decimal::ZERO {
            Some(((latest.diluted_shares - base.diluted_shares) / base.diluted_shares) * hundred())
        } else {
            None
        }
    } else {
        None
    };

    let latest_row = ttm_row.as_ref().or_else(|| annual_rows.last());
    let net_debt = latest_row.map(|r| r.net_debt);
    let net_debt_to_owner_earnings = latest_row.and_then(|r| {
        if r.strict_owner_earnings > Decimal::ZERO {
            Some(r.net_debt / r.strict_owner_earnings)
        } else {
            None
        }
    });

    let fy_valuation = annual_rows
        .last()
        .map(|row| compute_valuation("FY", row, price, &company.assumptions));
    let ttm_valuation = ttm_row
        .as_ref()
        .map(|row| compute_valuation("TTM", row, price, &company.assumptions));

    let mut warnings = Vec::new();
    if let Some(row) = latest_row {
        if row.strict_owner_earnings <= Decimal::ZERO {
            warnings.push(WarningFlag {
                code: "NEGATIVE_STRICT_OE".to_owned(),
                severity: WarningSeverity::Red,
                title: "严苛股东盈余为负".to_owned(),
                detail: format!(
                    "{} 期间 OCF - CapEx - SBC = {:.2}M，经营现金流未能覆盖资本开支与股权激励",
                    row.label,
                    row.strict_owner_earnings / Decimal::from(1_000_000)
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
        if ytd.capex > ytd.ocf {
            warnings.push(WarningFlag {
                code: "YTD_CAPEX_EXCEEDS_OCF".to_owned(),
                severity: WarningSeverity::Red,
                title: "最新季报期资本开支超过经营现金流".to_owned(),
                detail: format!(
                    "{} CapEx ({:.2}M) > OCF ({:.2}M)，自由现金流转为净消耗",
                    ytd.period_label,
                    ytd.capex / Decimal::from(1_000_000),
                    ytd.ocf / Decimal::from(1_000_000)
                ),
            });
        } else if let (Some(q_ocf), Some(q_capex)) = (ytd.latest_quarter_ocf, ytd.latest_quarter_capex)
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

        if ytd.buybacks.is_zero()
            && annual_rows
                .last()
                .is_some_and(|fy| fy.buybacks > Decimal::ZERO)
        {
            warnings.push(WarningFlag {
                code: "BUYBACKS_HALTED".to_owned(),
                severity: WarningSeverity::Amber,
                title: "最新财年迄今股票回购归零".to_owned(),
                detail: format!("{} 期未发生普通股回购，需核查资本配置转向", ytd.period_label),
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
    } else if net_debt.is_some_and(|nd| nd > Decimal::ZERO)
        && latest_row.is_some_and(|r| r.strict_owner_earnings <= Decimal::ZERO)
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

    let status = if company.bucket == Bucket::TooHard {
        StrikeZoneStatus::TooHard
    } else if company.bucket == Bucket::Out {
        StrikeZoneStatus::Out
    } else if annual_rows.is_empty() {
        StrikeZoneStatus::NoFinancials
    } else if warnings.iter().any(|w| w.severity == WarningSeverity::Red) {
        StrikeZoneStatus::WarningBlocked
    } else {
        let primary = ttm_valuation.as_ref().or(fy_valuation.as_ref());
        match primary.and_then(|v| v.distance_to_strike_pct) {
            Some(dist) if dist <= Decimal::ZERO => StrikeZoneStatus::InStrikeZone,
            Some(dist) if dist <= Decimal::from(15) => StrikeZoneStatus::NearStrikeZone,
            _ => StrikeZoneStatus::Wait,
        }
    };

    let provenance_summary = facts
        .and_then(|f| f.annuals.last())
        .map(|a| a.provenance.clone())
        .unwrap_or_default();

    let latest_filing_end = latest_row.map(|r| r.period_end.clone());

    MungerReport {
        schema_version: 1,
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
        raw_sha256: facts.map(|f| f.raw_sha256.clone()),
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
            cash_and_st: Decimal::from(200),
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
            ticker: "TEST".into(),
            entity_name: "Test Co".into(),
            cik: "0000000001".into(),
            source_url: "https://test".into(),
            raw_sha256: "abc".into(),
            fetched_at_ms: 1,
            annuals: vec![
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
        assert_eq!(fy.strict_owner_earnings, Decimal::from(200));
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
                cash_and_st: Decimal::from(200),
                total_debt: Decimal::from(500),
                equity: Decimal::from(900),
                diluted_shares: Decimal::from(100),
                ending_shares: Decimal::from(100),
                latest_ytd: YtdPeriodFinancials {
                    period_label: "2026 Q3 YTD".into(),
                    start: "2026-01-01".into(),
                    end: "2026-09-30".into(),
                    filed: None,
                    revenue: Decimal::from(800),
                    ocf: Decimal::from(127),
                    capex: Decimal::from(132),
                    sbc: Decimal::from(23),
                    buybacks: Decimal::ZERO,
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
}
