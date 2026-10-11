//! Recognising PXLogic devices and opening them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hsync_driver::usb::NusbTransport;
use hsync_driver::{Candidate, CandidateStatus, Device, Driver, DriverError, FirmwareDir};
use nusb::MaybeFuture;
use nusb::descriptors::language_id::US_ENGLISH;

use crate::device::PxlogicDevice;
use crate::profiles::{MANUFACTURER_PREFIX, Profile, UsbSpeed, find_profile, is_pxlogic};
use crate::protocol::Control;

const USB_CONFIGURATION: u8 = 1;
const USB_INTERFACE: u8 = 0;
const DESCRIPTOR_TIMEOUT: Duration = Duration::from_millis(1000);

/// The PXLogic driver. Firmware and FPGA bitstreams are read from `firmware`.
///
/// The current models share their USB ids, and say which they are only once opened; the driver
/// remembers that for the device's USB port, so later scans name it.
#[derive(Debug)]
pub struct PxlogicDriver {
    firmware: FirmwareDir,
    models: Mutex<HashMap<String, String>>,
}

impl PxlogicDriver {
    pub fn new(firmware: FirmwareDir) -> Self {
        Self {
            firmware,
            models: Mutex::new(HashMap::new()),
        }
    }

    fn model(&self, id: &str) -> Option<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    fn remember(&self, id: &str, model: String) {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned(), model);
    }
}

/// PXView takes only devices at USB 2.0 high speed or faster.
fn link_speed(usb: &nusb::DeviceInfo) -> Option<UsbSpeed> {
    match usb.speed()? {
        nusb::Speed::High => Some(UsbSpeed::High),
        nusb::Speed::Super | nusb::Speed::SuperPlus => Some(UsbSpeed::Super),
        _ => None,
    }
}

/// The profile a scan finds: PXView reads the logic mode only once the device is opened.
fn scanned_profile(usb: &nusb::DeviceInfo) -> Option<&'static Profile> {
    find_profile(usb.vendor_id(), usb.product_id(), link_speed(usb)?, 0)
}

/// Stable while the device stays on the same port, also across a firmware update.
fn location(usb: &nusb::DeviceInfo) -> String {
    let ports: Vec<String> = usb.port_chain().iter().map(u8::to_string).collect();
    format!(
        "{}:{}-{}",
        crate::DRIVER_NAME,
        usb.bus_id(),
        ports.join(".")
    )
}

/// Whether the device's manufacturer string names PXLogic (`logic_check_conf_profile()`).
/// Windows lists devices without it, so there it is read from the device.
fn made_by_pxlogic(usb: &nusb::DeviceInfo) -> bool {
    if let Some(manufacturer) = usb.manufacturer_string() {
        return manufacturer.starts_with(MANUFACTURER_PREFIX);
    }
    let read = || -> Option<String> {
        let device = usb.open().wait().ok()?;
        let index = device.device_descriptor().manufacturer_string_index()?;
        device
            .get_string_descriptor(index, US_ENGLISH, DESCRIPTOR_TIMEOUT)
            .wait()
            .ok()
    };
    read().is_some_and(|manufacturer| manufacturer.starts_with(MANUFACTURER_PREFIX))
}

impl Driver for PxlogicDriver {
    fn name(&self) -> &'static str {
        crate::DRIVER_NAME
    }

    fn identify(&self, usb: &nusb::DeviceInfo) -> Option<Candidate> {
        if !is_pxlogic(usb.vendor_id(), usb.product_id()) {
            return None;
        }
        // Other devices share these ids; a manufacturer string that is not PXLogic's rules them
        // out (Windows lists none, so those are told apart when opened).
        if usb
            .manufacturer_string()
            .is_some_and(|manufacturer| !manufacturer.starts_with(MANUFACTURER_PREFIX))
        {
            return None;
        }
        let profile = scanned_profile(usb)?;
        let id = location(usb);
        let model = if Profile::known_by_ids(usb.vendor_id(), usb.product_id()) {
            profile.model.to_owned()
        } else {
            self.model(&id)
                .unwrap_or_else(|| crate::DRIVER_NAME.to_owned())
        };
        Some(Candidate {
            id,
            driver: crate::DRIVER_NAME,
            vendor: crate::VENDOR.to_owned(),
            model,
            status: CandidateStatus::Ready,
            usb: Some(usb.clone()),
        })
    }

    fn open(&self, candidate: &Candidate) -> Result<Box<dyn Device>, DriverError> {
        let usb = candidate.usb.as_ref().ok_or(DriverError::Unsupported)?;
        let profile = scanned_profile(usb).ok_or(DriverError::Unsupported)?;
        if !made_by_pxlogic(usb) {
            return Err(DriverError::Unsupported);
        }
        log::debug!("opening the PXLogic at {}", candidate.id);
        let transport = NusbTransport::open(usb, USB_CONFIGURATION, USB_INTERFACE)
            .map_err(DriverError::opening)?;
        let device = PxlogicDevice::open(
            Control::new(Arc::new(transport)),
            profile,
            &self.firmware,
            candidate.id.clone(),
        )?;
        self.remember(&candidate.id, Device::info(&device).model);
        Ok(Box::new(device))
    }
}
