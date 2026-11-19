use crate::model::{OhlcBar, PriceHistorySnapshot, PriceSnapshot};
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use rust_decimal::Decimal;
use serde_json::Value;
use std::{fs, path::Path, str::FromStr, time::Duration};

pub fn market_http_client() -> Result<Client> {
    Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")
        .timeout(Duration::from_secs(12))
        .build()
        .context("failed to build market HTTP client")
}

pub fn safe_symbol_filename(symbol: &str) -> String {
    symbol
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn fetch_price_snapshot(
    client: &Client,
    symbol: &str,
    cache_path: &Path,
    fetched_at_ms: u64,
) -> Result<PriceSnapshot> {
    let encoded = symbol.replace('^', "%5E");
    let url = format!("https://query1.finance.yahoo.com/v8/finance/chart/{encoded}?interval=1d&range=5d");
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("fetch price for {symbol}"))?;
    ensure!(
        resp.status().is_success(),
        "Yahoo Finance returned HTTP {} for {symbol}",
        resp.status()
    );
    let body: Value = resp
        .json()
        .with_context(|| format!("decode Yahoo Finance JSON for {symbol}"))?;

    let snapshot = parse_yahoo_chart_json(&body, symbol, &url, fetched_at_ms)?;

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = cache_path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&snapshot)?)?;
    fs::rename(&tmp, cache_path)?;

    Ok(snapshot)
}

pub fn parse_yahoo_chart_json(
    body: &Value,
    symbol: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<PriceSnapshot> {
    let meta = body["chart"]["result"]
        .as_array()
        .and_then(|arr| arr.first())
        .map(|item| &item["meta"])
        .with_context(|| format!("Yahoo response missing chart.result[0].meta for {symbol}"))?;

    let raw_price = meta["regularMarketPrice"]
        .as_f64()
        .with_context(|| format!("missing regularMarketPrice for {symbol}"))?;
    let price = Decimal::from_str(&raw_price.to_string())
        .with_context(|| format!("invalid regularMarketPrice for {symbol}: {raw_price}"))?;
    ensure!(price > Decimal::ZERO, "non-positive price for {symbol}");

    let previous_close = meta["chartPreviousClose"]
        .as_f64()
        .or_else(|| meta["previousClose"].as_f64())
        .and_then(|v| Decimal::from_str(&v.to_string()).ok());

    let currency = meta["currency"].as_str().unwrap_or("USD").to_owned();
    let exchange = meta["fullExchangeName"]
        .as_str()
        .or_else(|| meta["exchangeName"].as_str())
        .unwrap_or("UNKNOWN")
        .to_owned();
    let market_time_ms = meta["regularMarketTime"].as_u64().map(|s| s * 1000);

    Ok(PriceSnapshot {
        schema_version: 1,
        symbol: symbol.to_owned(),
        price,
        previous_close,
        currency,
        exchange,
        market_time_ms,
        fetched_at_ms,
        source_url: source_url.to_owned(),
    })
}

pub fn fetch_price_history(
    client: &Client,
    symbol: &str,
    interval: &str,
    range: &str,
    cache_path: &Path,
    fetched_at_ms: u64,
) -> Result<PriceHistorySnapshot> {
    let encoded = symbol.replace('^', "%5E");
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{encoded}?interval={interval}&range={range}"
    );
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("fetch price history ({interval}/{range}) for {symbol}"))?;
    ensure!(
        resp.status().is_success(),
        "Yahoo Finance history returned HTTP {} for {symbol}",
        resp.status()
    );
    let body: Value = resp
        .json()
        .with_context(|| format!("decode Yahoo Finance history JSON for {symbol}"))?;

    let snapshot = parse_yahoo_history_json(&body, symbol, interval, range, &url, fetched_at_ms)?;

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = cache_path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&snapshot)?)?;
    fs::rename(&tmp, cache_path)?;

    Ok(snapshot)
}

