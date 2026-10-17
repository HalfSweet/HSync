// SPDX-License-Identifier: Apache-2.0

//! `uiharu-codegen <ir-dir> <out-dir> [--check]`: generates the Rust modules of every IR file
//! in `ir-dir` into `out-dir` (`<name>.rs` and `lib.rs`), formatted with `rustfmt`; with
//! `--check`, exits 1 when a file is missing or out of date.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use uiharu_codegen::{generate, generate_lib, module_name};
use uiharu_ir::Module;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = args.iter().any(|arg| arg == "--check");
    let paths: Vec<&String> = args.iter().filter(|arg| !arg.starts_with("--")).collect();
    let (Some(ir_dir), Some(out_dir)) = (paths.first(), paths.get(1)) else {
        eprintln!("usage: uiharu-codegen <ir-dir> <out-dir> [--check]");
        std::process::exit(2);
    };
    match run(Path::new(ir_dir), Path::new(out_dir), check) {
        Ok(()) => {}
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

fn run(ir_dir: &Path, out_dir: &Path, check: bool) -> Result<(), String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(ir_dir)
        .map_err(|error| format!("{}: {error}", ir_dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    let mut modules = Vec::new();
    for path in &files {
        let json = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let module: Module =
            serde_json::from_str(&json).map_err(|error| format!("{}: {error}", path.display()))?;
        let diagnostics = uiharu_ir::validate(&module);
        if !diagnostics.is_empty() {
            let lines: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
            return Err(format!("{}:\n{}", path.display(), lines.join("\n")));
        }
        modules.push(module);
    }
    let mut outputs = Vec::new();
    for module in &modules {
        let source = generate(module).map_err(|error| format!("{}: {error}", module.name))?;
        outputs.push((out_dir.join(format!("{}.rs", module_name(module))), source));
    }
    let names: Vec<String> = modules.iter().map(module_name).collect();
    outputs.push((out_dir.join("lib.rs"), generate_lib(&names)));
    let mut stale = 0;
    for (path, source) in outputs {
        let formatted = rustfmt(&source).map_err(|error| format!("{}: {error}", path.display()))?;
        if check {
            let current = std::fs::read_to_string(&path).ok();
            if current.as_deref() != Some(formatted.as_str()) {
                eprintln!(
                    "{} is out of date; run cargo run -p uiharu-codegen",
                    path.display()
                );
                stale += 1;
            }
            continue;
        }
        std::fs::create_dir_all(out_dir)
            .map_err(|error| format!("{}: {error}", out_dir.display()))?;
        std::fs::write(&path, formatted).map_err(|error| format!("{}: {error}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }
    if stale > 0 {
        return Err(format!("{stale} file(s) out of date"));
    }
    Ok(())
}

/// Formats `source` with the toolchain's rustfmt.
fn rustfmt(source: &str) -> Result<String, String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("rustfmt: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(source.as_bytes())
            .map_err(|error| format!("rustfmt: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("rustfmt: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustfmt failed:\n{}\n--- source ---\n{source}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("rustfmt: {error}"))
}
