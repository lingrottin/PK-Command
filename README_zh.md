# PKCommand

![Crates.io Version](https://img.shields.io/crates/v/pk-command)

此仓库包含 PK 命令的规范和实现，PK 命令是一种专为嵌入式设备设计的通信协议。

## Rust 库

此 Rust 库在主机和设备上通用。调用者需要在主机或设备上构造一个 `PkCommand` 实例，并持续调用该实例的 `poll` 方法。

变量和方法管理由实现 `PkVariableAccessor` 和 `PkMethodAccessor` 特征的类型提供，其中方法的非阻塞机制基于 `Pollable` 特征。此库还提供了预定义的 `PkVHashmapWrapper`（用于变量）、`PkMHashmapWrapper`（用于方法）和 `PkPollable`。

### 示例

请参阅[测试用例](/tests/pk_command_simulation.rs)。

## 协议

- [协议规范（英语）](/Specification.md)
- [协议规范（中文）](/Specification_zh.md)
