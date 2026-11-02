// SPDX-License-Identifier: GPL-3.0-or-later
//! The samples' way a capture once took, kept as the tests' reference: every sample unpacked
//! into a word as Sipeed's `slogic_unpack_range()` does, the trigger matched sample by sample
//! as in `slogic_data.c` at 0d728b3e1226a61738c99c3333ad16d5c2814728 (Copyright 2023–2025
//! Sipeed), and the kept samples packed bit by bit.

use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::CapturePlan;

use crate::{Settings, adc};

/// What a capture yields from a stream of transfer bytes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Expected {
    pub(crate) logic: Vec<u8>,
    pub(crate) codes: Vec<u16>,
    pub(crate) trigger: Option<u64>,
    pub(crate) done: bool,
}

/// The physical-channel samples of a wire mode, after the model's firmware head.
pub(crate) fn decode(wire: u8, head: usize, bytes: &[u8]) -> Vec<u32> {
    let bytes = &bytes[head.min(bytes.len())..];
    match wire {
        2 => bytes
            .iter()
            .flat_map(|&byte| [0, 2, 4, 6].map(|shift| u32::from(byte >> shift & 3)))
            .collect(),
        4 => bytes
            .iter()
            .flat_map(|&byte| [u32::from(byte & 0xf), u32::from(byte >> 4)])
            .collect(),
        8 => bytes.iter().map(|&byte| u32::from(byte)).collect(),
        16 => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u32::from(u16::from_le_bytes(pair)))
            .collect(),
        32 => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&word| u32::from_le_bytes(word))
            .collect(),
        _ => unreachable!("wire mode"),
    }
}

/// The wire bytes of these samples.
pub(crate) fn encode(wire: u8, samples: &[u32]) -> Vec<u8> {
    match wire {
        2 => samples
            .as_chunks::<4>()
            .0
            .iter()
            .map(|s| (s[0] & 3 | (s[1] & 3) << 2 | (s[2] & 3) << 4 | (s[3] & 3) << 6) as u8)
            .collect(),
        4 => samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| (s[0] & 0xf | (s[1] & 0xf) << 4) as u8)
            .collect(),
        8 => samples.iter().map(|&sample| sample as u8).collect(),
        16 => samples
            .iter()
            .flat_map(|&sample| (sample as u16).to_le_bytes())
            .collect(),
        32 => samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect(),
        _ => unreachable!("wire mode"),
    }
}

fn matches(
    conditions: &[(ChannelIndex, TriggerCondition)],
    previous: Option<u32>,
    sample: u32,
) -> bool {
    conditions.iter().all(|&(channel, condition)| {
        let bit = |sample: u32| sample >> channel & 1 == 1;
        let now = bit(sample);
        match condition {
            TriggerCondition::None => true,
            TriggerCondition::High => now,
            TriggerCondition::Low => !now,
            TriggerCondition::Rise => previous.is_some_and(|previous| !bit(previous) && now),
            TriggerCondition::Fall => previous.is_some_and(|previous| bit(previous) && !now),
            TriggerCondition::Both => previous.is_some_and(|previous| bit(previous) != now),
        }
    })
}

/// The capture of `bytes`, as whole blocks of 64 samples of the stream: a block the stream
/// has not completed yields nothing.
pub(crate) fn run(settings: &Settings, plan: &CapturePlan, head: usize, bytes: &[u8]) -> Expected {
    let mut samples = decode(settings.channels(), head, bytes);
    samples.truncate(samples.len() / 64 * 64);
    let logic = settings.logic();
    let conditions: Vec<_> = if plan.instant {
        Vec::new()
    } else {
        plan.triggers
            .iter()
            .copied()
            .filter(|&(channel, condition)| {
                channel < 32 && logic & (1 << channel) != 0 && condition != TriggerCondition::None
            })
            .collect()
    };
    let limit = (!plan.looping).then_some(settings.limit_samples / 64 * 64);
    let mut trigger = None;
    let start = if conditions.is_empty() {
        Some(0)
    } else {
        let history = (settings
            .limit_samples
            .saturating_mul(u64::from(plan.trigger_position.min(90)))
            / 100)
            .min(crate::stream::MAX_PRE_SAMPLES) as usize;
        (history..samples.len())
            .find(|&tick| {
                matches(
                    &conditions,
                    tick.checked_sub(1).map(|before| samples[before]),
                    samples[tick],
                )
            })
            .map(|tick| {
                trigger = Some(history as u64);
                tick - history
            })
    };
    let kept = start.map_or(&[][..], |start| {
        let end = limit.map_or(samples.len(), |limit| {
            (start + limit as usize).min(samples.len())
        });
        &samples[start..end]
    });
    let kept = &kept[..kept.len() / 64 * 64];
    let channels: Vec<_> = (0..32)
        .filter(|channel| logic & (1 << channel) != 0)
        .collect();
    let groups = settings.analog_groups();
    Expected {
        logic: kept
            .as_chunks::<64>()
            .0
            .iter()
            .flat_map(|block| {
                channels.iter().flat_map(|&channel| {
                    block
                        .iter()
                        .enumerate()
                        .fold(0u64, |word, (tick, sample)| {
                            word | u64::from(sample >> channel & 1) << tick
                        })
                        .to_le_bytes()
                })
            })
            .collect(),
        codes: kept
            .iter()
            .flat_map(|&sample| {
                groups
                    .iter()
                    .map(move |&group| adc::code((sample >> (u32::from(group) * 8)) as u8))
            })
            .collect(),
        trigger,
        done: limit.is_some_and(|limit| kept.len() as u64 == limit),
    }
}
