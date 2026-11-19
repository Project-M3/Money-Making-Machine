use super::*;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    thread,
    time::Instant,
};

const API: &str = "https://data-api.binance.vision/api/v3";
// Change this when strategy, instrument, fees, or journal semantics change; never mix experiments.
const PROTOCOL: &str = "sma20-cash-1000-qty001-fee10-slip5-v1";

fn integer(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .with_context(|| format!("missing integer {key}"))
}

fn decimal(value: &Value, key: &str) -> Result<Decimal> {
    Decimal::from_str(
        value[key]
            .as_str()
            .with_context(|| format!("missing decimal {key}"))?,
    )
    .with_context(|| format!("invalid decimal {key}"))
}

fn fetch(client: &reqwest::blocking::Client, last_open: Option<u64>) -> Result<Value> {
    let started = Instant::now();
    let get =
        |url: String| -> Result<Value> { Ok(client.get(url).send()?.error_for_status()?.json()?) };
    let before = integer(&get(format!("{API}/time"))?, "serverTime")?;
    let suffix = match last_open {
        Some(ts) => format!(
            "limit=1000&startTime={}",
            ts.checked_add(HOUR_MS).context("time overflow")?
        ),
        None => "limit=21".to_owned(),
    };
    let raw = get(format!("{API}/klines?symbol=BTCUSDT&interval=1h&{suffix}"))?;
    let quote = get(format!("{API}/ticker/bookTicker?symbol=BTCUSDT"))?;
    let after = integer(&get(format!("{API}/time"))?, "serverTime")?;
    ensure!(
        after >= before && after - before <= 10_000 && started.elapsed() <= Duration::from_secs(10),
        "stale/slow snapshot"
    );
    let mut bars = Vec::new();
    for row in raw.as_array().context("klines must be array")? {
        let fields = row.as_array().context("kline must be array")?;
        let (_, close, _) = parse_bar(fields)?;
        if close < before {
            bars.push(row.clone());
        }
    }
    Ok(
        json!({"protocol": PROTOCOL, "before_ms": before, "observed_ms": after, "bars": bars, "quote": quote}),
    )
}

fn batch_events(
    batch: &Value,
    last_open: Option<u64>,
    last_observed: u64,
) -> Result<(Vec<Data>, u64)> {
    ensure!(
        batch["protocol"] == PROTOCOL,
        "journal belongs to a different experiment"
    );
    let before = integer(batch, "before_ms")?;
    let observed = integer(batch, "observed_ms")?;
    ensure!(
        observed > last_observed && observed >= before && observed - before <= 10_000,
        "invalid observation time"
    );
    let ns = observed
        .checked_mul(1_000_000)
        .context("timestamp overflow")?;
    let rows = batch["bars"].as_array().context("missing bars")?;
    if last_open.is_none() {
        ensure!(rows.len() >= 20, "need 20 warmup bars");
    }
    let mut previous = last_open;
    let mut data = Vec::new();
    for row in rows {
        let (open, close, mut bar) = parse_bar(row.as_array().context("invalid bar")?)?;
        ensure!(
            open % HOUR_MS == 0 && close < before,
            "unclosed or unaligned bar"
        );
        if let Some(prev) = previous {
            ensure!(
                Some(open) == prev.checked_add(HOUR_MS),
                "missing/duplicate bar"
            );
        }
        // Catch-up bars become known now; do not invent trades during downtime.
        bar.ts_init = (ns - 1).into();
        data.push(Data::Bar(bar));
        previous = Some(open);
    }
    let latest = previous.context("missing warmup")?;
    ensure!(
        Some(latest)
            == (before / HOUR_MS)
                .checked_sub(1)
                .and_then(|v| v.checked_mul(HOUR_MS)),
        "snapshot missing latest closed bar"
    );
    let quote = &batch["quote"];
    ensure!(quote["symbol"] == "BTCUSDT", "unexpected symbol");
    let bid = decimal(quote, "bidPrice")?;
    let ask = decimal(quote, "askPrice")?;
    ensure!(bid > Decimal::ZERO && ask >= bid, "invalid spread");
    let bid_size = decimal(quote, "bidQty")?;
    let ask_size = decimal(quote, "askQty")?;
    ensure!(
        bid_size >= Decimal::new(1, 3) && ask_size >= Decimal::new(1, 3),
        "insufficient displayed depth"
    );
    // shortcut: observed top-of-book plus 5bp adverse slippage; no queue/latency model.
    data.push(Data::Quote(QuoteTick::new_checked(
        InstrumentId::from(INSTRUMENT),
        price(bid * Decimal::new(9995, 4))?,
        price(ask * Decimal::new(10005, 4))?,
        Quantity::from_str("0.00100").map_err(anyhow::Error::msg)?,
        Quantity::from_str("0.00100").map_err(anyhow::Error::msg)?,
        ns.into(),
        ns.into(),
    )?));
    Ok((data, latest))
}

