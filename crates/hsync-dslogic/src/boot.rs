//! Bringing a DSLogic up: FX2 firmware upload, FPGA configuration, the security handshake and
//! the clock synthesiser (DESIGN.md §4.3, §4.5, §4.6).

use std::path::PathBuf;
use std::time::Duration;

use hsync_driver::DriverError;
use hsync_driver::usb::{UsbError, UsbTransport};

use crate::protocol::{Control, ctr0, dest, reg, status, write};

/// Where firmware images and FPGA bitstreams are read from.
#[derive(Debug, Clone)]
pub struct FirmwareDir(pub PathBuf);

impl FirmwareDir {
    /// Reads a file by its DSView name.
    pub fn load(&self, name: &str) -> Result<Vec<u8>, DriverError> {
        std::fs::read(self.0.join(name)).map_err(|error| {
            log::warn!("reading firmware {name} from {}: {error}", self.0.display());
            DriverError::FirmwareMissing(name.to_owned())
        })
    }
}

/// EZ-USB "firmware load" vendor request.
const EZUSB_FIRMWARE_LOAD: u8 = 0xa0;
/// The CPUCS register: bit 0 holds the 8051 in reset.
const EZUSB_CPUCS: u16 = 0xe600;
const EZUSB_CHUNK: usize = 4096;
const EZUSB_TIMEOUT: Duration = Duration::from_millis(3000);

/// `ezusb_upload_firmware()`: holds the CPU in reset, writes the image from address 0 and
/// releases the reset. The device then disconnects and comes back running the firmware.
pub fn upload_firmware(usb: &dyn UsbTransport, image: &[u8]) -> Result<(), UsbError> {
    usb.control_out(EZUSB_FIRMWARE_LOAD, EZUSB_CPUCS, 0, &[1], EZUSB_TIMEOUT)?;
    for (index, chunk) in image.chunks(EZUSB_CHUNK).enumerate() {
        let address = (index * EZUSB_CHUNK) as u16;
        usb.control_out(EZUSB_FIRMWARE_LOAD, address, 0, chunk, EZUSB_TIMEOUT)?;
    }
    usb.control_out(EZUSB_FIRMWARE_LOAD, EZUSB_CPUCS, 0, &[0], EZUSB_TIMEOUT)
}

/// Time allowed for the bitstream transfer; DSView allows 1 s.
const BITSTREAM_TIMEOUT: Duration = Duration::from_secs(5);

/// `dsl_fpga_config()`: loads a bitstream into the FPGA.
pub fn configure_fpga(ctl: &Control, bitstream: &[u8]) -> Result<(), DriverError> {
    ctl.write_byte(dest::PROG_B, !write::PROG_B)?;
    ctl.write_byte(dest::LED, !write::LED_GREEN & !write::LED_RED)?;
    ctl.write_byte(dest::PROG_B, write::PROG_B)?;
    ctl.wait_status(status::FPGA_INIT_B, "FPGA INIT_B")?;
    ctl.write_byte(dest::INTRDY, !write::INTRDY)?;
    ctl.announce_bulk(bitstream.len() as u32)?;
    ctl.bulk_write(bitstream, BITSTREAM_TIMEOUT)?;
    ctl.write_byte(dest::INTRDY, write::INTRDY)?;
    ctl.wait_status(status::GPIF_DONE, "GPIF done after FPGA configuration")?;
    ctl.write_byte(dest::INTRDY, !write::INTRDY)?;
    ctl.wait_status(status::FPGA_DONE, "FPGA done")?;
    ctl.write_byte(dest::LED, write::LED_GREEN)?;
    ctl.write_byte(dest::WORDWIDE, write::WORDWIDE)?;
    log::info!("configured the FPGA with {} bytes", bitstream.len());
    Ok(())
}

