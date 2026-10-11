// SPDX-License-Identifier: Apache-2.0

//! Bit timing in ticks, in integer arithmetic.
//!
//! On a source of `R` ticks per second a bit at `B` bits per second lasts `R / B` ticks. The
//! fraction is kept exact: a position is a phase in hundredths of a bit, counted from a tick
//! where the phase is known (the anchor: an edge the bits were synchronised to, or the sample
//! point the bit rate switched at), and it becomes a tick by rounding down once,
//! `anchor + ⌊(phase − phase at the anchor) · R / (100 · B)⌋`. Rounding therefore never
//! accumulates from one bit to the next.

use uiharu_core::Tick;

/// Where the bits of one bit rate fall, counted from the last synchronisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Clock {
    /// A tick whose phase is known.
    anchor: Tick,
    /// The phase at `anchor`, in hundredths of a bit.
    phase0: u64,
    /// Ticks per second.
    rate: u64,
    /// Bits per second.
    bitrate: u64,
}

impl Clock {
    /// Bits at `bitrate` on a source of `rate` ticks per second, with `anchor` at `phase0`
    /// hundredths of a bit.
    pub(super) fn new(anchor: Tick, phase0: u64, rate: u64, bitrate: u64) -> Self {
        Self {
            anchor,
            phase0,
            rate,
            bitrate: bitrate.max(1),
        }
    }

    /// The bit rate.
    pub(super) fn bitrate(&self) -> u64 {
        self.bitrate
    }

    /// The tick at `phase` hundredths of a bit, rounded down; phases before the anchor's are
    /// placed at the anchor.
    pub(super) fn tick(&self, phase: u64) -> Tick {
        let elapsed = u128::from(phase.saturating_sub(self.phase0)) * u128::from(self.rate)
            / (100 * u128::from(self.bitrate));
        self.anchor
            .offset(u64::try_from(elapsed).unwrap_or(u64::MAX))
    }

    /// The first bit whose sample point, `sample` hundredths of a bit into it, lies at or after
    /// `tick`; bits are counted like the anchor's phase, from the bit that holds phase 0.
    pub(super) fn first_sampled_from(&self, tick: Tick, sample: u64) -> u64 {
        // tick(100 k + sample) ≥ tick  ⟺  100 k + sample ≥ phase0 + ⌈(tick − anchor) · 100 · B / R⌉
        let ticks = u128::from(self.anchor.until(tick));
        let needed =
            (ticks * 100 * u128::from(self.bitrate)).div_ceil(u128::from(self.rate.max(1)));
        let phase = needed + u128::from(self.phase0);
        let bit = phase.saturating_sub(u128::from(sample)).div_ceil(100);
        u64::try_from(bit).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_ticks_per_bit_place_bits_exactly() {
        let clock = Clock::new(Tick(1000), 0, 20_000_000, 500_000);
        assert_eq!(clock.tick(0), Tick(1000));
        assert_eq!(clock.tick(87), Tick(1034)); // 40 · 0.87 = 34.8, rounded down
        assert_eq!(clock.tick(100), Tick(1040));
        assert_eq!(clock.tick(10 * 100 + 87), Tick(1434));
    }

    #[test]
    fn fractional_bit_times_do_not_drift() {
        // 10 MHz at 3 Mbit/s: 3⅓ ticks per bit. Bit 30 starts exactly 100 ticks later.
        let clock = Clock::new(Tick(0), 0, 10_000_000, 3_000_000);
        assert_eq!(clock.tick(100), Tick(3));
        assert_eq!(clock.tick(200), Tick(6));
        assert_eq!(clock.tick(300), Tick(10));
        assert_eq!(clock.tick(3000), Tick(100));
    }

    #[test]
    fn a_switch_at_the_sample_point_keeps_the_rest_of_the_bit() {
        // The data phase begins at a sample point at 80 %: the rest of that bit is 20 % of a
        // data bit, 2 ticks at 10 ticks per bit.
        let clock = Clock::new(Tick(500), 80, 20_000_000, 2_000_000);
        assert_eq!(clock.tick(80), Tick(500));
        assert_eq!(clock.tick(100), Tick(502));
        assert_eq!(clock.tick(180), Tick(510));
        assert_eq!(
            clock.tick(50),
            Tick(500),
            "phases before the anchor stay at it"
        );
    }

    #[test]
    fn finds_the_first_bit_sampled_after_a_tick() {
        for (rate, bitrate, phase0, sample) in [
            (20_000_000, 500_000, 0, 87),
            (10_000_000, 3_000_000, 0, 75),
            (20_000_000, 2_000_000, 80, 80),
            (1_000_003, 9_601, 0, 50),
            (16_000_000, 1_000_000, 87, 87),
        ] {
            let clock = Clock::new(Tick(77), phase0, rate, bitrate);
            for tick in 0..400 {
                let tick = Tick(tick);
                let found = clock.first_sampled_from(tick, sample);
                let brute = (0..)
                    .find(|bit: &u64| {
                        bit * 100 + sample >= phase0 && clock.tick(bit * 100 + sample) >= tick
                    })
                    .expect("some bit is sampled later");
                assert_eq!(found, brute, "{rate} Hz, {bitrate} bit/s, from {tick}");
            }
        }
    }
}
