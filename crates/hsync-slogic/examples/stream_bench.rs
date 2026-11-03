// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU-only throughput of the production wire conversion, without USB or capture storage.
//! `cargo run --release -p hsync-slogic --example stream_bench -- [PACKED_BYTES]`

use std::hint::black_box;
use std::time::Instant;

use hsync_core::TriggerCondition;
use hsync_driver::{CapturePlan, Packet};
use hsync_slogic::{SLOGIC32U3, Settings, UsbSpeed};

// Compile the production internals into this benchmark without exposing them as public APIs.
#[allow(dead_code)]
#[path = "../src/assemble.rs"]
mod assemble;
#[allow(dead_code)]
#[path = "../src/stream.rs"]
mod stream;

#[derive(Clone, Copy)]
enum TriggerMode {
    None,
    Waiting,
    /// Fire on sample 1, keeping every subsequent output block shifted by one sample.
    Shifted,
}

/// What a case converts: the wire mode, logic channels, analog groups and trigger path.
struct Case {
    name: &'static str,
    wire: u8,
    enabled: u32,
    analog: u8,
    trigger: TriggerMode,
}

const CASES: &[Case] = &[
    Case {
        name: "2 channels",
        wire: 2,
        enabled: 0b11,
        analog: 0,
        trigger: TriggerMode::None,
    },
    Case {
        name: "4 channels",
        wire: 4,
        enabled: 0xf,
        analog: 0,
        trigger: TriggerMode::None,
    },
    Case {
        name: "8 channels",
        wire: 8,
        enabled: 0xff,
        analog: 0,
        trigger: TriggerMode::None,
    },
    Case {
        name: "16 channels",
        wire: 16,
        enabled: 0xffff,
        analog: 0,
        trigger: TriggerMode::None,
    },
    Case {
        name: "32 channels",
        wire: 32,
        enabled: u32::MAX,
        analog: 0,
        trigger: TriggerMode::None,
    },
    Case {
        name: "4 channels, waiting",
        wire: 4,
        enabled: 0xf,
        analog: 0,
        trigger: TriggerMode::Waiting,
    },
    Case {
        name: "8 channels, waiting",
        wire: 8,
        enabled: 0xff,
        analog: 0,
        trigger: TriggerMode::Waiting,
    },
    Case {
        name: "16 channels, waiting",
        wire: 16,
        enabled: 0xffff,
        analog: 0,
        trigger: TriggerMode::Waiting,
    },
    Case {
        name: "A0 A1 + D16-D31",
        wire: 32,
        enabled: u32::MAX,
        analog: 0b11,
        trigger: TriggerMode::None,
    },
    Case {
        name: "2 channels, shifted",
        wire: 2,
        enabled: 0b11,
        analog: 0,
        trigger: TriggerMode::Shifted,
    },
    Case {
        name: "4 channels, shifted",
        wire: 4,
        enabled: 0xf,
        analog: 0,
        trigger: TriggerMode::Shifted,
    },
    Case {
        name: "8 channels, shifted",
        wire: 8,
        enabled: 0xff,
        analog: 0,
        trigger: TriggerMode::Shifted,
    },
    Case {
        name: "16 channels, shifted",
        wire: 16,
        enabled: 0xffff,
        analog: 0,
        trigger: TriggerMode::Shifted,
    },
    Case {
        name: "32 channels, shifted",
        wire: 32,
        enabled: u32::MAX,
        analog: 0,
        trigger: TriggerMode::Shifted,
    },
    Case {
        name: "A0 A1 + D16-D31, shifted",
        wire: 32,
        enabled: u32::MAX,
        analog: 0b11,
        trigger: TriggerMode::Shifted,
    },
];

fn measure(case: &Case, input: &[u8], bytes: usize) {
    let mut settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus);
    settings.enabled = case.enabled;
    settings.analog = case.analog;
    settings.limit_samples = u64::MAX / 2;
    let trigger_channel = settings.logic().trailing_zeros() as u16;
    let plan = CapturePlan {
        triggers: match case.trigger {
            TriggerMode::None => Vec::new(),
            // D0 high and low at once: never.
            TriggerMode::Waiting => vec![(0, TriggerCondition::High), (0, TriggerCondition::Low)],
            TriggerMode::Shifted => vec![(trigger_channel, TriggerCondition::High)],
        },
        trigger_position: if matches!(case.trigger, TriggerMode::Waiting) {
            10
        } else {
            0
        },
        ..CapturePlan::default()
    };
    let mut stream = stream::Stream::new(&settings, &plan, case.wire, 0);
    if matches!(case.trigger, TriggerMode::Shifted) {
        let mut prefix = vec![0xff; usize::from(case.wire) * 8];
        prefix[usize::from(trigger_channel / 8)] &= !(1 << (trigger_channel % 8));
        let packets = stream.push(&prefix);
        assert!(packets.contains(&Packet::Trigger { sample: 0 }));
        black_box(packets);
    }
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
        "{:28}: {:8.1} MB/s, {:8.1} Msample/s, {:7.3} s ({bytes} packed bytes)",
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
    for case in CASES {
        measure(case, &input, bytes);
    }
}
