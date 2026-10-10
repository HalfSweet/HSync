//! Vendor control protocol of the DSLogic FX2/FX3 firmware (`command.h`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use hsync_driver::DriverError;
use hsync_driver::usb::{UsbError, UsbTransport};

/// Vendor request: write `ctl_header` plus data.
pub const CMD_CTL_WR: u8 = 0xb0;
/// Vendor request: announce a read with a `ctl_header`.
pub const CMD_CTL_RD_PRE: u8 = 0xb1;
/// Vendor request: read the announced bytes.
pub const CMD_CTL_RD: u8 = 0xb2;

/// Bulk OUT endpoint for FPGA bitstreams and capture settings.
pub const EP_SETTINGS: u8 = 0x02;
/// Bulk IN endpoint for the trigger header and sample data.
pub const EP_DATA: u8 = 0x86;

/// The firmware major version this driver speaks.
pub const REQUIRED_FIRMWARE_MAJOR: u8 = 2;
/// The FPGA logic version this driver speaks.
pub const REQUIRED_HDL_VERSION: u8 = 0x0e;

/// Targets of control reads and writes (`DSL_CTL_*`).
pub mod dest {
    pub const FW_VERSION: u8 = 0;
    pub const HW_STATUS: u8 = 2;
    pub const PROG_B: u8 = 3;
    pub const LED: u8 = 5;
    pub const INTRDY: u8 = 6;
    pub const WORDWIDE: u8 = 7;
    pub const START: u8 = 8;
    pub const STOP: u8 = 9;
    pub const BULK_WR: u8 = 10;
    pub const NVM: u8 = 12;
    pub const I2C_REG: u8 = 14;
    pub const I2C_STATUS: u8 = 15;
}

/// Bits of the `HW_STATUS` byte.
pub mod status {
    pub const GPIF_DONE: u8 = 1 << 7;
    pub const FPGA_DONE: u8 = 1 << 6;
    pub const FPGA_INIT_B: u8 = 1 << 5;
    pub const SYS_OVERFLOW: u8 = 1 << 4;
    pub const SYS_CLR: u8 = 1 << 3;
}

/// Values written to `PROG_B`, `INTRDY`, `WORDWIDE` and `LED`.
pub mod write {
    pub const PROG_B: u8 = 1 << 2;
    pub const INTRDY: u8 = 1 << 7;
    pub const WORDWIDE: u8 = 1 << 0;
    pub const LED_GREEN: u8 = 1 << 0;
    pub const LED_RED: u8 = 1 << 1;
}

/// FPGA registers, written through `I2C_REG` and read through `I2C_STATUS`.
pub mod reg {
    pub const HDL_VERSION: u16 = 0x04;
    pub const HW_STATUS: u16 = 0x05;
    pub const ADCC: u16 = 0x48;
    pub const CTR0: u16 = 0x70;
    pub const SEC_CTRL: u16 = 0x73;
    pub const SEC_DATA: u16 = 0x75;
    pub const VTH: u16 = 0x78;
}

/// Bits of the `CTR0` register.
pub mod ctr0 {
    pub const NONE: u8 = 0;
    pub const FORCE_RDY: u8 = 1 << 1;
    pub const FORCE_STOP: u8 = 1 << 2;
}

const CONTROL_TIMEOUT: Duration = Duration::from_millis(3000);
/// How long a status poll may wait; DSView polls forever.
pub const POLL_TIMEOUT: Duration = Duration::from_secs(2);
/// Largest data of one `CTL_WR`.
const MAX_WRITE: usize = 60;

/// Control requests to an opened DSLogic.
#[derive(Clone)]
pub struct Control {
    usb: Arc<dyn UsbTransport>,
    read_delay: Duration,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control").finish_non_exhaustive()
    }
}

fn header(dest: u8, offset: u16, size: u8) -> [u8; 4] {
    let [low, high] = offset.to_le_bytes();
    [dest, low, high, size]
}

impl Control {
    pub fn new(usb: Arc<dyn UsbTransport>) -> Self {
        Self {
            usb,
            read_delay: Duration::from_millis(10),
        }
    }

    /// Without the pause the firmware needs between announcing and fetching a read; for tests.
    #[must_use]
    pub fn without_read_delay(mut self) -> Self {
        self.read_delay = Duration::ZERO;
        self
    }

    /// The USB transport.
    pub fn usb(&self) -> &Arc<dyn UsbTransport> {
        &self.usb
    }

    /// `command_ctl_wr()`.
    pub fn write(&self, dest: u8, offset: u16, data: &[u8]) -> Result<(), UsbError> {
        debug_assert!(data.len() <= MAX_WRITE);
        let mut command = Vec::with_capacity(4 + data.len());
        command.extend_from_slice(&header(dest, offset, data.len() as u8));
        command.extend_from_slice(data);
        self.usb
            .control_out(CMD_CTL_WR, 0, 0, &command, CONTROL_TIMEOUT)
    }

