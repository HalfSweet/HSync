// SPDX-License-Identifier: GPL-3.0-or-later
// After Sipeed's ngscopeclient driver: sipeed/scopehal, branch feat/SLogic, commit
// 57363c8e5463762da1fbbaaf954d9c2e73b2c0a3, scopehal/SLogicOscilloscope.cpp
// (`AdcValue`, `GetSampleRatesNonInterleaved`, `AcquireData`) and the `clock_base_mhz`
// setting of scopehal/slogic/slogic.c (Copyright (C) 2023-2025 Shenzhen Sipeed Technology
// Co., Ltd.). The facts are ported; the code is HSync's own.
//! The SLogic32U3's ADC module, read on the host: each group of eight inputs it is wired to
//! carries one 8-bit converter's output, sampled like any other input.

use hsync_core::AnalogScale;

/// Inputs in a group the module reads as one analog channel, D0 upwards.
pub const GROUP: u8 = 8;
/// The volts of code 0 and code 255: the module's fixed ±15 V range.
pub const LOW: f32 = -15.0;
pub const HIGH: f32 = 15.0;
/// How codes become volts, linear over the range.
pub const SCALE: AnalogScale = AnalogScale {
    step: (HIGH - LOW) / 255.0,
    offset: LOW,
};
/// The PLL that also clocks the converter; the sample clock must come from it while the
/// module is read, as the 1400 MHz PLL's phase bears no relation to the converter's.
pub const CLOCK_MHZ: u16 = 800;
/// The module needs at least this sample rate.
const MIN_RATE: u64 = 100_000_000;

/// The converter's code in a group's byte. The module wires its data bus MSB first, input
/// 0 of the group carrying D7, so the byte is the code with its bits reversed.
pub fn code(byte: u8) -> u16 {
    u16::from(byte.reverse_bits())
}

/// Which of `rates` the module allows: those its PLL divides down to, from its minimum up,
/// or only the slower ones where the link cannot carry its minimum.
pub fn samplerates(rates: &[u64]) -> Vec<u64> {
    let clock = u64::from(CLOCK_MHZ) * 1_000_000;
    let (slow, fast): (Vec<u64>, Vec<u64>) = rates
        .iter()
        .copied()
        .filter(|&rate| clock.is_multiple_of(rate))
        .partition(|&rate| rate < MIN_RATE);
    if fast.is_empty() { slow } else { fast }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SLOGIC32U3;

    #[test]
    fn reads_the_code_msb_first_over_fifteen_volts_either_side() {
        assert_eq!(code(0b0000_0001), 0x80);
        assert_eq!(code(0b1100_0000), 0x03);
        assert_eq!(SCALE.volts(0), -15.0);
        assert!((SCALE.volts(255) - 15.0).abs() < 1e-4);
        assert!(SCALE.volts(128).abs() < 0.06);
    }

    #[test]
    fn keeps_the_rates_of_the_800_mhz_clock_from_100_mhz() {
        let mhz = |rates: Vec<u64>| {
            rates
                .iter()
                .map(|rate| rate / 1_000_000)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            mhz(samplerates(SLOGIC32U3.samplerates)),
            [100, 160, 200, 400, 800]
        );
        // Over USB 2.0 every input together stays below 100 MHz.
        let slow: Vec<_> = SLOGIC32U3.samplerates[..3].to_vec();
        assert_eq!(mhz(samplerates(&slow)), [5, 8, 10]);
    }
}
