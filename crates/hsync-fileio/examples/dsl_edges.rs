//! Dumps a `.dsl` capture as JSON edge lists, the fixture of the frontend's browser mock.
//!
//! `cargo run -p hsync-fileio --example dsl_edges -- capture.dsl > edges.json`

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or("usage: dsl_edges <capture.dsl>")?
        .into();
    let document = hsync_fileio::read_dsl_file(&path)?;
    let capture = &document.capture;
    let names: std::collections::BTreeMap<_, _> = document.header.probes.iter().cloned().collect();
    let channels: Vec<serde_json::Value> = capture
        .channels()
        .map(|(index, data)| {
            let mut edges = Vec::new();
            let mut at = 0;
            while let Some(edge) = data.next_change(at) {
                edges.push(edge);
                at = edge;
            }
            serde_json::json!({
                "index": index,
                "name": names.get(&index).cloned().unwrap_or_else(|| index.to_string()),
                "first": data.sample(0),
                "edges": edges,
            })
        })
        .collect();
    let info = capture.info();
    let fixture = serde_json::json!({
        "samplerate": info.samplerate,
        "sampleCount": info.sample_count,
        "triggerSample": info.trigger_sample,
        "triggerTimeMs": info.trigger_time_ms,
        "channels": channels,
        "decoders": document.decoders,
    });
    println!("{}", serde_json::to_string(&fixture)?);
    Ok(())
}
