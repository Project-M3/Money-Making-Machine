use crate::{
    model::{OhlcBar, PriceHistorySnapshot, PriceSnapshot},
    store,
};
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};

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

pub(crate) fn trade_date_from_symbol_and_market_time(
    symbol: &str,
    market_time_ms: Option<u64>,
) -> Option<String> {
    let ms = market_time_ms?;
    let offset_secs = if symbol.ends_with(".HK") {
        8 * 3600
    } else if symbol.ends_with(".T") {
        9 * 3600
    } else if symbol.ends_with(".PA") || symbol.ends_with(".DE") {
        3600
    } else if symbol.ends_with(".L") {
        0
    } else {
        // US/CA exchanges (NYSE/Nasdaq/CBOE/TSX): map session timestamp to ET date.
        -5 * 3600
    };
    Some(civil_iso_from_unix_secs((ms / 1000) as i64 + offset_secs))
}

pub fn trade_date_from_snapshot(snap: &PriceSnapshot) -> Option<String> {
    trade_date_from_symbol_and_market_time(&snap.symbol, snap.market_time_ms)
}

pub fn civil_iso_from_unix_secs(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let yr = y + i64::from(m <= 2);
    format!("{yr:04}-{m:02}-{d:02}")
}

fn parse_raw_decimal(raw: &RawValue) -> Option<Decimal> {
    crate::model::parse_raw_json_decimal_token(raw.get())
        .ok()
        .flatten()
}

pub(crate) const MAX_QUOTE_SESSION_AGE_MS: u64 = 10 * 86_400_000;
const MAX_QUOTE_FUTURE_SKEW_MS: u64 = 36 * 3_600_000;

pub fn validate_quote_session(snap: &PriceSnapshot, as_of_ms: u64) -> Result<()> {
    ensure!(
        snap.price > Decimal::ZERO,
        "non-positive price for {}",
        snap.symbol
    );
    let mkt_ms = snap
        .market_time_ms
        .filter(|ms| *ms > 0)
        .with_context(|| format!("missing regularMarketTime for {}", snap.symbol))?;
    ensure!(
        trade_date_from_snapshot(snap).is_some(),
        "unable to derive market trade date for {}",
        snap.symbol
    );
    ensure!(
        mkt_ms <= as_of_ms.saturating_add(MAX_QUOTE_FUTURE_SKEW_MS),
        "future-dated quote market_time_ms ({mkt_ms}) > as_of ({as_of_ms}) for {}",
        snap.symbol
    );
    ensure!(
        as_of_ms.saturating_sub(mkt_ms) <= MAX_QUOTE_SESSION_AGE_MS,
        "stale quote market session ({mkt_ms}) older than 10 days vs as_of ({as_of_ms}) for {}",
        snap.symbol
    );
    Ok(())
}

pub fn fetch_price_snapshot(
    client: &Client,
    symbol: &str,
    cache_path: &Path,
    fetched_at_ms: u64,
) -> Result<PriceSnapshot> {
    let (snap, _) = fetch_price_snapshot_with_status(client, symbol, cache_path, fetched_at_ms)?;
    Ok(snap)
}

pub(crate) fn fetch_price_snapshot_with_status(
    client: &Client,
    symbol: &str,
    cache_path: &Path,
    fetched_at_ms: u64,
) -> Result<(PriceSnapshot, bool)> {
    let encoded = symbol.replace('^', "%5E");
    let url =
        format!("https://query1.finance.yahoo.com/v8/finance/chart/{encoded}?interval=1d&range=5d");
    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("fetch price for {symbol}"))?;
    ensure!(
        resp.status().is_success(),
        "Yahoo Finance returned HTTP {} for {symbol}",
        resp.status()
    );
    let bytes = resp
        .bytes()
        .with_context(|| format!("read Yahoo Finance response bytes for {symbol}"))?;

    let snapshot = parse_yahoo_chart_bytes(&bytes, symbol, &url, fetched_at_ms)?;
    validate_quote_session(&snapshot, fetched_at_ms)?;
    persist_price_snapshot_if_newer_status(cache_path, &snapshot, fetched_at_ms)
}

fn quote_lock_path(cache_path: &Path) -> PathBuf {
    let name = cache_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("quote.json");
    cache_path.with_file_name(format!(".{name}.lock"))
}

