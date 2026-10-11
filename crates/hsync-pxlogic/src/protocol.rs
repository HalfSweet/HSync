//! The USB protocol of PXLogic devices, from PXView's `src/hardware/pxlogic/usb_ctrl.c` and
//! `pxlogic.c`: register reads and writes over bulk endpoints, the capture state behind a
//! vendor request, uploads into the MCU's flash or the FPGA, and the register values a capture
//! is set up with.

use std::sync::Arc;
use std::time::Duration;

use hsync_driver::DriverError;
use hsync_driver::usb::UsbTransport;

use crate::profiles::UsbSpeed;

/// Bulk OUT endpoint of register requests.
pub const EP_REGISTER_OUT: u8 = 0x01;
/// Bulk IN endpoint of register answers.
pub const EP_REGISTER_IN: u8 = 0x81;
/// Bulk IN endpoint of samples.
pub const EP_DATA: u8 = 0x82;
/// Bulk OUT endpoint of uploads (firmware and bitstreams).
pub const EP_UPLOAD: u8 = 0x03;
/// The second register endpoints, unused but cleared before a capture.
pub const EP_REGISTER2_OUT: u8 = 0x04;
pub const EP_REGISTER2_IN: u8 = 0x84;

/// Vendor request reading the capture state ([`CaptureState`]).
pub const CMD_CTL_RD: u8 = 0xb0;

/// First word of a register write.
const REGISTER_WRITE: u32 = 0xfefe_0000;
/// First word of a register read.
const REGISTER_READ: u32 = 0xfefe_0001;
/// Second word of register requests: the length of address and data.
const REGISTER_LENGTH: u32 = 8;
/// Last word of the answer to a write that took effect.
const REGISTER_ACK: u32 = 0xfefe_fefe;
/// Bytes of register requests and answers.
const REGISTER_PACKET: usize = 16;
/// How much a register read asks for. PXView asks for the 16 bytes of the answer; nusb asks
/// for whole packets, and the 16-byte answer ends the transfer early either way.
const REGISTER_READ_LENGTH: usize = 1024;
const REGISTER_TIMEOUT: Duration = Duration::from_millis(1000);
const STATE_TIMEOUT: Duration = Duration::from_millis(3000);
/// How long an upload may take; PXView waits without a limit.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// Uploads are whole 4 KiB pages.
const UPLOAD_PAGE: usize = 4096;

/// The FPGA's registers: index `i` is at `i << 2`.
pub const fn reg(index: u32) -> u32 {
    index << 2
}

/// The MCU's registers: index `i` is at `8192 + (i << 2)`.
pub const fn sys(index: u32) -> u32 {
    8192 + (index << 2)
}

/// Registers by what `pxlogic.c` uses them for.
pub mod regs {
    use super::{reg, sys};

    /// Control bits: `0x5` while a capture is set up, `0x2` for Stream mode, `0x8` for the
    /// filter, `0x10` pulsed once.
    pub const CONTROL: u32 = reg(0);
    /// Period of the threshold voltage's PWM.
    pub const VTH_PERIOD: u32 = reg(1);
    /// High time of the threshold voltage's PWM.
    pub const VTH_DUTY: u32 = reg(2);
    /// Bit `n`: channel `n` is captured.
    pub const CHANNELS: u32 = reg(4);
    /// The sample clock (`gpio_mode`), with bit 3 for its falling edge.
    pub const CLOCK: u32 = reg(5);
    /// The divider of slower sample clocks (`gpio_div`).
    pub const DIVIDER: u32 = reg(6);
    /// Bytes of each USB block.
    pub const BLOCK: u32 = reg(7);
    /// All ones while a capture is set up; zero starts it.
    pub const HOLD: u32 = reg(8);
    pub const TRIGGER_ZERO: u32 = reg(9);
    pub const TRIGGER_ONE: u32 = reg(10);
    pub const TRIGGER_RISE: u32 = reg(11);
    pub const TRIGGER_FALL: u32 = reg(12);
    /// What on the external trigger input fires the trigger.
    pub const EXTERNAL_TRIGGER: u32 = reg(15);
    pub const PWM0_ENABLE: u32 = reg(16);
    pub const PWM0_PERIOD: u32 = reg(17);
    pub const PWM0_DUTY: u32 = reg(18);
    pub const PWM1_ENABLE: u32 = reg(19);
    /// Drive the trigger output.
    pub const TRIGGER_OUT: u32 = reg(22);

