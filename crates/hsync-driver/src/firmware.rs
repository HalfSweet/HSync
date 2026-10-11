//! Firmware images and FPGA bitstreams that drivers load into devices.

use std::path::PathBuf;

use crate::error::DriverError;

/// Where firmware images and FPGA bitstreams are read from.
#[derive(Debug, Clone)]
pub struct FirmwareDir(pub PathBuf);

impl FirmwareDir {
    /// Reads a file by the name its vendor's software gives it.
    pub fn load(&self, name: &str) -> Result<Vec<u8>, DriverError> {
        std::fs::read(self.0.join(name)).map_err(|error| {
            log::warn!("reading firmware {name} from {}: {error}", self.0.display());
            DriverError::FirmwareMissing(name.to_owned())
        })
    }
}
