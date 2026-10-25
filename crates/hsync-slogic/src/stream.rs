// SPDX-License-Identifier: GPL-3.0-or-later
//! Host trigger semantics follow Sipeed's `slogic/slogic_data.c` at
//! 0d728b3e1226a61738c99c3333ad16d5c2814728 (Copyright 2023–2025 Sipeed).
//! The cross-format conversion is HSync's own; the device sends packed samples.

use std::collections::VecDeque;

use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::{CapturePlan, Packet};

use crate::Settings;

// Unlike the reference's 1 GiB allocation, reserve at most 64 MiB for history in
// the desktop application. The marker reports the actual retained history.
const MAX_PRE_SAMPLES: u64 = (64 << 20) / size_of::<u32>() as u64;
const MAX_PACKET: usize = 1 << 20;

#[derive(Default)]
struct Trigger {
    high: u32,
    low: u32,
    rise: u32,
    fall: u32,
    edge: u32,
}

impl Trigger {
    fn new(conditions: &[(ChannelIndex, TriggerCondition)], enabled: u32) -> Self {
        let mut trigger = Self::default();
        for &(channel, condition) in conditions {
            if channel >= 32 || enabled & (1 << channel) == 0 {
                continue;
            }
            let mask = 1 << channel;
            match condition {
                TriggerCondition::None => {}
                TriggerCondition::High => trigger.high |= mask,
                TriggerCondition::Low => trigger.low |= mask,
                TriggerCondition::Rise => trigger.rise |= mask,
                TriggerCondition::Fall => trigger.fall |= mask,
                TriggerCondition::Both => trigger.edge |= mask,
            }
        }
        trigger
    }

    fn empty(&self) -> bool {
        self.high | self.low | self.rise | self.fall | self.edge == 0
    }

    fn matches(&self, previous: Option<u32>, sample: u32) -> bool {
        if sample & self.high != self.high || sample & self.low != 0 {
            return false;
        }
        if self.rise | self.fall | self.edge == 0 {
            return true;
        }
        previous.is_some_and(|previous| {
            let changed = previous ^ sample;
            changed & sample & self.rise == self.rise
                && changed & !sample & self.fall == self.fall
                && changed & self.edge == self.edge
        })
    }
}

/// Transpose eight bytes of channel bits to eight bytes of time bits.
fn transpose(mut word: u64) -> u64 {
    let mut swap = (word ^ (word >> 7)) & 0x00aa_00aa_00aa_00aa;
    word ^= swap ^ (swap << 7);
    swap = (word ^ (word >> 14)) & 0x0000_cccc_0000_cccc;
    word ^= swap ^ (swap << 14);
    swap = (word ^ (word >> 28)) & 0x0000_0000_f0f0_f0f0;
    word ^ swap ^ (swap << 28)
}

struct CrossPacker {
    channels: Vec<ChannelIndex>,
    /// Byte planes of a sample up to the highest enabled channel.
    planes: usize,
    pending: [u32; 64],
    used: usize,
    output: Vec<u8>,
}

impl CrossPacker {
    fn new(enabled: u32) -> Self {
        Self {
            channels: (0..32)
                .filter(|channel| enabled & (1 << channel) != 0)
                .collect(),
            planes: enabled
                .checked_ilog2()
                .map_or(0, |highest| highest as usize / 8 + 1),
            pending: [0; 64],
            used: 0,
            output: Vec::new(),
        }
    }

    fn push(&mut self, samples: &[u32], packets: &mut Vec<Packet>) {
        let mut rest = samples;
        while !rest.is_empty() {
            let take = rest.len().min(64 - self.used);
            self.pending[self.used..self.used + take].copy_from_slice(&rest[..take]);
            self.used += take;
            rest = &rest[take..];
            if self.used != 64 {
                continue;
            }
            let mut words = [0u64; 32];
            for (group, samples) in self.pending.as_chunks::<8>().0.iter().enumerate() {
                let mut planes = [0u64; 4];
                for (index, plane) in planes[..self.planes].iter_mut().enumerate() {
                    for (i, &sample) in samples.iter().enumerate() {
                        *plane |= u64::from((sample >> (index * 8)) as u8) << (i * 8);
                    }
                    *plane = transpose(*plane);
                }
                for &channel in &self.channels {
                    let bits = (planes[usize::from(channel / 8)] >> ((channel % 8) * 8)) & 0xff;
                    words[usize::from(channel)] |= bits << (group * 8);
                }
            }
            for &channel in &self.channels {
                self.output
                    .extend_from_slice(&words[usize::from(channel)].to_le_bytes());
            }
            self.used = 0;
            if self.output.len() >= MAX_PACKET {
                self.flush(packets);
            }
        }
    }

