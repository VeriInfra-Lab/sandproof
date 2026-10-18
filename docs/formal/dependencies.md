# 外部依赖与可信链登记

[文档目录](../README.md) · [验证总览](../formal-verification-design.md) · [模块合同](../module-contracts.md) · [依赖登记](dependencies.md) · [证明工作流](../proof-workflow.md)

状态：详细证明计划；拟定定理尚待建立。适用代码定位基线与实际证明状态分别见总览和[覆盖记录](../proof-coverage.md)，不因本页完成而新增已证明结论。

本页是外部依赖 A1–A12 和获批可信薄适配 B1–B4 的唯一明细；模块计划引用合同编号，不再各自列一套“依赖不会出错”。当前采用原 Charon/Aeneas 抽取真实同步业务核心、窄接口模型和显式可信适配的路线，不以开发通用 async/Pin/Drop 翻译器为前置。可信范围来自项目 owner 的明确选择；其余自有业务继续由对应模块证明。
