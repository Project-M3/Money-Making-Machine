mod carry;
mod historical;
mod paper;

use std::{cell::RefCell, collections::VecDeque, fs, rc::Rc, str::FromStr, time::Duration};

use anyhow::{Context, Result, ensure};
use nautilus_backtest::{
    config::{BacktestEngineConfig, SimulatedVenueConfig},
    engine::BacktestEngine,
};
use nautilus_common::actor::DataActor;
use nautilus_execution::models::fee::{FeeModelAny, MakerTakerFeeModel};
use nautilus_model::{
    data::{Bar, BarType, Data, QuoteTick},
    enums::{AccountType, BookType, OmsType, OrderSide},
    events::{OrderDenied, OrderFilled, OrderRejected},
    identifiers::{InstrumentId, Symbol, Venue},
    instruments::{CurrencyPair, InstrumentAny},
    types::{Currency, Money, Price, Quantity},
};
use nautilus_trading::{
    nautilus_strategy,
    strategy::{Strategy, StrategyCore},
};
use rust_decimal::Decimal;
use serde_json::{Value, json};

const INSTRUMENT: &str = "BTCUSDT.SIM";
const BAR_TYPE: &str = "BTCUSDT.SIM-1-HOUR-LAST-EXTERNAL";
const HOUR_MS: u64 = 3_600_000;
const DATA_URL: &str =
    "https://data-api.binance.vision/api/v3/klines?symbol=BTCUSDT&interval=1h&limit=1000";

#[derive(Debug, Default)]
struct Audit {
    signals: Vec<Value>,
    fills: Vec<Value>,
    errors: Vec<String>,
}

#[derive(Debug)]
struct Trend {
    core: StrategyCore,
    closes: VecDeque<Decimal>,
    target: Option<(bool, u64)>,
    order_pending: bool,
    audit: Rc<RefCell<Audit>>,
    trade_start_ns: u64,
}

impl DataActor for Trend {
    fn on_start(&mut self) -> Result<()> {
        self.subscribe_bars(BarType::from_str(BAR_TYPE)?, None, None);
        self.subscribe_quotes(InstrumentId::from(INSTRUMENT), None, None);
        Ok(())
    }

    fn on_bar(&mut self, bar: &Bar) -> Result<()> {
        let close = bar.close.as_decimal();
        self.closes.push_back(close);
        if self.closes.len() > 20 {
            self.closes.pop_front();
        }
        if self.closes.len() == 20 {
            let mean = self.closes.iter().copied().sum::<Decimal>() / Decimal::from(20);
            self.target = Some((close > mean, bar.ts_init.as_u64()));
            self.audit.borrow_mut().signals.push(json!({
                "ts": bar.ts_init.as_u64(), "close": close.to_string(),
                "sma20": mean.to_string(), "hold": close > mean,
            }));
        }
        Ok(())
    }

