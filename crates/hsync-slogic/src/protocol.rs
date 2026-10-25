// SPDX-License-Identifier: GPL-3.0-or-later
// Ported from sipeed/libsigrok, slogic-dev, commit
// 0d728b3e1226a61738c99c3333ad16d5c2814728:
// src/hardware/sipeed-slogic-analyzer/slogic/{slogic.c,slogic_usb.c}.
// Copyright (C) 2023-2025 Shenzhen Sipeed Technology Co., Ltd.
//! The SLogic U3 register/AUX protocol over a claimed USB interface.

use std::sync::Arc;
use std::time::Duration;

use hsync_core::UiError;
use hsync_driver::DriverError;
use hsync_driver::usb::{UsbError, UsbTransport};

/// The U3 models' sample endpoint.
pub const EP_IN: u8 = 0x82;

const READ: u8 = 0;
const WRITE: u8 = 1;
const CTRL: u16 = 0x0004;
const AUX: u16 = 0x000c;
const PAYLOAD: u16 = 0x0010;
const CONTROL_TIMEOUT: Duration = Duration::from_millis(500);
const AUX_POLLS: usize = 8;

/// Settings carried by the U3 AUX commands.
#[derive(Debug, Clone, Copy)]
pub struct CaptureSettings {
    /// Physical channels in the wire format: 4, 8, 16 or 32, starting at D0.
    pub channels: u8,
    /// Requested samples per second.
    pub samplerate: u64,
    /// Input threshold in volts, from 0 through 6.
    pub threshold: f64,
}

/// The bounded queue of sample transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferPlan {
    /// Bytes in each request, aligned to 32 KiB.
    pub size: usize,
    /// Number of requests kept in flight.
    pub count: usize,
    /// The reference watchdog's allowance for a transfer.
    pub timeout: Duration,
    /// Expected packed payload bytes per second.
    pub expected_rate_bytes: u64,
}

impl TransferPlan {
    /// Uses the reference's quarter of a 250 ms target and 16-transfer queue.
    pub fn new(channels: u8, samplerate: u64) -> Self {
        let expected_rate_bytes = samplerate.saturating_mul(u64::from(channels)) / 8;
        // The reference probes and halves allocations that libusb/usbfs cannot
        // submit. nusb uses WinUSB RAW_IO, whose pipe may accept at most 1 MiB
        // per request. Bound each request instead of probing the host, as
        // HSync's PXLogic driver does, and retain the 16-request queue.
        let probe = (expected_rate_bytes / 1000)
            .saturating_mul(250)
            .min(4 * 1024 * 1024)
            .next_multiple_of(32 * 1024);
        let size = (probe / 4 / (32 * 1024) * (32 * 1024)).clamp(32 * 1024, 1024 * 1024) as usize;
        let duration_ms = (size as u64 / (expected_rate_bytes / 1000).max(1)).max(1);
        Self {
            size,
            count: 16,
            // xfer_timeout(c, others), allowing all 15 earlier requests.
            timeout: Duration::from_millis(((1.3 * duration_ms as f64 * 17.0) as u64).max(50)),
            expected_rate_bytes,
        }
    }
}

/// The DAC value the reference writes for an input threshold.
pub fn threshold_dac(volts: f64) -> u32 {
    (volts.clamp(0.0, 6.0) / 3.33 / 2.0 * 1024.0 + 0.5) as u32
}

/// Commands over one claimed interface. Callers serialize control operations.
#[derive(Clone)]
pub struct Link {
    usb: Arc<dyn UsbTransport>,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

struct AuxPayload {
    bytes: [u8; 16],
    len: usize,
}

impl Link {
    /// Uses the transport of an already claimed interface.
    pub fn new(usb: Arc<dyn UsbTransport>) -> Self {
        Self { usb }
    }

    /// The transport used to queue sample reads.
    pub fn usb(&self) -> &Arc<dyn UsbTransport> {
        &self.usb
    }

    fn write(&self, address: u16, data: &[u8]) -> Result<(), DriverError> {
        for (index, word) in data.as_chunks::<4>().0.iter().enumerate() {
            self.usb
                .control_out(WRITE, address + index as u16 * 4, 0, word, CONTROL_TIMEOUT)?;
        }
        Ok(())
    }

    fn read(&self, address: u16, data: &mut [u8]) -> Result<(), DriverError> {
        for (index, word) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let received =
                self.usb
                    .control_in(READ, address + index as u16 * 4, 0, 4, CONTROL_TIMEOUT)?;
            // The reference accepts short control reads. Reject them here so
            // stale/zero-filled bytes cannot silently alter capture settings.
            if received.len() != 4 {
                return Err(UsbError::Short {
                    expected: 4,
                    actual: received.len(),
                }
                .into());
            }
            word.copy_from_slice(&received);
        }
        Ok(())
    }

    fn write_word(&self, address: u16, value: u32) -> Result<(), DriverError> {
        self.write(address, &value.to_le_bytes())
    }

    fn aux_begin(&self, command: u32) -> Result<AuxPayload, DriverError> {
        self.write_word(AUX, command)?;
        for _ in 0..AUX_POLLS {
            let mut header = [0; 4];
            self.read(AUX, &mut header)?;
            let header = u32::from_le_bytes(header);
            if header & (1 << 16) == 0 {
                continue;
            }
            let len = (((header & 0xffff) >> 9) as usize)
                .next_multiple_of(4)
                .clamp(4, 16);
            let mut payload = AuxPayload {
                bytes: [0; 16],
                len,
            };
            self.read(PAYLOAD, &mut payload.bytes[..len])?;
            return Ok(payload);
        }
        Err(DriverError::Localized(UiError::new(
            "error.device.slogic.auxReady",
        )))
    }

