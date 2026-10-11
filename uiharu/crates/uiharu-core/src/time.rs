// SPDX-License-Identifier: Apache-2.0

//! Time as decoders see it: ticks on a source's sample clock, tick ranges and the time base
//! that turns them into seconds. Decoders never work in seconds; everything they emit is placed
//! in ticks so that results stay exact and comparable.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A sample index on a source's clock, counted from the first sample the source produced.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub u64);

impl Tick {
    /// The first tick of a source.
    pub const ZERO: Tick = Tick(0);

    /// The tick's index.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The tick after this one.
    pub const fn next(self) -> Tick {
        Tick(self.0.saturating_add(1))
    }

    /// The tick `n` samples later.
    pub const fn offset(self, n: u64) -> Tick {
        Tick(self.0.saturating_add(n))
    }

    /// How many samples lie between this tick and a later one; zero when `later` is not later.
    pub const fn until(self, later: Tick) -> u64 {
        later.0.saturating_sub(self.0)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for Tick {
    fn from(index: u64) -> Self {
        Tick(index)
    }
}

/// A half-open range of ticks, `start..end`. Serialized as the pair `[start, end]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TickRange {
    /// The first tick in the range.
    pub start: Tick,
    /// The first tick after the range.
    pub end: Tick,
}

impl TickRange {
    /// The ticks `start..end`; an `end` before `start` is clamped to `start`.
    pub fn new(start: Tick, end: Tick) -> Self {
        Self {
            start,
            end: end.max(start),
        }
    }

    /// The empty range at `tick`, used for results decided at one instant.
    pub const fn at(tick: Tick) -> Self {
        Self {
            start: tick,
            end: tick,
        }
    }

    /// The single tick `tick`.
    pub const fn one(tick: Tick) -> Self {
        Self {
            start: tick,
            end: Tick(tick.0.saturating_add(1)),
        }
    }

    /// The number of ticks in the range.
    pub const fn len(&self) -> u64 {
        self.start.until(self.end)
    }

    /// Whether the range holds no tick.
    pub const fn is_empty(&self) -> bool {
        self.start.0 >= self.end.0
    }

    /// Whether `tick` lies in the range.
    pub const fn contains(&self, tick: Tick) -> bool {
        self.start.0 <= tick.0 && tick.0 < self.end.0
    }

    /// The smallest range covering both; an empty range only contributes its position.
    pub fn join(self, other: TickRange) -> TickRange {
        TickRange {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

impl fmt::Display for TickRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl Serialize for TickRange {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (self.start, self.end).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TickRange {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (start, end) = <(Tick, Tick)>::deserialize(deserializer)?;
        Ok(TickRange::new(start, end))
    }
}

/// The clock of a source: how many ticks make a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeBase {
    /// Samples per second.
    pub rate_hz: u64,
}

impl TimeBase {
    /// A time base of `rate_hz` samples per second.
    pub const fn new(rate_hz: u64) -> Self {
        Self { rate_hz }
    }

    /// The time of `tick` in seconds from the first sample.
    pub fn seconds(&self, tick: Tick) -> f64 {
        if self.rate_hz == 0 {
            return 0.0;
        }
        tick.0 as f64 / self.rate_hz as f64
    }

    /// The tick nearest to `seconds` after the first sample.
    pub fn tick_at(&self, seconds: f64) -> Tick {
        Tick((seconds * self.rate_hz as f64).round().max(0.0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_half_open_and_clamped() {
        let range = TickRange::new(Tick(10), Tick(14));
        assert_eq!(range.len(), 4);
        assert!(range.contains(Tick(10)));
        assert!(!range.contains(Tick(14)));
        assert!(TickRange::new(Tick(5), Tick(2)).is_empty());
        assert_eq!(TickRange::new(Tick(5), Tick(2)).end, Tick(5));
        assert_eq!(TickRange::one(Tick(7)).len(), 1);
    }

    #[test]
    fn join_covers_both() {
        let joined = TickRange::new(Tick(10), Tick(12)).join(TickRange::at(Tick(20)));
        assert_eq!(joined, TickRange::new(Tick(10), Tick(20)));
    }

    #[test]
    fn ranges_serialize_as_pairs() {
        let json = serde_json::to_string(&TickRange::new(Tick(1), Tick(3))).expect("serializes");
        assert_eq!(json, "[1,3]");
        let back: TickRange = serde_json::from_str("[1,3]").expect("parses");
        assert_eq!(back, TickRange::new(Tick(1), Tick(3)));
    }

    #[test]
    fn time_base_converts_both_ways() {
        let base = TimeBase::new(1_000_000);
        assert_eq!(base.seconds(Tick(2_500_000)), 2.5);
        assert_eq!(base.tick_at(0.0000015), Tick(2));
        assert_eq!(TimeBase::new(0).seconds(Tick(9)), 0.0);
    }
}