    /// Bytes of each USB block, as the MCU sees it.
    pub const SYS_BLOCK: u32 = sys(2);
    /// Uploads: first address, end address and target ([`super::upload`]).
    pub const UPLOAD_START: u32 = sys(6);
    pub const UPLOAD_END: u32 = sys(7);
    pub const UPLOAD_TARGET: u32 = sys(8);
    /// Bytes of the capture, low and high word.
    pub const LIMIT_LOW: u32 = sys(9);
    pub const LIMIT_HIGH: u32 = sys(10);
    /// Restarts the USB blocks (`set_block_start`).
    pub const BLOCK_START: u32 = sys(11);
    /// Writing resets the MCU, which then connects anew.
    pub const RESET: u32 = sys(12);
    pub const FIRMWARE_VERSION: u32 = sys(13);
    /// Channels captured.
    pub const CHANNEL_COUNT: u32 = sys(19);
    /// Samples before the trigger.
    pub const TRIGGER_POSITION: u32 = sys(20);
    /// Which model the device is.
    pub const LOGIC_MODE: u32 = sys(22);
}

/// Targets of uploads (`usb_wr_data_update()`'s `mode`).
pub mod upload {
    /// The CH569W MCU's flash.
    pub const MCU_FLASH: u32 = 0;
    /// The FPGA configuration, as `firmware_config(…, 1)` uploads bitstreams.
    pub const FPGA: u32 = 4;
    /// Where the MCU firmware goes in its flash.
    pub const MCU_FIRMWARE_BASE: u32 = 48 * 1024;
    /// Room of one copy of the MCU firmware.
    pub const MCU_FIRMWARE_ROOM: usize = 48 * 1024;
}

/// The MCU firmware as `firmware_config(…, 0)` writes it: padded with `0xff` to 48 KiB, three
/// times over. `None` if the image does not fit.
pub fn mcu_firmware_image(image: &[u8]) -> Option<Vec<u8>> {
    if image.len() > upload::MCU_FIRMWARE_ROOM {
        return None;
    }
    let mut copy = image.to_vec();
    copy.resize(upload::MCU_FIRMWARE_ROOM, 0xff);
    Some(copy.repeat(3))
}

/// What the vendor request [`CMD_CTL_RD`] answers (`struct ctl_data`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureState {
    /// Samples captured so far.
    pub samples: u64,
    /// The trigger fired.
    pub triggered: bool,
    /// The sample the trigger fired at.
    pub trigger_position: u32,
}

impl CaptureState {
    /// Reads the 16 little-endian bytes of `struct ctl_data`.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let data = data.get(..16)?;
        let word =
            |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
        Some(Self {
            samples: u64::from(word(0)) | (u64::from(word(4)) << 32),
            triggered: word(8) != 0,
            trigger_position: word(12),
        })
    }
}

/// A register request: command, length, address and data, little-endian.
pub fn register_packet(command_read: bool, address: u32, data: u32) -> [u8; REGISTER_PACKET] {
    let command = if command_read {
        REGISTER_READ
    } else {
        REGISTER_WRITE
    };
    let mut packet = [0; REGISTER_PACKET];
    for (slot, word) in
        packet
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip([command, REGISTER_LENGTH, address, data])
    {
        *slot = word.to_le_bytes();
    }
    packet
}

/// Period of the PWM behind the threshold voltage: 120 MHz over 10 kHz.
pub const VTH_PERIOD: u32 = 120_000_000 / 10_000;

/// The threshold PWM's high time for `vth` volts (`start_transfers()`).
pub fn vth_duty(vth: f64) -> u32 {
    (vth * (100.0 / 200.0) / 3.334 * f64::from(VTH_PERIOD)) as u32
}

/// The sample clock and its divider for a rate (`gpio_mode`, `gpio_div`): 1 GHz to 125 MHz
/// and 800 to 100 MHz come straight from the clocks, slower rates divide 100 MHz.
pub fn sample_clock(samplerate: u64) -> (u32, u32) {
    const MHZ: u64 = 1_000_000;
    match samplerate {
        1_000_000_000 => (0, 0),
        500_000_000 => (1, 0),
        250_000_000 => (2, 0),
        125_000_000 => (3, 0),
        800_000_000 => (4, 0),
        400_000_000 => (5, 0),
        200_000_000 => (6, 0),
        100_000_000 => (7, 0),
        rate => {
            let divider = match rate {
                50_000_000 => 1,
                25_000_000 => 3,
                20_000_000 => 4,
                10_000_000 => 9,
                5_000_000 => 19,
                4_000_000 => 24,
                2_000_000 => 49,
                MHZ => 99,
                500_000 => 199,
                400_000 => 249,
                200_000 => 499,
                100_000 => 999,
                50_000 => 1999,
                40_000 => 2499,
                20_000 => 4999,
                10_000 => 9999,
                5_000 => 19999,
                2_000 => 49999,
                _ => 0,
            };
            (7, divider)
        }
    }
}