    /// `command_ctl_rd()`.
    pub fn read(&self, dest: u8, offset: u16, size: u8) -> Result<Vec<u8>, UsbError> {
        self.usb.control_out(
            CMD_CTL_RD_PRE,
            0,
            0,
            &header(dest, offset, size),
            CONTROL_TIMEOUT,
        )?;
        if !self.read_delay.is_zero() {
            std::thread::sleep(self.read_delay);
        }
        let data = self
            .usb
            .control_in(CMD_CTL_RD, 0, 0, u16::from(size), CONTROL_TIMEOUT)?;
        if data.len() != usize::from(size) {
            return Err(UsbError::Short {
                expected: usize::from(size),
                actual: data.len(),
            });
        }
        Ok(data)
    }

    /// Firmware `(major, minor)`.
    pub fn firmware_version(&self) -> Result<(u8, u8), UsbError> {
        let data = self.read(dest::FW_VERSION, 0, 2)?;
        Ok((data[0], data[1]))
    }

    /// The `HW_STATUS` byte.
    pub fn hw_status(&self) -> Result<u8, UsbError> {
        Ok(self.read(dest::HW_STATUS, 0, 1)?[0])
    }

    /// Polls `HW_STATUS` until any of `bits` is set.
    pub fn wait_status(&self, bits: u8, what: &'static str) -> Result<u8, DriverError> {
        let deadline = Instant::now() + POLL_TIMEOUT;
        loop {
            let value = self.hw_status()?;
            if value & bits != 0 {
                return Ok(value);
            }
            if Instant::now() >= deadline {
                return Err(DriverError::Timeout(what));
            }
        }
    }

    /// `dsl_wr_reg()`.
    pub fn write_reg(&self, addr: u16, value: u8) -> Result<(), UsbError> {
        self.write(dest::I2C_REG, addr, &[value])
    }

    /// `dsl_rd_reg()`.
    pub fn read_reg(&self, addr: u16) -> Result<u8, UsbError> {
        Ok(self.read(dest::I2C_STATUS, addr, 1)?[0])
    }

    /// `dsl_hdl_version()`: the fifth byte of the FPGA status.
    pub fn hdl_version(&self) -> Result<u8, UsbError> {
        let data = self.read(dest::I2C_STATUS, 0, reg::HDL_VERSION as u8 + 1)?;
        Ok(data[usize::from(reg::HDL_VERSION)])
    }

    /// `dsl_rd_nvm()`.
    pub fn read_nvm(&self, addr: u16, len: u8) -> Result<Vec<u8>, UsbError> {
        self.read(dest::NVM, addr, len)
    }

    /// A single-byte control write.
    pub fn write_byte(&self, dest: u8, value: u8) -> Result<(), UsbError> {
        self.write(dest, 0, &[value])
    }

    /// Announces the length of the next bulk write (`DSL_CTL_BULK_WR`, 24 bits).
    pub fn announce_bulk(&self, length: u32) -> Result<(), UsbError> {
        let [a, b, c, _] = length.to_le_bytes();
        self.write(dest::BULK_WR, 0, &[a, b, c])
    }

    /// Writes to the settings endpoint.
    pub fn bulk_write(&self, data: &[u8], timeout: Duration) -> Result<(), UsbError> {
        self.usb.bulk_out(EP_SETTINGS, data, timeout)
    }
}

/// The capture status read while waiting for the trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureStatus {
    pub triggered: bool,
    /// Before the trigger: samples captured. After it: samples still to capture.
    pub count: u64,
}

impl CaptureStatus {
    /// Decodes the first four bytes of `I2C_STATUS`: `trig_hit, cnt3, cnt2, cnt1`.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let [hit, cnt3, cnt2, cnt1] = *data.first_chunk::<4>()?;
        Some(Self {
            triggered: hit & 1 != 0,
            count: (u64::from(hit >> 2) << 32)
                | (u64::from(cnt3) << 24)
                | (u64::from(cnt2) << 16)
                | (u64::from(cnt1) << 8),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_are_packed_little_endian() {
        assert_eq!(header(dest::I2C_REG, 0x0170, 1), [14, 0x70, 0x01, 1]);
    }

    #[test]
    fn parses_the_capture_status() {
        let status = CaptureStatus::parse(&[0b101, 0x01, 0x02, 0x03]).expect("status");
        assert!(status.triggered);
        assert_eq!(status.count, (1 << 32) | 0x0102_0300);
        assert_eq!(CaptureStatus::parse(&[0, 0]), None);
    }
}
