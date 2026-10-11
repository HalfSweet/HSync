// SPDX-License-Identifier: Apache-2.0

//! The Microwire bits of 93Cxx instructions, for the Microwire synthesizer
//! ([`crate::microwire::synth::MicrowireWaveform`]): pairs of the DI and DO levels of each
//! clock pulse. DO is high — released — wherever the device does not drive it.

use super::decoder::Eeprom93xxConfig;

/// The bits of `value`'s lowest `count` bits, most significant first.
fn bits(value: u16, count: u8) -> impl Iterator<Item = bool> {
    (0..count.min(16))
        .rev()
        .map(move |bit| value >> bit & 1 != 0)
}

/// The start bit, the opcode and the address bits, on DI.
fn instruction(config: &Eeprom93xxConfig, opcode: [bool; 2], address: u16) -> Vec<(bool, bool)> {
    let mut di = vec![true, opcode[0], opcode[1]];
    di.extend(bits(address, config.address_bits));
    di.into_iter().map(|bit| (bit, true)).collect()
}

/// The words of `values` on DI, as WRITE and WRAL send them.
fn words_in(config: &Eeprom93xxConfig, values: &[u16]) -> Vec<(bool, bool)> {
    let data_bits = config.organisation.data_bits();
    values
        .iter()
        .flat_map(|value| bits(*value, data_bits))
        .map(|bit| (bit, true))
        .collect()
}

/// READ at `address`: the instruction, the dummy 0 the device drives on DO with the address's
/// last bit, then `words` on DO — more than one is a sequential read.
pub fn read(config: &Eeprom93xxConfig, address: u16, words: &[u16]) -> Vec<(bool, bool)> {
    let mut pulses = instruction(config, [true, false], address);
    if let Some(last) = pulses.last_mut() {
        last.1 = false;
    }
    let data_bits = config.organisation.data_bits();
    pulses.extend(
        words
            .iter()
            .flat_map(|word| bits(*word, data_bits))
            .map(|bit| (false, bit)),
    );
    pulses
}

/// WRITE of `value` at `address`.
pub fn write(config: &Eeprom93xxConfig, address: u16, value: u16) -> Vec<(bool, bool)> {
    let mut pulses = instruction(config, [false, true], address);
    pulses.extend(words_in(config, &[value]));
    pulses
}

/// ERASE at `address`.
pub fn erase(config: &Eeprom93xxConfig, address: u16) -> Vec<(bool, bool)> {
    instruction(config, [true, true], address)
}

/// The address field of the instructions with opcode `00`: the two top bits select the
/// instruction, the rest are zeros.
fn selector(config: &Eeprom93xxConfig, top: u16) -> u16 {
    top << (config.address_bits - 2)
}

/// EWEN: erasing and writing enabled.
pub fn ewen(config: &Eeprom93xxConfig) -> Vec<(bool, bool)> {
    instruction(config, [false, false], selector(config, 0b11))
}

/// EWDS: erasing and writing disabled.
pub fn ewds(config: &Eeprom93xxConfig) -> Vec<(bool, bool)> {
    instruction(config, [false, false], selector(config, 0b00))
}

/// ERAL: the whole array erased.
pub fn eral(config: &Eeprom93xxConfig) -> Vec<(bool, bool)> {
    instruction(config, [false, false], selector(config, 0b10))
}

/// WRAL: `value` written to every word.
pub fn wral(config: &Eeprom93xxConfig, value: u16) -> Vec<(bool, bool)> {
    let mut pulses = instruction(config, [false, false], selector(config, 0b01));
    pulses.extend(words_in(config, &[value]));
    pulses
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eeprom93xx::Organisation;

    fn text(pulses: &[(bool, bool)], line: fn(&(bool, bool)) -> bool) -> String {
        pulses
            .iter()
            .map(|pulse| if line(pulse) { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn instructions_follow_the_datasheet_tables() {
        let config = Eeprom93xxConfig::default();
        let di = |pulses: &[(bool, bool)]| text(pulses, |pulse| pulse.0);
        assert_eq!(di(&erase(&config, 0x2a)), "111101010");
        assert_eq!(di(&ewen(&config)), "100110000");
        assert_eq!(di(&ewds(&config)), "100000000");
        assert_eq!(di(&eral(&config)), "100100000");
        assert_eq!(di(&wral(&config, 0xa001)), "1000100001010000000000001");
        let x8 = Eeprom93xxConfig {
            address_bits: 7,
            organisation: Organisation::X8,
        };
        assert_eq!(di(&write(&x8, 0x7f, 0x5a)), "101111111101011010");
    }

    #[test]
    fn a_read_drives_a_dummy_zero_then_the_words() {
        let config = Eeprom93xxConfig::default();
        let pulses = read(&config, 1, &[0x8001]);
        assert_eq!(pulses.len(), 9 + 16);
        assert_eq!(text(&pulses, |pulse| pulse.1), "1111111101000000000000001");
    }
}