fn apply(engine: &mut BacktestEngine, audit: &Rc<RefCell<Audit>>, data: Vec<Data>) -> Result<()> {
    engine.clear_data();
    engine.add_data(data, None, true, true)?;
    engine.run(None, None, None, true)?;
    ensure!(
        audit.borrow().errors.is_empty(),
        "execution errors: {:?}",
        audit.borrow().errors
    );
    Ok(())
}

fn read_journal(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path)?;
    let committed = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let batches = bytes[..committed]
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect::<std::result::Result<Vec<Value>, _>>()?;
    if committed != bytes.len() {
        // An unterminated write was never applied; preserve it for diagnosis before truncation.
        fs::write(path.with_extension("torn"), &bytes[committed..])?;
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_len(committed as u64)?;
        file.sync_all()?;
    }
    Ok(batches)
}

fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn report(
    engine: &BacktestEngine,
    audit: &Audit,
    first: &Value,
    latest: &Value,
    batches: usize,
) -> Result<Value> {
    let result = engine.get_result();
    let balance = |currency: &str| -> Result<Decimal> {
        let key = format!("account.SIM.balance.{currency}.total");
        let money = result
            .summary
            .get(&key)
            .with_context(|| format!("missing {key}"))?;
        Ok(Money::from_str(money)
            .map_err(anyhow::Error::msg)?
            .as_decimal())
    };
    let cash = balance("USDT")?;
    let btc = balance("BTC")?;
    let bid = price(decimal(&latest["quote"], "bidPrice")? * Decimal::new(9995, 4))?.as_decimal();
    let ask = price(decimal(&first["quote"], "askPrice")? * Decimal::new(10005, 4))?.as_decimal();
    let fee = Decimal::new(1, 3);
    let qty = Decimal::new(1, 3);
    let liquidation = cash + btc * bid * (Decimal::ONE - fee);
    let buy_hold =
        Decimal::from(1000) - qty * ask * (Decimal::ONE + fee) + qty * bid * (Decimal::ONE - fee);
    Ok(json!({
        "scope": "public live observations, local streaming backtest simulation only", "protocol": PROTOCOL,
        "batches": batches, "observed_ms": latest["observed_ms"], "first_observed_ms": first["observed_ms"],
        "cash_usdt": cash.to_string(), "btc": btc.to_string(), "estimated_liquidation_usdt": liquidation.to_string(),
        "cash_baseline_usdt": "1000", "buy_hold_001_btc_liquidation_usdt": buy_hold.to_string(),
        "assumptions": "1000 USDT fixture, fixed 0.001 BTC, 10bp fee each side, observed spread plus 5bp adverse slippage each side; baseline same size and starting cash; no leverage",
        "signals": audit.signals, "fills": audit.fills, "execution_errors": audit.errors,
        "note": "bookTicker has no exchange timestamp; observed_ms is server time after response. Missed bars update signals but never receive invented historical fills. Liquidation values include estimated exit fee; no forced sale."
    }))
}

fn record_event(dir: &Path, event: &str, detail: Value) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let value = json!({"schema_version":1,"time_ms":now,"pid":std::process::id(),"protocol":PROTOCOL,"event":event,"detail":detail});
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("engine-events.jsonl"))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

