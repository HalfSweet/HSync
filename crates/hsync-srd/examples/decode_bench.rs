//! Decoding speed on this machine: DSView's protocol demo repeated many times, through the
//! native decoders and through DSView's own Python decoders in the embedded interpreter
//! (DESIGN.md §7).
//!
//! ```sh
//! cargo run --release -p hsync-srd --example decode_bench [-- REPEATS]
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use hsync_capture::LogicCapture;
use hsync_decode::Spec;
use hsync_decode::spec::Decoded;
use hsync_srd::{DecoderHost, Stack, run_stack};
use parking_lot::RwLock;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `capture` played `times` times in a row.
fn repeated(capture: &LogicCapture, times: usize) -> LogicCapture {
    let channels: Vec<_> = capture.channels().map(|(index, _)| index).collect();
    let mut out = LogicCapture::new(capture.samplerate(), channels.iter().copied());
    for &index in &channels {
        let data = capture.channel(index).expect("channel");
        let words: Vec<u64> = (0..data.word_count()).map(|word| data.word(word)).collect();
        let target = out.channel_mut(index).expect("channel");
        for _ in 0..times {
            target.append_words(&words);
        }
    }
    out
}

fn report(label: &str, samples: u64, began: Instant) {
    let seconds = began.elapsed().as_secs_f64();
    println!(
        "{label:<22} {:>8.1} Msample/s  ({:.0} ms for {} samples)",
        samples as f64 / seconds / 1e6,
        seconds * 1e3,
        samples
    );
}

fn main() {
    let repeats: usize = std::env::args()
        .nth(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(128);
    let document =
        hsync_fileio::read_dsl_file(&root().join("src-tauri/resources/demo/protocol.dsl"))
            .expect("demo");
    let entries = document
        .decoders
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    let native = repeated(&document.capture, repeats);
    let len = native.len();
    println!("protocol demo × {repeats}: {len} samples at 25 MHz");
    for entry in &entries {
        let Some(spec) = Spec::from_dsview(entry) else {
            continue;
        };
        let began = Instant::now();
        let decoded = spec.run(&native, 0..len).expect("channels");
        report(
            &format!("native {}", entry["id"].as_str().unwrap_or("?")),
            len,
            began,
        );
        let items = match &decoded {
            Decoded::I2c { trace, .. } => trace.items.len(),
            Decoded::Uart(trace) => trace.frames.len(),
            Decoded::Spi(trace) => trace.words.len(),
            Decoded::Can(trace) => trace.frames.len(),
        };
        println!("{:<22} {items} items", "");
    }

    // The interpreter walks a sample at a time through wait(); a shorter run is enough.
    let python = Arc::new(RwLock::new(repeated(
        &document.capture,
        repeats.div_ceil(16),
    )));
    let len = python.read().len();
    let host = DecoderHost::new(root().join("src-tauri/resources/decoders"));
    let _ = host.catalog();
    for entry in &entries {
        let id = entry["id"].as_str().unwrap_or("?");
        let Some(mut stack) = Stack::from_dsview(entry) else {
            continue;
        };
        // The bottom decoder alone, as the native one decodes.
        stack.layers.truncate(1);
        let began = Instant::now();
        match run_stack(
            &host,
            &python,
            0..len,
            &stack,
            &Arc::new(AtomicBool::new(false)),
        ) {
            Ok(outcome) if outcome.error.is_none() => report(&format!("python {id}"), len, began),
            Ok(outcome) => println!("python {id}: {:?}", outcome.error),
            Err(error) => println!("python {id}: {error}"),
        }
    }
}