    fn aux_confirm(&self, mut payload: AuxPayload) -> Result<(), DriverError> {
        self.write(PAYLOAD, &payload.bytes[..payload.len])?;
        // This read acknowledges the AUX write. Its bytes may reflect the
        // device's interpretation of the setting, so the reference does not
        // compare them with the write.
        self.read(PAYLOAD, &mut payload.bytes[..payload.len])
    }

    fn aux_word(&self, command: u32, value: u32) -> Result<(), DriverError> {
        let mut payload = self.aux_begin(command)?;
        payload.bytes[..4].copy_from_slice(&value.to_le_bytes());
        self.aux_confirm(payload)
    }

    fn samplerate(&self, want: u64) -> Result<(), DriverError> {
        let mut payload = self.aux_begin(2)?;
        payload.len = payload.len.max(8);
        let mut prefer_800 = 800_000_000_u64.is_multiple_of(want);
        // The reference searches twice: prefer 800 MHz, then accept any
        // available clock that divides the requested rate. Empty slots wrap.
        for attempt in 0..16 {
            if attempt == 8 {
                prefer_800 = false;
            }
            let index = u16::from_le_bytes([payload.bytes[0], payload.bytes[1]]);
            let mhz = u16::from_le_bytes([payload.bytes[2], payload.bytes[3]]);
            let base = u64::from(mhz) * 1_000_000;
            if base != 0 && base.is_multiple_of(want) && (!prefer_800 || mhz == 800) {
                let divider = (base / want - 1) as u32;
                payload.bytes[4..8].copy_from_slice(&divider.to_le_bytes());
                return self.aux_confirm(payload);
            }
            let next = if base == 0 { 0 } else { index.wrapping_add(1) };
            payload.bytes[..2].copy_from_slice(&next.to_le_bytes());
            self.write(PAYLOAD, &payload.bytes[..4])?;
            self.read(PAYLOAD, &mut payload.bytes[..payload.len])?;
        }
        Err(DriverError::Localized(UiError::new(
            "error.device.slogic.sampleClock",
        )))
    }

    /// Stops a previous capture, drains stale samples, and resets the device.
    pub fn initialize(&self) -> Result<(), DriverError> {
        // Unlike the reference, fail if STOP never succeeds rather than open
        // a device that may still be capturing with another configuration.
        self.stop_retry(2)?;
        let _ = self.usb.clear_halt(EP_IN);
        for _ in 0..32 {
            match self
                .usb
                .bulk_read(EP_IN, 64 * 1024, Duration::from_millis(50))
            {
                Ok(bytes) if !bytes.is_empty() => {}
                Ok(_) | Err(UsbError::Timeout) => return self.reset(),
                Err(error) => return Err(error.into()),
            }
        }
        // The reference proceeds after exhausting its drain budget. Refuse
        // to open with stale samples still arriving, so a later RUN cannot
        // mistake them for a new capture.
        Err(DriverError::Localized(UiError::new(
            "error.device.slogic.drain",
        )))
    }

    /// Pulses RESET and leaves the capture gate stopped.
    pub fn reset(&self) -> Result<(), DriverError> {
        self.write_word(CTRL, 2)?;
        self.stop()
    }

    /// Resets the device, clears its endpoint, and programs a stopped capture.
    pub fn prepare(&self, settings: &CaptureSettings) -> Result<(), DriverError> {
        self.reset()?;
        let _ = self.usb.clear_halt(EP_IN);
        self.configure(settings)
    }

    /// Programs channel mode, sample clock, threshold, then normal pattern mode.
    /// The caller queues bulk transfers before calling [`Self::run`].
    pub fn configure(&self, settings: &CaptureSettings) -> Result<(), DriverError> {
        if !matches!(settings.channels, 4 | 8 | 16 | 32)
            || settings.samplerate == 0
            || !settings.threshold.is_finite()
            || !(0.0..=6.0).contains(&settings.threshold)
        {
            return Err(DriverError::Localized(UiError::new(
                "error.device.slogic.captureSettings",
            )));
        }
        self.stop()?;
        self.aux_word(1, u32::MAX >> (32 - settings.channels))?;
        self.samplerate(settings.samplerate)?;
        self.aux_word(3, threshold_dac(settings.threshold))?;
        self.aux_word(5, 0)
    }

    /// Opens the capture gate, after sample transfers have been queued.
    pub fn run(&self) -> Result<(), DriverError> {
        self.write_word(CTRL, 1)
    }

    /// Closes the capture gate. Send this BEFORE cancelling sample transfers:
    /// otherwise the reference warns that firmware may reset off the USB bus.
    pub fn stop(&self) -> Result<(), DriverError> {
        self.write_word(CTRL, 0)
    }

    /// Retries STOP at 10 ms intervals, as the reference's teardown does.
    pub fn stop_retry(&self, attempts: usize) -> Result<(), DriverError> {
        for attempt in 0..attempts.max(1) {
            match self.stop() {
                Ok(()) => return Ok(()),
                Err(error) if attempt + 1 == attempts.max(1) => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        unreachable!("at least one STOP attempt is made")
    }
}
