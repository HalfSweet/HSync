// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU-only throughput of the production wire conversion, without USB or capture storage.
//! `cargo run --release -p hsync-slogic --example stream_bench -- [PACKED_BYTES]`

use std::hint::black_box;
use std::time::Instant;

use hsync_core::TriggerCondition;
use hsync_driver::CapturePlan;
use hsync_slogic::{SLOGIC32U3, Settings, UsbSpeed};

// Compile the production internals into this benchmark without exposing them as public APIs.
#[allow(dead_code)]
#[path = "../src/assemble.rs"]
mod assemble;
#[allow(dead_code)]
#[path = "../src/stream.rs"]
mod stream;

/// What a case converts: the wire mode, the logic channels, the analog groups, and whether
/// the capture waits all along for a trigger that never fires.
struct Case {
    name: &'static str,
    wire: u8,
    enabled: u32,
    analog: u8,
    waits: bool,
}

const CASES: [Case; 9] = [
    Case {
        name: "2 channels",
        wire: 2,
        enabled: 0b11,
        analog: 0,
        waits: false,
    },
    Case {
        name: "4 channels",
        wire: 4,
        enabled: 0xf,
        analog: 0,
        waits: false,
    },
    Case {
        name: "8 channels",
        wire: 8,
        enabled: 0xff,
        analog: 0,
        waits: false,
    },
    Case {
        name: "16 channels",
        wire: 16,
        enabled: 0xffff,
        analog: 0,
        waits: false,
    },
    Case {
        name: "32 channels",
        wire: 32,
        enabled: u32::MAX,
        analog: 0,
        waits: false,
    },
    Case {
        name: "4 channels, waiting",
        wire: 4,
        enabled: 0xf,
        analog: 0,
        waits: true,
    },
    Case {
        name: "8 channels, waiting",
        wire: 8,
        enabled: 0xff,
        analog: 0,
        waits: true,
    },
    Case {
        name: "16 channels, waiting",
        wire: 16,
        enabled: 0xffff,
        analog: 0,
        waits: true,
    },
    Case {
        name: "A0 A1 + D16-D31",
        wire: 32,
        enabled: u32::MAX,
        analog: 0b11,
        waits: false,
    },
];

fn measure(case: &Case, input: &[u8], bytes: usize) {
    let mut settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus);
    settings.enabled = case.enabled;
    settings.analog = case.analog;
    settings.limit_samples = u64::MAX / 2;
    let plan = CapturePlan {
        // D0 high and low at once: never.
        triggers: if case.waits {
            vec![(0, TriggerCondition::High), (0, TriggerCondition::Low)]
        } else {
            Vec::new()
        },
        trigger_position: 10,
        ..CapturePlan::default()
    };
    let mut stream = stream::Stream::new(&settings, &plan, case.wire, 0);
    let started = Instant::now();
    let mut remaining = bytes;
    while remaining > 0 {
        let take = remaining.min(input.len());
        black_box(stream.push(black_box(&input[..take])));
        remaining -= take;
    }
    let elapsed = started.elapsed().as_secs_f64();
    let samples = bytes as f64 * 8.0 / f64::from(case.wire);
    println!(
        "{:22}: {:8.1} MB/s, {:8.1} Msample/s, {:7.3} s ({bytes} packed bytes)",
        case.name,
        bytes as f64 / elapsed / 1e6,
        samples / elapsed / 1e6,
        elapsed
    );
}

fn main() {
    let bytes = std::env::args()
        .nth(1)
        .map(|s| s.parse().expect("integer packed byte count"))
        .unwrap_or(256 << 20);
    let mut state = 0x1234_5678u32;
    let input: Vec<u8> = (0..1 << 20)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    for case in &CASES {
        measure(case, &input, bytes);
    }
}
