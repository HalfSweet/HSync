//! The demo's analog channels: two sine waves, as a 12-bit converter over ±5 V would read them.

use std::f64::consts::TAU;

use hsync_core::{ANALOG_BASE, AnalogScale, ChannelIndex};

/// Codes of the demo's converter.
const CODES: u16 = 4096;
/// Its range, in volts.
const LOW: f32 = -5.0;
const HIGH: f32 = 5.0;
const SCALE: AnalogScale = AnalogScale {
    step: (HIGH - LOW) / (CODES - 1) as f32,
    offset: LOW,
};

struct Sine {
    hertz: u64,
    centre: f64,
    amplitude: f64,
}

/// A0: 1 kHz between 0 and 3.3 V; A1: 3 kHz, ±2.5 V.
const SINES: [Sine; 2] = [
    Sine {
        hertz: 1_000,
        centre: 1.65,
        amplitude: 1.65,
    },
    Sine {
        hertz: 3_000,
        centre: 0.0,
        amplitude: 2.5,
    },
];

/// The analog channels the demo captures and how their codes become volts.
pub(crate) fn channels() -> Vec<(ChannelIndex, AnalogScale)> {
    (0..SINES.len() as ChannelIndex)
        .map(|n| (ANALOG_BASE + n, SCALE))
        .collect()
}

fn code(volts: f64) -> u16 {
    let code = ((volts - f64::from(SCALE.offset)) / f64::from(SCALE.step)).round();
    code.clamp(0.0, f64::from(CODES - 1)) as u16
}

/// Appends `samples` samples from sample `first` on, one code per channel for each sample in
/// turn. Each call starts from the exact phase of `first`, so long captures do not drift.
pub(crate) fn fill(first: u64, samples: usize, samplerate: u64, out: &mut Vec<u16>) {
    let begin = out.len();
    out.resize(begin + samples * SINES.len(), 0);
    let rate = u128::from(samplerate.max(1));
    for (position, sine) in SINES.iter().enumerate() {
        // The phase of sample `first`, in turns, from integers so it stays exact.
        let turns = (u128::from(first) * u128::from(sine.hertz) % rate) as f64 / rate as f64;
        let step = TAU * sine.hertz as f64 / samplerate.max(1) as f64;
        // sin(x + step) = 2 cos(step) sin(x) - sin(x - step), restarted each call.
        let twice_cos = 2.0 * step.cos();
        let mut before = (TAU * turns - step).sin();
        let mut now = (TAU * turns).sin();
        for sample in 0..samples {
            out[begin + sample * SINES.len() + position] = code(sine.centre + sine.amplitude * now);
            let next = twice_cos * now - before;
            before = now;
            now = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volts(code: u16) -> f64 {
        f64::from(SCALE.volts(code))
    }

    #[test]
    fn sines_reach_their_peaks_at_the_right_samples() {
        // 1 MHz: a quarter of A0's period is 250 samples, of A1's 83⅓.
        let mut codes = Vec::new();
        fill(0, 1000, 1_000_000, &mut codes);
        assert_eq!(codes.len(), 2000);
        assert!((volts(codes[0]) - 1.65).abs() < 0.01);
        assert!((volts(codes[250 * 2]) - 3.3).abs() < 0.01);
        assert!((volts(codes[750 * 2]) - 0.0).abs() < 0.01);
        assert!((volts(codes[1]) - 0.0).abs() < 0.01);
        let peak = codes.iter().skip(1).step_by(2).map(|&c| volts(c));
        let peak = peak.fold(f64::MIN, f64::max);
        assert!((peak - 2.5).abs() < 0.01);
    }

    #[test]
    fn a_later_start_continues_the_same_wave() {
        let mut whole = Vec::new();
        fill(0, 4096, 25_000_000, &mut whole);
        let mut parts = Vec::new();
        for first in (0..4096).step_by(64) {
            fill(first, 64, 25_000_000, &mut parts);
        }
        let difference = whole.iter().zip(&parts).map(|(&a, &b)| a.abs_diff(b)).max();
        assert!(difference <= Some(1));
    }
}
