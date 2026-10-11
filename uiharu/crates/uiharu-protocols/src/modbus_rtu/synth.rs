// SPDX-License-Identifier: Apache-2.0

//! Making Modbus RTU traffic: the bytes of frames, for the UART synthesizer to send.

use super::crc::crc16;

/// The bytes of an RTU frame: `address`, the PDU `pdu` (a function code and its data) and the
/// CRC, low byte first. [`crate::uart::synth::UartWaveform::bytes`] sends them.
pub fn rtu_frame(address: u8, pdu: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(pdu.len() + 3);
    frame.push(address);
    frame.extend_from_slice(pdu);
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_end_with_their_crc_low_byte_first() {
        assert_eq!(
            rtu_frame(0x11, &[0x03, 0x00, 0x6b, 0x00, 0x03]),
            vec![0x11, 0x03, 0x00, 0x6b, 0x00, 0x03, 0x76, 0x87]
        );
        assert_eq!(
            rtu_frame(0x11, &[0x83, 0x02]),
            vec![0x11, 0x83, 0x02, 0xc1, 0x34]
        );
    }
}
