// SPDX-License-Identifier: GPL-3.0-or-later
//! Host trigger semantics follow Sipeed's `slogic/slogic_data.c` at
//! 0d728b3e1226a61738c99c3333ad16d5c2814728 (Copyright 2023–2025 Sipeed).
//! The cross-format conversion is HSync's own; the device sends packed samples.

use std::collections::VecDeque;
use std::marker::PhantomData;

use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::{CapturePlan, Packet};

use crate::assemble::{SampleWord, Samples, WireFormat};
use crate::{Settings, adc};

// Keep the existing history duration cap: at most 16,777,216 samples. Native-width
// storage needs at most 64 MiB, and proportionally less for narrower wire formats.
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

/// Pre-trigger samples in native words; 2/4-bit samples share each u8, including at the head.
struct History<F: WireFormat> {
    words: VecDeque<F::Word>,
    head: usize,
    len: usize,
}

impl<F: WireFormat> Default for History<F> {
    fn default() -> Self {
        Self {
            words: VecDeque::new(),
            head: 0,
            len: 0,
        }
    }
}

impl<F: WireFormat> History<F> {
    fn push(&mut self, sample: F::Word) {
        let offset = (self.head + self.len) % F::PER_WORD;
        if offset == 0 {
            self.words.push_back(sample);
        } else {
            let last = self.words.back_mut().expect("partly filled history word");
            *last = F::Word::from_u32((*last).into() | (sample.into() << (offset * F::BITS)));
        }
        self.len += 1;
    }

    fn pop(&mut self) -> Option<F::Word> {
        if self.len == 0 {
            return None;
        }
        let word = *self.words.front().expect("history contains its samples");
        let sample = if F::PER_WORD == 1 {
            word
        } else {
            F::Word::from_u32((word.into() >> (self.head * F::BITS)) & ((1 << F::BITS) - 1))
        };
        self.len -= 1;
        self.head += 1;
        if self.head == F::PER_WORD {
            self.words.pop_front();
            self.head = 0;
        }
        Some(sample)
    }
}

/// Extract one channel from a packed 64-bit word, compacting the selected bits in time order.
/// These masks are a bit transpose, not a per-sample unpack into a wider integer array.
fn packed_channel<const BITS: usize>(mut word: u64) -> u64 {
    if BITS == 2 {
        word &= 0x5555_5555_5555_5555;
        word = (word | word >> 1) & 0x3333_3333_3333_3333;
        word = (word | word >> 2) & 0x0f0f_0f0f_0f0f_0f0f;
        word = (word | word >> 4) & 0x00ff_00ff_00ff_00ff;
        word = (word | word >> 8) & 0x0000_ffff_0000_ffff;
        (word | word >> 16) & 0xffff_ffff
    } else {
        word &= 0x1111_1111_1111_1111;
        word = (word | word >> 3) & 0x0303_0303_0303_0303;
        word = (word | word >> 6) & 0x000f_000f_000f_000f;
        word = (word | word >> 12) & 0x0000_00ff_0000_00ff;
        (word | word >> 24) & 0xffff
    }
}

/// Convert 64 samples in their wire representation to the existing capture packet format.
fn pack_block<F: WireFormat>(
    bytes: &[u8],
    channels: &[ChannelIndex],
    groups: &[u8],
    output: &mut Vec<u8>,
    codes: &mut Vec<u16>,
) {
    if F::BITS < 8 {
        for &channel in channels {
            let mut word = 0u64;
            for (part, raw) in bytes.as_chunks::<8>().0.iter().enumerate() {
                let raw = u64::from_le_bytes(*raw) >> channel;
                let bits = if F::BITS == 2 {
                    packed_channel::<2>(raw)
                } else {
                    packed_channel::<4>(raw)
                };
                word |= bits << (part * (64 / F::BITS));
            }
            output.extend_from_slice(&word.to_le_bytes());
        }
    } else if let Some(&last) = channels.last() {
        let mut words = [0u64; 32];
        for group in 0..8 {
            let mut planes = [0u64; 4];
            for (plane_index, plane) in planes[..usize::from(last / 8) + 1].iter_mut().enumerate() {
                for sample in 0..8 {
                    *plane |= u64::from(bytes[(group * 8 + sample) * F::Word::BYTES + plane_index])
                        << (sample * 8);
                }
                *plane = transpose(*plane);
            }
            for &channel in channels {
                let bits = (planes[usize::from(channel / 8)] >> ((channel % 8) * 8)) & 0xff;
                words[usize::from(channel)] |= bits << (group * 8);
            }
        }
        for &channel in channels {
            output.extend_from_slice(&words[usize::from(channel)].to_le_bytes());
        }
    }
    // ADC groups are whole bytes of each native sample; no full-sample expansion is needed.
    if !groups.is_empty() {
        for sample in 0..64 {
            for &group in groups {
                codes.push(adc::code(
                    bytes[sample * F::Word::BYTES + usize::from(group)],
                ));
            }
        }
    }
}

/// Complete blocks transpose directly from the transfer. Only a partial or bit-offset block
/// uses the bounded staging array, which also keeps logic and analog output aligned.
struct CrossPacker<F: WireFormat> {
    channels: Vec<ChannelIndex>,
    groups: Vec<u8>,
    pending: [u8; 256],
    used: usize,
    output: Vec<u8>,
    codes: Vec<u16>,
    format: PhantomData<F>,
}

