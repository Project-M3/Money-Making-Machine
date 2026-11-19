# 首条模拟闭环的现成框架

核验日期：2026-10-15。范围：BTC/USDT 现货、小时级策略、公开行情、本地模拟与历史回放。一周内贯通是目标，最终全 Rust 栈与 GPUI 是用户已确认约束，其他语言工具可作过渡。本页是研究依据与建议，不是已完成选型或运行验证。

## 官方资料确认的能力

| 候选 | 公开行情与本地模拟 | 回测与策略语言 | macOS / 许可 | 第一周摩擦（判断） |
| --- | --- | --- | --- | --- |
| Freqtrade | dry-run 文档明确可留空 API key，虚拟钱包和订单留在本地；支持 Binance | 内置回测，Python 策略；不是 Rust 框架 | 官方列出 macOS；GPL-3.0 | 最接近可配置的成品：下载数据、写策略、回测、dry-run、SQLite 记录与现成 Web UI；仍须检查默认 ROI/止损等规则是否改变实验 |
| NautilusTrader | 官方支持实时数据 + sandbox 虚拟执行；Binance Spot JSON 公共流与需 key 的 SBE 流要区分 | 最新 2.x 文档明确支持纯 Rust 策略、回测与 LiveNode，也提供 Python；不是仅凭“Rust 内核”推断 | 当前矩阵列 macOS ARM64；LGPL-3.0 | Rust 长期契合度高，但版本、节点配置与适配器组合比 Freqtrade 多；latest 文档是 2.x，普通 PyPI 安装仍指向 API 不同的 1.x |
| Barter | README 有 live public market data + mock execution 示例；Barter-Data 接公开流 | 原生 Rust，已有 backtest 模块与可替换策略组件 | MIT；所查 CI 仅 Ubuntu，未证实 macOS 支持矩阵 | 适合嵌入 Rust 工具；仍需组合行情、历史输入、策略、持久化与展示，不是即装即用的研究应用 |

来源：

- Freqtrade：[dry-run 配置与模拟成交规则](https://www.freqtrade.io/en/stable/configuration/#using-dry-run-mode)、[官方功能及系统说明](https://github.com/freqtrade/freqtrade)、[安装](https://www.freqtrade.io/en/stable/installation/)、[GPL 许可](https://raw.githubusercontent.com/freqtrade/freqtrade/stable/LICENSE)。
- NautilusTrader：[Rust 能力矩阵](https://nautilustrader.io/docs/latest/concepts/rust/)、[backtest / sandbox / live](https://nautilustrader.io/docs/latest/concepts/overview/)、[Binance 数据接入](https://nautilustrader.io/docs/latest/integrations/binance/)、[版本与安装注意事项](https://nautilustrader.io/docs/latest/getting_started/installation/)、[平台与许可](https://github.com/nautechsystems/nautilus_trader)。
- Barter：[官方仓库、mock 示例与 MIT 许可](https://github.com/barter-rs/barter-rs)、[回测 API](https://docs.rs/barter/latest/barter/backtest/fn.backtest.html)、[所查 CI](https://raw.githubusercontent.com/barter-rs/barter-rs/develop/.github/workflows/ci.yml)。

## 建议：按最终 Rust 栈约束选择

用户在初步研究后明确最终全 Rust 栈。因此当前建议优先验证 NautilusTrader 原生 Rust 路径；Freqtrade 作为过渡或参考，不是默认实现路线。以下 Freqtrade 快跑建议仅是放宽最终栈约束时的比较，不作为当前选择。

如果“一周内看到完整实验”优先，建议 Freqtrade 先承担回测和实时模拟，M3 只保存策略、配置、实验说明与结果。GPUI 在确有查看需求时再接入；不先写自有撮合、交易所 SDK 或框架封装。这是基于现成功能覆盖的工程判断，尚无本机安装耗时证据。

如果“策略与运行链从第一天就是 Rust”也是必要条件，优先给 NautilusTrader 一次有界的接入验证：锁定相匹配的版本与文档，验证公开 JSON 行情、本地 sandbox、历史回放和结果导出。不把最新文档能力等同于所有已发布版本都可用。Barter 是较小的 Rust 库组合候选，只有其示例确实减少工作时才选。

无论选哪一个，首轮只实现固定规则，不启用自动选参、机器学习或多市场。使用现成框架仍须验证信号时间、成交价格、费用以及现金/买入持有对照；回测与模拟的默认成交规则未必一致。

## 尚未验证

初始对比时未安装或运行框架。后续已完成 [NautilusTrader 0.65.0 原生 Rust 历史回放验证](nautilus-rust-spike.md)；Freqtrade 与 Barter 未安装或运行，无人值守恢复未验证。Nautilus 的完整“无 key 数据客户端 + sandbox”组合尚未实跑；Barter 的小时 K 线历史输入与 macOS 编译仍需最小验证。公开数据访问条件不等于未来实盘准入。许可只记录官方标识，本次不判断未来分发方案的合规性。