fn align_4k(value: u64) -> u64 {
    value.next_multiple_of(4096)
}

/// Bytes of each USB block (`devc->block_size`): 10 ms of samples per channel, whole pages,
/// at most 4 MiB over USB 3.0 or 10 ms of USB 2.0's 480 Mbit/s.
pub fn block_size(samplerate: u64, channels: u32, speed: UsbSpeed) -> u32 {
    let channels = u64::from(channels.max(1));
    let per_channel = align_4k(samplerate / 100 / 8);
    let most = align_4k(match speed {
        UsbSpeed::Super => 4 * 1024 * 1024,
        UsbSpeed::High => 480 * 1000 * 1000 / 100 / 8,
    });
    let size = if per_channel * channels > most {
        most / channels / 4096 * 4096 * channels
    } else {
        per_channel * channels
    };
    u32::try_from(size).unwrap_or(u32::MAX)
}

/// Samples per channel the memory holds with `channels` enabled (`px_channel_depth()`).
pub fn channel_depth(memory_bits: u64, channels: u32) -> u64 {
    (memory_bits / u64::from(channels.max(1))) & !1023
}

/// Samples before the trigger for a position in percent (`set_trigger()`): at least 64, and in
/// Stream mode at most a tenth of the memory, in Buffer mode at most 90 % of it.
pub fn trigger_position(position: u8, limit: u64, depth: u64, stream: bool) -> u32 {
    let wanted = (f64::from(position) / 100.0 * limit as f64) as u32;
    let wanted = u64::from(wanted.max(64));
    let most = if stream {
        depth * 10 / 100
    } else {
        depth * 90 / 100
    };
    // C's MIN() of a u32 and a u64 is a u64, cut back to 32 bits.
    wanted.min(most) as u32
}

/// Requests to an opened PXLogic.
#[derive(Clone)]
pub struct Control {
    usb: Arc<dyn UsbTransport>,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

impl Control {
    pub fn new(usb: Arc<dyn UsbTransport>) -> Self {
        Self { usb }
    }

    /// The USB transport.
    pub fn usb(&self) -> &Arc<dyn UsbTransport> {
        &self.usb
    }

    /// Sends a register request and returns the four words of the answer; a failed transfer
    /// clears the halt of its endpoint, as `usb_wr_reg()` does.
    fn exchange(&self, read: bool, address: u32, data: u32) -> Result<[u32; 4], DriverError> {
        let packet = register_packet(read, address, data);
        if let Err(error) = self
            .usb
            .bulk_out(EP_REGISTER_OUT, &packet, REGISTER_TIMEOUT)
        {
            self.clear_halt(EP_REGISTER_OUT);
            return Err(error.into());
        }
        let answer =
            match self
                .usb
                .bulk_read(EP_REGISTER_IN, REGISTER_READ_LENGTH, REGISTER_TIMEOUT)
            {
                Ok(answer) if answer.len() >= REGISTER_PACKET => answer,
                Ok(answer) => {
                    self.clear_halt(EP_REGISTER_IN);
                    return Err(if answer.is_empty() {
                        DriverError::Timeout("a register answer")
                    } else {
                        DriverError::Protocol(format!(
                            "a register answer of {} bytes",
                            answer.len()
                        ))
                    });
                }
                Err(error) => {
                    self.clear_halt(EP_REGISTER_IN);
                    return Err(error.into());
                }
            };
        let mut words = [0; 4];
        for (word, bytes) in words.iter_mut().zip(answer.as_chunks::<4>().0) {
            *word = u32::from_le_bytes(*bytes);
        }
        Ok(words)
    }

    /// `usb_wr_reg()`.
    pub fn write(&self, address: u32, data: u32) -> Result<(), DriverError> {
        let answer = self.exchange(false, address, data)?;
        if answer[3] != REGISTER_ACK {
            return Err(DriverError::Protocol(format!(
                "register {address:#06x} refused {data:#x}"
            )));
        }
        Ok(())
    }

    /// `usb_rd_reg()`.
    pub fn read(&self, address: u32) -> Result<u32, DriverError> {
        Ok(self.exchange(true, address, 0)?[3])
    }

    /// `libusb_clear_halt()`; PXView goes on whatever it returns.
    pub fn clear_halt(&self, endpoint: u8) {
        if let Err(error) = self.usb.clear_halt(endpoint) {
            log::debug!("clearing the halt of endpoint {endpoint:#04x} failed: {error}");
        }
    }

