// SPDX-License-Identifier: GPL-3.0-or-later
//! Host trigger semantics follow Sipeed's `slogic/slogic_data.c` at
//! 0d728b3e1226a61738c99c3333ad16d5c2814728 (Copyright 2023–2025 Sipeed): one stage, per
//! channel 0, 1, rising, falling or either edge, all ANDed, armed once the history has filled.
//! They are matched here 64 samples at a time on HSync's cross format, which is HSync's own.

use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::{CapturePlan, Packet};

use crate::Settings;
use crate::assemble::{Assembler, BLOCK, Blocks};

// Unlike the reference's 1 GiB allocation, keep at most this much history in the desktop
// application. The marker reports the actual retained history.
pub(crate) const MAX_PRE_SAMPLES: u64 = (64 << 20) / size_of::<u32>() as u64;
const MAX_PACKET: usize = 1 << 20;

/// The word of kept channel `index` in a block of the cross format.
fn word(block: &[u8], index: usize) -> u64 {
    u64::from_le_bytes(
        block[index * 8..index * 8 + 8]
            .try_into()
            .expect("a whole word"),
    )
}

/// A trigger's conditions on the kept channels, matched on a block's words.
struct Trigger {
    /// The channel's place among a block's words, and its condition.
    conditions: Vec<(usize, TriggerCondition)>,
    /// Each condition's word of the previous block, whose last sample comes before the next
    /// block's first; none before the first block, whose first sample makes no edge.
    previous: Option<Vec<u64>>,
}

impl Trigger {
    /// The conditions on kept channels; none, if none of them is.
    fn new(conditions: &[(ChannelIndex, TriggerCondition)], channels: &[u8]) -> Option<Self> {
        let conditions: Vec<_> = conditions
            .iter()
            .filter(|&&(_, condition)| condition != TriggerCondition::None)
            .filter_map(|&(channel, condition)| {
                let index = channels
                    .iter()
                    .position(|&kept| ChannelIndex::from(kept) == channel)?;
                Some((index, condition))
            })
            .collect();
        (!conditions.is_empty()).then_some(Self {
            conditions,
            previous: None,
        })
    }

    /// Bit `t` is set where sample `t` of the block meets every condition.
    fn matches(&mut self, block: &[u8]) -> u64 {
        let previous = self.previous.get_or_insert_with(Vec::new);
        let first = previous.is_empty();
        // The first block's first sample has no predecessor, so it makes no edge.
        let edges = if first { !1 } else { u64::MAX };
        let mut hits = u64::MAX;
        for (place, &(index, condition)) in self.conditions.iter().enumerate() {
            let now = word(block, index);
            let before = now << 1 | previous.get(place).map_or(0, |word| word >> 63);
            hits &= match condition {
                TriggerCondition::None => u64::MAX,
                TriggerCondition::High => now,
                TriggerCondition::Low => !now,
                TriggerCondition::Rise => now & !before & edges,
                TriggerCondition::Fall => !now & before & edges,
                TriggerCondition::Both => (now ^ before) & edges,
            };
        }
        previous.clear();
        previous.extend(self.conditions.iter().map(|&(index, _)| word(block, index)));
        hits
    }
}

/// The latest blocks before the trigger, in a fixed number of slots used round and round, so
/// that whole runs of blocks are kept with a copy or two.
#[derive(Default)]
struct History {
    logic: Vec<u8>,
    codes: Vec<u16>,
    block_bytes: usize,
    block_codes: usize,
    /// Blocks it holds at most, and those it holds.
    slots: usize,
    filled: usize,
    /// The slot of the oldest block.
    oldest: usize,
}

impl History {
    fn new(slots: usize, block_bytes: usize, block_codes: usize) -> Self {
        Self {
            block_bytes,
            block_codes,
            slots,
            ..Self::default()
        }
    }

