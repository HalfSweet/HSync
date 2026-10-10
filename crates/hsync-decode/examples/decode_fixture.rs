//! Writes the decoders of a `.dsl` capture, decoded and presented, as JSON: the browser mock of
//! the UI replays them (`src/ipc/mock/fixtures/protocol-decoded.json`).
//!
//! ```sh
//! cargo run -p hsync-decode --example decode_fixture -- src-tauri/resources/demo/protocol.dsl \
//!     > src/ipc/mock/fixtures/protocol-decoded.json
//! ```

use hsync_decode::{Presentation, Spec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: decode_fixture <capture.dsl>")?;
    let document = hsync_fileio::read_dsl_file(std::path::Path::new(&path))?;
    let capture = &document.capture;
    let mut out = Vec::new();
    for (index, entry) in document
        .decoders
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(spec) = Spec::from_dsview(entry) else {
            continue;
        };
        let Some(decoded) = spec.run(capture, capture.len()) else {
            continue;
        };
        let presentation = Presentation::new(&decoded);
        out.push(serde_json::json!({
            "index": index,
            "id": spec.id,
            "presentation": presentation.snapshot(),
        }));
    }
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}
