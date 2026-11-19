//! Fixed-protocol historical research using the same native strategy and execution engine.
use super::*;
use std::{fs::OpenOptions, io::Write, path::Path};

fn decimal(v: &Value, key: &str) -> Result<Decimal> {
    Decimal::from_str(v[key].as_str().with_context(|| format!("missing {key}"))?)
        .with_context(|| format!("invalid {key}"))
}
fn integer(v: &Value, key: &str) -> Result<u64> {
    v[key].as_u64().with_context(|| format!("missing {key}"))
}
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn ratio(a: Decimal, b: Decimal) -> Value {
    if b.is_zero() {
        Value::Null
    } else {
        json!((a / b).to_string())
    }
}
fn drawdown(values: &[Decimal]) -> (Decimal, Decimal) {
    let mut peak = Decimal::from(1000);
    let (mut amount, mut fraction) = (Decimal::ZERO, Decimal::ZERO);
    for &equity in values {
        peak = peak.max(equity);
        amount = amount.max(peak - equity);
        fraction = fraction.max((peak - equity) / peak);
    }
    (amount, fraction)
}
fn metrics(values: &[Decimal]) -> Result<Value> {
    let last = *values.last().context("empty equity")?;
    let pnl = last - Decimal::from(1000);
    let (dd, dd_fraction) = drawdown(values);
    Ok(
        json!({"terminal_liquidation_usdt":last.to_string(), "net_profit_usdt":pnl.to_string(),
        "net_return_percent":(pnl / Decimal::from(10)).to_string(),
        "max_drawdown_usdt":dd.to_string(), "max_drawdown_percent":(dd_fraction * Decimal::from(100)).to_string(),
        "profit_to_drawdown":ratio(pnl, dd)}),
    )
}
fn month_ends(year: u64, start: u64) -> Vec<u64> {
    let feb = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
        29
    } else {
        28
    };
    [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        .into_iter()
        .scan(start, |at, days| {
            *at += days * 24 * HOUR_MS;
            Some(*at)
        })
        .collect()
}