    /// Adds `count` consecutive blocks, dropping the oldest beyond its slots.
    fn extend(&mut self, mut logic: &[u8], mut codes: &[u16], mut count: usize) {
        if count > self.slots {
            let skip = count - self.slots;
            logic = &logic[skip * self.block_bytes..];
            codes = &codes[skip * self.block_codes..];
            count = self.slots;
        }
        while count > 0 {
            let (bytes, codes_now);
            if self.filled < self.slots {
                // Grow only as blocks arrive, even when a long history was requested.
                let take = count.min(self.slots - self.filled);
                (bytes, codes_now) = (take * self.block_bytes, take * self.block_codes);
                self.logic.extend_from_slice(&logic[..bytes]);
                self.codes.extend_from_slice(&codes[..codes_now]);
                self.filled += take;
                count -= take;
            } else {
                let take = count.min(self.slots - self.oldest);
                (bytes, codes_now) = (take * self.block_bytes, take * self.block_codes);
                let (at, at_codes) = (
                    self.oldest * self.block_bytes,
                    self.oldest * self.block_codes,
                );
                self.logic[at..at + bytes].copy_from_slice(&logic[..bytes]);
                self.codes[at_codes..at_codes + codes_now].copy_from_slice(&codes[..codes_now]);
                self.oldest = (self.oldest + take) % self.slots;
                count -= take;
            }
            logic = &logic[bytes..];
            codes = &codes[codes_now..];
        }
    }

    /// Block `index` of those held, oldest first.
    fn block(&self, index: usize) -> (&[u8], &[u16]) {
        let slot = (self.oldest + index) % self.slots;
        (
            &self.logic[slot * self.block_bytes..(slot + 1) * self.block_bytes],
            &self.codes[slot * self.block_codes..(slot + 1) * self.block_codes],
        )
    }
}

/// The cross-format stream a capture sends: the trigger, the history before it and the limit,
/// applied to whole blocks of 64 samples.
pub(crate) struct Stream {
    assembler: Assembler,
    /// Reused for each transfer's blocks.
    blocks: Blocks,
    /// Bytes of logic and codes in a block.
    block_bytes: usize,
    block_codes: usize,
    /// The trigger, while the capture waits for it.
    trigger: Option<Trigger>,
    pre_limit: u64,
    /// While waiting: the latest blocks, enough to hold the history.
    history: History,
    /// Blocks of the stream seen so far.
    seen: u64,
    /// After the trigger: how far the first sent sample lies into its block. The blocks sent
    /// are then shifted by as much, each completed by the start of the next.
    shift: usize,
    held: Option<(Vec<u8>, Vec<u16>)>,
    /// Samples still to send, in whole blocks; none when looping.
    remaining: Option<u64>,
    output: Vec<u8>,
    codes: Vec<u16>,
}

impl Stream {
    /// The stream of wire mode `wire` (2, 4, 8, 16 or 32 channels from D0) with these
    /// settings, after a firmware head of `head_bytes` once per RUN.
    pub(crate) fn new(
        settings: &Settings,
        plan: &CapturePlan,
        wire: u8,
        head_bytes: usize,
    ) -> Self {
        let assembler =
            Assembler::new(wire, head_bytes, settings.logic(), settings.analog_groups());
        let trigger = if plan.instant {
            None
        } else {
            Trigger::new(&plan.triggers, assembler.channels())
        };
        let pre_limit = if trigger.is_some() {
            (settings
                .limit_samples
                .saturating_mul(u64::from(plan.trigger_position.min(90)))
                / 100)
                .min(MAX_PRE_SAMPLES)
        } else {
            0
        };
        let (block_bytes, block_codes) =
            (assembler.channels().len() * 8, assembler.groups() * BLOCK);
        Self {
            block_bytes,
            block_codes,
            assembler,
            blocks: Blocks::default(),
            trigger,
            pre_limit,
            history: History::new(
                pre_limit.div_ceil(BLOCK as u64) as usize,
                block_bytes,
                block_codes,
            ),
            seen: 0,
            shift: 0,
            held: None,
            remaining: (!plan.looping).then_some(settings.limit_samples / 64 * 64),
            output: Vec::new(),
            codes: Vec::new(),
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.remaining == Some(0)
    }

    /// Adds a transfer's bytes and returns the packets they complete.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<Packet> {
        let mut packets = Vec::new();
        let mut blocks = std::mem::take(&mut self.blocks);
        self.assembler.push(bytes, &mut blocks);
        let (logic, codes) = (self.block_bytes, self.block_codes);
        let mut index = 0;
        while index < blocks.count && !self.done() {
            if self.trigger.is_none() && self.shift == 0 {
                // Untriggered or triggered on a block boundary: the blocks go as they are.
                let run = self.remaining.map_or(blocks.count - index, |remaining| {
                    (blocks.count - index).min((remaining / BLOCK as u64) as usize)
                });
                let end = index + run;
                self.send(
                    &blocks.logic[index * logic..end * logic],
                    &blocks.codes[index * codes..end * codes],
                    run,
                    &mut packets,
                );
                self.seen += run as u64;
                index = end;
                continue;
            }
            if self.trigger.is_some() {
                // Find the trigger in the transfer's blocks; those before it join the history
                // at once.
                let from = index;
                let mut tick = None;
                while index < blocks.count {
                    tick = self.find(&blocks.logic[index * logic..(index + 1) * logic]);
                    if tick.is_some() {
                        break;
                    }
                    index += 1;
                }
                self.history.extend(
                    &blocks.logic[from * logic..index * logic],
                    &blocks.codes[from * codes..index * codes],
                    index - from,
                );
                let Some(tick) = tick else {
                    break;
                };
                self.fire(
                    tick,
                    &blocks.logic[index * logic..(index + 1) * logic],
                    &blocks.codes[index * codes..(index + 1) * codes],
                    &mut packets,
                );
                index += 1;
                continue;
            }
            // Triggered within a block: each goes shifted.
            self.seen += 1;
            self.emit(
                &blocks.logic[index * logic..(index + 1) * logic],
                &blocks.codes[index * codes..(index + 1) * codes],
                &mut packets,
            );
            index += 1;
        }
        self.blocks = blocks;
        self.flush(&mut packets);
        packets
    }