    /// `command_ctl_rddata()`.
    pub fn state(&self) -> Result<CaptureState, DriverError> {
        let data = self.usb.control_in(CMD_CTL_RD, 0, 0, 16, STATE_TIMEOUT)?;
        CaptureState::parse(&data).ok_or_else(|| {
            DriverError::Protocol(format!("a capture state of {} bytes", data.len()))
        })
    }

    /// `usb_wr_data_update()`: announces the address range and target, then sends the data in
    /// whole pages. PXView sends whatever its buffer holds past the end; HSync sends zeros.
    pub fn upload(&self, base: u32, data: &[u8], target: u32) -> Result<(), DriverError> {
        let length = data.len().next_multiple_of(UPLOAD_PAGE);
        let end = u32::try_from(length)
            .ok()
            .and_then(|length| base.checked_add(length))
            .ok_or_else(|| DriverError::InvalidRequest("an upload beyond 4 GiB".into()))?;
        self.write(regs::UPLOAD_START, base)?;
        self.write(regs::UPLOAD_END, end)?;
        self.write(regs::UPLOAD_TARGET, target)?;
        let mut pages = data.to_vec();
        pages.resize(length, 0);
        Ok(self.usb.bulk_out(EP_UPLOAD, &pages, UPLOAD_TIMEOUT)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_register_requests() {
        assert_eq!(
            register_packet(false, sys(22), 0x1234_5678),
            [
                0x00, 0x00, 0xfe, 0xfe, 0x08, 0, 0, 0, 0x58, 0x20, 0, 0, 0x78, 0x56, 0x34, 0x12
            ]
        );
        assert_eq!(
            register_packet(true, reg(6), 0)[..4],
            [0x01, 0x00, 0xfe, 0xfe]
        );
        assert_eq!(regs::LOGIC_MODE, 8192 + 88);
        assert_eq!(regs::VTH_PERIOD, 4);
    }

    #[test]
    fn reads_the_capture_state() {
        let mut data = vec![0u8; 16];
        data[..8].copy_from_slice(&0x1_0000_0002u64.to_le_bytes());
        data[8] = 1;
        data[12..].copy_from_slice(&777u32.to_le_bytes());
        assert_eq!(
            CaptureState::parse(&data),
            Some(CaptureState {
                samples: 0x1_0000_0002,
                triggered: true,
                trigger_position: 777
            })
        );
        assert_eq!(CaptureState::parse(&data[..15]), None);
    }

    #[test]
    fn lays_out_the_mcu_firmware() {
        let image = mcu_firmware_image(&[1, 2, 3]).expect("fits");
        assert_eq!(image.len(), 3 * 48 * 1024);
        assert_eq!(&image[..4], &[1, 2, 3, 0xff]);
        assert_eq!(&image[48 * 1024..48 * 1024 + 4], &[1, 2, 3, 0xff]);
        assert!(mcu_firmware_image(&vec![0; 48 * 1024 + 1]).is_none());
    }

    #[test]
    fn encodes_capture_settings() {
        assert_eq!(VTH_PERIOD, 12_000);
        // 2.0 V: 2.0 × 0.5 / 3.334 × 12 000 = 3599.28.
        assert_eq!(vth_duty(2.0), 3599);
        assert_eq!(sample_clock(1_000_000_000), (0, 0));
        assert_eq!(sample_clock(100_000_000), (7, 0));
        assert_eq!(sample_clock(25_000_000), (7, 3));
        assert_eq!(sample_clock(2_000), (7, 49999));
        assert_eq!(sample_clock(10), (7, 0));
        // 16 channels at 250 MHz want 77 pages each per 10 ms, beyond USB 3.0's 4 MiB.
        assert_eq!(
            block_size(250_000_000, 16, UsbSpeed::Super),
            4 * 1024 * 1024
        );
        // USB 2.0 carries 602 112 bytes in 10 ms: 9 pages for each of 16 channels.
        assert_eq!(block_size(500_000_000, 16, UsbSpeed::High), 9 * 4096 * 16);
        assert_eq!(block_size(2_000, 4, UsbSpeed::Super), 4 * 4096);
        assert_eq!(channel_depth(4_000_000_000, 16), 250_000_000 & !1023);
        assert_eq!(trigger_position(50, 10_000, 1 << 20, false), 5_000);
        assert_eq!(trigger_position(1, 1_000, 1 << 20, false), 64);
        assert_eq!(trigger_position(50, 1 << 30, 1 << 20, true), (1 << 20) / 10);
    }
}
