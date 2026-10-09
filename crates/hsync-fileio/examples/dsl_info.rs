//! Prints a summary of a `.dsl` capture: header, channels and their activity.
//!
//! `cargo run -p hsync-fileio --example dsl_info -- capture.dsl`

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or("usage: dsl_info <capture.dsl>")?
        .into();
    let document = hsync_fileio::read_dsl_file(&path)?;
    let capture = &document.capture;
    let rate = capture.samplerate() as f64;
    println!("{:#?}", document.header);
    for (index, data) in capture.channels() {
        let edges = data.count_edges(0, data.len());
        // Bursts: edges separated by more than 50 µs start a new burst.
        let gap = (rate * 50e-6) as u64;
        let mut bursts = Vec::new();
        let mut at = 0;
        let mut current: Option<(u64, u64)> = None;
        while let Some(edge) = data.next_change(at) {
            current = match current {
                Some((start, end)) if edge - end <= gap => Some((start, edge)),
                Some(burst) => {
                    bursts.push(burst);
                    Some((edge, edge))
                }
                None => Some((edge, edge)),
            };
            at = edge;
        }
        bursts.extend(current);
        let ms = |sample: u64| sample as f64 / rate * 1e3;
        let shown: Vec<String> = bursts
            .iter()
            .take(12)
            .map(|(start, end)| format!("{:.4}-{:.4}", ms(*start), ms(*end)))
            .collect();
        println!(
            "D{index:<2} first={} rise={:<5} fall={:<5} bursts={:<3} {}",
            u8::from(data.sample(0)),
            edges.rising,
            edges.falling,
            bursts.len(),
            shown.join(" ")
        );
    }
    Ok(())
}