    /// Where the trigger fires in the next block of the stream, if it does.
    fn find(&mut self, logic: &[u8]) -> Option<u64> {
        let start = self.seen * BLOCK as u64;
        self.seen += 1;
        let trigger = self.trigger.as_mut().expect("waiting for the trigger");
        let mut hits = trigger.matches(logic);
        // Armed once the history holds `pre_limit` samples.
        if let Some(unarmed) = self.pre_limit.checked_sub(start) {
            hits &= u64::MAX.checked_shl(unarmed as u32).unwrap_or(0);
        }
        (hits != 0).then(|| start + u64::from(hits.trailing_zeros()))
    }

    /// Fires the trigger at sample `tick` of the block just found: sends the history from the
    /// block holding its first sample, then this block.
    fn fire(&mut self, tick: u64, logic: &[u8], codes: &[u16], packets: &mut Vec<Packet>) {
        let first = tick - self.pre_limit;
        packets.push(Packet::Trigger {
            sample: self.pre_limit,
        });
        self.trigger = None;
        self.shift = (first % BLOCK as u64) as usize;
        let history = std::mem::take(&mut self.history);
        let oldest = self.seen - 1 - history.filled as u64;
        let skip = (first / BLOCK as u64 - oldest) as usize;
        for index in skip..history.filled {
            let (logic, codes) = history.block(index);
            self.emit(logic, codes, packets);
        }
        self.emit(logic, codes, packets);
    }

    /// Sends a block of the stream from the first sent sample on, shifted to start there.
    fn emit(&mut self, logic: &[u8], codes: &[u16], packets: &mut Vec<Packet>) {
        if self.shift == 0 {
            self.send(logic, codes, 1, packets);
            return;
        }
        let Some((mut held_logic, mut held_codes)) = self.held.take() else {
            self.held = Some((logic.to_vec(), codes.to_vec()));
            return;
        };
        let (shift, groups) = (self.shift, self.block_codes / BLOCK);
        let shifted: Vec<u8> = held_logic
            .as_chunks::<8>()
            .0
            .iter()
            .zip(logic.as_chunks::<8>().0)
            .flat_map(|(&held, &next)| {
                (u64::from_le_bytes(held) >> shift | u64::from_le_bytes(next) << (BLOCK - shift))
                    .to_le_bytes()
            })
            .collect();
        let mut shifted_codes = held_codes[shift * groups..].to_vec();
        shifted_codes.extend_from_slice(&codes[..shift * groups]);
        self.send(&shifted, &shifted_codes, 1, packets);
        held_logic.copy_from_slice(logic);
        held_codes.copy_from_slice(codes);
        self.held = Some((held_logic, held_codes));
    }

