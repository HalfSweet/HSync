use hsync_core::UiError;

use crate::usb::UsbError;

/// Errors of drivers and devices.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// A device-specific i18n key and parameters, passed unchanged to the interface.
    #[error(transparent)]
    Localized(UiError),
    /// USB communication failed.
    #[error(transparent)]
    Usb(#[from] UsbError),
    /// A firmware or FPGA bitstream file is missing.
    #[error("missing firmware file {0}")]
    FirmwareMissing(String),
    /// The device runs firmware this driver cannot work with.
    #[error("incompatible firmware {0}")]
    FirmwareIncompatible(String),
    /// The device needs firmware and is restarting after receiving it.
    #[error("the device is restarting with new firmware")]
    Restarting,
    /// The device's firmware needs an update that only this vendor tool performs.
    #[error("the device firmware needs an update with {0}")]
    FirmwareUpdate(&'static str),
    /// The device reported a fault; plugging it in again usually clears it.
    #[error("the device reported a fault")]
    Fault,
    /// Another program holds the device.
    #[error("the device is in use by another program")]
    Busy,
    /// The operating system has no usable driver for the device (Windows needs WinUSB).
    #[error("no usable USB driver for the device")]
    NoDriver,
    /// The device waited for a state that never came.
    #[error("the device did not respond in time: {0}")]
    Timeout(&'static str),
    /// The device reported something the driver does not understand.
    #[error("unexpected response: {0}")]
    Protocol(String),
    /// A setting or capture request the device cannot honour.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The device is not supported by this version.
    #[error("unsupported device")]
    Unsupported,
}

impl DriverError {
    /// Why opening a USB device failed, in terms the user can act on.
    pub fn opening(error: UsbError) -> Self {
        if let UsbError::Device(device) = &error {
            match device.kind() {
                nusb::ErrorKind::Busy => return Self::Busy,
                nusb::ErrorKind::NotFound | nusb::ErrorKind::Unsupported if cfg!(windows) => {
                    return Self::NoDriver;
                }
                _ => {}
            }
        }
        Self::Usb(error)
    }
}

impl From<&DriverError> for UiError {
    fn from(error: &DriverError) -> Self {
        match error {
            DriverError::Localized(error) => error.clone(),
            DriverError::Usb(usb) => {
                UiError::new("error.device.usb").with("detail", usb.to_string())
            }
            DriverError::FirmwareMissing(file) => {
                UiError::new("error.device.firmwareMissing").with("file", file.as_str())
            }
            DriverError::FirmwareIncompatible(version) => {
                UiError::new("error.device.firmwareIncompatible").with("version", version.as_str())
            }
            DriverError::Restarting => UiError::new("error.device.restarting"),
            DriverError::FirmwareUpdate(tool) => {
                UiError::new("error.device.firmwareUpdate").with("tool", *tool)
            }
            DriverError::Fault => UiError::new("error.device.fault"),
            DriverError::Busy => UiError::new("error.device.busy"),
            DriverError::NoDriver => UiError::new("error.device.noDriver"),
            DriverError::Timeout(what) => {
                UiError::new("error.device.timeout").with("detail", *what)
            }
            DriverError::Protocol(detail) => {
                UiError::new("error.device.protocol").with("detail", detail.as_str())
            }
            DriverError::InvalidRequest(detail) => {
                UiError::new("error.device.invalidRequest").with("detail", detail.as_str())
            }
            DriverError::Unsupported => UiError::new("error.device.unsupported"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_localized_device_errors_without_natural_language_details() {
        let message = UiError::new("error.device.slogic.channel").with("channel", 16);
        let error = DriverError::Localized(message.clone());
        assert_eq!(UiError::from(&error), message);
    }
}