fn evaluate(
    raw: &Value,
    audit: &Audit,
    engine: &BacktestEngine,
    period: &Value,
    fee_bps: u32,
    slip_bps: u32,
) -> Result<Value> {
    let start = integer(period, "start_ms")?;
    let end = integer(period, "end_exclusive_ms")?;
    let rows = raw.as_array().context("missing klines")?;
    let rows: Vec<_> = rows
        .iter()
        .filter(|r| r[0].as_u64().is_some_and(|ts| ts >= start && ts < end))
        .collect();
    ensure!(
        rows.len() as u64 == (end - start) / HOUR_MS,
        "incomplete evaluation period"
    );
    let qty = Decimal::new(1, 3);
    let fee = Decimal::from(fee_bps) / Decimal::from(10_000);
    let slip = Decimal::from(slip_bps) / Decimal::from(10_000);
    let entry_price =
        (number(rows[0].as_array().context("row")?, 1)? * (Decimal::ONE + slip)).round_dp(2);
    let baseline_entry_fee = (entry_price * qty * fee).round_dp(8);
    let baseline_cash = Decimal::from(1000) - qty * entry_price - baseline_entry_fee;
    let (mut cash, mut btc, mut fees) = (Decimal::from(1000), Decimal::ZERO, Decimal::ZERO);
    let (mut cursor, mut held_hours) = (0usize, 0u64);
    let mut entry_cost: Option<Decimal> = None;
    let mut trades = Vec::new();
    let mut equity = Vec::new();
    let mut baseline = Vec::new();
    let mut timeline = Vec::new();
    let mut exit_fee = Decimal::ZERO;
    for row in &rows {
        let row = row.as_array().context("row")?;
        let open = timestamp(row, 0)?;
        let close_ms = timestamp(row, 6)?;
        while let Some(fill) = audit.fills.get(cursor) {
            let fill_ns = integer(fill, "fill_ts")?;
            if fill_ns > close_ms * 1_000_000 {
                break;
            }
            ensure!(
                fill_ns == open * 1_000_000 && fill_ns > integer(fill, "signal_ts")?,
                "fill outside next-open execution"
            );
            let amount = decimal(fill, "quantity")?;
            let price = decimal(fill, "price")?;
            let commission = Money::from_str(fill["commission"].as_str().context("commission")?)
                .map_err(anyhow::Error::msg)?
                .as_decimal();
            ensure!(
                amount == qty && commission == (qty * price * fee).round_dp(8),
                "quantity/fee mismatch"
            );
            match fill["side"].as_str() {
                Some("BUY") => {
                    ensure!(btc.is_zero() && entry_cost.is_none(), "duplicate buy");
                    cash -= qty * price + commission;
                    btc = qty;
                    entry_cost = Some(qty * price + commission);
                }
                Some("SELL") => {
                    ensure!(btc == qty, "sell without holding");
                    let proceeds = qty * price - commission;
                    let cost = entry_cost.take().context("missing entry")?;
                    trades.push(json!({"exit_ts_ms":fill_ns / 1_000_000,"net_profit_usdt":(proceeds-cost).to_string()}));
                    cash += proceeds;
                    btc = Decimal::ZERO;
                }
                _ => anyhow::bail!("invalid side"),
            }
            fees += commission;
            ensure!(cash >= Decimal::ZERO, "negative cash");
            cursor += 1;
        }
        if btc > Decimal::ZERO {
            held_hours += 1;
        }
        let bid = (number(row, 4)? * (Decimal::ONE - slip)).round_dp(2);
        exit_fee = btc * bid * fee;
        let value = cash + btc * bid - exit_fee;
        let hold = baseline_cash + qty * bid * (Decimal::ONE - fee);
        equity.push(value);
        baseline.push(hold);
        timeline.push(json!({"time_ms":close_ms,"cash_usdt":cash.to_string(),"btc":btc.to_string(),
            "liquidation_usdt":value.to_string(),"buy_hold_liquidation_usdt":hold.to_string(),"fees_paid_usdt":fees.to_string()}));
    }
    ensure!(cursor == audit.fills.len(), "unaccounted fills");
    for (currency, expected) in [("USDT", cash), ("BTC", btc)] {
        let key = format!("account.SIM.balance.{currency}.total");
        let result = engine.get_result();
        let actual = Money::from_str(
            result
                .summary
                .get(&key)
                .with_context(|| format!("missing {key}"))?,
        )
        .map_err(anyhow::Error::msg)?
        .as_decimal();
        ensure!(
            actual == expected,
            "native account/ledger mismatch for {currency}"
        );
    }
    let (mut monthly, mut quarters) = (Vec::new(), Vec::new());
    let (mut previous, mut previous_hold) = (Decimal::from(1000), Decimal::from(1000));
    let (mut quarter_previous, mut quarter_hold) = (previous, previous_hold);
    for (i, boundary) in month_ends(integer(period, "year")?, start)
        .iter()
        .enumerate()
    {
        let index = ((boundary - start) / HOUR_MS - 1) as usize;
        let current = equity[index];
        let hold = baseline[index];
        monthly.push(json!({"month":i+1,"net_profit_usdt":(current-previous).to_string(),"buy_hold_profit_usdt":(hold-previous_hold).to_string()}));
        previous = current;
        previous_hold = hold;
        if (i + 1) % 3 == 0 {
            quarters.push(json!({"quarter":(i+1)/3,"net_profit_usdt":(current-quarter_previous).to_string(),"buy_hold_profit_usdt":(hold-quarter_hold).to_string()}));
            quarter_previous = current;
            quarter_hold = hold;
        }
    }
    let profits: Vec<Decimal> = trades
        .iter()
        .map(|v| decimal(v, "net_profit_usdt"))
        .collect::<Result<_>>()?;
    let wins = profits.iter().filter(|v| **v > Decimal::ZERO).count();
    let win_sum: Decimal = profits.iter().copied().filter(|v| *v > Decimal::ZERO).sum();
    let loss_sum: Decimal = profits.iter().copied().filter(|v| *v < Decimal::ZERO).sum();
    Ok(
        json!({"metrics":metrics(&equity)?,"buy_hold":metrics(&baseline)?,"cash_baseline_usdt":"1000",
        "cash_usdt":cash.to_string(),"btc":btc.to_string(),"actual_fees_usdt":fees.to_string(),
        "hypothetical_exit_fee_usdt":exit_fee.to_string(),"buy_hold_entry_fee_usdt":baseline_entry_fee.to_string(),
        "exposure_fraction":(Decimal::from(held_hours)/Decimal::from(rows.len())).to_string(),
        "hours":rows.len(),"held_hours":held_hours,"completed_round_trips":trades.len(),"winning_round_trips":wins,
        "average_round_trip_profit_usdt":if profits.is_empty() {Value::Null} else {json!((profits.iter().sum::<Decimal>()/Decimal::from(profits.len())).to_string())},
        "profit_factor":ratio(win_sum,-loss_sum),"open_position":btc>Decimal::ZERO,
        "monthly":monthly,"quarters":quarters,"round_trips":trades,"equity":timeline}),
    )
}

