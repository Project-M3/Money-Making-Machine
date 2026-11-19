# BTCUSDT 现货／永续资金费测算：公开数据与口径核验

核验日期：2026-10-19。仅查询第一方文档、数据路径 HEAD 与当前交易规则接口；本资料调查没有查看历史资金费数值、下载收益样本或运行策略。本文件服务于个人非生产研究，不构成实盘或账户准入判断。

## 最小数据路径

沿用 Binance Vision 的 BTCUSDT 现货小时线，再补 USDⓈ-M BTCUSDT 永续成交小时线、标记价格小时线和结算资金费。不同价格分别用于双腿成交模型、保证金浮亏诊断、资金费金额，不能相互混称。

| 数据 | 官方路径模板／用途 | 实际可达证据 |
| --- | --- | --- |
| 资金费归档 | `https://data.binance.vision/data/futures/um/monthly/fundingRate/BTCUSDT/BTCUSDT-fundingRate-YYYY-MM.zip` | 2024-01 至 2025-12 共 24 个 URL 的 HEAD 全部 HTTP 200 |
| 永续成交小时线 | `https://data.binance.vision/data/futures/um/monthly/klines/BTCUSDT/1h/BTCUSDT-1h-YYYY-MM.zip` | 2024-01、2025-12 HEAD 200 |
| 标记价格小时线 | `https://data.binance.vision/data/futures/um/monthly/markPriceKlines/BTCUSDT/1h/BTCUSDT-1h-YYYY-MM.zip` | 2024-01、2025-12 HEAD 200 |
| 指数价格小时线（非必需） | `https://data.binance.vision/data/futures/um/monthly/indexPriceKlines/BTCUSDT/1h/BTCUSDT-1h-YYYY-MM.zip` | 2024-01、2025-12 HEAD 200 |
| 现货小时线 | `https://data.binance.vision/data/spot/monthly/klines/BTCUSDT/1h/BTCUSDT-1h-YYYY-MM.zip` | 已有前一实验来源；本次未重复下载 |