pub fn parse_yahoo_history_json(
    body: &Value,
    symbol: &str,
    interval: &str,
    range: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<PriceHistorySnapshot> {
    let result = body["chart"]["result"]
        .as_array()
        .and_then(|arr| arr.first())
        .with_context(|| format!("Yahoo history missing chart.result[0] for {symbol}"))?;

    let currency = result["meta"]["currency"]
        .as_str()
        .unwrap_or("USD")
        .to_owned();
    let timestamps = result["timestamp"]
        .as_array()
        .with_context(|| format!("Yahoo history missing timestamp array for {symbol}"))?;
    let quote = result["indicators"]["quote"]
        .as_array()
        .and_then(|arr| arr.first())
        .with_context(|| format!("Yahoo history missing indicators.quote[0] for {symbol}"))?;

    let opens = quote["open"].as_array();
    let highs = quote["high"].as_array();
    let lows = quote["low"].as_array();
    let closes = quote["close"].as_array();
    let volumes = quote["volume"].as_array();

    let mut bars = Vec::with_capacity(timestamps.len());
    for (idx, ts_val) in timestamps.iter().enumerate() {
        let Some(ts_sec) = ts_val.as_u64() else {
            continue;
        };
        let parse_opt = |arr: Option<&Vec<Value>>| -> Option<Decimal> {
            arr.and_then(|a| a.get(idx))
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v > 0.0)
                .and_then(|v| Decimal::from_str(&v.to_string()).ok())
        };
        let (Some(open), Some(high), Some(low), Some(close)) = (
            parse_opt(opens),
            parse_opt(highs),
            parse_opt(lows),
            parse_opt(closes),
        ) else {
            continue;
        };
        let volume = volumes
            .and_then(|a| a.get(idx))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        bars.push(OhlcBar {
            ts_ms: ts_sec * 1000,
            open,
            high: high.max(open).max(close),
            low: low.min(open).min(close),
            close,
            volume,
        });
    }

    ensure!(
        !bars.is_empty(),
        "Yahoo history contained zero valid OHLC bars for {symbol}"
    );

    Ok(PriceHistorySnapshot {
        schema_version: 1,
        symbol: symbol.to_owned(),
        currency,
        interval: interval.to_owned(),
        range: range.to_owned(),
        fetched_at_ms,
        source_url: source_url.to_owned(),
        bars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yahoo_chart_meta_and_preserves_decimal_price() {
        let sample = serde_json::json!({
            "chart": {
                "result": [{
                    "meta": {
                        "currency": "EUR",
                        "symbol": "DSY.PA",
                        "exchangeName": "PAR",
                        "fullExchangeName": "Paris",
                        "regularMarketTime": 1795019709u64,
                        "regularMarketPrice": 32.45,
                        "chartPreviousClose": 31.90
                    }
                }]
            }
        });
        let snap = parse_yahoo_chart_json(&sample, "DSY.PA", "https://yahoo.test", 999).unwrap();
        assert_eq!(snap.currency, "EUR");
        assert_eq!(snap.exchange, "Paris");
        assert_eq!(snap.price.to_string(), "32.45");
        assert_eq!(snap.previous_close.unwrap().to_string(), "31.9");
        assert_eq!(snap.market_time_ms, Some(1_795_019_709_000));
    }

    #[test]
    fn parses_yahoo_ohlc_history_and_skips_null_bars() {
        let sample = serde_json::json!({
            "chart": {
                "result": [{
                    "meta": { "currency": "USD" },
                    "timestamp": [1_700_000_000u64, 1_700_604_800u64, 1_701_209_600u64],
                    "indicators": {
                        "quote": [{
                            "open": [100.5, null, 102.0],
                            "high": [105.0, null, 108.0],
                            "low": [99.0, null, 101.0],
                            "close": [103.0, null, 106.5],
                            "volume": [1000u64, 0u64, 2500u64]
                        }]
                    }
                }]
            }
        });
        let hist = parse_yahoo_history_json(
            &sample,
            "AAPL",
            "1wk",
            "5y",
            "https://yahoo.test",
            777,
        )
        .unwrap();
        assert_eq!(hist.bars.len(), 2);
        assert_eq!(hist.bars[0].open.to_string(), "100.5");
        assert_eq!(hist.bars[1].close.to_string(), "106.5");
        assert_eq!(hist.bars[1].volume, 2500);
    }
}