fn decision(base: &Value, stress: &Value) -> Result<Value> {
    let b = &base["evaluation"];
    let s = &stress["evaluation"];
    let profit = decimal(&b["metrics"], "net_profit_usdt")?;
    let stress_profit = decimal(&s["metrics"], "net_profit_usdt")?;
    let dd = decimal(&b["metrics"], "max_drawdown_usdt")?;
    let bh_profit = decimal(&b["buy_hold"], "net_profit_usdt")?;
    let bh_dd = decimal(&b["buy_hold"], "max_drawdown_usdt")?;
    let positive_quarters = b["quarters"]
        .as_array()
        .context("quarters")?
        .iter()
        .map(|q| decimal(q, "net_profit_usdt"))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .filter(|p| **p > Decimal::ZERO)
        .count();
    let dominance = profit >= bh_profit && dd <= bh_dd;
    let risk_tradeoff = dd <= bh_dd * Decimal::new(75, 2)
        && dd > Decimal::ZERO
        && bh_dd > Decimal::ZERO
        && profit / dd > bh_profit / bh_dd;
    let verdict = if profit <= Decimal::ZERO && stress_profit <= Decimal::ZERO {
        "abandon_fixed_version"
    } else if profit > Decimal::ZERO
        && stress_profit > Decimal::ZERO
        && positive_quarters >= 3
        && (dominance || risk_tradeoff)
    {
        "continue_to_forward_test_only"
    } else {
        "insufficient_evidence"
    };
    Ok(
        json!({"verdict":verdict,"base_positive":profit>Decimal::ZERO,"stress_positive":stress_profit>Decimal::ZERO,
        "positive_quarters":positive_quarters,"return_drawdown_dominance":dominance,"risk_tradeoff_screen":risk_tradeoff,
        "note":"predeclared research-budget screen, not statistical proof or live-trading authorization"}),
    )
}

