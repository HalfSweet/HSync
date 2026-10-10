//! Prints what a waveform file (VCD, FST, GHW) holds and how it lays out as a capture: the
//! hierarchy, the sample grid, and the activity of the imported channels.
//!
//! `cargo run -p hsync-fileio --example waveform_info -- dump.vcd [samplerate-hz] [name…]`
//!
//! With names, only the signals called so are imported; without, every logic signal.

use std::path::PathBuf;
use std::time::Instant;

use hsync_fileio::{ImportRequest, WaveformItemKind, read_waveform_file, samplerate_text};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path: PathBuf = args
        .next()
        .ok_or("usage: waveform_info <dump> [hz] [name…]")?
        .into();
    let mut names: Vec<String> = args.collect();
    let samplerate = names
        .first()
        .and_then(|first| first.parse::<u64>().ok())
        .inspect(|_| {
            names.remove(0);
        });

    let started = Instant::now();
    let mut waveform = read_waveform_file(&path, &mut |_| {})?;
    let summary = waveform.summary();
    println!(
        "{:?}: read in {:.2?}; {} items, grid {} ({} samples, unit {:e} s, span {} units, finest {} units{})",
        summary.format,
        started.elapsed(),
        summary.items.len(),
        samplerate_text(summary.samplerate),
        summary.sample_count,
        summary.unit,
        summary.span,
        summary.finest,
        if summary.coarsened { ", coarsened" } else { "" }
    );
    let mut logic_bits = 0;
    for item in &summary.items {
        let tag = match item.kind {
            WaveformItemKind::Scope => String::new(),
            WaveformItemKind::Logic => {
                logic_bits += item.width;
                match item.range {
                    Some((msb, lsb)) => format!(" [{msb}:{lsb}]"),
                    None => format!(" ({} bit)", item.width),
                }
            }
            WaveformItemKind::Other => " (no logic level)".to_owned(),
        };
        println!("{}{}{}", "  ".repeat(item.depth as usize), item.name, tag);
    }
    println!("{logic_bits} logic bits in all");

    let signals: Vec<u32> = summary
        .items
        .iter()
        .filter(|item| names.is_empty() || names.contains(&item.name))
        .filter_map(|item| item.signal)
        .collect();
    let started = Instant::now();
    let imported = waveform.import(
        &ImportRequest {
            signals,
            samplerate,
        },
        &mut |_| true,
    )?;
    let capture = &imported.capture;
    println!(
        "imported {} channels of {} samples at {} in {:.2?}; {} MiB of sample data",
        imported.channel_names.len(),
        capture.len(),
        samplerate_text(capture.samplerate()),
        started.elapsed(),
        capture.heap_bytes() >> 20
    );
    for (index, name) in &imported.channel_names {
        let Some(data) = capture.channel(*index) else {
            continue;
        };
        let edges = data.count_edges(0, data.len());
        let mut changes = Vec::new();
        let mut at = 0;
        while changes.len() < 6
            && let Some(edge) = data.next_change(at)
        {
            changes.push(edge);
            at = edge;
        }
        if names.is_empty() && edges.rising + edges.falling == 0 {
            continue;
        }
        println!(
            "  {index:>4} {name:<40} starts {} edges {}/{} first changes at {:?}",
            u8::from(data.sample(0)),
            edges.rising,
            edges.falling,
            changes
        );
    }
    Ok(())
}