fn acquire_quote_file_lock(cache_path: &Path) -> Result<File> {
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = quote_lock_path(cache_path);
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("open quote lock {}", lock_path.display()))?;
    lock_file
        .lock()
        .with_context(|| format!("acquire quote lock {}", lock_path.display()))?;
    Ok(lock_file)
}

#[cfg(test)]
pub(crate) fn persist_price_snapshot_if_newer(
    cache_path: &Path,
    snapshot: &PriceSnapshot,
    now_ms: u64,
) -> Result<PriceSnapshot> {
    let (snap, _) = persist_price_snapshot_if_newer_status(cache_path, snapshot, now_ms)?;
    Ok(snap)
}

pub(crate) fn persist_price_snapshot_if_newer_status(
    cache_path: &Path,
    snapshot: &PriceSnapshot,
    now_ms: u64,
) -> Result<(PriceSnapshot, bool)> {
    let _lock = acquire_quote_file_lock(cache_path)?;
    if cache_path.exists()
        && let Ok(existing) = store::read_json::<PriceSnapshot>(cache_path)
        && existing.symbol.eq_ignore_ascii_case(&snapshot.symbol)
        && validate_quote_session(&existing, now_ms).is_ok()
    {
        let incoming_key = (
            snapshot.market_time_ms.is_some(),
            snapshot.market_time_ms.unwrap_or(0),
            snapshot.fetched_at_ms,
        );
        let existing_key = (
            existing.market_time_ms.is_some(),
            existing.market_time_ms.unwrap_or(0),
            existing.fetched_at_ms,
        );
        if incoming_key < existing_key {
            return Ok((existing, false));
        }
    }

    let tmp = store::atomic_temp_path(cache_path);
    let payload = serde_json::to_vec_pretty(snapshot)?;
    if let Err(err) = fs::write(&tmp, payload).and_then(|_| fs::rename(&tmp, cache_path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }

    Ok((snapshot.clone(), true))
}

#[allow(dead_code)]
pub fn parse_yahoo_chart_json(
    body: &Value,
    symbol: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<PriceSnapshot> {
    let bytes = serde_json::to_vec(body)?;
    parse_yahoo_chart_bytes(&bytes, symbol, source_url, fetched_at_ms)
}

pub fn parse_yahoo_chart_bytes(
    bytes: &[u8],
    symbol: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<PriceSnapshot> {
    #[derive(Deserialize)]
    struct ChartWrapper<'a> {
        #[serde(borrow)]
        chart: ChartBody<'a>,
    }
    #[derive(Deserialize)]
    struct ChartBody<'a> {
        #[serde(borrow, default)]
        result: Option<Vec<ChartResult<'a>>>,
    }
    #[derive(Deserialize)]
    struct ChartResult<'a> {
        #[serde(borrow)]
        meta: ChartMeta<'a>,
    }
    #[derive(Deserialize)]
    struct ChartMeta<'a> {
        #[serde(borrow, rename = "regularMarketPrice")]
        regular_market_price: Option<&'a RawValue>,
        #[serde(borrow, rename = "chartPreviousClose")]
        chart_previous_close: Option<&'a RawValue>,
        #[serde(borrow, rename = "previousClose")]
        previous_close: Option<&'a RawValue>,
        currency: Option<String>,
        #[serde(rename = "fullExchangeName")]
        full_exchange_name: Option<String>,
        #[serde(rename = "exchangeName")]
        exchange_name: Option<String>,
        #[serde(rename = "regularMarketTime")]
        regular_market_time: Option<u64>,
    }

    let parsed: ChartWrapper<'_> = serde_json::from_slice(bytes)
        .with_context(|| format!("decode Yahoo Finance JSON for {symbol}"))?;
    let meta = parsed
        .chart
        .result
        .as_ref()
        .and_then(|arr| arr.first())
        .map(|item| &item.meta)
        .with_context(|| format!("Yahoo response missing chart.result[0].meta for {symbol}"))?;

    let raw_price = meta
        .regular_market_price
        .with_context(|| format!("missing regularMarketPrice for {symbol}"))?;
    let price = parse_raw_decimal(raw_price).with_context(|| {
        format!(
            "invalid regularMarketPrice for {symbol}: {}",
            raw_price.get()
        )
    })?;
    ensure!(price > Decimal::ZERO, "non-positive price for {symbol}");

    let previous_close = meta
        .chart_previous_close
        .and_then(parse_raw_decimal)
        .or_else(|| meta.previous_close.and_then(parse_raw_decimal));

    let currency = meta.currency.clone().unwrap_or_else(|| "USD".to_owned());
    let exchange = meta
        .full_exchange_name
        .clone()
        .or_else(|| meta.exchange_name.clone())
        .unwrap_or_else(|| "UNKNOWN".to_owned());
    let market_time_ms = meta.regular_market_time.map(|s| s * 1000);

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
    let bytes = resp
        .bytes()
        .with_context(|| format!("read Yahoo Finance history bytes for {symbol}"))?;

    let snapshot = parse_yahoo_history_bytes(&bytes, symbol, interval, range, &url, fetched_at_ms)?;

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = store::atomic_temp_path(cache_path);
    let payload = serde_json::to_vec_pretty(&snapshot)?;
    if let Err(err) = fs::write(&tmp, payload).and_then(|_| fs::rename(&tmp, cache_path)) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }

    Ok(snapshot)
}

