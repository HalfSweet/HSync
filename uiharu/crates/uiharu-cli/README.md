# uiharu-cli

The uiharu command decodes digital captures to NDJSON, generates synthetic captures and benchmarks decoder pipelines. Published packages build with Cargo alone.

```sh
cargo install uiharu-cli --locked
uiharu gen flash.bin
uiharu decode flash.bin --stats
```

Part of [uiharu](https://crates.io/crates/uiharu), a streaming protocol decode engine
licensed under Apache-2.0. Requires Rust 1.90 or newer. The crates share a release
version independent of HSync.

- [API documentation](https://docs.rs/uiharu-cli)
- [Engine guide and protocol coverage](https://github.com/HalfSweet/HSync/tree/main/uiharu)
- [Compatibility and releases](https://github.com/HalfSweet/HSync/blob/main/uiharu/docs/releasing.md)
