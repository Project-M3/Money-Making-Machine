# 确定 Rust 与 GPUI 工具的最小范围及 Alan 分工

Type: grilling
Labels: wayfinder:grilling
Status: open
Assignee: none
Parent: [量化验证路径地图](../map.md)
Blocked by: 02

## Question

为首个实验必须建设哪些工具能力，哪些可复用？首个使用平台、必要交互和输出是什么？如何以 Rust + GPUI 实现最小范围，Alan 通过哪些已有能力参与？需要何种原型验证技术可行性？

围绕首条端到端模拟交易闭环限定范围，不以完整 UI、通用框架或深度 Alan 集成为前置条件；先明确必要的交互与结果可见性。

如何用最少功能支持预算内的自主模拟研究与费曼式学习，保留用户解释结果和决定重要方向的位置，并确保实盘、风险上限调整、追加预算需要用户确认？

用户已确认最终全 Rust 栈与 GPUI，其他技术栈可作过渡工具；优先复用适用 Rust 能力，确有缺口再参考其他社区实现最小 Rust 版本。选型需检验原生 Rust 运行路径，不以 Rust 内核加其他语言策略自动视作满足最终约束。