impl<F: WireFormat> CrossPacker<F> {
    fn new(enabled: u32, groups: Vec<u8>) -> Self {
        Self {
            channels: (0..32)
                .filter(|channel| enabled & (1 << channel) != 0)
                .collect(),
            groups,
            pending: [0; 256],
            used: 0,
            output: Vec::new(),
            codes: Vec::new(),
            format: PhantomData,
        }
    }

    fn sample(&mut self, sample: F::Word, packets: &mut Vec<Packet>) {
        let index = self.used / F::PER_WORD * F::Word::BYTES;
        let offset = self.used % F::PER_WORD;
        if offset == 0 {
            sample.write(&mut self.pending[index..index + F::Word::BYTES]);
        } else {
            self.pending[index] |= (sample.into() << (offset * F::BITS)) as u8;
        }
        self.used += 1;
        if self.used == 64 {
            pack_block::<F>(
                &self.pending[..8 * F::BITS],
                &self.channels,
                &self.groups,
                &mut self.output,
                &mut self.codes,
            );
            self.used = 0;
            self.flush_full(packets);
        }
    }

    fn push(&mut self, mut samples: Samples<'_, F>, packets: &mut Vec<Packet>) {
        let blocks = (samples.len() + self.used) / 64;
        self.output
            .reserve((blocks * self.channels.len() * 8).min(MAX_PACKET));
        self.codes
            .reserve((blocks * self.groups.len() * 64).min(MAX_PACKET / 2));
        if self.used > 0 {
            let take = samples.len().min(64 - self.used);
            for index in 0..take {
                self.sample(samples.at(index), packets);
            }
            samples = samples.slice(take, samples.len());
        }
        while samples.len() >= 64 {
            let block = if let Some(block) = samples.block() {
                block
            } else {
                samples.copy_block(&mut self.pending[..8 * F::BITS]);
                &self.pending[..8 * F::BITS]
            };
            pack_block::<F>(
                block,
                &self.channels,
                &self.groups,
                &mut self.output,
                &mut self.codes,
            );
            self.flush_full(packets);
            samples = samples.slice(64, samples.len());
        }
        // Only the tail needs individual samples; trigger-offset blocks above stay packed.
        for index in 0..samples.len() {
            self.sample(samples.at(index), packets);
        }
    }

    fn flush_full(&mut self, packets: &mut Vec<Packet>) {
        if self.output.len().max(self.codes.len() * 2) >= MAX_PACKET {
            self.flush(packets);
        }
    }

    fn flush(&mut self, packets: &mut Vec<Packet>) {
        if !self.output.is_empty() {
            packets.push(Packet::Logic(std::mem::take(&mut self.output)));
        }
        if !self.codes.is_empty() {
            packets.push(Packet::Analog(std::mem::take(&mut self.codes)));
        }
    }
}

pub(crate) struct Stream<F: WireFormat> {
    trigger: Trigger,
    previous: Option<u32>,
    waiting: bool,
    pre: History<F>,
    pre_limit: usize,
    remaining: Option<u64>,
    packer: CrossPacker<F>,
}

impl<F: WireFormat> Stream<F> {
    pub(crate) fn new(settings: &Settings, plan: &CapturePlan) -> Self {
        let trigger = Trigger::new(
            if plan.instant { &[] } else { &plan.triggers },
            settings.logic(),
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
            pre: History::default(),
            pre_limit,
            remaining: (!plan.looping).then_some(settings.limit_samples),
            packer: CrossPacker::new(settings.logic(), settings.analog_groups()),
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.remaining == Some(0)
    }

    pub(crate) fn push(&mut self, samples: Samples<'_, F>, packets: &mut Vec<Packet>) {
        let mut offset = 0;
        if self.waiting {
            while offset < samples.len() {
                let sample = samples.at(offset);
                // Widen only the current sample for mask comparisons, never the transfer or
                // its history. Fill the requested history before arming, as before.
                let matched = self.pre.len == self.pre_limit
                    && self.trigger.matches(self.previous, sample.into());
                self.previous = Some(sample.into());
                if matched {
                    self.waiting = false;
                    packets.push(Packet::Trigger {
                        sample: self.pre.len as u64,
                    });
                    let mut pre = std::mem::take(&mut self.pre);
                    while let Some(sample) = pre.pop() {
                        if self.done() {
                            break;
                        }
                        self.packer.sample(sample, packets);
                        if let Some(remaining) = &mut self.remaining {
                            *remaining -= 1;
                        }
                    }
                    break;
                }
                if self.pre_limit != 0 {
                    if self.pre.len == self.pre_limit {
                        self.pre.pop();
                    }
                    self.pre.push(sample);
                }
                offset += 1;
            }
        }
        if !self.waiting {
            let count = self.remaining.map_or(samples.len() - offset, |left| {
                (samples.len() - offset).min(usize::try_from(left).unwrap_or(usize::MAX))
            });
            self.packer
                .push(samples.slice(offset, offset + count), packets);
            if let Some(remaining) = &mut self.remaining {
                *remaining -= count as u64;
            }
        }
        self.packer.flush(packets);
    }
}

#[cfg(test)]
#[path = "stream/tests.rs"]
mod tests;
