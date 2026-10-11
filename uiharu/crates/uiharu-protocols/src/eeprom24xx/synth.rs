// SPDX-License-Identifier: Apache-2.0

//! The I²C traffic of 24xx EEPROM commands, as symbols for
//! [`I2cWaveform::transaction`](crate::i2c::synth::I2cWaveform::transaction): for tests, tools
//! and benchmarks.

use crate::i2c::synth::{Symbol, read_7bit, write_7bit};

/// The word address as the bus carries it: `address_bytes` bytes, most significant first.
fn word_address(address_bytes: u8, word: u16) -> Vec<u8> {
    let [high, low] = word.to_be_bytes();
    if address_bytes >= 2 {
        vec![high, low]
    } else {
        vec![low]
    }
}

/// A write to `device` of the word address `word`, `address_bytes` bytes wide, and `data`: a
/// set-address without data, a byte write with one byte, a page write with more.
pub fn write(device: u8, address_bytes: u8, word: u16, data: &[u8]) -> Vec<Symbol> {
    let mut bytes = word_address(address_bytes, word);
    bytes.extend_from_slice(data);
    write_7bit(device, &bytes)
}

/// A current-address read of `data` from `device`, the controller acknowledging every byte but
/// the last.
pub fn current_address_read(device: u8, data: &[u8]) -> Vec<Symbol> {
    read_7bit(device, data)
}

/// A random read of `data` from `word`: a set-address, a repeated START and a current-address
/// read.
pub fn random_read(device: u8, address_bytes: u8, word: u16, data: &[u8]) -> Vec<Symbol> {
    let mut symbols = write(device, address_bytes, word, &[]);
    symbols.push(Symbol::Restart);
    symbols.extend(read_7bit(device, data));
    symbols
}

/// An acknowledge poll: the control byte of a write alone, acknowledged once the device is
/// ready.
pub fn ack_poll(device: u8, acked: bool) -> Vec<Symbol> {
    vec![Symbol::Byte {
        value: device << 1,
        ack: acked,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(symbols: &[Symbol]) -> Vec<String> {
        symbols
            .iter()
            .map(|symbol| match symbol {
                Symbol::Byte { value, ack: true } => format!("{value:02x}"),
                Symbol::Byte { value, ack: false } => format!("{value:02x}!"),
                Symbol::Restart => "Sr".to_owned(),
            })
            .collect()
    }

    #[test]
    fn commands_are_laid_out_as_the_datasheets_show() {
        assert_eq!(
            bytes(&write(0x50, 2, 0x1234, &[0xaa])),
            ["a0", "12", "34", "aa"]
        );
        assert_eq!(bytes(&write(0x51, 1, 0x1234, &[])), ["a2", "34"]);
        assert_eq!(
            bytes(&random_read(0x57, 2, 0x0102, &[1, 2])),
            ["ae", "01", "02", "Sr", "af", "01", "02!"]
        );
        assert_eq!(bytes(&current_address_read(0x50, &[9])), ["a1", "09!"]);
        assert_eq!(bytes(&ack_poll(0x50, false)), ["a0!"]);
    }
}
