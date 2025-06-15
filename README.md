# PK Command

![Crates.io Version](https://img.shields.io/crates/v/pk-command)

\[[中文](/README_zh.md)\]

This repository contains the specification and an implementation of the PK Command, a communication protocol designed for embedded devices.

## Library usage

This Rust library is universal on both host and device. The caller needs to construct a `PkCommand` instance on the host or device and keep calling the instance's `poll` method.

Variable and method management is provided by types that implement the `PkVariableAccessor` and `PkMethodAccessor` traits, where the non-blocking mechanism of methods is based on the `Pollable` trait. This library also provides predefined `PkVHashmapWrapper` (for variables), `PkMHashmapWrapper` (for methods), and `PkPollable`.

### Example

See the [test case](/tests/pk_command_simulation.rs).

## The Protocol

- [Specification doc](/Specification.md)
- [协议规范（中文）](/Specification_zh.md)
