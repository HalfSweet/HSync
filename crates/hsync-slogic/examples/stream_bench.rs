// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU-only throughput of the production wire conversion, without USB or capture storage.
//! `cargo run --release -p hsync-slogic --example stream_bench -- [PACKED_BYTES]`

use std::hint::black_box;
use std::time::Instant;

use hsync_driver::{CapturePlan, Packet};
use hsync_slogic::{SLOGIC32U3, Settings, UsbSpeed, adc};

// Compile the production internals into this benchmark without exposing them as public APIs.
#[path = "../src/assemble.rs"]
mod assemble;
#[path = "../src/stream.rs"]
mod stream;

fn measure<F: assemble::WireFormat>(bytes: usize) {
    let bytes = bytes / (8 * F::BITS) * (8 * F::BITS);
    let samples = bytes as u64 * 8 / F::BITS as u64;
    let mut settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus);
    settings.enabled = u32::MAX >> (32 - F::BITS);
    settings.limit_samples = samples;
    let mut stream = stream::Stream::<F>::new(&settings, &CapturePlan::default());
    let mut decoder = assemble::WireDecoder::<F>::new(0);
    let mut state = 0x1234_5678u32;
    let input: Vec<_> = (0..1 << 20)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let started = Instant::now();
    let mut remaining = bytes;
    let mut output_bytes = 0;
    while remaining > 0 {
        let take = remaining.min(input.len());
        let mut packets = Vec::new();
        decoder.push(black_box(&input[..take]), |view| {
            stream.push(view, &mut packets)
        });
        for packet in packets {
            if let Packet::Logic(output) = packet {
                output_bytes += output.len();
                black_box(output);
            }
        }
        remaining -= take;
    }
    let elapsed = started.elapsed().as_secs_f64();
    assert!(stream.done());
    assert_eq!(output_bytes, bytes);
    println!(
        "{:2} channels: {:8.1} MB/s, {:8.1} Msample/s, {:7.3} s ({bytes} packed bytes)",
        F::BITS,
        bytes as f64 / elapsed / 1e6,
        samples as f64 / elapsed / 1e6,
        elapsed
    );
}

fn main() {
    let bytes = std::env::args()
        .nth(1)
        .map(|s| s.parse().expect("integer packed byte count"))
        .unwrap_or(128 << 20);
    assert!(
        bytes >= 256,
        "at least one 64-sample block in every wire mode"
    );
    measure::<assemble::Packed<2>>(bytes);
    measure::<assemble::Packed<4>>(bytes);
    measure::<u8>(bytes);
    measure::<u16>(bytes);
    measure::<u32>(bytes);
}