pub(super) fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 5,
        "usage: --historical <protocol.json> <calibration|holdout> <artifact-root>"
    );
    let protocol: Value = serde_json::from_slice(&fs::read(&args[2])?)?;
    let label = &args[3];
    ensure!(
        label == "calibration" || label == "holdout",
        "invalid period"
    );
    let period = &protocol["periods"][label];
    let start = integer(period, "start_ms")?;
    let end = integer(period, "end_exclusive_ms")?;
    ensure!(
        end > start
            && protocol["sma_hours"] == 20
            && protocol["warmup_bars"] == 20
            && protocol["starting_cash_usdt"] == "1000"
            && protocol["target_btc"] == "0.001",
        "unsupported protocol"
    );
    let root = Path::new(&args[4]);
    let out = root.join(label);
    ensure!(
        !out.exists(),
        "output already exists; preserve previous research results"
    );
    let input: Value =
        serde_json::from_slice(&fs::read(root.join(format!("{label}-input.json")))?)?;
    let raw = &input["klines"];
    let rows = raw.as_array().context("klines")?;
    ensure!(
        rows.len() as u64 == (end - start) / HOUR_MS + 20
            && rows[0][0].as_u64() == Some(start - 20 * HOUR_MS)
            && rows.last().context("rows")?[6].as_u64() == Some(end - 1),
        "warmup/period boundaries invalid"
    );
    let mut reports = Vec::new();
    let mut engine_states = Vec::new();
    for cost in protocol["costs"].as_array().context("costs")? {
        let name = cost["name"].as_str().context("cost name")?;
        ensure!(name == "base" || name == "stress", "invalid cost name");
        let fee = u32::try_from(integer(cost, "fee_each_side_bps")?)?;
        let slip = u32::try_from(integer(cost, "slippage_each_side_bps")?)?;
        let (data, _) = events_with_slippage(raw, end, slip)?;
        let (mut engine, audit) =
            configured_engine(fee, start.checked_mul(1_000_000).context("start overflow")?)?;
        engine.add_data(data, None, true, true)?;
        engine.run(None, None, None, false)?;
        let audit = audit.borrow();
        ensure!(
            audit.errors.is_empty(),
            "execution errors: {:?}",
            audit.errors
        );
        let evaluation = evaluate(raw, &audit, &engine, period, fee, slip)?;
        reports.push(json!({"experiment_id":protocol["experiment_id"],"period":label,"year":period["year"],"cost":cost,
            "evaluation":evaluation,"signals":audit.signals,"fills":audit.fills,"execution_errors":audit.errors}));
        engine_states.push(engine.get_canonical_result()?.to_bytes()?);
    }
    ensure!(
        reports.len() == 2
            && reports[0]["cost"]["name"] == "base"
            && reports[1]["cost"]["name"] == "stress",
        "expected base and stress"
    );
    let result = json!({"period":label,"year":period["year"],"decision":if label == "holdout" { decision(&reports[0],&reports[1])? } else { json!({"verdict":"calibration_only_no_selection"}) },
        "base":{ "metrics":reports[0]["evaluation"]["metrics"],"buy_hold":reports[0]["evaluation"]["buy_hold"]},
        "stress":{ "metrics":reports[1]["evaluation"]["metrics"],"buy_hold":reports[1]["evaluation"]["buy_hold"]}});
    fs::create_dir(&out)?;
    for (report, state) in reports.iter_mut().zip(engine_states) {
        let name = report["cost"]["name"].as_str().context("name")?.to_owned();
        let equity = report["evaluation"]
            .as_object_mut()
            .context("evaluation")?
            .remove("equity")
            .context("equity")?;
        let mut bytes = Vec::new();
        for item in equity.as_array().context("equity array")? {
            serde_json::to_writer(&mut bytes, item)?;
            bytes.push(b'\n');
        }
        save(&out.join(format!("{name}-equity.jsonl")), &bytes)?;
        save(
            &out.join(format!("{name}-report.json")),
            &serde_json::to_vec_pretty(report)?,
        )?;
        save(&out.join(format!("{name}-engine-state.json")), &state)?;
    }
    save(
        &out.join("result.json"),
        &serde_json::to_vec_pretty(&result)?,
    )?;
    println!("HISTORICAL_OK {}", serde_json::to_string(&result)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn warmup_cannot_trade_and_ledger_matches_native_account() -> Result<()> {
        let start = 1_735_689_600_000u64;
        let end = start + 8760 * HOUR_MS;
        let raw = json!(
            (0..8780u64)
                .map(|i| {
                    let ts = start - 20 * HOUR_MS + i * HOUR_MS;
                    let px = match i {
                        8777 => 110,
                        8778 => 90,
                        _ => 100,
                    }
                    .to_string();
                    json!([
                        ts,
                        px,
                        px,
                        px,
                        px,
                        "1",
                        ts + HOUR_MS - 1,
                        "0",
                        1,
                        "0",
                        "0",
                        "0"
                    ])
                })
                .collect::<Vec<_>>()
        );
        let (data, _) = events_with_slippage(&raw, end, 5)?;
        let (mut engine, audit) = configured_engine(10, start * 1_000_000)?;
        engine.add_data(data, None, true, true)?;
        engine.run(None, None, None, false)?;
        let audit = audit.borrow();
        ensure!(
            audit.fills.len() == 2
                && audit.fills.iter().all(|f| f["fill_ts"]
                    .as_u64()
                    .is_some_and(|t| t >= start * 1_000_000)),
            "warmup traded"
        );
        let period = json!({"start_ms":start,"end_exclusive_ms":end,"year":2025});
        let report = evaluate(&raw, &audit, &engine, &period, 10, 5)?;
        assert_eq!(
            decimal(&report["metrics"], "net_profit_usdt")?,
            Decimal::from_str("0.00972001")?
        );
        assert_eq!(report["completed_round_trips"], 1);
        assert_eq!(report["monthly"].as_array().context("months")?.len(), 12);
        assert_eq!(report["quarters"].as_array().context("quarters")?.len(), 4);
        let mut bad_period = period.clone();
        bad_period["end_exclusive_ms"] = json!(end + HOUR_MS);
        assert!(evaluate(&raw, &audit, &engine, &bad_period, 10, 5).is_err());
        assert_eq!(
            drawdown(&[Decimal::from(1200), Decimal::from(1080)]),
            (Decimal::from(120), Decimal::new(1, 1))
        );
        assert_eq!(month_ends(2024, 0).last(), Some(&(8784 * HOUR_MS)));
        assert_eq!(month_ends(2025, 0).last(), Some(&(8760 * HOUR_MS)));
        Ok(())
    }
    #[test]
    fn negative_holdout_cannot_pass_for_smaller_drawdown() -> Result<()> {
        let b = json!({"evaluation":{"metrics":{"net_profit_usdt":"-1","max_drawdown_usdt":"1"},"buy_hold":{"net_profit_usdt":"-10","max_drawdown_usdt":"10"},"quarters":[{"net_profit_usdt":"1"},{"net_profit_usdt":"1"},{"net_profit_usdt":"1"},{"net_profit_usdt":"-4"}]}});
        assert_eq!(decision(&b, &b)?["verdict"], "abandon_fixed_version");
        Ok(())
    }
}
