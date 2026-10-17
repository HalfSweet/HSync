// SPDX-License-Identifier: Apache-2.0

//! The generated crate is what the backend writes for the IR checked in: generating every
//! module of `uiharu/dsl/ir/` again gives the files under `uiharu-dsl-protocols/src/` as they
//! are. When this fails, run `cargo run -p uiharu-codegen -- uiharu/dsl/ir
//! uiharu/crates/uiharu-dsl-protocols/src` and commit the result.

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
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} reads: {error}", path.display()))
}

#[test]
fn the_generated_crate_is_up_to_date() {
    let ir_dir = root().join("../../dsl/ir");
    let out_dir = root().join("../uiharu-dsl-protocols/src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&ir_dir)
        .expect("the IR directory")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "modules under {}", ir_dir.display());
    let mut names = Vec::new();
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
        assert_eq!(
            read(&target),
            rustfmt(&source),
            "{} is out of date: regenerate with uiharu-codegen",
            target.display()
        );
        names.push(module_name(&module));
    }
    assert_eq!(
        read(&out_dir.join("lib.rs")),
        rustfmt(&generate_lib(&names)),
        "lib.rs is out of date: regenerate with uiharu-codegen"
    );
}
