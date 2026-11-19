# BTCUSDT 公开现货数据：首轮小范围核验

核验日期：2026-10-15。为「选择首个可检验的量化机会」提供候选数据源证据；不代表已决定市场或已验证盈利。

## 官方文档证据

- Binance 明确说明 market-data-only 域名无需认证或 API key。REST 为 `https://data-api.binance.vision`，允许 `klines`、`ticker/bookTicker`、`trades` 等公开接口；WebSocket 为 `wss://data-stream.binance.vision`，不提供账户数据流。[官方说明](https://developers.binance.com/en/docs/products/spot/faqs/market_data_only)
- `GET /api/v3/klines?symbol=BTCUSDT&interval=1h` 支持一小时历史 K 线，按开盘时间唯一识别，单次默认 500、最多 1000 根，请求权重为 2；可用 `startTime` / `endTime` 分页。默认时区 UTC，开始和结束参数始终按 UTC 解释。[REST 市场数据](https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/rest-api/market)
- `btcusdt@bookTicker` 实时推送最佳买卖价和数量；`btcusdt@kline_1h` 含 `k.x` 收盘标记，可区分未完成与完成的 K 线。[WebSocket 市场流](https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/ws-streams/~)

## 本机公开读取证据

仅执行以下两次无认证 GET；均返回 HTTP 200，没有账户或 API key：

```text
https://data-api.binance.vision/api/v3/klines?symbol=BTCUSDT&interval=1h&limit=2
https://data-api.binance.vision/api/v3/ticker/bookTicker?symbol=BTCUSDT
```

第一项返回两根 OHLCV，首根开盘时间为 `1792044000000`；第二项返回 `symbol=BTCUSDT` 及 `bidPrice`、`bidQty`、`askPrice`、`askQty`。这些只证明本次 REST 可达与基本响应形状，不证明历史覆盖完整、长期连接稳定或 WebSocket 实测通过。

## 对首周闭环的含义

可以将此作为无需开户的候选：REST 获取历史 K 线并定期读取最新完成 K 线；信号产生后，读取新的买卖报价用于本地模拟。是否采用 WebSocket 留到实现时按需要决定，首周无须先建完整盘口。

模拟规则应明确：完成 K 线才出信号，之后才读取模拟成交报价；买入用卖价、卖出用买价，并单独计手续费与滑点。历史 OHLCV 无法还原当时盘口，历史成交模型只能是显式假设，不能冒充真实报价成交。以上为实验设计建议，不是交易所提供的成交保证。

未核验完整使用条款、地区适用性、长期限流策略或 WebSocket 运维限制；实际采用前补查必要条款与接口限制。公开读取成功不等于实盘账户准入，也不保证未来持续可用。
