// SPDX-License-Identifier: Apache-2.0

//! Word lists of SPI NOR commands for [`SpiWaveform`](crate::spi::synth::SpiWaveform), one
//! builder per command family: the host's bytes on MOSI, the flash's on MISO. Where a line
//! carries nothing, MISO reads `0xFF` (the flash leaves it to its pull-up) and MOSI `0x00`
//! (the host only clocks).

/// What MISO reads while the flash drives nothing.
const FLASH_IDLE: u8 = 0xff;

/// What MOSI carries while the host only clocks: dummy bytes and the flash's answers.
const HOST_IDLE: u8 = 0x00;

/// The low `bytes` bytes of `address`, most significant first.
fn address_words(address: u32, bytes: u8) -> impl Iterator<Item = (u8, u8)> {
    let skip = 4 - usize::from(bytes.min(4));
    address
        .to_be_bytes()
        .into_iter()
        .skip(skip)
        .map(|byte| (byte, FLASH_IDLE))
}

fn dummy_words(dummy: u8) -> impl Iterator<Item = (u8, u8)> {
    (0..dummy).map(|_| (HOST_IDLE, FLASH_IDLE))
}

fn from_flash(bytes: &[u8]) -> impl Iterator<Item = (u8, u8)> + '_ {
    bytes.iter().map(|byte| (HOST_IDLE, *byte))
}

fn from_host(bytes: &[u8]) -> impl Iterator<Item = (u8, u8)> + '_ {
    bytes.iter().map(|byte| (*byte, FLASH_IDLE))
}

/// A command without operands, the opcode alone: Write Enable, Chip Erase, Enter 4-Byte
/// Address Mode, Reset …
pub fn command(opcode: u8) -> Vec<(u8, u8)> {
    vec![(opcode, FLASH_IDLE)]
}

/// An addressed command without data, such as an erase or a block lock: the opcode and the
/// low `address_bytes` bytes of `address`, most significant first.
pub fn addressed(opcode: u8, address: u32, address_bytes: u8) -> Vec<(u8, u8)> {
    std::iter::once((opcode, FLASH_IDLE))
        .chain(address_words(address, address_bytes))
        .collect()
}

/// A read — `READ`, `FAST_READ`, their four-byte forms, SFDP, a security register: the
/// opcode, the address, `dummy` dummy bytes, then `data` from the flash on MISO.
pub fn read(opcode: u8, address: u32, address_bytes: u8, dummy: u8, data: &[u8]) -> Vec<(u8, u8)> {
    let mut words = addressed(opcode, address, address_bytes);
    words.extend(dummy_words(dummy));
    words.extend(from_flash(data));
    words
}

/// A program — `PP`, `PP4B`, a security register: the opcode, the address, then `data` from
/// the host on MOSI.
pub fn program(opcode: u8, address: u32, address_bytes: u8, data: &[u8]) -> Vec<(u8, u8)> {
    let mut words = addressed(opcode, address, address_bytes);
    words.extend(from_host(data));
    words
}

/// A status register read: the opcode, then `values` from the flash on MISO, one after the
/// other as the host keeps reading.
pub fn read_status(opcode: u8, values: &[u8]) -> Vec<(u8, u8)> {
    let mut words = command(opcode);
    words.extend(from_flash(values));
    words
}

/// A status register write: the opcode, then `values` from the host on MOSI.
pub fn write_status(opcode: u8, values: &[u8]) -> Vec<(u8, u8)> {
    let mut words = command(opcode);
    words.extend(from_host(values));
    words
}

/// An identification read — `RDID` (no dummy bytes), `RUID` (four), `RES` (three): the
/// opcode, `dummy` dummy bytes, then `id` from the flash on MISO.
pub fn read_id(opcode: u8, dummy: u8, id: &[u8]) -> Vec<(u8, u8)> {
    let mut words = command(opcode);
    words.extend(dummy_words(dummy));
    words.extend(from_flash(id));
    words
}

/// `REMS`: the opcode `0x90`, a three-byte address (0 for the manufacturer first, 1 for the
/// device first), then `id` from the flash on MISO.
pub fn read_manufacturer_device_id(address: u32, id: &[u8]) -> Vec<(u8, u8)> {
    read(0x90, address, 3, 0, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spi::synth::nor_read_words;

    #[test]
    fn a_read_is_the_words_the_spi_synthesizer_makes() {
        let data = [0x12, 0x34, 0x56];
        assert_eq!(
            read(0x03, 0x00ab_cdef, 3, 0, &data),
            nor_read_words(0xabcdef, &data)
        );
    }

    #[test]
    fn addresses_are_most_significant_byte_first() {
        assert_eq!(
            addressed(0x21, 0x0102_0304, 4),
            vec![(0x21, 0xff), (1, 0xff), (2, 0xff), (3, 0xff), (4, 0xff)]
        );
        assert_eq!(
            read(0x0b, 0x0001_0203, 3, 1, &[0xaa]),
            vec![
                (0x0b, 0xff),
                (1, 0xff),
                (2, 0xff),
                (3, 0xff),
                (0, 0xff),
                (0, 0xaa)
            ]
        );
        assert_eq!(
            program(0x02, 0x10, 3, &[7]),
            vec![(0x02, 0xff), (0, 0xff), (0, 0xff), (0x10, 0xff), (7, 0xff)]
        );
        assert_eq!(read_id(0x9f, 0, &[0xef]), vec![(0x9f, 0xff), (0, 0xef)]);
    }
}
