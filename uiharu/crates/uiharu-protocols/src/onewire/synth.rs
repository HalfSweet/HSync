// SPDX-License-Identifier: Apache-2.0

//! Making 1-Wire waveforms: for tests, for tools and for benchmarks.
//!
//! [`OneWireWaveform`] lays out resets and time slots on the data line at a chosen sample
//! rate, with the pulse lengths of a [`Timing`] — by default those application note 126
//! recommends, with the slave's answers well inside the datasheets' windows. Generators of
//! higher protocols build lists of [`Slot`]s for it rather than samples.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick, TimeBase};

use super::link::{LinkConfig, Speed};

/// One step of the bus: a reset, or a time slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// The master's reset pulse, answered by a presence pulse when `presence`.
    Reset {
        /// Whether a slave answers.
        presence: bool,
    },
    /// A write slot: the master sends the bit.
    Write(bool),
    /// A read slot: the master starts it and the slaves answer the bit, holding the line low
    /// for a 0.
    Read(bool),
}

/// The pulse lengths a waveform uses, in nanoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timing {
    /// The master's low for a write-1.
    pub write_one_low: u64,
    /// The master's low for a write-0.
    pub write_zero_low: u64,
    /// The master's low that starts a read slot.
    pub read_low: u64,
    /// How long the line stays low in a read slot a slave answers with 0, from the fall.
    pub read_zero_low: u64,
    /// A time slot, from its fall to the next one.
    pub slot: u64,
    /// The master's reset pulse.
    pub reset_low: u64,
    /// From the release of the reset to the presence pulse.
    pub presence_wait: u64,
    /// The presence pulse.
    pub presence_low: u64,
    /// From the release of the reset to the first slot after it.
    pub reset_high: u64,
}

impl Timing {
    /// Standard speed.
    pub const STANDARD: Timing = Timing {
        write_one_low: 6_000,
        write_zero_low: 60_000,
        read_low: 6_000,
        read_zero_low: 30_000,
        slot: 70_000,
        reset_low: 500_000,
        presence_wait: 30_000,
        presence_low: 120_000,
        reset_high: 480_000,
    };

    /// Overdrive.
    pub const OVERDRIVE: Timing = Timing {
        write_one_low: 1_000,
        write_zero_low: 7_500,
        read_low: 1_000,
        read_zero_low: 3_000,
        slot: 10_000,
        reset_low: 70_000,
        presence_wait: 3_000,
        presence_low: 12_000,
        reset_high: 48_000,
    };

    /// The timing of `speed`.
    pub const fn of(speed: Speed) -> Timing {
        match speed {
            Speed::Standard => Timing::STANDARD,
            Speed::Overdrive => Timing::OVERDRIVE,
        }
    }
}

/// Builds the samples of a 1-Wire bus, one reset or slot at a time; the line idles high and
/// every other channel stays low.
#[derive(Debug)]
pub struct OneWireWaveform {
    builder: LogicBlockBuilder,
    data: u8,
    rate_hz: u64,
    timing: Timing,
}

impl OneWireWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, sampled on `time_base`,
    /// starting at `start`, with the timing of the configured speed.
    pub fn new(config: LinkConfig, channels: u8, time_base: TimeBase, start: Tick) -> Self {
        Self {
            builder: LogicBlockBuilder::new(start, channels),
            data: config.data,
            rate_hz: time_base.rate_hz,
            timing: Timing::of(config.speed),
        }
    }

    /// The pulse lengths in use.
    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// Uses `timing` from here on.
    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = timing;
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    /// `ns` nanoseconds in ticks, to the nearest, at least one.
    pub fn ticks(&self, ns: u64) -> u64 {
        let ticks = (u128::from(ns) * u128::from(self.rate_hz) + 500_000_000) / 1_000_000_000;
        u64::try_from(ticks).unwrap_or(u64::MAX).max(1)
    }

    fn hold(&mut self, high: bool, ticks: u64) {
        let levels = if high {
            Levels(1 << self.data)
        } else {
            Levels(0)
        };
        self.builder.push_repeated(levels, ticks);
    }

    /// Holds the line high for `ticks` samples.
    pub fn idle(&mut self, ticks: u64) {
        self.hold(true, ticks);
    }

    /// Holds the line low for `ticks` samples.
    pub fn low(&mut self, ticks: u64) {
        self.hold(false, ticks);
    }

    /// A low pulse of `low` ticks, then the line high for `high` ticks: any slot, bad slots
    /// and window edges included.
    pub fn pulse(&mut self, low: u64, high: u64) {
        self.hold(false, low);
        self.hold(true, high);
    }

    /// A low pulse of `low_ns` nanoseconds in a time slot of the usual length (longer, when
    /// the pulse is): a bad slot when it fits no window.
    pub fn bad_slot(&mut self, low_ns: u64) {
        let (low, slot) = (self.ticks(low_ns), self.ticks(self.timing.slot));
        self.pulse(low, slot.saturating_sub(low).max(1));
    }

    /// One reset or time slot.
    pub fn slot(&mut self, slot: Slot) {
        let timing = self.timing;
        let (low_ns, slot_ns) = match slot {
            Slot::Reset { presence } => {
                let release = self.ticks(timing.reset_high);
                self.low(self.ticks(timing.reset_low));
                if presence {
                    let wait = self.ticks(timing.presence_wait);
                    let pulse = self.ticks(timing.presence_low);
                    self.pulse(0, wait);
                    self.pulse(pulse, release.saturating_sub(wait + pulse).max(1));
                } else {
                    self.idle(release);
                }
                return;
            }
            Slot::Write(true) => (timing.write_one_low, timing.slot),
            Slot::Write(false) => (timing.write_zero_low, timing.slot),
            Slot::Read(true) => (timing.read_low, timing.slot),
            Slot::Read(false) => (timing.read_low.max(timing.read_zero_low), timing.slot),
        };
        let (low, slot) = (self.ticks(low_ns), self.ticks(slot_ns));
        self.pulse(low, slot.saturating_sub(low).max(1));
    }

    /// Resets and slots in order.
    pub fn slots(&mut self, slots: impl IntoIterator<Item = Slot>) {
        for slot in slots {
            self.slot(slot);
        }
    }

    /// A reset, answered when `presence`.
    pub fn reset(&mut self, presence: bool) {
        self.slot(Slot::Reset { presence });
    }

    /// A write slot carrying `value`.
    pub fn bit(&mut self, value: bool) {
        self.slot(Slot::Write(value));
    }

    /// A read slot the slaves answer with `value`.
    pub fn read_bit(&mut self, value: bool) {
        self.slot(Slot::Read(value));
    }

    /// Eight write slots, least significant bit first.
    pub fn byte(&mut self, value: u8) {
        self.slots(write_byte(value));
    }

    /// Eight read slots answered with `value`, least significant bit first.
    pub fn read_byte(&mut self, value: u8) {
        self.slots(read_byte(value));
    }

    /// A ROM code the master sends (after Match ROM): 64 write slots.
    pub fn rom(&mut self, id: u64) {
        self.slots(write_rom(id));
    }

    /// A ROM code the slave sends (after Read ROM): 64 read slots.
    pub fn read_rom(&mut self, id: u64) {
        self.slots(read_rom(id));
    }

    /// The 64 triplets of one Search ROM pass among the devices `ids`; see [`search_rom`].
    /// Returns the ROM code found.
    pub fn search_rom(&mut self, ids: &[u64]) -> Option<u64> {
        let (slots, found) = search_rom(ids);
        self.slots(slots);
        found
    }

    /// The samples so far; at least one must have been produced.
    pub fn finish(self) -> LogicBlock {
        self.builder.finish().unwrap_or_else(|_| {
            LogicBlockBuilder::new(Tick::ZERO, 1)
                .finish()
                .expect("no samples were produced")
        })
    }

    /// The samples so far, if any.
    pub fn try_finish(self) -> Option<LogicBlock> {
        self.builder.finish().ok()
    }
}

