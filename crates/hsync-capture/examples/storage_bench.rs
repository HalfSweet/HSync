//! Throughput of the sample storage on this machine: appending cross-format data as a device
//! streams it, summarising columns for the waveform, finding edges and counting them, and
//! dropping old samples as loop captures do (DESIGN.md §5).
//!
//! ```sh
//! cargo run --release -p hsync-capture --example storage_bench [-- SAMPLES_PER_CHANNEL]
//! ```

use std::time::Instant;

use hsync_capture::LogicCapture;

const CHANNELS: u16 = 16;

/// A busy, irregular signal: a different xorshift stream per channel, so words are mixed and
/// edges plentiful, as on a loaded bus.
fn cross_data(words_per_channel: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut bytes = Vec::with_capacity(words_per_channel * usize::from(CHANNELS) * 8);
    for _ in 0..words_per_channel {
        for _ in 0..CHANNELS {
            // Runs of a few dozen samples: an edge every ~32 samples.
            let word = next() & next() | next() & next();
            bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
    bytes
}

fn rate(label: &str, amount: f64, unit: &str, began: Instant) {
    let seconds = began.elapsed().as_secs_f64();
    println!(
        "{label:<34} {:>9.1} {unit}/s  ({:.1} ms)",
        amount / seconds,
        seconds * 1e3
    );
}

fn main() {
    let samples: u64 = std::env::args()
        .nth(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(64 << 20);
    let words = (samples / 64) as usize;
    println!(
        "{CHANNELS} channels × {samples} samples ({} MiB packed)",
        (samples * u64::from(CHANNELS) / 8) >> 20
    );
    let data = cross_data(words);

    // Appending in 4 MiB packets, as the DSLogic stream delivers them.
    let mut capture = LogicCapture::new(100_000_000, 0..CHANNELS);
    let mut deinterleaver = capture.deinterleaver();
    let began = Instant::now();
    for packet in data.chunks(4 << 20) {
        capture.append_cross(&mut deinterleaver, packet);
    }
    rate(
        "append (cross format)",
        data.len() as f64 / 1e6,
        "MB",
        began,
    );

    // A full-width waveform: 2000 columns over everything, then over 1 % of it.
    let channel = capture.channel(3).expect("channel");
    let began = Instant::now();
    for index in 0..CHANNELS {
        let _ = capture
            .channel(index)
            .expect("channel")
            .bins(0, samples, 2000);
    }
    rate(
        "bins, whole capture, 16 × 2000",
        f64::from(CHANNELS) * samples as f64 / 1e9,
        "Gsample",
        began,
    );
    let began = Instant::now();
    for index in 0..CHANNELS {
        let _ = capture.channel(index).expect("channel").bins(
            samples / 2,
            samples / 2 + samples / 100,
            2000,
        );
    }
    rate(
        "bins, 1 % zoom, 16 × 2000",
        f64::from(CHANNELS) * (samples / 100) as f64 / 1e9,
        "Gsample",
        began,
    );

    // Walking edges one by one, as hover read-outs and decoders do.
    let began = Instant::now();
    let mut at = 0;
    let mut edges = 0u64;
    while let Some(next) = channel.next_change(at) {
        at = next;
        edges += 1;
    }
    rate(
        "next_change, one channel",
        edges as f64 / 1e6,
        "Medge",
        began,
    );

    let began = Instant::now();
    let count = channel.count_edges(0, samples);
    rate(
        "count_edges, one channel",
        samples as f64 / 1e9,
        "Gsample",
        began,
    );
    assert_eq!(count.rising + count.falling, edges);

    // A loop capture moving on by a third, then lining its chunks up again.
    let dropped = samples / 3 / 64 * 64;
    let began = Instant::now();
    capture.drop_front(dropped);
    rate("drop_front, a third", 1.0, "op", began);
    let began = Instant::now();
    capture.compact();
    rate(
        "compact after drop_front",
        f64::from(CHANNELS) * (samples - dropped) as f64 / 8e6,
        "MB",
        began,
    );
}
