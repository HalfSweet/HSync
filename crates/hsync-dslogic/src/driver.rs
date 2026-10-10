//! Recognising DSLogic devices and opening them (DESIGN.md §4.3).

use std::sync::Arc;

use hsync_driver::usb::{NusbTransport, UsbError};
use hsync_driver::{Candidate, CandidateStatus, Device, Driver, DriverError};

use crate::boot::{self, FirmwareDir};
use crate::device::DslogicDevice;
use crate::profiles::{
    DSCOPE_PRODUCT_IDS, PROFILES, PROFILES_PGL12_2, Profile, UsbSpeed, VENDOR_ID, find_profile,
};
use crate::protocol::Control;

/// Product string of a device running DreamSourceLab's firmware (`dsl_check_conf_profile`).
const FIRMWARE_PRODUCT: &str = "USB-based DSL Instrument v2";
const USB_CONFIGURATION: u8 = 1;
const USB_INTERFACE: u8 = 0;

/// The DSLogic driver. Firmware images and FPGA bitstreams are read from `firmware`.
#[derive(Debug, Clone)]
pub struct DslogicDriver {
    firmware: FirmwareDir,
}

impl DslogicDriver {
    pub fn new(firmware: FirmwareDir) -> Self {
        Self { firmware }
    }
}

fn link_speed(usb: &nusb::DeviceInfo) -> Option<UsbSpeed> {
    match usb.speed()? {
        nusb::Speed::High => Some(UsbSpeed::High),
        nusb::Speed::Super | nusb::Speed::SuperPlus => Some(UsbSpeed::Super),
        _ => None,
    }
}

fn any_profile(pid: u16) -> Option<&'static Profile> {
    PROFILES
        .iter()
        .chain(PROFILES_PGL12_2.iter())
        .find(|profile| profile.pid == pid)
}

/// Stable while the device stays on the same port, also across the firmware upload.
fn location(usb: &nusb::DeviceInfo) -> String {
    let ports: Vec<String> = usb.port_chain().iter().map(u8::to_string).collect();
    format!(
        "{}:{}-{}",
        crate::DRIVER_NAME,
        usb.bus_id(),
        ports.join(".")
    )
}

/// Why opening the USB device failed, in terms the user can act on.
fn open_error(error: UsbError) -> DriverError {
    if let UsbError::Device(device) = &error {
        match device.kind() {
            nusb::ErrorKind::Busy => return DriverError::Busy,
            nusb::ErrorKind::NotFound | nusb::ErrorKind::Unsupported if cfg!(windows) => {
                return DriverError::NoDriver;
            }
            _ => {}
        }
    }
    DriverError::Usb(error)
}

impl Driver for DslogicDriver {
    fn name(&self) -> &'static str {
        crate::DRIVER_NAME
    }

    fn identify(&self, usb: &nusb::DeviceInfo) -> Option<Candidate> {
        if usb.vendor_id() != VENDOR_ID {
            return None;
        }
        let pid = usb.product_id();
        let candidate = |model: &str, status| Candidate {
            id: location(usb),
            driver: crate::DRIVER_NAME,
            vendor: crate::VENDOR.to_owned(),
            model: model.to_owned(),
            status,
            usb: Some(usb.clone()),
        };
        if DSCOPE_PRODUCT_IDS.contains(&pid) {
            return Some(candidate("DSCope", CandidateStatus::Unsupported));
        }
        let Some(speed) = link_speed(usb) else {
            let profile = any_profile(pid)?;
            return Some(candidate(profile.model, CandidateStatus::SpeedTooLow));
        };
        let profile = find_profile(VENDOR_ID, pid, speed)?;
        let status = match usb.product_string() {
            Some(product) if !product.starts_with(FIRMWARE_PRODUCT) => {
                CandidateStatus::NeedsFirmware
            }
            _ => CandidateStatus::Ready,
        };
        Some(candidate(profile.model, status))
    }

    fn open(&self, candidate: &Candidate) -> Result<Box<dyn Device>, DriverError> {
        let usb = candidate.usb.as_ref().ok_or(DriverError::Unsupported)?;
        let profile = link_speed(usb)
            .and_then(|speed| find_profile(VENDOR_ID, usb.product_id(), speed))
            .ok_or(DriverError::Unsupported)?;
        match candidate.status {
            CandidateStatus::Ready => {
                let transport = NusbTransport::open(usb, USB_INTERFACE).map_err(open_error)?;
                let ctl = Control::new(Arc::new(transport));
                let device =
                    DslogicDevice::open(ctl, profile, self.firmware.clone(), candidate.id.clone())?;
                Ok(Box::new(device))
            }
            CandidateStatus::NeedsFirmware => {
                let image = self.firmware.load(profile.firmware)?;
                let transport =
                    NusbTransport::open_configured(usb, USB_CONFIGURATION, USB_INTERFACE)
                        .map_err(open_error)?;
                boot::upload_firmware(&transport, &image)?;
                log::info!(
                    "uploaded {} to {}, waiting for it to restart",
                    profile.firmware,
                    profile.model
                );
                Err(DriverError::Restarting)
            }
            CandidateStatus::SpeedTooLow | CandidateStatus::Unsupported => {
                Err(DriverError::Unsupported)
            }
        }
    }
}
