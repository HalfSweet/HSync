// SPDX-License-Identifier: Apache-2.0

//! Export built-in catalog metadata from the same DSL declarations used for loaded modules.

use std::{error::Error, fs, path::PathBuf};
use uiharu_registry::Catalog;

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut catalog = Catalog::new();
    for name in std::env::args().skip(1) {
        let module =
            serde_json::from_slice(&fs::read(root.join(format!("../../dsl/ir/{name}.json")))?)?;
        let loaded = catalog.load(&module)?;
        let contents = serde_json::to_string_pretty(loaded.manifest())? + "\n";
        fs::write(root.join(format!("manifests/{name}.json")), contents)?;
    }
    Ok(())
}