#[allow(dead_code)]
pub fn parse_yahoo_history_json(
    body: &Value,
    symbol: &str,
    interval: &str,
    range: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<PriceHistorySnapshot> {
    let bytes = serde_json::to_vec(body)?;
    parse_yahoo_history_bytes(&bytes, symbol, interval, range, source_url, fetched_at_ms)
}

pub fn parse_yahoo_history_bytes(
    bytes: &[u8],
    symbol: &str,
    interval: &str,
    range: &str,
    source_url: &str,
    fetched_at_ms: u64,
) -> Result<PriceHistorySnapshot> {
    #[derive(Deserialize)]
    struct HistoryWrapper<'a> {
        #[serde(borrow)]
        chart: HistoryBody<'a>,
    }
    #[derive(Deserialize)]
    struct HistoryBody<'a> {
        #[serde(borrow, default)]
        result: Option<Vec<HistoryResult<'a>>>,
    }
    #[derive(Deserialize)]
    struct HistoryResult<'a> {
        meta: HistoryMeta,
        #[serde(default)]
        timestamp: Option<Vec<Option<u64>>>,
        #[serde(borrow)]
        indicators: HistoryIndicators<'a>,
    }
    #[derive(Deserialize)]
    struct HistoryMeta {
        currency: Option<String>,
    }
    #[derive(Deserialize)]
    struct HistoryIndicators<'a> {
        #[serde(borrow, default)]
        quote: Option<Vec<HistoryQuote<'a>>>,
    }
    #[derive(Deserialize)]
    struct HistoryQuote<'a> {
        #[serde(borrow, default)]
        open: Option<Vec<Option<&'a RawValue>>>,
        #[serde(borrow, default)]
        high: Option<Vec<Option<&'a RawValue>>>,
        #[serde(borrow, default)]
        low: Option<Vec<Option<&'a RawValue>>>,
        #[serde(borrow, default)]
        close: Option<Vec<Option<&'a RawValue>>>,
        #[serde(default)]
        volume: Option<Vec<Option<u64>>>,
    }

    let parsed: HistoryWrapper<'_> = serde_json::from_slice(bytes)
        .with_context(|| format!("decode Yahoo Finance history JSON for {symbol}"))?;
    let result = parsed
        .chart
        .result
        .as_ref()
        .and_then(|arr| arr.first())
        .with_context(|| format!("Yahoo history missing chart.result[0] for {symbol}"))?;

    let currency = result
        .meta
        .currency
        .clone()
        .unwrap_or_else(|| "USD".to_owned());
    let timestamps = result
        .timestamp
        .as_ref()
        .with_context(|| format!("Yahoo history missing timestamp array for {symbol}"))?;
    let quote = result
        .indicators
        .quote
        .as_ref()
        .and_then(|arr| arr.first())
        .with_context(|| format!("Yahoo history missing indicators.quote[0] for {symbol}"))?;

    let parse_opt = |arr: Option<&Vec<Option<&RawValue>>>, idx: usize| -> Option<Decimal> {
        arr.and_then(|a| a.get(idx))
            .and_then(|opt| *opt)
            .and_then(parse_raw_decimal)
            .filter(|v| *v > Decimal::ZERO)
    };

    let mut bars = Vec::with_capacity(timestamps.len());
    for (idx, ts_opt) in timestamps.iter().enumerate() {
        let Some(ts_sec) = *ts_opt else {
            continue;
        };
        let (Some(open), Some(high), Some(low), Some(close)) = (
            parse_opt(quote.open.as_ref(), idx),
            parse_opt(quote.high.as_ref(), idx),
            parse_opt(quote.low.as_ref(), idx),
            parse_opt(quote.close.as_ref(), idx),
        ) else {
            continue;
        };
        let volume = quote
            .volume
            .as_ref()
            .and_then(|a| a.get(idx))
            .and_then(|opt| *opt)
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
    fn parses_yahoo_bytes_without_f64_precision_loss_and_derives_trade_date() {
        // 19-digit price that f64 cannot represent without rounding error.
        let raw = br#"{
            "chart": {
                "result": [{
                    "meta": {
                        "currency": "USD",
                        "exchangeName": "NMS",
                        "regularMarketTime": 1795122000,
                        "regularMarketPrice": 12345678901234.56789,
                        "chartPreviousClose": 12345678901234.12345
                    }
                }]
            }
        }"#;
        let snap = parse_yahoo_chart_bytes(raw, "AAPL", "https://yahoo.test", 1234).unwrap();
        assert_eq!(snap.price.to_string(), "12345678901234.56789");
        assert_eq!(
            snap.previous_close.unwrap().to_string(),
            "12345678901234.12345"
        );
        // 1795122000 = 2026-11-19 21:00:00 UTC
        assert_eq!(
            trade_date_from_snapshot(&snap).as_deref(),
            Some("2026-11-19")
        );
        let without_mkt_time = PriceSnapshot {
            market_time_ms: None,
            ..snap
        };
        assert_eq!(trade_date_from_snapshot(&without_mkt_time), None);
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
        let hist =
            parse_yahoo_history_json(&sample, "AAPL", "1wk", "5y", "https://yahoo.test", 777)
                .unwrap();
        assert_eq!(hist.bars.len(), 2);
        assert_eq!(hist.bars[0].open.to_string(), "100.5");
        assert_eq!(hist.bars[1].close.to_string(), "106.5");
        assert_eq!(hist.bars[1].volume, 2500);
    }

    #[test]
    fn validate_quote_session_enforces_present_timely_market_time() {
        let now_ms = 1_795_420_000_000u64;
        let fresh = PriceSnapshot {
            schema_version: 1,
            symbol: "AAPL".into(),
            price: Decimal::from(225),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(now_ms - 86_400_000),
            fetched_at_ms: now_ms,
            source_url: "https://yahoo.test".into(),
        };
        assert!(validate_quote_session(&fresh, now_ms).is_ok());

        let missing_time = PriceSnapshot {
            market_time_ms: None,
            ..fresh.clone()
        };
        assert!(validate_quote_session(&missing_time, now_ms).is_err());

        let stale = PriceSnapshot {
            market_time_ms: Some(now_ms - 15 * 86_400_000),
            ..fresh.clone()
        };
        assert!(validate_quote_session(&stale, now_ms).is_err());

        let future = PriceSnapshot {
            market_time_ms: Some(now_ms + 72 * 3_600_000),
            ..fresh
        };
        assert!(validate_quote_session(&future, now_ms).is_err());
    }

    #[test]
    fn persist_price_snapshot_if_newer_rejects_older_and_undated_observations() {
        let dir = std::env::temp_dir().join(format!("m3-market-ts-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let cache_path = dir.join("AAPL.json");
        let now_ms = 1_795_420_000_000u64;

        let newest = PriceSnapshot {
            schema_version: 1,
            symbol: "AAPL".into(),
            price: Decimal::from(225),
            previous_close: None,
            currency: "USD".into(),
            exchange: "NMS".into(),
            market_time_ms: Some(now_ms - 3_600_000),
            fetched_at_ms: now_ms,
            source_url: "https://yahoo.test/new".into(),
        };
        let saved = persist_price_snapshot_if_newer(&cache_path, &newest, now_ms).unwrap();
        assert_eq!(saved.price, Decimal::from(225));

        // Older market_time fetched later must NOT overwrite newer cached snapshot.
        let older_mkt = PriceSnapshot {
            price: Decimal::from(210),
            market_time_ms: Some(now_ms - 7_200_000),
            fetched_at_ms: now_ms + 10_000,
            source_url: "https://yahoo.test/old".into(),
            ..newest.clone()
        };
        let retained = persist_price_snapshot_if_newer(&cache_path, &older_mkt, now_ms).unwrap();
        assert_eq!(retained.price, Decimal::from(225));
        assert_eq!(
            store::read_json::<PriceSnapshot>(&cache_path)
                .unwrap()
                .price,
            Decimal::from(225)
        );

        // Undated quote must NOT overwrite timestamped valid cache either.
        let undated = PriceSnapshot {
            price: Decimal::from(200),
            market_time_ms: None,
            fetched_at_ms: now_ms + 20_000,
            ..newest.clone()
        };
        assert_eq!(
            persist_price_snapshot_if_newer(&cache_path, &undated, now_ms)
                .unwrap()
                .price,
            Decimal::from(225)
        );

        // Same market_time with older fetched_at_ms must NOT regress cache.
        let older_fetch = PriceSnapshot {
            price: Decimal::from(220),
            market_time_ms: newest.market_time_ms,
            fetched_at_ms: now_ms - 5_000,
            ..newest.clone()
        };
        assert_eq!(
            persist_price_snapshot_if_newer(&cache_path, &older_fetch, now_ms)
                .unwrap()
                .price,
            Decimal::from(225)
        );

        // Truly newer market_time replaces cache cleanly.
        let fresher = PriceSnapshot {
            price: Decimal::from(230),
            market_time_ms: Some(now_ms - 1_800_000),
            fetched_at_ms: now_ms + 5_000,
            ..newest
        };
        assert_eq!(
            persist_price_snapshot_if_newer(&cache_path, &fresher, now_ms)
                .unwrap()
                .price,
            Decimal::from(230)
        );
        assert_eq!(
            store::read_json::<PriceSnapshot>(&cache_path)
                .unwrap()
                .price,
            Decimal::from(230)
        );

        // Concurrent writers must serialize compare-and-replace and report acceptance accurately.
        let stale_snap = PriceSnapshot {
            price: Decimal::from(199),
            market_time_ms: Some(now_ms - 7_200_000),
            fetched_at_ms: now_ms + 50_000,
            ..fresher.clone()
        };
        let (returned, accepted) =
            persist_price_snapshot_if_newer_status(&cache_path, &stale_snap, now_ms).unwrap();
        assert!(!accepted);
        assert_eq!(returned.price, Decimal::from(230));

        std::thread::scope(|s| {
            for offset in 0..8u64 {
                let path = &cache_path;
                let base = &fresher;
                s.spawn(move || {
                    let snap = PriceSnapshot {
                        price: Decimal::from(240 + offset),
                        market_time_ms: Some(now_ms - 900_000 + offset * 1_000),
                        fetched_at_ms: now_ms + 10_000 + offset,
                        ..base.clone()
                    };
                    let _ = persist_price_snapshot_if_newer_status(path, &snap, now_ms).unwrap();
                });
            }
        });
        assert_eq!(
            store::read_json::<PriceSnapshot>(&cache_path)
                .unwrap()
                .price,
            Decimal::from(247)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn round11_parse_yahoo_chart_bytes_normalizes_scientific_exponents_without_silent_zero() {
        let valid_body = br#"{"chart":{"result":[{"meta":{"symbol":"TINY","currency":"USD","exchangeName":"NMS","regularMarketPrice":0.00000000000000000000000000001e1,"regularMarketTime":1795400000}}]}}"#;
        let snap = parse_yahoo_chart_bytes(valid_body, "TINY", "https://yahoo.test", 1).unwrap();
        assert_eq!(snap.price, Decimal::new(1, 28));

        let excess_precision = br#"{"chart":{"result":[{"meta":{"symbol":"TINY","currency":"USD","exchangeName":"NMS","regularMarketPrice":1e-29,"regularMarketTime":1795400000}}]}}"#;
        assert!(
            parse_yahoo_chart_bytes(excess_precision, "TINY", "https://yahoo.test", 1).is_err()
        );

        let overflow = br#"{"chart":{"result":[{"meta":{"symbol":"TINY","currency":"USD","exchangeName":"NMS","regularMarketPrice":1e30,"regularMarketTime":1795400000}}]}}"#;
        assert!(parse_yahoo_chart_bytes(overflow, "TINY", "https://yahoo.test", 1).is_err());
    }
}
