# uiharu-codegen

Emits Rust decoders and contract types from validated uiharu IR. The uiharu-codegen binary accepts an IR directory and output directory; rustfmt must be available when running the generator.

```sh
cargo install uiharu-codegen --locked
uiharu-codegen path/to/ir path/to/generated
```

Part of [uiharu](https://crates.io/crates/uiharu), a streaming protocol decode engine
licensed under Apache-2.0. Requires Rust 1.90 or newer. The crates share a release
version independent of HSync.

- [API documentation](https://docs.rs/uiharu-codegen)
- [Engine guide and protocol coverage](https://github.com/HalfSweet/HSync/tree/main/uiharu)
- [Compatibility and releases](https://github.com/HalfSweet/HSync/blob/main/uiharu/docs/releasing.md)