pub(super) fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    ensure!(
        args.iter().all(|a| a == "--once" || a == "--managed"),
        "usage: --paper [--once] [--managed]"
    );
    let once = args.iter().any(|a| a == "--once");
    if args.iter().any(|a| a == "--managed") {
        // GUI owns the stdin pipe: even a GUI crash must not leave an orphan trader.
        thread::spawn(|| {
            use std::io::Read;
            let mut byte = [0u8; 1];
            loop {
                match std::io::stdin().read(&mut byte) {
                    Ok(0) | Err(_) => std::process::exit(0),
                    Ok(_) => {}
                }
            }
        });
    }
    let dir = Path::new("artifacts/paper");
    fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("run.lock"))?;
    lock.try_lock()
        .context("another paper process holds this state directory")?;
    record_event(
        dir,
        "engine_started",
        json!({"once":once,"managed":args.iter().any(|a|a=="--managed")}),
    )?;
    let result = run_locked(dir, once);
    if let Err(error) = &result {
        record_event(dir, "engine_failed", json!({"error":format!("{error:#}")}))?;
        atomic_json(
            &dir.join("status.json"),
            &json!({"state": "stopped_error", "error": format!("{error:#}")}),
        )?;
    }
    result
}

fn run_locked(dir: &Path, once: bool) -> Result<()> {
    let path = dir.join("inputs.jsonl");
    let batches = read_journal(&path)?;
    let (mut engine, audit) = new_engine()?;
    let mut last_open = None;
    let mut observed = 0;
    for batch in &batches {
        let (data, open) = batch_events(batch, last_open, observed)?;
        apply(&mut engine, &audit, data)?;
        last_open = Some(open);
        observed = integer(batch, "observed_ms")?;
    }
    record_event(
        dir,
        "state_restored",
        json!({"batches":batches.len(),"signals":audit.borrow().signals.len(),"fills":audit.borrow().fills.len(),"last_observed_ms":observed}),
    )?;
    let mut signal_count = audit.borrow().signals.len();
    let mut fill_count = audit.borrow().fills.len();
    let mut count = batches.len();
    let mut first = batches.first().cloned();
    if let (Some(first), Some(last)) = (first.as_ref(), batches.last()) {
        atomic_json(
            &dir.join("summary.json"),
            &report(&engine, &audit.borrow(), first, last, count)?,
        )?;
    }
    drop(batches);
    let mut journal = OpenOptions::new().create(true).append(true).open(path)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut failures = 0u64;
    loop {
        let fetched = fetch(&client, last_open).and_then(|batch| {
            let (data, open) = batch_events(&batch, last_open, observed)?;
            Ok((batch, data, open))
        });
        let (batch, data, open) = match fetched {
            Ok(value) => value,
            Err(error) => {
                failures += 1;
                record_event(
                    dir,
                    "data_retry",
                    json!({"consecutive_failures":failures,"last_observed_ms":observed,"error":format!("{error:#}")}),
                )?;
                atomic_json(
                    &dir.join("status.json"),
                    &json!({"state": "waiting_for_valid_data", "consecutive_failures": failures, "last_observed_ms": observed, "error": format!("{error:#}")}),
                )?;
                eprintln!("PAPER_RETRY failures={failures}: {error:#}");
                if once {
                    return Err(error);
                }
                thread::sleep(Duration::from_secs((5 * failures).min(60)));
                continue;
            }
        };
        let mut line = serde_json::to_vec(&batch)?;
        line.push(b'\n');
        journal.write_all(&line)?;
        journal.sync_all()?;
        // Fatal persistence/engine errors exit: the next start reconstructs committed inputs.
        apply(&mut engine, &audit, data)?;
        count += 1;
        last_open = Some(open);
        observed = integer(&batch, "observed_ms")?;
        let initial = first.get_or_insert_with(|| batch.clone());
        let summary = report(&engine, &audit.borrow(), initial, &batch, count)?;
        atomic_json(&dir.join("summary.json"), &summary)?;
        record_event(
            dir,
            "batch_completed",
            json!({"batch":count,"observed_ms":observed,
            "recovered_after_failures":failures,
            "signals":&audit.borrow().signals[signal_count..],"fills":&audit.borrow().fills[fill_count..],
            "account":{"cash_usdt":summary["cash_usdt"],"btc":summary["btc"],"estimated_liquidation_usdt":summary["estimated_liquidation_usdt"]},
            "baselines":{"cash_usdt":summary["cash_baseline_usdt"],"buy_hold_usdt":summary["buy_hold_001_btc_liquidation_usdt"]}}),
        )?;
        signal_count = audit.borrow().signals.len();
        fill_count = audit.borrow().fills.len();
        atomic_json(
            &dir.join("status.json"),
            &json!({"state": if once { "stopped_after_one_batch" } else { "running" }, "pid": std::process::id(), "last_observed_ms": observed, "batches": count, "consecutive_failures": 0}),
        )?;
        println!(
            "PAPER_OK batches={count} signals={} fills={} observed_ms={observed}",
            audit.borrow().signals.len(),
            audit.borrow().fills.len()
        );
        failures = 0;
        if once {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(prices: &[u64], start: usize, observed: u64) -> Value {
        let bars: Vec<Value> = prices
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let open = 1_735_689_600_000 + (start + i) as u64 * HOUR_MS;
                json!([
                    open,
                    p.to_string(),
                    p.to_string(),
                    p.to_string(),
                    p.to_string(),
                    "1.00000",
                    open + HOUR_MS - 1,
                    "0",
                    1,
                    "0",
                    "0",
                    "0"
                ])
            })
            .collect();
        json!({"protocol": PROTOCOL,"before_ms":observed-1,"observed_ms":observed,"bars":bars,
            "quote":{"symbol":"BTCUSDT","bidPrice":"100.00","askPrice":"100.01","bidQty":"1","askQty":"1"}})
    }

    #[test]
    fn streaming_restart_matches_and_duplicate_bars_fail() -> Result<()> {
        let base = 1_735_689_600_000;
        let a = batch(&vec![100; 20], 0, base + 20 * HOUR_MS + 10);
        let b = batch(&[110], 20, base + 21 * HOUR_MS + 10);
        let c = batch(&[], 21, base + 21 * HOUR_MS + 100);
        let d = batch(&[90], 21, base + 22 * HOUR_MS + 10);
        let batches = [&a, &b, &c, &d];
        let run = || -> Result<Value> {
            let (mut engine, audit) = new_engine()?;
            let mut last = None;
            let mut observed = 0;
            for value in batches {
                let (data, open) = batch_events(value, last, observed)?;
                apply(&mut engine, &audit, data)?;
                last = Some(open);
                observed = integer(value, "observed_ms")?;
            }
            let output = report(&engine, &audit.borrow(), &a, &d, 4)?;
            assert_eq!(audit.borrow().fills.len(), 2);
            assert_eq!(decimal(&output, "btc")?, Decimal::ZERO);
            let expected = Decimal::from_str("999.99968999")?;
            assert_eq!(decimal(&output, "estimated_liquidation_usdt")?, expected);
            assert_eq!(
                decimal(&output, "buy_hold_001_btc_liquidation_usdt")?,
                expected
            );
            Ok(output)
        };
        assert_eq!(run()?, run()?);
        assert!(batch_events(&b, Some(base + 20 * HOUR_MS), base + 20 * HOUR_MS + 10).is_err());
        let mut unfinished = b.clone();
        unfinished["before_ms"] = json!(base + 21 * HOUR_MS - 1);
        assert!(batch_events(&unfinished, Some(base + 19 * HOUR_MS), 0).is_err());
        Ok(())
    }

    #[test]
    fn journal_recovers_only_uncommitted_tail() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("m3-journal-{}", std::process::id()));
        fs::create_dir_all(&dir)?;
        record_event(&dir, "test_event", json!({"amount":"1.25"}))?;
        let event: Value = serde_json::from_slice(&fs::read(dir.join("engine-events.jsonl"))?)?;
        assert_eq!(event["schema_version"], 1);
        assert_eq!(event["detail"]["amount"], "1.25");
        let lock_path = dir.join("run.lock");
        let lock = File::create(&lock_path)?;
        lock.try_lock()?;
        let competing = OpenOptions::new().write(true).open(&lock_path)?;
        assert!(competing.try_lock().is_err());
        drop(lock);
        competing.try_lock()?;
        drop(competing);
        let path = dir.join("inputs.jsonl");
        fs::write(&path, b"{\"ok\":1}\n{\"torn\":")?;
        assert_eq!(read_journal(&path)?, vec![json!({"ok":1})]);
        assert_eq!(fs::read(&path)?, b"{\"ok\":1}\n");
        fs::write(&path, b"invalid\n")?;
        assert!(read_journal(&path).is_err());
        assert_eq!(fs::read(&path)?, b"invalid\n");
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
