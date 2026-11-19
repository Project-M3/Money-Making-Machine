# BTCUSDT 小时历史档案：来源与导入约束

核实日期：2026-10-17（Asia/Shanghai）。范围为 Binance Spot 月归档；只调查协议及 HEAD 元数据，未读取 2025 年价格，未运行或评价策略。

## 下载与完整性

官方提供公开日／月归档，月文件命名模板如下；本轮完整枚举 `YYYY=2024,2025`、`MM=01..12`，不能按结果挑选月份。[官方归档说明](https://github.com/binance/binance-public-data#how-to-download-programatically)

```text
https://data.binance.vision/data/spot/monthly/klines/BTCUSDT/1h/BTCUSDT-1h-YYYY-MM.zip
https://data.binance.vision/data/spot/monthly/klines/BTCUSDT/1h/BTCUSDT-1h-YYYY-MM.zip.CHECKSUM
```

同目录 `.CHECKSUM` 为 ZIP 的 SHA-256 校验依据；macOS 可用 `shasum -a 256 -c 文件.zip.CHECKSUM`。官方明确归档可能被修订，并提供替换前后校验值的更新记录。[校验与修订说明](https://github.com/binance/binance-public-data#checksum)

本轮导入要求（项目约定）：保留原 ZIP、CHECKSUM、获取时间、实际 SHA-256、解压 CSV 的 SHA-256 与转换版本；失败时不继续回测。以后远端内容变化应生成新数据版本，不能覆盖本次证据。ETag 或 Last-Modified 不能替代 SHA-256。

## 字段与时间

档案 K 线来自 `/api/v3/klines`；12 列按序为：开盘时间、开高低收、基础资产成交量、收盘时间、计价资产成交量、笔数、主动买入基础量、主动买入计价量、忽略字段。开盘时间唯一标识 K 线；API 的默认时区为 UTC。[官方字段定义](https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/rest-api/market#klinecandlestick-data)

Spot 档案自 **2025-01-01 起使用微秒**，不能沿用 2024 年毫秒解析。[官方单位变更说明](https://github.com/binance/binance-public-data#spot)

本轮转换规则（项目约定，由上述单位推导）：

| 档案年份 | 原始时间 | 转纳秒 | 转毫秒（若接口需要） |
| --- | --- | --- | --- |
| 2024 | 毫秒 | `raw * 1_000_000` | `raw` |
| 2025 | 微秒 | `raw * 1_000` | `raw / 1_000`，整数向下取整 |

2025 小时线收盘原始时间为下一开盘微秒减一，不能要求它被 1000 整除。纳秒表示保留精度；转毫秒会舍弃末尾 999 微秒。解析应校验单位对应的年份、UTC 小时对齐、收盘边界、月份范围、严格递增、跨月连续、无重复，以及价格／成交量和 OHLC 关系。完整年度小时数应为 2024 年 8784、2025 年 8760；缺失时报告缺口，不填造价格。这些检查是本轮导入要求，官方不保证数据无误或完整。

K 线没有当时的 bid/ask、盘口深度或订单执行结果。因此“下一根开盘成交”仍是回测模型假设，不能描述成已观察到的可成交报价。[K 线与订单簿字段对照](https://developers.binance.com/en/docs/catalog/core-trading-spot-trading/api/rest-api/market)

## 当前使用条款

当前数据专用条款为 **v1.0，2026-08-26 更新**，明确纳入 Binance 使用条款；不能用仓库 README 的 MIT 标签替代数据许可。数据许可为 CC BY-NC-SA 4.0，明示允许纯个人、非生产的算法历史回测研究；商业使用需要另行书面许可。禁止将其用于自营实盘执行、商业自动下单、收费信号或商业金融产品；再分发派生作品须署名并遵守相同许可，访问须遵守限流规则。数据也不承诺准确、连续、完整或盈利。[数据专用条款，§1、3–4、7、10](https://github.com/binance/binance-public-data/blob/master/TERMS_AND_CONDITIONS.md)

本轮只做本地个人非生产历史研究，不分发原行情；可以积累通用 Rust 导入和评估能力，但不能据此认定允许把数据或派生结果接入 Alan 商业产品。本文不判断开户、KYC、地区实盘准入，也不把公开下载等同于交易资格。

## HEAD 抽查证据

以下 ZIP 及对应 CHECKSUM 均在核实日返回 HTTP 200，仅请求 HEAD，没有读取价格。未逐一验证剩余月份；正式导入必须校验全部文件。

| 月份 | ZIP 字节数 | ZIP Last-Modified（UTC） |
| --- | ---: | --- |
| 2024-01 | 43482 | 2024-02-05 12:06:34 |
| 2024-12 | 44271 | 2025-01-08 00:50:07 |
| 2025-01 | 44655 | 2025-02-08 12:19:33 |
| 2025-12 | 43894 | 2026-01-05 10:09:01 |

四份 CHECKSUM 的 Content-Length 均为 88。文档链接指向核实当日的官方页面，尚未固定其 Git 提交；正式实验 manifest 应保存获取当日文档摘要或哈希以保留条款与格式依据。
