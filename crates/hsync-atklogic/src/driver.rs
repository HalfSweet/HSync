//! Recognising ATK-Logic devices and opening them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use hsync_driver::usb::NusbTransport;
use hsync_driver::{Candidate, CandidateStatus, Device, Driver, DriverError};

use crate::device::AtkLogicDevice;
use crate::protocol::{Link, PRODUCT_ID, VENDOR_ID};

const USB_CONFIGURATION: u8 = 1;
const USB_INTERFACE: u8 = 0;

/// The ATK-Logic driver. Every model has the same USB ids and says which it is only once
/// opened; the driver remembers that for the device's USB port, so later scans name it.
#[derive(Debug, Default)]
pub struct AtkLogicDriver {
    models: Mutex<HashMap<String, String>>,
}

impl AtkLogicDriver {
    pub fn new() -> Self {
        Self::default()
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

/// Stable while the device stays on the same port.
fn location(usb: &nusb::DeviceInfo) -> String {
    let ports: Vec<String> = usb.port_chain().iter().map(u8::to_string).collect();
    format!(
        "{}:{}-{}",
        crate::DRIVER_NAME,
        usb.bus_id(),
        ports.join(".")
    )
}

impl Driver for AtkLogicDriver {
    fn name(&self) -> &'static str {
        crate::DRIVER_NAME
    }

    fn identify(&self, usb: &nusb::DeviceInfo) -> Option<Candidate> {
        if usb.vendor_id() != VENDOR_ID || usb.product_id() != PRODUCT_ID {
            return None;
        }
        let id = location(usb);
        // The device moves its samples in 512-byte packets, which USB 2.0 high speed carries.
        let status = match usb.speed() {
            Some(nusb::Speed::Low | nusb::Speed::Full) => CandidateStatus::SpeedTooLow,
            _ => CandidateStatus::Ready,
        };
        Some(Candidate {
            model: self
                .model(&id)
                .unwrap_or_else(|| crate::DRIVER_NAME.to_owned()),
            id,
            driver: crate::DRIVER_NAME,
            vendor: crate::VENDOR.to_owned(),
            status,
            usb: Some(usb.clone()),
        })
    }

    fn open(&self, candidate: &Candidate) -> Result<Box<dyn Device>, DriverError> {
        if candidate.status != CandidateStatus::Ready {
            return Err(DriverError::Unsupported);
        }
        let usb = candidate.usb.as_ref().ok_or(DriverError::Unsupported)?;
        log::debug!("opening the ATK-Logic at {}", candidate.id);
        let transport = NusbTransport::open(usb, USB_CONFIGURATION, USB_INTERFACE)
            .map_err(DriverError::opening)?;
        let device = AtkLogicDevice::open(Link::new(Arc::new(transport)), candidate.id.clone())?;
        self.remember(&candidate.id, hsync_driver::Device::info(&device).model);
        Ok(Box::new(device))
    }
}
