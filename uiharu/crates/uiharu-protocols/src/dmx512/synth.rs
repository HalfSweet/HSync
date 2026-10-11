// SPDX-License-Identifier: Apache-2.0

//! Making DMX512 traffic: packets sent on a UART waveform, for tests, tools and benchmarks.

use crate::uart::synth::UartWaveform;

/// The times of a packet that are not characters, in samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PacketTiming {
    /// The break: the line at space.
    pub break_samples: u64,
    /// The mark after break, before the start code.
    pub mark_after_break: u64,
    /// The mark between two slots; zero sends them back to back.
    pub between_slots: u64,
}

/// Sends a packet on `waveform`, a line configured as [`super::Dmx512::uart_config`] says: the
/// break, the mark after break, the start code and the slots, each a character without errors.
/// Follow it with idle time or the next packet's break.
pub fn packet(waveform: &mut UartWaveform, timing: PacketTiming, start_code: u8, slots: &[u8]) {
    waveform.break_(timing.break_samples);
    waveform.idle(timing.mark_after_break);
    waveform.byte(u16::from(start_code));
    for slot in slots {
        if timing.between_slots > 0 {
            waveform.idle(timing.between_slots);
        }
        waveform.byte(u16::from(*slot));
    }
}

#[cfg(test)]
mod tests {
    use uiharu_core::{LogicBlock, Tick};

    use super::*;
    use crate::dmx512::Dmx512;

    fn levels(block: &LogicBlock) -> String {
        (block.start().0..block.end().0)
            .map(|tick| {
                if block.level(0, Tick(tick)).unwrap_or(false) {
                    '1'
                } else {
                    '0'
                }
            })
            .collect()
    }

    #[test]
    fn a_packet_is_a_break_a_mark_and_characters() {
        let mut waveform = UartWaveform::new(Dmx512::uart_config(0), 1, 1, Tick::ZERO);
        waveform.idle(2);
        let timing = PacketTiming {
            break_samples: 23,
            mark_after_break: 3,
            between_slots: 1,
        };
        packet(&mut waveform, timing, 0x00, &[0x81]);
        waveform.idle(2);
        // Idle, the break, the mark after break, the start code, a mark between slots, the
        // slot and idle again. Each character is a start bit, eight data bits least
        // significant first and two stop bits.
        assert_eq!(
            levels(&waveform.finish()),
            format!(
                "11{}111{}1{}11",
                "0".repeat(23),
                "00000000011",
                "01000000111"
            )
        );
    }
}
