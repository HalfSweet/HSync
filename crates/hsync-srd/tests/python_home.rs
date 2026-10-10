//! Starting Python from the home release builds bundle. Python starts once per
//! process, so this has a test binary of its own. It runs where `HSYNC_TEST_PYTHON_HOME` names a
//! runtime laid out by `scripts/python-runtime.mjs`, as CI does, and passes trivially elsewhere.

use std::path::{Path, PathBuf};

use hsync_srd::DecoderHost;
use pyo3::prelude::*;

#[test]
fn starts_python_from_the_bundled_home() {
    let Some(home) = std::env::var_os("HSYNC_TEST_PYTHON_HOME").map(PathBuf::from) else {
        eprintln!("HSYNC_TEST_PYTHON_HOME is not set; skipping");
        return;
    };
    let decoders = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/decoders");
    let host = DecoderHost::new(decoders).with_python_home(home.clone());
    // Every decoder imports from the trimmed standard library.
    let catalog = host.catalog().expect("catalog");
    assert!(catalog.len() >= 145, "{} decoders", catalog.len());
    let prefix: String = Python::attach(|py| {
        py.import("sys")
            .and_then(|sys| sys.getattr("prefix"))
            .and_then(|prefix| prefix.extract())
    })
    .expect("sys.prefix");
    assert_eq!(
        Path::new(&prefix).canonicalize().expect("prefix"),
        home.canonicalize().expect("home")
    );
}