HEAD 只证明对象当时存在，不证明 CSV schema、数据完整性或收益。归档首次下载后必须核对 `.CHECKSUM`，保存 ZIP、来源、取数时间和 SHA-256，再验证字段、单位、重复与月边界。归档可能被官方修订，不能只保存一个可变 URL。[官方归档说明](https://raw.githubusercontent.com/binance/binance-public-data/master/README.md)

现货归档自 2025-01-01 起使用微秒；此前为毫秒。USD-M Kline 文档示例为毫秒，不能把现货的变更套到 futures。时间统一为 UTC；下载后显式验证每个源的单位和 `openTime`／`closeTime`，不能依赖数字位数静默猜测。[官方数据 schema](https://raw.githubusercontent.com/binance/binance-public-data/master/README.md)

官方仓库也提供 markPriceKlines 下载脚本，证实它是独立于成交 K 线的数据类别。标记价的无效成交量占位不应触发现货 OHLCV 的正成交量约束。[官方脚本](https://raw.githubusercontent.com/binance/binance-public-data/master/python/download-futures-markPriceKlines.py)

## 精确结算与公开 API 的实际限制

`GET /fapi/v1/fundingRate` 的 schema 提供 `symbol`、`fundingTime`、`fundingRate`、与该次结算关联的 `markPrice`。查询 `startTime`／`endTime` 为含端点毫秒，`limit` 最大 1000，并与 fundingInfo 共享 500 次／5 分钟／IP 的限制。分页须去重并检查时间严格递增；文档未承诺特定历史年份一定有完整 markPrice。[官方 API](https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Get-Funding-Rate-History)

归档 fundingRate CSV 在登记前未读取，本次不能声称它含精确结算 markPrice；下载后按实际表头校验。若只有费率和结算时间，则仍需要 API 的结算 markPrice，或者明确登记标记价 K 线近似，不能把小时开盘／收盘价叫作精确结算价。

本机匿名请求 `https://fapi.binance.com/fapi/v1/exchangeInfo` 返回 **HTTP 451**；未修改地区、代理或规避访问限制。因此，API 文档的公开接口分类不等于本机可用，也没有实测确认 2024／2025 fundingRate API 的可用性。若精确 markPrice 接口同样不可达，应保留失败证据，并如实缩小测算结论或使用事先明示的近似，不能伪造精确数据。

线性 USDT 合约的资金费金额按持有 BTC 数量 × 结算标记价 × 当次费率计算；对空头，正率收入、负率支出。资金费在交易者之间转移，不能再加一笔 Binance“资金费手续费”。默认结算间隔为 8 小时，但可能调整；实际结算有约 15 秒偏移，边界刚开仓不保证能收取。避免在结算瞬间虚构成交，也不能把后验结算率作为此前入场决策已知量。[官方资金费机制](https://www.binance.com/en-AU/support/faq/detail/360033525031)

## 双腿费用与数量

现货官方 Academy 列出的典型 taker 费率为 **0.10%**；USD-M 官方 FAQ 的 Regular User 示例为 **0.05% taker**，并明确示例费率是假设、实际须以费用表及账户条件为准。现行费用表在本次无登录访问中未返回可读费率行，因而不能声称已经逐项核实现行账户费率或还原 2024／2025 费率历史。研究可登记现货 10 bp、合约 5 bp 为标准费率假设；不得假设 BNB、VIP、返佣、免手续费或 maker 成交。每条腿的开仓、平仓分别收费，基数为各自成交名义金额。[现货官方说明](https://www.binance.com/en/academy/articles/how-to-calculate-transaction-fees-on-binance)、[合约官方说明](https://www.binance.com/en-AE/support/faq/detail/360033544231)、[现货费用表](https://www.binance.com/en/fee/trading)、[合约费用表](https://www.binance.com/en/fee/futureFee)

数量精度不能替代 `LOT_SIZE.stepSize`。BTCUSDT 当前及历史 LOT_SIZE／MARKET_LOT_SIZE／MIN_NOTIONAL 规则均未被本次请求核实；exchangeInfo 的示例币种不能冒充 BTC。若模型采用 0.001 BTC 的整数步长，应明确是研究假设，并在可执行性判断中保留限制。现货买入手续费可能从买到的 BTC 中扣除，模型须保证两腿对冲的是实际净 BTC，或明确使用等价计价、预留费用并记录其局限。[官方交易规则接口](https://developers.binance.com/docs/derivatives/usds-margined-futures/market-data/rest-api/Exchange-Information)、[官方现货佣金 FAQ](https://github.com/binance/binance-spot-api-docs/blob/master/faqs/commission_faq.md)

## 保证金与“最差”结果的边界

`GET /fapi/v1/leverageBracket` 是签名 USER_DATA 接口，需要 API Key，且包含用户调整项；它不是匿名历史维护保证金数据库。本次没有获取账户或历史档位。固定维护保证金比例只能是诊断 proxy，不能声称复原了真实清算、清算费或 ADL。[官方档位接口](https://developers.binance.com/docs/derivatives/usds-margined-futures/account/rest-api/Notional-and-Leverage-Brackets)

建议对协议的 40% 现货、40% 独立 USDT 抵押、20% 现金缓冲分别记账；抵押与缓冲是占用的本金，不是已经花掉的交易成本。合约空头在上涨时消耗合约钱包权益；现货浮盈不能假设自动到账抵押，更不能把现货当作已启用组合保证金。若不模拟转账与处置，就不能把外部现金缓冲当作瞬时可用的保证金。

小时标记价 high 可诊断该小时空头的较差权益，但 OHLC 不提供点序，也不证明两腿可同时按理想价平仓。报告应区分“历史样本最差月份／可观察最低保证金裕度”和“未来最坏损失”；平台违约、USDT 风险、跳空、双腿失配和系统性清算不能由这批小时线给出可靠损失上限。

## 数据使用条件

README 的代码许可证行写 MIT，同时明确将数据集使用指向单独条款。当前数据条款为 **CC BY-NC-SA 4.0**，允许个人非生产历史回测，禁止将数据集用于真实自营执行、自动商业下单、收费信号或商业金融工具；商业用途需另行书面许可。再发布派生作品需要署名并适用相同条款。因此，本轮个人离线研究与未来实盘／Alan 商业数据集成必须分别判断，不把“可公开下载”解读为无限商业授权。[官方数据条款 §§3–4](https://raw.githubusercontent.com/binance/binance-public-data/master/TERMS_AND_CONDITIONS.md)

未检查真实账户准入、开户、实际手续费、历史交易过滤器或完整清算引擎。公开数据可达性、法律条件及 API schema 都可能变化；本页是本次可复查的来源核验，不是实盘可执行保证。