/// Eight write slots carrying `value`, least significant bit first.
pub fn write_byte(value: u8) -> impl Iterator<Item = Slot> {
    (0..8).map(move |n| Slot::Write(value >> n & 1 != 0))
}

/// Eight read slots answered with `value`, least significant bit first.
pub fn read_byte(value: u8) -> impl Iterator<Item = Slot> {
    (0..8).map(move |n| Slot::Read(value >> n & 1 != 0))
}

/// 64 write slots carrying the ROM code `id`, its bit 0 first.
pub fn write_rom(id: u64) -> impl Iterator<Item = Slot> {
    (0..64).map(move |n| Slot::Write(id >> n & 1 != 0))
}

/// 64 read slots answered with the ROM code `id`, its bit 0 first.
pub fn read_rom(id: u64) -> impl Iterator<Item = Slot> {
    (0..64).map(move |n| Slot::Read(id >> n & 1 != 0))
}

/// The 64 triplets of one Search ROM pass among the devices `ids`, and the ROM code found.
///
/// Every device still in the search sends its bit and then the complement; the line carries
/// the AND of what they send. Where they disagree the master takes the 0 branch, and the
/// devices whose bit differs from the direction it writes leave the search. With no device,
/// or none left, both reads are 1 and the master writes 1; nothing is found then.
pub fn search_rom(ids: &[u64]) -> (Vec<Slot>, Option<u64>) {
    let mut active = ids.to_vec();
    let mut slots = Vec::with_capacity(3 * 64);
    let mut found = 0u64;
    for n in 0..64 {
        let any_one = active.iter().any(|id| id >> n & 1 == 1);
        let any_zero = active.iter().any(|id| id >> n & 1 == 0);
        let (bit, complement) = (!any_zero, !any_one);
        let direction = match (bit, complement) {
            (false, false) => false,
            (bit, _) => bit,
        };
        slots.extend([
            Slot::Read(bit),
            Slot::Read(complement),
            Slot::Write(direction),
        ]);
        active.retain(|id| (id >> n & 1 == 1) == direction);
        found |= u64::from(direction) << n;
    }
    (slots, (!active.is_empty()).then_some(found))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_follows_the_zero_branch() {
        let (slots, found) = search_rom(&[0b0110, 0b0010]);
        assert_eq!(found, Some(0b0010));
        assert_eq!(slots.len(), 192);
        // Bit 0: both devices send 0.
        assert_eq!(
            slots[..3],
            [Slot::Read(false), Slot::Read(true), Slot::Write(false)]
        );
        // Bit 2: they disagree, and the master takes 0.
        assert_eq!(
            slots[6..9],
            [Slot::Read(false), Slot::Read(false), Slot::Write(false)]
        );
        let (_, none) = search_rom(&[]);
        assert_eq!(none, None);
    }

    #[test]
    fn durations_round_to_the_nearest_tick() {
        let config = LinkConfig::default();
        let waveform = OneWireWaveform::new(config, 1, TimeBase::new(3_000_000), Tick::ZERO);
        assert_eq!(waveform.ticks(6_000), 18);
        assert_eq!(waveform.ticks(7_500), 23); // 22.5
        assert_eq!(waveform.ticks(1), 1);
    }
}