    fn flush(&mut self, packets: &mut Vec<Packet>) {
        if !self.output.is_empty() {
            packets.push(Packet::Logic(std::mem::take(&mut self.output)));
        }
    }
}

pub(crate) struct Stream {
    trigger: Trigger,
    previous: Option<u32>,
    waiting: bool,
    pre: VecDeque<u32>,
    pre_limit: usize,
    remaining: Option<u64>,
    packer: CrossPacker,
}

impl Stream {
    pub(crate) fn new(settings: &Settings, plan: &CapturePlan) -> Self {
        let trigger = Trigger::new(
            if plan.instant { &[] } else { &plan.triggers },
            settings.enabled,
        );
        let waiting = !trigger.empty();
        let pre_limit = if waiting {
            (settings
                .limit_samples
                .saturating_mul(u64::from(plan.trigger_position.min(90)))
                / 100)
                .min(MAX_PRE_SAMPLES) as usize
        } else {
            0
        };
        Self {
            trigger,
            previous: None,
            waiting,
            // Grow only as samples arrive, even when a long history was requested.
            pre: VecDeque::new(),
            pre_limit,
            remaining: (!plan.looping).then_some(settings.limit_samples),
            packer: CrossPacker::new(settings.enabled),
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.remaining == Some(0)
    }

    fn emit(&mut self, samples: &[u32], packets: &mut Vec<Packet>) {
        let count = self.remaining.map_or(samples.len(), |left| {
            samples
                .len()
                .min(usize::try_from(left).unwrap_or(usize::MAX))
        });
        self.packer.push(&samples[..count], packets);
        if let Some(remaining) = &mut self.remaining {
            *remaining -= count as u64;
        }
    }

    pub(crate) fn push(&mut self, samples: &[u32]) -> Vec<Packet> {
        let mut packets = Vec::new();
        let mut offset = 0;
        if self.waiting {
            while offset < samples.len() {
                let sample = samples[offset];
                // Fill the requested history before arming, including the preceding
                // sample when an edge lands exactly at the end of the history.
                let matched =
                    self.pre.len() == self.pre_limit && self.trigger.matches(self.previous, sample);
                self.previous = Some(sample);
                if matched {
                    self.waiting = false;
                    packets.push(Packet::Trigger {
                        sample: self.pre.len() as u64,
                    });
                    let pre = std::mem::take(&mut self.pre);
                    let (first, second) = pre.as_slices();
                    self.emit(first, &mut packets);
                    self.emit(second, &mut packets);
                    break;
                }
                if self.pre_limit != 0 {
                    if self.pre.len() == self.pre_limit {
                        self.pre.pop_front();
                    }
                    self.pre.push_back(sample);
                }
                offset += 1;
            }
        }
        if !self.waiting {
            self.emit(&samples[offset..], &mut packets);
        }
        self.packer.flush(&mut packets);
        packets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SLOGIC32U3, UsbSpeed};

    fn settings(enabled: u32, limit: u64) -> Settings {
        let mut settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus);
        settings.enabled = enabled;
        settings.limit_samples = limit;
        settings
    }

