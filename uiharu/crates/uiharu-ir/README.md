# uiharu-ir

Serde types and validation for the decoder IR shared by the TypeScript compiler, Rust code generator and interpreter. IR modules identify their format through the ir field; unsupported versions are rejected.

Part of [uiharu](https://crates.io/crates/uiharu), a streaming protocol decode engine
licensed under Apache-2.0. Requires Rust 1.90 or newer. The crates share a release
version independent of HSync.

- [API documentation](https://docs.rs/uiharu-ir)
- [Engine guide and protocol coverage](https://github.com/HalfSweet/HSync/tree/main/uiharu)
- [Compatibility and releases](https://github.com/HalfSweet/HSync/blob/main/uiharu/docs/releasing.md)
