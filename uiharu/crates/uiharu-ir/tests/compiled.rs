// SPDX-License-Identifier: Apache-2.0

//! Freshly compiled TypeScript intermediates (`uiharu/dsl/ir/*.json`) parse and pass the
//! checks: the contract between the TypeScript front end and Rust backends. Prepare these
//! ignored files with `pnpm dsl:compile` before running Cargo tests.

use std::path::PathBuf;

use uiharu_ir::{Module, validate};

fn compiled_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dsl/ir")
}

#[test]
fn every_compiled_module_passes_the_checks() {
    let dir = compiled_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| {
            panic!(
                "{} is readable: {error}; run `pnpm dsl:compile` before Cargo tests",
                dir.display()
            )
        })
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "the compiler has written modules to {}; run `pnpm dsl:compile` before Cargo tests",
        dir.display()
    );
    for path in files {
        let json = std::fs::read_to_string(&path).expect("the module reads");
        let module: Module = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("{} parses: {error}", path.display()));
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        assert_eq!(
            module.name,
            stem,
            "{} is named after its decoder",
            path.display()
        );
        let diagnostics = validate(&module);
        assert!(
            diagnostics.is_empty(),
            "{} passes the checks: {diagnostics:#?}",
            path.display()
        );
    }
}
