// SPDX-License-Identifier: Apache-2.0

//! Making DS18B20 traffic: scratchpads and the slots of whole transactions, for
//! [`OneWireWaveform`](crate::onewire::synth::OneWireWaveform).
//!
//! Every transaction begins with a reset the device answers and addresses it with Skip ROM,
//! or with Match ROM when a ROM code is given; its end is the next reset.

use super::events::{
    CONVERT_T, READ_POWER_SUPPLY, READ_SCRATCHPAD, SCRATCHPAD_LEN, WRITE_SCRATCHPAD,
};
use crate::onewire::rom::{MATCH_ROM, SKIP_ROM, crc8};
use crate::onewire::synth::{Slot, read_byte, write_byte, write_rom};

/// A scratchpad holding these registers, the reserved bytes as a DS18B20 leaves them
/// (`FF`, `0C`, `10`) and the CRC-8 of the first eight bytes.
pub fn scratchpad(temperature_raw: i16, th: i8, tl: i8, config: u8) -> [u8; SCRATCHPAD_LEN] {
    let [lsb, msb] = temperature_raw.to_le_bytes();
    let [th] = th.to_le_bytes();
    let [tl] = tl.to_le_bytes();
    let mut bytes = [lsb, msb, th, tl, config, 0xff, 0x0c, 0x10, 0];
    bytes[SCRATCHPAD_LEN - 1] = crc8(&bytes[..SCRATCHPAD_LEN - 1]);
    bytes
}

/// A reset the device answers, then Skip ROM, or Match ROM with `rom`, then the function
/// command `code`.
pub fn command(rom: Option<u64>, code: u8) -> Vec<Slot> {
    let mut slots = vec![Slot::Reset { presence: true }];
    match rom {
        Some(id) => {
            slots.extend(write_byte(MATCH_ROM));
            slots.extend(write_rom(id));
        }
        None => slots.extend(write_byte(SKIP_ROM)),
    }
    slots.extend(write_byte(code));
    slots
}

/// Convert T.
pub fn convert_t(rom: Option<u64>) -> Vec<Slot> {
    command(rom, CONVERT_T)
}

/// Read Scratchpad, the master reading as many bytes as `bytes` holds.
pub fn read_scratchpad(rom: Option<u64>, bytes: &[u8]) -> Vec<Slot> {
    let mut slots = command(rom, READ_SCRATCHPAD);
    for byte in bytes {
        slots.extend(read_byte(*byte));
    }
    slots
}

/// Write Scratchpad with TH, TL and the configuration.
pub fn write_scratchpad(rom: Option<u64>, th: i8, tl: i8, config: u8) -> Vec<Slot> {
    let mut slots = command(rom, WRITE_SCRATCHPAD);
    for byte in [th.to_le_bytes()[0], tl.to_le_bytes()[0], config] {
        slots.extend(write_byte(byte));
    }
    slots
}

/// Read Power Supply and the read slot after it, pulled low by a parasite-powered device.
pub fn read_power_supply(rom: Option<u64>, parasite: bool) -> Vec<Slot> {
    let mut slots = command(rom, READ_POWER_SUPPLY);
    slots.push(Slot::Read(!parasite));
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_power_up_scratchpad_has_its_known_crc() {
        // 85 °C, TH 75, TL 70, 12 bits: the scratchpad a DS18B20 holds after power-up.
        assert_eq!(
            scratchpad(0x0550, 75, 70, 0x7f),
            [0x50, 0x05, 0x4b, 0x46, 0x7f, 0xff, 0x0c, 0x10, 0x1c]
        );
    }

    #[test]
    fn transactions_are_addressed_as_asked() {
        let skip = convert_t(None);
        assert_eq!(skip.len(), 1 + 8 + 8);
        assert_eq!(skip[0], Slot::Reset { presence: true });
        let matched = read_power_supply(Some(0x28), true);
        assert_eq!(matched.len(), 1 + 8 + 64 + 8 + 1);
        assert_eq!(matched.last(), Some(&Slot::Read(false)));
    }
}