    fn on_quote(&mut self, quote: &QuoteTick) -> Result<()> {
        if quote.ts_init.as_u64() < self.trade_start_ns {
            return Ok(());
        }
        let Some((hold, signal_ts)) = self.target else {
            return Ok(());
        };
        // Only a quote arriving after the completed bar can execute its signal.
        if quote.ts_init.as_u64() <= signal_ts || self.order_pending {
            return Ok(());
        }
        let position = self.portfolio().net_position(&quote.instrument_id);
        ensure!(position >= Decimal::ZERO, "unexpected short position");
        if hold == (position > Decimal::ZERO) {
            return Ok(());
        }
        let side = if hold {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };
        let order = self.order().market(
            quote.instrument_id,
            side,
            Quantity::from_str("0.00100").map_err(anyhow::Error::msg)?,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        self.order_pending = true;
        self.submit_order(order, None, None, None)?;
        Ok(())
    }
}

nautilus_strategy!(Trend, {
    fn on_order_filled(&mut self, event: &OrderFilled) {
        self.order_pending = false;
        self.audit.borrow_mut().fills.push(json!({
            "signal_ts": self.target.map(|(_, ts)| ts), "fill_ts": event.ts_event.as_u64(),
            "side": event.order_side.to_string(), "quantity": event.last_qty.to_string(),
            "price": event.last_px.to_string(), "commission": event.commission.map(|fee| fee.to_string()),
        }));
    }
    fn on_order_denied(&mut self, event: OrderDenied) {
        self.audit.borrow_mut().errors.push(format!("{event:?}"));
    }
    fn on_order_rejected(&mut self, event: OrderRejected) {
        self.audit.borrow_mut().errors.push(format!("{event:?}"));
    }
});

fn number(row: &[Value], index: usize) -> Result<Decimal> {
    Decimal::from_str(
        row.get(index)
            .and_then(Value::as_str)
            .context("missing decimal string")?,
    )
    .context("invalid decimal")
}

fn price(value: Decimal) -> Result<Price> {
    Ok(Price::from_decimal_dp(value.round_dp(2), 2)?)
}

fn timestamp(row: &[Value], index: usize) -> Result<u64> {
    row.get(index)
        .and_then(Value::as_u64)
        .context("missing timestamp")
}

fn parse_bar(row: &[Value]) -> Result<(u64, u64, Bar)> {
    ensure!(row.len() == 12, "unexpected kline field count");
    let open_ms = timestamp(row, 0)?;
    let close_ms = timestamp(row, 6)?;
    ensure!(
        close_ms.checked_add(1) == open_ms.checked_add(HOUR_MS),
        "not a 1h kline"
    );
    let open = number(row, 1)?;
    let high = number(row, 2)?;
    let low = number(row, 3)?;
    let close = number(row, 4)?;
    let volume = number(row, 5)?;
    ensure!(
        low > Decimal::ZERO
            && volume >= Decimal::ZERO
            && low <= open
            && low <= close
            && high >= open
            && high >= close,
        "invalid price or volume"
    );
    let ts = close_ms
        .checked_mul(1_000_000)
        .context("timestamp overflow")?;
    let bar = Bar::new_checked(
        BarType::from_str(BAR_TYPE)?,
        price(open)?,
        price(high)?,
        price(low)?,
        price(close)?,
        Quantity::from_str(&volume.to_string()).map_err(anyhow::Error::msg)?,
        ts.into(),
        ts.into(),
    )?;
    Ok((open_ms, close_ms, bar))
}

fn events(raw: &Value, now_ms: u64) -> Result<(Vec<Data>, usize)> {
    events_with_slippage(raw, now_ms, 5)
}

fn events_with_slippage(raw: &Value, now_ms: u64, slippage_bps: u32) -> Result<(Vec<Data>, usize)> {
    ensure!(slippage_bps < 10_000, "invalid slippage");
    let slippage = Decimal::from(slippage_bps) / Decimal::from(10_000);
    let rows = raw.as_array().context("klines response must be an array")?;
    let id = InstrumentId::from(INSTRUMENT);
    let mut data = Vec::new();
    let mut previous_open = None;
    let mut bars = 0;
    for value in rows {
        let row = value.as_array().context("kline must be an array")?;
        let (open_ms, close_ms, bar) = parse_bar(row)?;
        if let Some(previous) = previous_open {
            ensure!(
                open_ms == previous + HOUR_MS,
                "duplicate, missing or unordered kline"
            );
        }
        previous_open = Some(open_ms);
        let open = bar.open.as_decimal();
        // shortcut: OHLCV lacks historical quotes; use next open +/- 5bp until recorded quotes exist.
        let quote_ts = open_ms
            .checked_mul(1_000_000)
            .context("timestamp overflow")?;
        let bid = price(open * (Decimal::ONE - slippage))?;
        let ask = price(open * (Decimal::ONE + slippage))?;
        if open_ms <= now_ms {
            data.push(Data::Quote(QuoteTick::new_checked(
                id,
                bid,
                ask,
                Quantity::from_str("1.00000").map_err(anyhow::Error::msg)?,
                Quantity::from_str("1.00000").map_err(anyhow::Error::msg)?,
                quote_ts.into(),
                quote_ts.into(),
            )?));
        }
        if close_ms >= now_ms {
            continue;
        }
        data.push(Data::Bar(bar));
        bars += 1;
    }
    ensure!(bars >= 20, "need at least 20 completed bars");
    Ok((data, bars))
}

fn new_engine() -> Result<(BacktestEngine, Rc<RefCell<Audit>>)> {
    configured_engine(10, 0)
}

fn configured_engine(
    fee_bps: u32,
    trade_start_ns: u64,
) -> Result<(BacktestEngine, Rc<RefCell<Audit>>)> {
    ensure!(fee_bps < 10_000, "invalid fee");
    let fee = Decimal::from(fee_bps) / Decimal::from(10_000);
    let id = InstrumentId::from(INSTRUMENT);
    let instrument = CurrencyPair::builder()
        .instrument_id(id)
        .raw_symbol(Symbol::from("BTCUSDT"))
        .base_currency(Currency::BTC())
        .quote_currency(Currency::USDT())
        .price_precision(2)
        .size_precision(5)
        .price_increment(Price::from_str("0.01").map_err(anyhow::Error::msg)?)
        .size_increment(Quantity::from_str("0.00001").map_err(anyhow::Error::msg)?)
        .ts_event(0.into())
        .ts_init(0.into())
        .build()?;
    let mut engine = BacktestEngine::new(BacktestEngineConfig::default())?;
    engine.add_venue(
        SimulatedVenueConfig::builder()
            .venue(Venue::from("SIM"))
            .oms_type(OmsType::Netting)
            .account_type(AccountType::Cash)
            .book_type(BookType::L1_MBP)
            .starting_balances(vec![
                Money::from_str("1000 USDT").map_err(anyhow::Error::msg)?,
                Money::from_str("0 BTC").map_err(anyhow::Error::msg)?,
            ])
            .bar_execution(false)
            .fee_model(FeeModelAny::MakerTaker(MakerTakerFeeModel::new(fee, fee)).into())
            .build()?,
    )?;
    engine.add_instrument(&InstrumentAny::CurrencyPair(instrument))?;
    let audit = Rc::new(RefCell::new(Audit::default()));
    engine.add_strategy(Trend {
        core: StrategyCore::new(Default::default()),
        closes: VecDeque::new(),
        target: None,
        order_pending: false,
        audit: audit.clone(),
        trade_start_ns,
    })?;
    Ok((engine, audit))
}

fn replay(data: Vec<Data>) -> Result<(BacktestEngine, Rc<RefCell<Audit>>)> {
    let (mut engine, audit) = new_engine()?;
    engine.add_data(data, None, true, true)?;
    engine.run(None, None, None, false)?;
    ensure!(
        audit.borrow().errors.is_empty(),
        "execution errors: {:?}",
        audit.borrow().errors
    );
    Ok((engine, audit))
}

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--carry") {
        return carry::run();
    }
    if std::env::args().nth(1).as_deref() == Some("--historical") {
        return historical::run();
    }
    if std::env::args().nth(1).as_deref() == Some("--paper") {
        return paper::run();
    }
    fs::create_dir_all("artifacts")?;
    let snapshot: Value = if let Some(path) = std::env::args().nth(1) {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()?;
        let raw: Value = client.get(DATA_URL).send()?.error_for_status()?.json()?;
        let time: Value = client
            .get("https://data-api.binance.vision/api/v3/time")
            .send()?
            .error_for_status()?
            .json()?;
        json!({"url": DATA_URL, "time": time, "klines": raw})
    };
    let now = snapshot["time"]["serverTime"]
        .as_u64()
        .context("missing server time")?;
    let (data, bars) = events(&snapshot["klines"], now)?;
    fs::write(
        "artifacts/input.json",
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    let (engine, audit) = replay(data)?;
    let audit = audit.borrow();
    let result = engine.get_result();
    let report = json!({
        "scope": "native Rust framework spike, historical public-data replay, no live orders",
        "completed_bars": bars,
        "assumptions": {"starting_cash_usdt": "1000", "fixed_btc_quantity": "0.00100", "fee_each_side_bps": 10,
            "synthetic_quote_offset_each_side_bps": 5, "funding": "none (spot)",
            "note": "test amounts only; not a budget allocation, real fee quote or profitability claim"},
        "signals": audit.signals, "fills": audit.fills, "engine_result": result,
    });
    fs::write("artifacts/report.json", serde_json::to_vec_pretty(&report)?)?;
    fs::write(
        "artifacts/engine-state.json",
        engine.get_canonical_result()?.to_bytes()?,
    )?;
    println!(
        "SPIKE_OK bars={bars} signals={} fills={} orders={} positions={}",
        audit.signals.len(),
        audit.fills.len(),
        result.total_orders,
        result.total_positions
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        let prices: Vec<u64> = (0..20).map(|_| 100).chain([110, 90, 100]).collect();
        json!(
            prices
                .iter()
                .enumerate()
                .map(|(i, price)| {
                    let ts = 1_735_689_600_000 + i as u64 * HOUR_MS;
                    json!([
                        ts,
                        price.to_string(),
                        price.to_string(),
                        price.to_string(),
                        price.to_string(),
                        "1.00000",
                        ts + HOUR_MS - 1,
                        "0",
                        1,
                        "0",
                        "0",
                        "0"
                    ])
                })
                .collect::<Vec<_>>()
        )
    }

    #[test]
    fn native_orders_follow_closed_bar_on_next_quote_and_never_short() -> Result<()> {
        let (data, bars) = events(&fixture(), u64::MAX)?;
        assert_eq!(bars, 23);
        let (engine, audit) = replay(data)?;
        let audit = audit.borrow();
        assert_eq!(audit.fills.len(), 2);
        assert_eq!(audit.fills[0]["side"], "BUY");
        assert_eq!(audit.fills[1]["side"], "SELL");
        // Signal closes at 110, but next open is 90: prevents same-bar fill/look-ahead.
        assert_eq!(audit.fills[0]["price"], "90.04");
        for fill in &audit.fills {
            assert!(fill["fill_ts"].as_u64() > fill["signal_ts"].as_u64());
            let fee = Money::from_str(fill["commission"].as_str().context("missing fee")?)
                .map_err(anyhow::Error::msg)?;
            assert!(fee.as_decimal() > Decimal::ZERO);
        }
        assert_eq!(engine.get_result().total_orders, 2);
        Ok(())
    }

    #[test]
    fn excludes_unfinished_bars_and_rejects_gaps() -> Result<()> {
        let mut raw = fixture();
        let current_open = raw[22][0].as_u64().context("fixture time")?;
        let (_, bars) = events(&raw, current_open + 1)?;
        assert_eq!(bars, 22);
        raw.as_array_mut().context("fixture array")?.remove(5);
        assert!(events(&raw, u64::MAX).is_err());
        Ok(())
    }
}
