// SPDX-License-Identifier: Apache-2.0

//! Making MIDI traffic: the bytes of messages, for the UART synthesizer to send at
//! [`Midi::BAUD`](super::Midi::BAUD).

use super::events::{ChannelStatus, ChannelStatusExt, CommonStatus, CommonStatusExt};

/// The status byte of a channel message `status` on `channel`, 0 to 15 for MIDI channels 1 to
/// 16.
pub fn channel_status(status: ChannelStatus, channel: u8) -> u8 {
    (status.code() << 4) | (channel & 0x0f)
}

/// The bytes of a channel message: its status byte, then `data` with the top bit of each byte
/// cleared.
pub fn channel_message(status: ChannelStatus, channel: u8, data: &[u8]) -> Vec<u8> {
    std::iter::once(channel_status(status, channel))
        .chain(data.iter().map(|byte| byte & 0x7f))
        .collect()
}

/// The bytes of a system common message: its status byte, then `data` with the top bit of each
/// byte cleared.
pub fn common_message(status: CommonStatus, data: &[u8]) -> Vec<u8> {
    std::iter::once(status.byte())
        .chain(data.iter().map(|byte| byte & 0x7f))
        .collect()
}

/// The two data bytes of a 14-bit value, a pitch bend or a song position: its low seven bits,
/// then its high seven bits.
pub const fn fourteen_bits(value: u16) -> [u8; 2] {
    [(value & 0x7f) as u8, ((value >> 7) & 0x7f) as u8]
}

/// The bytes of a system exclusive message: the start of exclusive, `data` with the top bit of
/// each byte cleared, and the end of exclusive.
pub fn sysex(data: &[u8]) -> Vec<u8> {
    std::iter::once(0xf0)
        .chain(data.iter().map(|byte| byte & 0x7f))
        .chain(std::iter::once(0xf7))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_status_and_data_bytes() {
        assert_eq!(channel_status(ChannelStatus::NoteOn, 9), 0x99);
        assert_eq!(
            channel_message(ChannelStatus::ControlChange, 0, &[7, 0xff]),
            vec![0xb0, 0x07, 0x7f]
        );
        assert_eq!(
            common_message(CommonStatus::SongPositionPointer, &fourteen_bits(0x1234)),
            vec![0xf2, 0x34, 0x24]
        );
        // The middle of the pitch bend range.
        assert_eq!(fourteen_bits(8192), [0x00, 0x40]);
        assert_eq!(fourteen_bits(0x3fff), [0x7f, 0x7f]);
        assert_eq!(sysex(&[0x43, 0x90]), vec![0xf0, 0x43, 0x10, 0xf7]);
    }
}