    /// Sends `blocks` whole blocks, as far as the limit allows, in packets of at most
    /// [`MAX_PACKET`] bytes each (at least a block).
    fn send(
        &mut self,
        mut logic: &[u8],
        mut codes: &[u16],
        mut blocks: usize,
        packets: &mut Vec<Packet>,
    ) {
        if let Some(remaining) = self.remaining {
            // The history and the trigger's block may hold more than the limit.
            blocks = blocks.min((remaining / BLOCK as u64) as usize);
        }
        if let Some(remaining) = &mut self.remaining {
            *remaining -= (blocks * BLOCK) as u64;
        }
        while blocks > 0 {
            let room = |used: usize, per_block: usize, limit: usize| {
                limit
                    .saturating_sub(used)
                    .checked_div(per_block)
                    .unwrap_or(usize::MAX)
            };
            let fit = room(self.output.len(), self.block_bytes, MAX_PACKET).min(room(
                self.codes.len() * 2,
                self.block_codes * 2,
                MAX_PACKET,
            ));
            if fit == 0 {
                self.flush(packets);
                continue;
            }
            let take = fit.min(blocks);
            self.output
                .extend_from_slice(&logic[..take * self.block_bytes]);
            self.codes
                .extend_from_slice(&codes[..take * self.block_codes]);
            logic = &logic[take * self.block_bytes..];
            codes = &codes[take * self.block_codes..];
            blocks -= take;
        }
    }