/// EEPROM address of the security handshake words.
pub const SECURITY_EEPROM_ADDR: u16 = 0x3c00;
/// Words in the security handshake.
pub const SECURITY_STEPS: usize = 8;
const SECU_READY: u8 = 1 << 3;
const SECU_PASS: u8 = 1 << 4;
const SECU_START: u16 = 0x0513;
const SECU_CHECK: u16 = 0x0219;
const SECU_TRY_COUNT: u32 = 8;

fn secu_write(ctl: &Control, command: u16, data: u16) -> Result<(), UsbError> {
    let [data_low, data_high] = data.to_le_bytes();
    let [command_low, command_high] = command.to_le_bytes();
    ctl.write_reg(reg::SEC_DATA, data_low)?;
    ctl.write_reg(reg::SEC_DATA + 1, data_high)?;
    ctl.write_reg(reg::SEC_CTRL, command_low)?;
    ctl.write_reg(reg::SEC_CTRL + 1, command_high)
}

fn secu_read(ctl: &Control) -> Result<u16, UsbError> {
    let high = ctl.read_reg(reg::SEC_DATA + 1)?;
    let low = ctl.read_reg(reg::SEC_DATA)?;
    Ok(u16::from_le_bytes([low, high]))
}

/// `dsl_secuCheck()`: replays the handshake words from the EEPROM. Returns whether the check
/// passed; DSView only logs the outcome, and so do we.
pub fn security_check(ctl: &Control, words: &[u16; SECURITY_STEPS]) -> Result<bool, UsbError> {
    ctl.write_reg(reg::SEC_CTRL, 0)?;
    ctl.write_reg(reg::SEC_CTRL + 1, 0)?;
    std::thread::sleep(Duration::from_millis(10));
    ctl.write_reg(reg::SEC_CTRL, 1)?;
    ctl.write_reg(reg::SEC_CTRL + 1, 0)?;
    if ctl.read_reg(reg::SEC_CTRL)? & SECU_PASS != 0 {
        return Ok(false);
    }
    secu_write(ctl, SECU_START, 0)?;
    let mut tries = SECU_TRY_COUNT;
    for step in (0..SECURITY_STEPS).rev() {
        if ctl.read_reg(reg::SEC_CTRL)? & SECU_PASS != 0 {
            return Ok(false);
        }
        while ctl.read_reg(reg::SEC_CTRL)? & SECU_READY == 0 {
            if tries == 0 {
                log::warn!("the security handshake never became ready");
                return Ok(false);
            }
            tries -= 1;
        }
        if secu_read(ctl)? != 0 {
            return Ok(false);
        }
        secu_write(ctl, SECU_CHECK, words[step])?;
    }
    Ok(true)
}

/// One step of an ADC or clock chip setup (`struct DSL_adc_config`).
struct AdcStep {
    register: u16,
    delay_ms: u64,
    bytes: &'static [u8],
}

/// `adc_clk_init_500m`: starts the ADF4360-7 clock synthesiser.
const ADF4360_INIT: [AdcStep; 4] = [
    AdcStep {
        register: reg::ADCC + 2,
        delay_ms: 0,
        bytes: &[0x01],
    },
    AdcStep {
        register: reg::ADCC,
        delay_ms: 0,
        bytes: &[0x01, 0x61, 0x00, 0x30],
    },
    AdcStep {
        register: reg::ADCC,
        delay_ms: 0,
        bytes: &[0x01, 0x40, 0xf1, 0x46],
    },
    AdcStep {
        register: reg::ADCC,
        delay_ms: 10,
        bytes: &[0x01, 0x62, 0x3d, 0x40],
    },
];

/// `dsl_config_adc(sdi, adc_clk_init_500m)`.
pub fn init_clock(ctl: &Control) -> Result<(), UsbError> {
    for step in &ADF4360_INIT {
        if step.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(step.delay_ms));
        }
        for &byte in step.bytes {
            ctl.write_reg(step.register, byte)?;
        }
    }
    Ok(())
}

/// Clears `CTR0` (`bmNONE`): leaves forced ready or stop states.
pub fn clear_control(ctl: &Control) -> Result<(), UsbError> {
    ctl.write_reg(reg::CTR0, ctr0::NONE)
}
