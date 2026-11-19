# NautilusTrader 原生 Rust 趋势验证

已验证 macOS ARM64、Rust 1.98.1、NautilusTrader 0.65.0 的原生历史回放路径。没有 Python 运行时、交易凭据或真实执行客户端。

这是有界的框架验证，现已加入公开实时行情驱动的本地持续模拟，尚不构成长期运行验收或盈利证明。策略只实现已确认的 20 小时简单均线：完成小时 K 线收盘价高于均值时持有，否则空仓。

## 运行

在本目录执行：

```sh
cargo test --locked -j 4
cargo run --locked -j 4
# 使用已保存输入离线重放，不重新请求行情：
cargo run --locked -j 4 -- artifacts/input.json
# 公开实时行情，每分钟采集；日志恢复后继续：
cargo run --locked -j 4 -- --paper
# 只采集一批后退出：
cargo run --locked -j 4 -- --paper --once
```

`rust-toolchain.toml` 固定 Rust 1.98.1；首次依赖构建需要联网。`Cargo.lock` 纳入版本控制。程序默认只读取 Binance 无认证公共行情接口的最近 1000 根小时 K 线与服务器时间；不需要账户或 API key。

## 数据流与产物

公开 K 线 → 完成 K 线信号 → 后续模拟报价 → 原生策略下单 → Nautilus 现金账户撮合／持仓／费用 → JSON。

- `artifacts/input.json`：源 URL、服务器时间与原始 K 线，支持同一输入重放。
- `artifacts/report.json`：全部信号、成交、费用、假设与引擎统计。
- `artifacts/engine-state.json`：框架标准化账户、订单和持仓状态。

实时模式另外保留 `artifacts/paper/inputs.jsonl` 输入日志、`summary.json` 账户与对照、`status.json` 运行心跳。重跑 `--paper` 从日志恢复，不覆盖历史输入；不要删除正在运行的状态目录。详细行为与局限见[持续模拟验证](../../docs/research/live-paper-spike.md)。

结果及构建目录均被 Git 忽略。运行会覆盖本目录产物，保留实验时先复制到独立目录；当前尚不是多实验管理工具。

## 显式简化

- 初始现金 1000 USDT，每次固定买卖 0.001 BTC；这些只是验证夹具，不是实际资金分配。
- 手续费每侧 10bp；历史 OHLCV 没有报价，使用下一小时开盘价上下各偏移 5bp 的模拟买卖价。均为明确假设，不是账户实际费率或历史盘口。
- 使用现金账户，不做空、不加杠杆；价格精度 2 位、数量步长 0.00001 是本验证的 instrument 配置，不是实盘过滤规则校验。
- 关闭框架的 bar execution，信号只能等待后续报价。报价规模假设足够，不模拟排队、部分成交或冲击。
- 未平仓头寸不强制在结尾出售；查看账户、持仓和未实现盈亏，不把现金余额当作任意结束状态下的完整净值。
- 内置统计可能使用 252 日年化；本验证不将其 Sharpe 等值作为适用于 24/7 市场的收益证据。

## 本轮检查

2026-10-19 新增[资金费率对冲经济测算](../../docs/research/funding-carry-review.md)，入口 `--carry <protocol.json> <input.json> <new-output-dir>`。这是研究账本，复用本 crate 十进制与价格精度，计算双腿费用、保证金占用、资金费估计区间、月度净值与独立持有月诊断；不冒充原生双腿执行或真实清算引擎。完整命令和输入来源见报告，已有输出拒绝覆盖。该实验当前不满足进入模拟执行的登记条件。

2026-10-17 新增[固定规则历史筛选](../../docs/research/sma20-history-review.md)，复用相同原生策略和引擎，标准／双倍成本两种场景。每年独立账户，20 小时预热、整年小时清算净值、现金与同仓位持有对照、回撤、费用、交易及分月／季度均保存为 JSON／JSONL；独立研究审计逐项核对。2025 留出结果按登记规则为放弃当前固定版本，不自动改参数。

在本目录使用已保存输入运行历史评估：

```sh
cargo run --locked -j 4 -- --historical ../../docs/research/sma20-history-protocol.json calibration artifacts/history/sma20-v1
cargo run --locked -j 4 -- --historical ../../docs/research/sma20-history-protocol.json holdout artifacts/history/sma20-v1
```

输入为对应 artifact 根目录下的 `calibration-input.json`、`holdout-input.json`；输出为 `calibration/`、`holdout/` 子目录，已有结果会拒绝覆盖。正式结果已存在，复现时复制输入至新目录，步骤见历史筛选报告。评估不写入 paper 日志、不管理 GUI，不改变实时账户。最终运行实现仍为 Rust，源数据获取和独立对账使用一次性 Python 研究脚本，未增加运行时依赖。

原有两个测试分别验证未完成 K 线与数据缺口处理，以及原生订单只在信号之后成交、买卖顺序与正费用。特别用信号收盘价 110、后续开盘价 90 的夹具，避免误把信号当根收盘价作为成交价。

详细版本、结果与未验证范围见[验证报告](../../docs/research/nautilus-rust-spike.md)。

另外两个测试覆盖分批模拟和恢复一致性、对照费用、日志残尾与互斥锁；历史评估新增完整年边界及原生账户核对、负收益筛选两项；carry 新增负资金费上下界方向、双腿预算与抵押占用两项，当前共八个测试。