    /// Each block's logic comes before its codes.
    fn flush(&mut self, packets: &mut Vec<Packet>) {
        if !self.output.is_empty() {
            packets.push(Packet::Logic(std::mem::take(&mut self.output)));
        }
        if !self.codes.is_empty() {
            packets.push(Packet::Analog(std::mem::take(&mut self.codes)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::{self, Expected};
    use crate::{SLOGIC32U3, UsbSpeed};

    fn settings(wire: u8, enabled: u32, limit: u64) -> Settings {
        let mut settings = Settings::defaults(&SLOGIC32U3, UsbSpeed::SuperPlus).with_wire(wire);
        settings.enabled = enabled;
        settings.limit_samples = limit;
        settings
    }

    /// What the stream sends for `bytes`, cut into transfers of the given lengths in turn.
    fn capture(
        settings: &Settings,
        plan: &CapturePlan,
        head: usize,
        bytes: &[u8],
        cuts: &[usize],
    ) -> Expected {
        let mut stream = Stream::new(settings, plan, settings.channels(), head);
        let (mut logic, mut codes, mut trigger) = (Vec::new(), Vec::new(), None);
        let mut rest = bytes;
        for &cut in cuts.iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (part, after) = rest.split_at(cut.min(rest.len()));
            rest = after;
            for packet in stream.push(part) {
                match packet {
                    Packet::Logic(data) => logic.extend(data),
                    Packet::Analog(data) => codes.extend(data),
                    Packet::Trigger { sample } => {
                        assert!(trigger.is_none() && logic.is_empty(), "one trigger, first");
                        trigger = Some(sample);
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        }
        Expected {
            logic,
            codes,
            trigger,
            done: stream.done(),
        }
    }

    /// A small xorshift generator, so that the cases are the same on every run.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
    }

    #[test]
    fn matches_the_samples_way_for_any_wire_trigger_history_limit_and_cut() {
        let conditions = [
            TriggerCondition::High,
            TriggerCondition::Low,
            TriggerCondition::Rise,
            TriggerCondition::Fall,
            TriggerCondition::Both,
            TriggerCondition::None,
        ];
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        for case in 0..600 {
            let wire = [2, 4, 8, 16, 32][case % 5];
            let all = u32::MAX >> (32 - wire);
            let enabled = (random.next() as u32 & all).max(1 << random.below(u64::from(wire)));
            let mut settings = settings(
                wire,
                enabled,
                [64, 640, 1024, 1984][random.below(4) as usize],
            );
            if wire >= 8 && random.below(3) == 0 {
                settings.analog = random.next() as u8 & ((1 << (wire / 8)) - 1);
            }
            let triggers = (0..random.below(4))
                .map(|_| {
                    (
                        random.below(u64::from(wire) + 2) as ChannelIndex,
                        conditions[random.below(6) as usize],
                    )
                })
                .collect();
            let plan = CapturePlan {
                triggers,
                trigger_position: [0, 1, 5, 10, 33, 50, 90][random.below(7) as usize],
                instant: random.below(8) == 0,
                looping: random.below(6) == 0,
                ..CapturePlan::default()
            };
            // Signals that change now and then, so that triggers fire anywhere.
            let mut sample = random.next() as u32 & all;
            let samples: Vec<u32> = (0..random.below(3000) + 64)
                .map(|_| {
                    for channel in 0..wire {
                        if random.below(12) == 0 {
                            sample ^= 1 << channel;
                        }
                    }
                    sample
                })
                .collect();
            let head = [0, 4][random.below(2) as usize];
            let mut bytes = vec![0xa5; head];
            bytes.extend(reference::encode(wire, &samples));
            let want = reference::run(&settings, &plan, head, &bytes);
            for cuts in [
                vec![bytes.len()],
                vec![1],
                vec![3, 64, 7],
                (0..8).map(|_| random.below(300) as usize + 1).collect(),
            ] {
                assert_eq!(
                    capture(&settings, &plan, head, &bytes, &cuts),
                    want,
                    "case {case}: {wire} wire, enabled {enabled:#x}, analog {:#b}, {plan:?}, \
                     {} samples, head {head}, cuts {cuts:?}",
                    settings.analog,
                    samples.len()
                );
            }
        }
    }

    #[test]
    fn trigger_history_and_edge_are_independent_of_blocks() {
        let samples: Vec<_> = (0..400).map(|n| if n >= 173 { 3 } else { 2 }).collect();
        let bytes = reference::encode(8, &samples);
        let plan = CapturePlan {
            triggers: vec![(0, TriggerCondition::Rise), (1, TriggerCondition::High)],
            trigger_position: 25,
            ..Default::default()
        };
        let want: Vec<u8> = reference::run(&settings(8, 3, 128), &plan, 0, &bytes).logic;
        assert_eq!(want.len(), 2 * 2 * 8);
        for cut in 1..=180 {
            let got = capture(&settings(8, 3, 128), &plan, 0, &bytes, &[cut]);
            assert_eq!(got.trigger, Some(32), "cut {cut}");
            // From sample 141, 32 before the edge at 173.
            assert_eq!(got.logic, want, "cut {cut}");
            assert_eq!(got.logic[..8], (!0u64 << 32).to_le_bytes());
            assert!(got.done);
        }
    }

    #[test]
    fn first_sample_cannot_make_an_edge_and_conditions_are_conjunctive() {
        let ones = reference::encode(8, &[1; 128]);
        for condition in [
            TriggerCondition::Rise,
            TriggerCondition::Fall,
            TriggerCondition::Both,
        ] {
            let plan = CapturePlan {
                triggers: vec![(0, condition)],
                ..Default::default()
            };
            assert_eq!(
                capture(&settings(8, 1, 64), &plan, 0, &ones, &[128]).trigger,
                None
            );
        }
        // D0 falls at sample 10 while D1 is high, and at sample 70 while D1 is low.
        let samples: Vec<u32> = (0..256)
            .map(|n| match n {
                0..10 => 3,
                10..60 => 2,
                60..70 => 1,
                _ => 0,
            })
            .collect();
        let plan = CapturePlan {
            triggers: vec![(0, TriggerCondition::Fall), (1, TriggerCondition::Low)],
            ..Default::default()
        };
        let got = capture(
            &settings(8, 3, 64),
            &plan,
            0,
            &reference::encode(8, &samples),
            &[256],
        );
        assert_eq!(got.trigger, Some(0));
        assert_eq!(
            got.logic,
            reference::run(
                &settings(8, 3, 64),
                &plan,
                0,
                &reference::encode(8, &samples)
            )
            .logic
        );
        assert_eq!(got.logic[..8], [0; 8]);
    }

    #[test]
    fn triggers_on_the_upper_inputs_of_a_32_channel_sample() {
        let samples: Vec<u32> = (0..256)
            .map(|n| match n {
                100.. => 1 << 31,
                _ => 1 << 16,
            })
            .collect();
        let plan = CapturePlan {
            triggers: vec![(31, TriggerCondition::Rise), (16, TriggerCondition::Low)],
            ..Default::default()
        };
        let got = capture(
            &settings(32, u32::MAX, 64),
            &plan,
            0,
            &reference::encode(32, &samples),
            &[1000],
        );
        assert_eq!(got.trigger, Some(0));
        // D31 from sample 100 on.
        assert_eq!(got.logic[31 * 8..32 * 8], u64::MAX.to_le_bytes());
        // Conditions on disabled or missing inputs are ignored.
        for channel in [31, 32] {
            let plan = CapturePlan {
                triggers: vec![(channel, TriggerCondition::High)],
                ..Default::default()
            };
            let got = capture(
                &settings(32, 0xffff, 64),
                &plan,
                0,
                &reference::encode(32, &samples),
                &[1000],
            );
            assert_eq!(got.trigger, None, "D{channel}");
            assert!(got.done);
        }
    }

    #[test]
    fn instant_and_looping_bypass_limit_and_trigger_as_requested() {
        let plan = CapturePlan {
            triggers: vec![(0, TriggerCondition::Rise)],
            instant: true,
            looping: true,
            ..Default::default()
        };
        let got = capture(&settings(8, 1, 64), &plan, 0, &[1; 192], &[192]);
        assert_eq!(got.logic, vec![255; 24]);
        assert!(!got.done);
        assert_eq!(got.trigger, None);
    }

    #[test]
    fn history_is_bounded_and_does_not_arm_early() {
        let plan = CapturePlan {
            triggers: vec![(0, TriggerCondition::High)],
            trigger_position: 90,
            ..Default::default()
        };
        let stream = Stream::new(&settings(8, 1, 1 << 34), &plan, 8, 0);
        assert_eq!(stream.pre_limit, MAX_PRE_SAMPLES);
        assert_eq!(stream.history.logic.capacity(), 0);
        let mut stream = Stream::new(&settings(8, 1, 128), &plan, 8, 0);
        // 115 samples of history must come first; the blocks hold 64.
        assert!(stream.push(&[1; 64]).is_empty());
        assert!(
            stream
                .push(&[1; 128])
                .contains(&Packet::Trigger { sample: 115 })
        );
    }

    #[test]
    fn large_transfers_keep_packets_bounded_and_stop_at_the_exact_sample_limit() {
        // After HSync a72e8b3's test of the same name.
        let input = vec![255; MAX_PACKET * 2];
        let count = MAX_PACKET as u64 * 2 + 64;
        let settings = settings(4, 0xf, count);
        let mut stream = Stream::new(&settings, &CapturePlan::default(), 4, 0);
        let packets = stream.push(&input);
        assert!(stream.done());
        let mut bytes = 0;
        for packet in &packets {
            let Packet::Logic(data) = packet else {
                panic!("logic only");
            };
            assert!(data.len() <= MAX_PACKET);
            assert!(data.iter().all(|&byte| byte == 255));
            bytes += data.len();
        }
        // Four channels: 32 bytes of the cross format a block of 64 samples.
        assert_eq!(bytes as u64 * 2, count);
        assert!(stream.push(&input).is_empty());
        // Analog codes are bounded too, beside their logic.
        let mut settings = super::tests::settings(32, 0xffff_0000, u64::MAX / 2);
        settings.analog = 0b11;
        let mut stream = Stream::new(&settings, &CapturePlan::default(), 32, 0);
        for packet in stream.push(&vec![0; MAX_PACKET * 4]) {
            match packet {
                Packet::Logic(data) => assert!(data.len() <= MAX_PACKET),
                Packet::Analog(codes) => assert!(codes.len() * 2 <= MAX_PACKET),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn reads_analog_groups_as_codes_beside_the_logic_inputs() {
        // Inputs 0–7 carry a code MSB first, inputs 16–23 another; 8–15 stay logic.
        let samples: Vec<u32> = (0u32..256)
            .map(|n| {
                u32::from((n as u8).reverse_bits())
                    | u32::from((255 - n as u8).reverse_bits()) << 16
                    | (n & 3) << 8
            })
            .collect();
        let bytes = reference::encode(32, &samples);
        let mut settings = settings(32, 0xff_ffff, 128);
        settings.analog = 0b101;
        let codes: Vec<u16> = (0..128).flat_map(|n| [n, 255 - n]).collect();
        for cut in [1, 63, 64, 100, 1024] {
            let mut stream =
                Stream::new(&settings, &CapturePlan::default(), settings.channels(), 0);
            let mut packets = Vec::new();
            for part in bytes.chunks(cut) {
                packets.extend(stream.push(part));
            }
            assert!(
                matches!(packets.first(), Some(Packet::Logic(_))),
                "cut {cut}"
            );
            let got = capture(&settings, &CapturePlan::default(), 0, &bytes, &[cut]);
            assert_eq!(got.codes, codes, "cut {cut}");
            assert_eq!(
                got,
                reference::run(&settings, &CapturePlan::default(), 0, &bytes)
            );
            assert!(stream.done());
        }
        // Analog channels alone.
        settings.enabled = 0;
        let got = capture(&settings, &CapturePlan::default(), 0, &bytes, &[1024]);
        assert!(got.logic.is_empty());
        assert_eq!(got.codes, codes);
    }
}
