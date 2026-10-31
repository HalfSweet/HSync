// SPDX-License-Identifier: GPL-3.0-or-later
//! SLogic discovery, following Sipeed's libsigrok `slogic-dev` driver:
//! `api.c` and `slogic/slogic_usb.c`.

use std::sync::Arc;

use hsync_driver::usb::NusbTransport;
use hsync_driver::{Candidate, CandidateStatus, Device, Driver, DriverError};

use crate::device::SlogicDevice;
use crate::model::{self, Model, UsbSpeed};
use crate::protocol::Link;

/// Sipeed's USB vendor id.
pub const VENDOR_ID: u16 = 0x359f;

const USB_CONFIGURATION: u8 = 1;
const USB_INTERFACE: u8 = 0;

/// Sipeed SLogic Combo 8, SLogic16U3 and SLogic32U3 driver. Firmware is not
/// uploaded by the host.
#[derive(Debug, Default)]
pub struct SlogicDriver;

impl SlogicDriver {
    /// Creates a driver ready to discover devices.
    pub fn new() -> Self {
        Self
    }
}

fn recognized(vendor_id: u16, product_id: u16) -> Option<&'static Model> {
    model::find(product_id).filter(|_| vendor_id == VENDOR_ID)
}

fn link_speed(speed: Option<nusb::Speed>) -> Option<UsbSpeed> {
    match speed {
        Some(nusb::Speed::SuperPlus) => Some(UsbSpeed::SuperPlus),
        Some(nusb::Speed::Super) => Some(UsbSpeed::Super),
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
        let model = recognized(usb.vendor_id(), usb.product_id())?;
        Some(Candidate {
            id: location(usb),
            driver: crate::DRIVER_NAME,
            vendor: crate::VENDOR.to_owned(),
            model: model.name.to_owned(),
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
        let model = recognized(usb.vendor_id(), usb.product_id())
            .filter(|_| candidate.status == CandidateStatus::Ready)
            .ok_or(DriverError::Unsupported)?;
        let speed = link_speed(usb.speed()).ok_or(DriverError::Unsupported)?;
        let transport = NusbTransport::open(usb, USB_CONFIGURATION, USB_INTERFACE)
            .map_err(DriverError::opening)?;
        Ok(Box::new(SlogicDevice::open(
            Link::for_model(Arc::new(transport), model),
            model,
            speed,
            candidate.id.clone(),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_the_capture_ids() {
        assert_eq!(recognized(0x359f, 0x3031), Some(&model::SLOGIC16U3));
        assert_eq!(recognized(0x359f, 0x3032), Some(&model::SLOGIC32U3));
        assert_eq!(recognized(0x359f, 0x0300), Some(&model::SLOGIC_COMBO8));
        assert_eq!(recognized(0x359f, 0x3000), Some(&model::SLOGIC_COMBO8));
        for product in [0x3030, 0x30f1] {
            assert_eq!(recognized(0x359f, product), None, "{product:04x}");
        }
        assert_eq!(recognized(0x0000, 0x3031), None);
        assert_eq!(recognized(0x0000, 0x0300), None);
        assert_eq!(recognized(0x0000, 0x3000), None);
    }

    #[test]
    fn classifies_supported_usb_links_and_caps_unknown_links_conservatively() {
        assert_eq!(
            link_speed(Some(nusb::Speed::SuperPlus)),
            Some(UsbSpeed::SuperPlus)
        );
        assert_eq!(link_speed(Some(nusb::Speed::Super)), Some(UsbSpeed::Super));
        assert_eq!(link_speed(Some(nusb::Speed::High)), Some(UsbSpeed::High));
        assert_eq!(link_speed(None), Some(UsbSpeed::High));
        assert_eq!(link_speed(Some(nusb::Speed::Full)), None);
        assert_eq!(link_speed(Some(nusb::Speed::Low)), None);
    }
}
