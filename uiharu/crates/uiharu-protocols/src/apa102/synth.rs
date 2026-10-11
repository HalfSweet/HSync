// SPDX-License-Identifier: Apache-2.0

//! Making APA102 traffic: the bytes of frames, for the SPI synthesizer
//! ([`crate::spi::synth::SpiWaveform`]) to clock out on MOSI.

/// The start frame: 32 zero bits.
pub const START_FRAME: [u8; 4] = [0; 4];

/// The reset frame an SK9822 chain takes after its LED frames: 32 zero bits.
pub const RESET_FRAME: [u8; 4] = [0; 4];

/// An LED's values, as its LED frame carries them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Led {
    /// The global brightness, 0 to 31; higher bits are dropped.
    pub brightness: u8,
    /// The blue byte.
    pub blue: u8,
    /// The green byte.
    pub green: u8,
    /// The red byte.
    pub red: u8,
}

impl Led {
    /// An LED of `brightness` with these colour bytes.
    pub const fn new(brightness: u8, blue: u8, green: u8, red: u8) -> Self {
        Self {
            brightness,
            blue,
            green,
            red,
        }
    }

    /// Its LED frame: the bits `111` and the five bits of brightness, then blue, green and red.
    pub const fn frame(self) -> [u8; 4] {
        [
            0b1110_0000 | (self.brightness & 0b1_1111),
            self.blue,
            self.green,
            self.red,
        ]
    }
}

/// The bytes of end frame the datasheets ask for after `leds` LEDs: at least one bit for every
/// two LEDs, in whole bytes.
pub fn end_frame_len(leds: usize) -> usize {
    leds.div_ceil(16)
}

/// The start frame and the LED frames of `leds`.
fn led_frames(leds: &[Led]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 * leds.len() + 8);
    bytes.extend_from_slice(&START_FRAME);
    for led in leds {
        bytes.extend_from_slice(&led.frame());
    }
    bytes
}

/// An APA102 frame: the start frame, the LED frames, and an end frame of `end` bytes of ones.
pub fn apa102_frame(leds: &[Led], end: usize) -> Vec<u8> {
    let mut bytes = led_frames(leds);
    bytes.extend(std::iter::repeat_n(0xff, end));
    bytes
}

/// An SK9822 frame: the start frame, the LED frames, the reset frame, and an end frame of
/// `end` zero bytes.
pub fn sk9822_frame(leds: &[Led], end: usize) -> Vec<u8> {
    let mut bytes = led_frames(leds);
    bytes.extend_from_slice(&RESET_FRAME);
    bytes.extend(std::iter::repeat_n(0x00, end));
    bytes
}

/// The words that clock `bytes` out on MOSI, MISO left high.
pub fn words(bytes: &[u8]) -> Vec<(u8, u8)> {
    bytes.iter().map(|byte| (*byte, 0xff)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_the_datasheets() {
        assert_eq!(Led::new(31, 1, 2, 3).frame(), [0xff, 1, 2, 3]);
        assert_eq!(Led::new(0, 0, 0, 0).frame(), [0xe0, 0, 0, 0]);
        assert_eq!(Led::new(0x25, 9, 8, 7).frame(), [0xe5, 9, 8, 7]);
        let leds = [Led::new(1, 0x10, 0x20, 0x30), Led::new(2, 0x40, 0x50, 0x60)];
        assert_eq!(
            apa102_frame(&leds, 1),
            vec![
                0, 0, 0, 0, 0xe1, 0x10, 0x20, 0x30, 0xe2, 0x40, 0x50, 0x60, 0xff
            ]
        );
        assert_eq!(
            sk9822_frame(&leds, 2),
            vec![
                0, 0, 0, 0, 0xe1, 0x10, 0x20, 0x30, 0xe2, 0x40, 0x50, 0x60, 0, 0, 0, 0, 0, 0
            ]
        );
        assert_eq!(end_frame_len(0), 0);
        assert_eq!(end_frame_len(1), 1);
        assert_eq!(end_frame_len(16), 1);
        assert_eq!(end_frame_len(17), 2);
        assert_eq!(words(&[0x12, 0x34]), vec![(0x12, 0xff), (0x34, 0xff)]);
    }
}
