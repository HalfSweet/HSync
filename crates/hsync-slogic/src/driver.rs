// SPDX-License-Identifier: GPL-3.0-or-later
//! SLogic16U3 discovery, following Sipeed's libsigrok `slogic-dev` driver:
//! `api.c` and `slogic/slogic_usb.c`.

use std::sync::Arc;

use hsync_driver::usb::NusbTransport;
use hsync_driver::{Candidate, CandidateStatus, Device, Driver, DriverError};

use crate::device::{SlogicDevice, UsbSpeed};
use crate::protocol::Link;

/// Sipeed's USB vendor id.
pub const VENDOR_ID: u16 = 0x359f;
/// SLogic16U3's USB product id, distinct from the Combo8 and SLogic32U3.
pub const PRODUCT_ID: u16 = 0x3031;

const USB_CONFIGURATION: u8 = 1;
const USB_INTERFACE: u8 = 0;

/// Sipeed SLogic16U3 driver. Firmware runs on the device and is not uploaded by the host.
#[derive(Debug, Default)]
pub struct SlogicDriver;

impl SlogicDriver {
    /// Creates a driver ready to discover devices.
    pub fn new() -> Self {
        Self
    }
}

fn recognized(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == VENDOR_ID && product_id == PRODUCT_ID
}

fn link_speed(speed: Option<nusb::Speed>) -> Option<UsbSpeed> {
    match speed {
        Some(nusb::Speed::Super | nusb::Speed::SuperPlus) => Some(UsbSpeed::Super),
        Some(nusb::Speed::High) => Some(UsbSpeed::High),
        // Be conservative when the platform does not report the speed. Unlike the official
        // adapter's uncapped unknown link, this avoids offering rates USB 2.0 cannot carry.
        None => Some(UsbSpeed::High),
        _ => None,
    }
}

/// Stable across resets and reconnects on the same USB port.
fn location(usb: &nusb::DeviceInfo) -> String {
    let ports: Vec<String> = usb.port_chain().iter().map(u8::to_string).collect();
    format!(
        "{}:{}-{}",
        crate::DRIVER_NAME,
        usb.bus_id(),
        ports.join(".")
    )
}

impl Driver for SlogicDriver {
    fn name(&self) -> &'static str {
        crate::DRIVER_NAME
    }

    fn identify(&self, usb: &nusb::DeviceInfo) -> Option<Candidate> {
        if !recognized(usb.vendor_id(), usb.product_id()) {
            return None;
        }
        Some(Candidate {
            id: location(usb),
            driver: crate::DRIVER_NAME,
            vendor: crate::VENDOR.to_owned(),
            model: "SLogic16U3".to_owned(),
            status: if link_speed(usb.speed()).is_some() {
                CandidateStatus::Ready
            } else {
                CandidateStatus::SpeedTooLow
            },
            usb: Some(usb.clone()),
        })
    }

    fn open(&self, candidate: &Candidate) -> Result<Box<dyn Device>, DriverError> {
        let usb = candidate.usb.as_ref().ok_or(DriverError::Unsupported)?;
        if candidate.status != CandidateStatus::Ready
            || !recognized(usb.vendor_id(), usb.product_id())
        {
            return Err(DriverError::Unsupported);
        }
        let speed = link_speed(usb.speed()).ok_or(DriverError::Unsupported)?;
        let transport = NusbTransport::open(usb, USB_CONFIGURATION, USB_INTERFACE)
            .map_err(DriverError::opening)?;
        Ok(Box::new(SlogicDevice::open(
            Link::new(Arc::new(transport)),
            speed,
            candidate.id.clone(),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_the_slogic16u3_ids() {
        assert!(recognized(0x359f, 0x3031));
        assert!(!recognized(0x359f, 0x3030));
        assert!(!recognized(0x359f, 0x3032));
        assert!(!recognized(0x0000, 0x3031));
    }

    #[test]
    fn classifies_supported_usb_links_and_caps_unknown_links_conservatively() {
        assert_eq!(
            link_speed(Some(nusb::Speed::SuperPlus)),
            Some(UsbSpeed::Super)
        );
        assert_eq!(link_speed(Some(nusb::Speed::Super)), Some(UsbSpeed::Super));
        assert_eq!(link_speed(Some(nusb::Speed::High)), Some(UsbSpeed::High));
        assert_eq!(link_speed(None), Some(UsbSpeed::High));
        assert_eq!(link_speed(Some(nusb::Speed::Full)), None);
        assert_eq!(link_speed(Some(nusb::Speed::Low)), None);
    }
}
