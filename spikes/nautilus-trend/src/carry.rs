//! Personal carry feasibility ledger, not an execution engine or exchange liquidation simulator.
use super::*;
use std::{fs::OpenOptions, io::Write, path::Path};
fn dec(v: &Value, key: &str) -> Result<Decimal> {
    Decimal::from_str(v[key].as_str().with_context(|| format!("missing {key}"))?)
        .with_context(|| format!("invalid {key}"))
}
fn uint(v: &Value, key: &str) -> Result<u64> {
    v[key].as_u64().with_context(|| format!("missing {key}"))
}
fn save(path: &Path, v: &Value) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(v)?)?;
    file.sync_all()?;
    Ok(())
}
fn ohlc(row: &Value, key: &str) -> Result<[Decimal; 4]> {
    let values = row[key]
        .as_array()
        .with_context(|| format!("missing {key}"))?;
    ensure!(values.len() == 4, "invalid OHLC");
    let d = values
        .iter()
        .map(|x| Decimal::from_str(x.as_str().context("price string")?).context("invalid price"))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        d[2] > Decimal::ZERO && d[1] >= d[0].max(d[3]) && d[2] <= d[0].min(d[3]),
        "invalid OHLC range"
    );
    Ok([d[0], d[1], d[2], d[3]])
}
fn bounds(mark: [Decimal; 4], rate: Decimal) -> (Decimal, Decimal, Decimal) {
    let a = mark[2] * rate;
    let b = mark[1] * rate;
    (mark[0] * rate, a.min(b), a.max(b))
}
fn budget_quantity(
    spot: Decimal,
    future: Decimal,
    spot_budget: Decimal,
    margin: Decimal,
    sf: Decimal,
    ff: Decimal,
    slip: Decimal,
    step: Decimal,
) -> Decimal {
    let a = spot_budget / (spot * (Decimal::ONE + slip) * (Decimal::ONE + sf));
    let b = margin / (future * (Decimal::ONE + ff));
    (a.min(b) / step).floor() * step
}
fn months(year: u64, start: u64) -> Vec<(u64, u64)> {
    let feb = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
        29
    } else {
        28
    };
    let mut previous = start;
    [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        .into_iter()
        .map(|days| {
            let next = previous + days * 24 * HOUR_MS;
            let pair = (previous, next);
            previous = next;
            pair
        })
        .collect()
}
fn ledger(input: &Value, p: &Value, cost: &Value, start: u64, end: u64) -> Result<Value> {
    let all = input["hours"].as_array().context("hours")?;
    let rows = all
        .iter()
        .filter(|r| r["t"].as_u64().is_some_and(|t| t >= start && t < end))
        .collect::<Vec<_>>();
    ensure!(
        end > start && rows.len() as u64 == (end - start) / HOUR_MS && !rows.is_empty(),
        "incomplete period"
    );
    let funding = input["funding"].as_array().context("funding")?;
    let mut fcursor = 0;
    while fcursor < funding.len() && uint(&funding[fcursor], "t")? <= start + 60_000 {
        fcursor += 1;
    }
    let capital = dec(p, "capital_cny")?;
    let fx = dec(p, "cny_per_usdt")?;
    let sf = Decimal::from(uint(cost, "spot_fee_bps")?) / Decimal::from(10000);
    let ff = Decimal::from(uint(cost, "futures_fee_bps")?) / Decimal::from(10000);
    let slip = Decimal::from(uint(cost, "slippage_bps")?) / Decimal::from(10000);
    let conversion = Decimal::from(uint(cost, "conversion_each_side_bps")?) / Decimal::from(10000);
    ensure!(
        capital > Decimal::ZERO
            && fx > Decimal::ZERO
            && sf < Decimal::ONE
            && ff < Decimal::ONE
            && slip < Decimal::ONE
            && conversion < Decimal::ONE,
        "invalid budget/cost"
    );
    let initial = capital / fx * (Decimal::ONE - conversion);
    let sb = initial * dec(&p["allocation"], "spot_budget_fraction")?;
    let collateral = initial * dec(&p["allocation"], "separate_futures_cash_fraction")?;
    let reserve = initial * dec(&p["allocation"], "reserve_cash_fraction")?;
    ensure!(
        sb + collateral + reserve == initial,
        "allocation must sum to capital"
    );
    let s0 = ohlc(rows[0], "spot")?[0];
    let f0 = ohlc(rows[0], "future")?[0];
    let step = dec(p, "quantity_step_btc")?;
    ensure!(step > Decimal::ZERO, "invalid lot step");
    let q = budget_quantity(s0, f0, sb, collateral, sf, ff, slip, step);
    ensure!(q > Decimal::ZERO, "capital below assumed lot size");
    let se = price(s0 * (Decimal::ONE + slip))?.as_decimal();
    let fe = price(f0 * (Decimal::ONE - slip))?.as_decimal();
    let spot_entry_fee = (q * se * sf).round_dp(8);
    let future_entry_fee = (q * fe * ff).round_dp(8);
    let spot_cash = sb - q * se - spot_entry_fee;
    ensure!(spot_cash >= Decimal::ZERO, "spot overspend after rounding");
    let wallet_cash = collateral - future_entry_fee;
    let proxy = dec(&p["risk"], "margin_diagnostic_fraction")?;
    let (mut accumulated, mut lower, mut upper) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let mut count = 0u64;
    let mut negative_count = 0u64;
    let mut peak = capital;
    let mut drawdown = Decimal::ZERO;
    let mut worst_eq = capital;
    let mut min_ratio = Decimal::MAX;
    let mut min_ratio_with_reserve = Decimal::MAX;
    let mut breach = Value::Null;
    let mut reserve_breach = Value::Null;
    let mut equity = Vec::new();
    let mut payment_times = Vec::new();
    let mut previous = None;
    let (mut last_se, mut last_fe) = (Decimal::ZERO, Decimal::ZERO);
    for row in rows {
        let t = uint(row, "t")?;
        ensure!(previous.is_none_or(|old| t == old + HOUR_MS), "hour gap");
        previous = Some(t);
        let spot = ohlc(row, "spot")?;
        let future = ohlc(row, "future")?;
        let mark = ohlc(row, "mark")?;
        let lower_before = lower;
        while let Some(event) = funding.get(fcursor) {
            let ft = uint(event, "t")?;
            if ft >= t + HOUR_MS || ft >= end {
                break;
            }
            ensure!(ft >= t, "funding stream out of order");
            let rate = dec(event, "rate")?;
            let (v, lo, hi) = bounds(mark, rate);
            accumulated += q * v;
            lower += q * lo;
            upper += q * hi;
            count += 1;
            if rate < Decimal::ZERO {
                negative_count += 1;
            }
            payment_times
                .push(json!({"t":ft,"cumulative_estimate_cny":(accumulated*fx).to_string()}));
            fcursor += 1;
        }
        let exit_spot = price(spot[3] * (Decimal::ONE - slip))?.as_decimal();
        let exit_future = price(future[3] * (Decimal::ONE + slip))?.as_decimal();
        last_se = exit_spot;
        last_fe = exit_future;
        let without_funding = spot_cash
            + reserve
            + wallet_cash
            + q * exit_spot * (Decimal::ONE - sf)
            + q * (fe - exit_future)
            - q * exit_future * ff;
        let value = |fund: Decimal| (without_funding + fund) * fx * (Decimal::ONE - conversion);
        let eq = value(accumulated);
        let eq_lower = value(lower);
        let eq_upper = value(upper);
        peak = peak.max(eq);
        drawdown = drawdown.max(peak - eq);
        // Intrahour high and payment ordering give a conservative wallet diagnostic, not an observed synchronized tick.
        let stress_wallet = wallet_cash + lower_before.min(lower) + q * (fe - mark[1]);
        let marked_notional = q * mark[1];
        let ratio = stress_wallet / marked_notional;
        min_ratio = min_ratio.min(ratio);
        min_ratio_with_reserve =
            min_ratio_with_reserve.min((stress_wallet + reserve) / marked_notional);
        if breach.is_null() && stress_wallet < proxy * marked_notional {
            breach = json!({"hour_ms":t,"wallet_equity_lower_cny":(stress_wallet*fx).to_string(),"mark_high":mark[1].to_string(),"margin_ratio_lower":ratio.to_string(),"note":"assumed 2% separate-wallet proxy; subsequent full-period funding is theoretical"});
        }
        if reserve_breach.is_null() && stress_wallet + reserve < proxy * marked_notional {
            reserve_breach = json!({"hour_ms":t,"margin_ratio_with_all_initial_reserve":((stress_wallet+reserve)/marked_notional).to_string()});
        }
        let worst_spot = price(spot[2] * (Decimal::ONE - slip))?.as_decimal();
        let worst_future = price(future[1] * (Decimal::ONE + slip))?.as_decimal();
        let conservative = (spot_cash
            + reserve
            + wallet_cash
            + lower_before.min(lower)
            + q * worst_spot * (Decimal::ONE - sf)
            + q * (fe - worst_future)
            - q * worst_future * ff)
            * fx
            * (Decimal::ONE - conversion);
        worst_eq = worst_eq.min(conservative);
        equity.push(json!({"hour_ms":t,"equity_cny":eq.to_string(),"funding_lower_equity_cny":eq_lower.to_string(),"funding_upper_equity_cny":eq_upper.to_string(),"funding_estimate_cny":(accumulated*fx).to_string(),"funding_lower_cny":(lower*fx).to_string(),"funding_upper_cny":(upper*fx).to_string(),"short_wallet_close_cny":((wallet_cash+lower+q*(fe-mark[3]))*fx).to_string(),"short_wallet_intrahour_ratio_lower":ratio.to_string()}));
    }
    let last = equity.last().context("empty equity")?;
    let terminal = dec(last, "equity_cny")?;
    let entry_fees = spot_entry_fee + future_entry_fee;
    let exit_fees = q * last_se * sf + q * last_fe * ff;
    let entry_slip = q * (se - s0 + f0 - fe);
    let exit_row = all
        .iter()
        .find(|r| r["t"].as_u64() == Some(end - HOUR_MS))
        .context("last row")?;
    let exit_slip =
        q * (ohlc(exit_row, "spot")?[3] - last_se + last_fe - ohlc(exit_row, "future")?[3]);
    let trade_cost = (entry_fees + exit_fees + entry_slip + exit_slip) * fx;
    let break_even = payment_times
        .iter()
        .find(|v| dec(v, "cumulative_estimate_cny").is_ok_and(|v| v >= trade_cost));
    let mut monthly = Vec::new();
    let mut prev = capital;
    for (i, (_, boundary)) in months(uint(input, "year")?, uint(&all[0], "t")?)
        .iter()
        .enumerate()
    {
        if *boundary <= start || *boundary > end {
            continue;
        }
        let row = equity
            .iter()
            .find(|v| v["hour_ms"].as_u64() == Some(boundary - HOUR_MS))
            .context("month boundary")?;
        let current = dec(row, "equity_cny")?;
        monthly.push(json!({"month":i+1,"net_change_cny":(current-prev).to_string(),"terminal_equity_cny":current.to_string(),"funding_cumulative_cny":row["funding_estimate_cny"],"diagnostic_breached_by_month_end":!breach.is_null() && uint(&breach,"hour_ms")?<*boundary}));
        prev = current;
    }
    let funding_stress = -q * f0 * Decimal::new(1, 4) * Decimal::from(90) * fx;
    let double_wallet = wallet_cash + q * (fe - f0 * Decimal::from(2));
    Ok(
        json!({"start_ms":start,"end_exclusive_ms":end,"quantity_btc":q.to_string(),"initial_spot_notional_cny":(q*s0*fx).to_string(),"initial_futures_notional_cny":(q*f0*fx).to_string(),"collateral_cny":(collateral*fx).to_string(),"reserve_cny":(reserve*fx).to_string(),"undeployed_spot_cash_cny":(spot_cash*fx).to_string(),
        "net_profit_estimate_cny":(terminal-capital).to_string(),"net_profit_lower_cny":(dec(last,"funding_lower_equity_cny")?-capital).to_string(),"net_profit_upper_cny":(dec(last,"funding_upper_equity_cny")?-capital).to_string(),
        "net_profit_without_conversion_cost_same_quantity_cny":(terminal/(Decimal::ONE-conversion)+capital*conversion-capital).to_string(),
        "funding_estimate_cny":(accumulated*fx).to_string(),"funding_lower_cny":(lower*fx).to_string(),"funding_upper_cny":(upper*fx).to_string(),"funding_payments":count,"negative_payments":negative_count,
        "trading_fees_cny":((entry_fees+exit_fees)*fx).to_string(),"synthetic_slippage_cny":((entry_slip+exit_slip)*fx).to_string(),"conversion_entry_cost_cny":(capital*conversion).to_string(),"conversion_exit_cost_cny":(terminal/(Decimal::ONE-conversion)*conversion).to_string(),
        "max_hourly_close_drawdown_cny":drawdown.to_string(),"max_hourly_close_drawdown_fraction_of_initial":(drawdown/capital).to_string(),"minimum_conservative_intrahour_equity_cny":worst_eq.to_string(),"minimum_short_wallet_intrahour_ratio_lower":min_ratio.to_string(),"minimum_ratio_with_entire_initial_reserve":min_ratio_with_reserve.to_string(),"first_margin_proxy_breach":breach,"first_breach_even_with_all_initial_reserve":reserve_breach,
        "attainable_full_period_profit_certified":false,"full_period_path_margin_compatible_under_proxy":breach.is_null(),"post_breach_cashflows_theoretical":!breach.is_null(),
        "funding_only_trade_cost_break_even_ms":break_even.map(|v|v["t"].clone()),"break_even_note":"first cumulative estimated funding covering full-period four-fill trading cost; excludes FX and basis; later reversals may undo this, not a forward forecast",
        "synthetic_stress":{"negative_1bp_every8h_30days_cny":funding_stress.to_string(),"perpetual_premium_widens_5pct_cny":(-q*f0*Decimal::new(5,2)*fx).to_string(),"both_prices_double_short_wallet_cny":(double_wallet*fx).to_string(),"both_prices_double_with_initial_reserve_cny":((double_wallet+reserve)*fx).to_string(),"all_on_venue_capital_loss_cny":capital.to_string()},
        "monthly":monthly,"equity":equity}),
    )
}
pub(super) fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 5,
        "usage: --carry <protocol.json> <input.json> <new-output-dir>"
    );
    let p: Value = serde_json::from_slice(&fs::read(&args[2])?)?;
    let input: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let out = Path::new(&args[4]);
    ensure!(
        !out.exists(),
        "preserve previous output; directory already exists"
    );
    let rows = input["hours"].as_array().context("hours")?;
    ensure!(!rows.is_empty(), "empty input");
    let year = uint(&input, "year")?;
    ensure!(year == 2024 || year == 2025, "unsupported year");
    let start = uint(&rows[0], "t")?;
    let periods = months(year, start);
    let end = periods.last().context("months")?.1;
    ensure!(
        rows.len() as u64 == (end - start) / HOUR_MS,
        "not a full year"
    );
    let mut prev = None;
    for r in input["funding"].as_array().context("funding")? {
        let t = uint(r, "t")?;
        ensure!(
            t >= start && t < end && prev.is_none_or(|old| t > old),
            "bad funding time"
        );
        ensure!(
            dec(r, "rate")?.abs() < Decimal::ONE && uint(r, "interval_hours")? > 0,
            "bad funding"
        );
        prev = Some(t);
    }
    let mut result = Vec::new();
    let mut annual_equity = Vec::new();
    for cost in p["costs"].as_array().context("costs")? {
        let name = cost["name"].as_str().context("cost name")?;
        ensure!(name == "base" || name == "stress", "invalid name");
        let mut annual = ledger(&input, &p, cost, start, end)?;
        annual_equity.push((
            name.to_owned(),
            annual
                .as_object_mut()
                .context("annual")?
                .remove("equity")
                .context("equity")?,
        ));
        let mut monthly = Vec::new();
        for (i, (a, b)) in periods.iter().enumerate() {
            let mut m = ledger(&input, &p, cost, *a, *b)?;
            m.as_object_mut().context("month")?.remove("equity");
            m["month"] = json!(i + 1);
            monthly.push(m);
        }
        result.push(json!({"cost":cost,"continuous_year":annual,"independent_months":monthly}));
    }
    fs::create_dir(out)?;
    for (name, equity) in annual_equity {
        save(&out.join(format!("{name}-equity.json")), &equity)?;
    }
    let report = json!({"experiment_id":p["experiment_id"],"year":year,"capital_cny":p["capital_cny"],"funding_mode":input["funding_mark_mode"],"results":result,"cash_baseline_cny":p["capital_cny"],"scope":"historical feasibility only, no real orders or exchange liquidation reconstruction"});
    save(&out.join("report.json"), &report)?;
    println!("CARRY_OK year={year}; outputs saved with two costs and all calendar months");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negative_funding_reverses_price_bound_order() {
        let mark = [
            Decimal::from(100),
            Decimal::from(120),
            Decimal::from(80),
            Decimal::from(105),
        ];
        assert_eq!(
            bounds(mark, Decimal::new(-1, 2)),
            (Decimal::from(-1), Decimal::new(-12, 1), Decimal::new(-8, 1))
        );
    }
    #[test]
    fn collateral_and_fees_limit_both_legs() {
        let q = budget_quantity(
            Decimal::from(100),
            Decimal::from(100),
            Decimal::from(40),
            Decimal::from(40),
            Decimal::new(1, 3),
            Decimal::new(5, 4),
            Decimal::new(5, 4),
            Decimal::new(1, 3),
        );
        assert_eq!(q, Decimal::new(399, 3));
        // A price-neutral hedge still loses four-fill costs; collateral is returned capital, never a second expense.
        let initial = Decimal::from(100);
        let spot_cash = Decimal::from(40) - q * Decimal::from(100);
        let collateral = Decimal::from(40);
        let reserve = Decimal::from(20);
        assert_eq!(
            spot_cash + q * Decimal::from(100) + collateral + reserve,
            initial
        );
        assert!(collateral + q * (Decimal::from(100) - Decimal::from(210)) < Decimal::ZERO);
    }
}
