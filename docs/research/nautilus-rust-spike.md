# NautilusTrader 原生 Rust 最小验证结果

日期：2026-10-15。用户授权：先验证 Nautilus 原生 Rust 路径，有明显阻碍再看 Barter，不长时间停留在选型。

## 结论

**原生 Rust 历史回放链路已实际跑通，当前没有转向 Barter 的明显阻碍。** 可以继续以 Nautilus 验证后续实时模拟；本次不宣称实时 sandbox、无人值守恢复或策略盈利已通过。

程序见 [spikes/nautilus-trend](../../spikes/nautilus-trend/README.md)。自有代码只负责单均线、公开数据解析、少量框架接线和结果保存；订单、现金账户、费用与持仓复用框架。没有复刻撮合引擎。

## 固定环境

- macOS ARM64。
- NautilusTrader crates 固定 `=0.65.0`，发布源码 commit `7b766f8825b2539c5b2ac1375e9d97b41c509edb`，许可 LGPL-3.0-only。
- 本机原默认 Rust 1.93.0；`cargo info` 会选取兼容的旧版 0.52.0。显式查询 0.65.0 确认 MSRV 为 1.98.1。
- 安装 Rust 1.98.1 并通过验证目录的 `rust-toolchain.toml` 固定，全局默认仍为 1.93.0。固定 `Cargo.lock`。
- 普通依赖树无 `pyo3`/Python；`otool -L` 显示系统动态库，不含 Python 运行时。

原生接口参考：[Rust 能力与示例](https://nautilustrader.io/docs/latest/concepts/rust/)、[0.65.0 发布包](https://crates.io/crates/nautilus-backtest/0.65.0)。接口验证以本机下载的 0.65.0 发布源码为准，不直接照搬 develop 示例。

## 本机执行证据

- `cargo check -j 4` 通过。
- `cargo test --locked -j 4`：2 passed，0 failed。
- `cargo fmt --check` 通过。
- `cargo run --locked -j 4` 无认证读取公开 K 线和服务器时间，运行原生 BacktestEngine 成功。
- `cargo run --locked -j 4 -- artifacts/input.json` 离线重放成功；逐项比较全部 signals、fills 及账户／订单／持仓 summary，一致。
- 输出：`SPIKE_OK bars=999 signals=980 fills=116 orders=116 positions=58`。

输入共 1000 行，小时开盘范围 UTC 2026-09-03 16:00 至 2026-10-15 07:00，最后一根未完成，不产生信号。输入文件 SHA-256：`53c0b53741bf33b4db26b0617ea7cdfdd47dd90e6f4bdea2a011f67ac7b1c673`。

本轮固定夹具为 1000 USDT 模拟现金、每笔 0.001 BTC、每侧 10bp 费用及围绕下一开盘价上下 5bp 的模拟报价。最终无未平仓 BTC，现金 985.49413578 USDT，116 次成交、58 次持仓周期。该结果仅描述这一段数据与假设，不能推导盈利能力，也不是实际资金配置。

## 关键行为验证

- 金额和 SMA 比较使用 Decimal，框架价格与数量使用固定精度。
- 拒绝缺失、重复、乱序的小时 K 线及无效 OHLC；未完成 K 线不触发信号。
- 禁用 bar execution；策略在完成 K 线上生成目标状态，在后续报价上才提交订单。
- 测试通过特意错开的信号价和下个报价，验证没有按信号当根价格成交；同时检验完整买卖与非零费用。
- 使用原生 CASH 账户，不接入任何真实执行适配器。输入、信号、成交与框架标准化状态保存为可重放产物。

## 后续状态

已继续实现[公开实时行情驱动的本地模拟](live-paper-spike.md)，含输入日志恢复及现金／固定仓位买入持有对照。以下条目保留为本次历史回放验证当时的边界；实时路径使用 streaming backtest，不声称验证 LiveNode + SandboxExecutionClient。

## 本次历史回放验证的边界

- 尚未接入原生实时数据客户端 + SandboxExecutionClient，当前是 HTTP 公共数据输入后的历史回放；实时模拟是下一步，不需要在此先扩建框架。
- 未验证长时间自动恢复、重启对账、部分成交及预算强制执行。
- 尚未完成买入持有／现金基准、成本敏感性和独立样本评估；不关闭策略验证决策票，不据此决定实盘。
- 历史报价、手续费和 instrument 精度是验证假设；未核验账户费率或实时交易过滤规则。
- 未构建 GPUI，也未深度集成 Alan。未安装或测试 Barter。
- 原始数据及结果保留在验证目录的 Git 忽略产物内；代码、锁文件、命令与本报告可提交，当前未提交。
