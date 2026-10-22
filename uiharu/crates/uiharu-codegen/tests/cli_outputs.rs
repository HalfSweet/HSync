// SPDX-License-Identifier: Apache-2.0

//! Renames and removals prune only obsolete Rust output; checks never mutate files.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use uiharu_ir::Module;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "uiharu-codegen-outputs-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("ir")).expect("temporary IR directory");
        Self(path)
    }

    fn ir(&self) -> PathBuf {
        self.0.join("ir")
    }

    fn output(&self) -> PathBuf {
        self.0.join("generated")
    }

    fn run(&self, check: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_uiharu-codegen"));
        command.arg(self.ir()).arg(self.output());
        if check {
            command.arg("--check");
        }
        command.output().expect("codegen runs")
    }

    fn generate(&self) {
        let output = self.run(false);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn module(&self, name: &str) {
        let mut module: Module =
            serde_json::from_str(include_str!("../../uiharu-ir/tests/fixtures/pwm.json"))
                .expect("hand-authored PWM fixture");
        module.name = name.to_owned();
        module.decoder.as_mut().expect("PWM decoder").name = name.to_owned();
        std::fs::write(
            self.ir().join(format!("{name}.json")),
            serde_json::to_vec(&module).expect("module serializes"),
        )
        .expect("temporary module writes");
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn snapshot(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    std::fs::read_dir(directory)
        .expect("output directory")
        .map(|entry| {
            let entry = entry.expect("output entry");
            (
                entry.file_name().into(),
                std::fs::read(entry.path()).expect("output reads"),
            )
        })
        .collect()
}

#[test]
fn check_rejects_orphans_without_mutation_and_generation_prunes_renames_and_removals() {
    let directory = Directory::new();
    directory.module("pwm");
    directory.generate();
    std::fs::write(directory.output().join("notes.txt"), b"keep me").expect("non-Rust file");
    assert!(directory.run(true).status.success());

    // The current lib.rs can be correct while an old tracked Rust module remains.
    std::fs::write(directory.output().join("orphan.rs"), b"// obsolete\n").expect("orphan");
    let before = snapshot(&directory.output());
    let check = directory.run(true);
    assert!(!check.status.success());
    assert!(String::from_utf8_lossy(&check.stderr).contains("orphan.rs is unexpected"));
    assert_eq!(snapshot(&directory.output()), before);
    directory.generate();
    assert!(!directory.output().join("orphan.rs").exists());

    std::fs::remove_file(directory.ir().join("pwm.json")).expect("old module removed");
    directory.module("renamed");
    let before = snapshot(&directory.output());
    let check = directory.run(true);
    assert!(!check.status.success());
    assert!(String::from_utf8_lossy(&check.stderr).contains("pwm.rs is unexpected"));
    assert_eq!(snapshot(&directory.output()), before);
    directory.generate();
    assert!(!directory.output().join("pwm.rs").exists());
    assert!(directory.output().join("renamed.rs").is_file());
    assert!(directory.run(true).status.success());

    std::fs::remove_file(directory.ir().join("renamed.json")).expect("last module removed");
    let before = snapshot(&directory.output());
    assert!(!directory.run(true).status.success());
    assert_eq!(snapshot(&directory.output()), before);
    directory.generate();
    assert!(!directory.output().join("renamed.rs").exists());
    assert_eq!(
        std::fs::read(directory.output().join("notes.txt")).expect("preserved non-Rust file"),
        b"keep me"
    );
    assert!(directory.output().join("lib.rs").is_file());
    assert!(directory.run(true).status.success());
}
