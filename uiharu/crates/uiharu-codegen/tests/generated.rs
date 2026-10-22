// SPDX-License-Identifier: Apache-2.0

//! The tracked generated Rust is what the backend writes from freshly compiled TypeScript
//! intermediates. Run `pnpm dsl:compile` before these tests to prepare production and backend
//! fixture IR. When generated Rust differs, regenerate it with `uiharu-codegen` and commit
//! the Rust output; the intermediate JSON is not tracked.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use uiharu_codegen::{generate, generate_lib, module_name};
use uiharu_ir::Module;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rustfmt(source: &str) -> String {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("rustfmt is in the toolchain");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(source.as_bytes())
        .expect("writes");
    let output = child.wait_with_output().expect("rustfmt runs");
    assert!(
        output.status.success(),
        "rustfmt: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "{} reads: {error}; prepare missing IR with `pnpm dsl:compile` before Cargo tests",
            path.display()
        )
    })
}

#[test]
fn the_generated_crate_is_up_to_date() {
    assert_generated(
        root().join("../../dsl/ir"),
        root().join("../uiharu-dsl-protocols/src"),
    );
}

#[test]
fn backend_acceptance_sources_are_up_to_date() {
    let fixture = root().join("../uiharu-interp/tests/fixtures/backend");
    assert_generated(fixture.join("ir"), fixture.join("generated"));
}

fn assert_generated(ir_dir: PathBuf, out_dir: PathBuf) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(&ir_dir)
        .unwrap_or_else(|error| {
            panic!(
                "{} reads: {error}; run `pnpm dsl:compile` before Cargo tests",
                ir_dir.display()
            )
        })
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no freshly compiled modules under {}; run `pnpm dsl:compile` before Cargo tests",
        ir_dir.display()
    );
    let mut names = Vec::new();
    let mut expected = BTreeSet::from([std::ffi::OsString::from("lib.rs")]);
    for path in files {
        let module: Module = serde_json::from_str(&read(&path)).expect("the module parses");
        assert_eq!(
            uiharu_ir::validate(&module),
            Vec::new(),
            "{}",
            path.display()
        );
        let source =
            generate(&module).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let target = out_dir.join(format!("{}.rs", module_name(&module)));
        expected.insert(
            target
                .file_name()
                .expect("a generated file name")
                .to_owned(),
        );
        assert_eq!(
            read(&target),
            rustfmt(&source),
            "{} is out of date: regenerate with uiharu-codegen",
            target.display()
        );
        names.push(module_name(&module));
    }
    let actual: BTreeSet<_> = std::fs::read_dir(&out_dir)
        .expect("the generated Rust directory")
        .map(|entry| entry.expect("a generated directory entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| path.file_name().expect("a generated file name").to_owned())
        .collect();
    assert_eq!(
        actual,
        expected,
        "{} has unexpected Rust files; regenerate with uiharu-codegen",
        out_dir.display()
    );
    assert_eq!(
        read(&out_dir.join("lib.rs")),
        rustfmt(&generate_lib(&names)),
        "lib.rs is out of date: regenerate with uiharu-codegen"
    );
}