    fn logic(packets: &[Packet]) -> Vec<u8> {
        packets
            .iter()
            .flat_map(|packet| match packet {
                Packet::Logic(data) => data.clone(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn expected(samples: &[u32], channels: &[u16]) -> Vec<u8> {
        let mut out = Vec::new();
        for group in samples.as_chunks::<64>().0.iter() {
            for &channel in channels {
                let word = group.iter().enumerate().fold(0u64, |word, (tick, sample)| {
                    word | (u64::from((sample >> channel) & 1) << tick)
                });
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn cross_format_matches_naive_at_all_transfer_boundaries() {
        let samples: Vec<_> = (0u32..320).map(|n| n.wrapping_mul(0x9e37_79b9)).collect();
        for enabled in [
            0xffff,
            0x8001,
            0xf,
            0x100,
            0x55aa,
            u32::MAX,
            0x8000_0001,
            0x00ff_0000,
            0x5555_aaaa,
            1 << 31,
        ] {
            let channels: Vec<_> = (0..32).filter(|c| enabled & (1 << c) != 0).collect();
            let want = expected(&samples[..256], &channels);
            for cut in 1..=129 {
                let mut stream = Stream::new(&settings(enabled, 256), &CapturePlan::default());
                let mut got = Vec::new();
                for part in samples.chunks(cut) {
                    got.extend(logic(&stream.push(part)));
                }
                assert_eq!(got, want, "mask {enabled:x}, cut {cut}");
                assert!(stream.done());
            }
        }
    }

    #[test]
    fn trigger_history_and_edge_are_independent_of_blocks() {
        let samples: Vec<_> = (0..300).map(|n| if n >= 173 { 3 } else { 2 }).collect();
        let plan = CapturePlan {
            triggers: vec![(0, TriggerCondition::Rise), (1, TriggerCondition::High)],
            trigger_position: 25,
            ..Default::default()
        };
        for cut in 1..=180 {
            let mut stream = Stream::new(&settings(3, 128), &plan);
            let mut packets = Vec::new();
            for part in samples.chunks(cut) {
                packets.extend(stream.push(part));
            }
            assert_eq!(
                packets
                    .iter()
                    .filter(|p| matches!(p, Packet::Trigger { .. }))
                    .count(),
                1
            );
            assert!(packets.contains(&Packet::Trigger { sample: 32 }));
            assert_eq!(logic(&packets), expected(&samples[141..269], &[0, 1]));
            assert!(stream.done());
        }
    }

    #[test]
    fn first_sample_cannot_make_an_edge_and_conditions_are_conjunctive() {
        for condition in [
            TriggerCondition::Rise,
            TriggerCondition::Fall,
            TriggerCondition::Both,
        ] {
            let trigger = Trigger::new(&[(0, condition)], 1);
            assert!(!trigger.matches(None, 0));
            assert!(!trigger.matches(None, 1));
        }
        let trigger = Trigger::new(
            &[(0, TriggerCondition::Fall), (1, TriggerCondition::Low)],
            3,
        );
        assert!(trigger.matches(Some(1), 0));
        assert!(!trigger.matches(Some(1), 2));
        assert!(!trigger.matches(Some(0), 0));
    }

    #[test]
    fn triggers_on_the_upper_inputs_of_a_32_channel_sample() {
        let trigger = Trigger::new(
            &[(31, TriggerCondition::Rise), (16, TriggerCondition::Low)],
            u32::MAX,
        );
        assert!(trigger.matches(Some(0), 1 << 31));
        assert!(!trigger.matches(Some(0), 1 << 31 | 1 << 16));
        assert!(!trigger.matches(Some(1 << 31), 1 << 31));
        // Conditions on disabled or missing inputs are ignored.
        assert!(Trigger::new(&[(31, TriggerCondition::High)], 0xffff).empty());
        assert!(Trigger::new(&[(32, TriggerCondition::High)], u32::MAX).empty());
    }

    #[test]
    fn instant_and_looping_bypass_limit_and_trigger_as_requested() {
        let plan = CapturePlan {
            triggers: vec![(0, TriggerCondition::Rise)],
            instant: true,
            looping: true,
            ..Default::default()
        };
        let mut stream = Stream::new(&settings(1, 64), &plan);
        let packets = stream.push(&[1; 192]);
        assert_eq!(logic(&packets), vec![255; 24]);
        assert!(!stream.done());
        assert!(!packets.iter().any(|p| matches!(p, Packet::Trigger { .. })));
    }

    #[test]
    fn history_is_bounded_and_does_not_arm_early() {
        let plan = CapturePlan {
            triggers: vec![(0, TriggerCondition::High)],
            trigger_position: 90,
            ..Default::default()
        };
        let stream = Stream::new(&settings(1, 1 << 34), &plan);
        assert_eq!(stream.pre_limit, MAX_PRE_SAMPLES as usize);
        assert_eq!(stream.pre.capacity(), 0);
        let mut stream = Stream::new(&settings(1, 128), &plan);
        assert!(stream.push(&[1; 115]).is_empty());
        assert!(
            stream
                .push(&[1; 128])
                .contains(&Packet::Trigger { sample: 115 })
        );
    }
}
