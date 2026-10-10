//! USB access for drivers. Hardware drivers talk to their devices only through
//! [`UsbTransport`], so their protocols can be tested against a scripted transport
//! (`mock` feature) and run on [`NusbTransport`] in the application.

use std::sync::Mutex;
use std::time::Duration;

use nusb::transfer::{
    Buffer, Bulk, Completion, ControlIn, ControlOut, ControlType, In, Out, Recipient, TransferError,
};
use nusb::{Endpoint, Interface, MaybeFuture};

/// Errors of USB transfers.
#[derive(Debug, Clone, thiserror::Error)]
pub enum UsbError {
    /// Opening or configuring the device failed.
    #[error("usb: {0}")]
    Device(#[from] nusb::Error),
    /// A transfer failed.
    #[error("usb transfer: {0}")]
    Transfer(#[from] TransferError),
    /// A transfer moved fewer bytes than expected.
    #[error("usb transfer moved {actual} of {expected} bytes")]
    Short { expected: usize, actual: usize },
    /// A transfer did not complete in time.
    #[error("usb transfer timed out")]
    Timeout,
}

impl UsbError {
    /// Whether the device has gone away.
    pub fn is_disconnected(&self) -> bool {
        match self {
            Self::Device(error) => error.kind() == nusb::ErrorKind::Disconnected,
            Self::Transfer(error) => *error == TransferError::Disconnected,
            _ => false,
        }
    }
}

/// Vendor requests and bulk transfers on one claimed interface.
pub trait UsbTransport: Send + Sync {
    /// Vendor-type OUT control request to the device.
    fn control_out(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), UsbError>;

    /// Vendor-type IN control request to the device.
    fn control_in(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        timeout: Duration,
    ) -> Result<Vec<u8>, UsbError>;

    /// Writes `data` to a bulk OUT endpoint.
    fn bulk_out(&self, endpoint: u8, data: &[u8], timeout: Duration) -> Result<(), UsbError>;

    /// A queue of bulk IN transfers on `endpoint`.
    fn bulk_in(&self, endpoint: u8) -> Result<Box<dyn BulkIn>, UsbError>;
}

/// A queue of bulk IN transfers that complete in submission order.
pub trait BulkIn: Send {
    /// Queues a transfer of up to `length` bytes; `length` must be a multiple of the endpoint's
    /// maximum packet size.
    fn submit(&mut self, length: usize);

    /// The next completed transfer, waiting at most `timeout`; `None` if none completed. A
    /// transfer still completes later after a timeout. A cancelled transfer completes with the
    /// bytes it received before it was cancelled, possibly none.
    fn next(&mut self, timeout: Duration) -> Option<Result<Vec<u8>, UsbError>>;

    /// Transfers submitted and not yet returned by [`Self::next`].
    fn pending(&self) -> usize;

    /// Cancels every pending transfer; each still completes, with what it had received.
    fn cancel_all(&mut self);
}

/// [`UsbTransport`] on a claimed nusb interface.
#[derive(Debug)]
pub struct NusbTransport {
    interface: Interface,
    bulk_out: Mutex<Option<(u8, Endpoint<Bulk, Out>)>>,
}

/// The bytes an IN transfer received; a cancelled one keeps what arrived before it was
/// cancelled.
fn received(completion: Completion) -> Result<Vec<u8>, UsbError> {
    match completion.status {
        Ok(()) | Err(TransferError::Cancelled) => {
            let mut data = completion.buffer.into_vec();
            data.truncate(completion.actual_len);
            Ok(data)
        }
        Err(error) => Err(error.into()),
    }
}

impl NusbTransport {
    /// Opens `info` and claims `interface`, detaching kernel drivers where the platform needs it.
    /// A device left unconfigured (macOS does that for vendor-class devices without a driver)
    /// first gets `configuration`, as libusb does when claiming.
    pub fn open(
        info: &nusb::DeviceInfo,
        configuration: u8,
        interface: u8,
    ) -> Result<Self, UsbError> {
        let device = info.open().wait()?;
        match Self::claim(&device, interface) {
            // The device's own answer to GET_CONFIGURATION cannot be trusted here: DSLogic
            // firmware reports configuration 1 while macOS still holds it unconfigured.
            Err(UsbError::Device(error)) if error.kind() == nusb::ErrorKind::NotFound => {
                log::debug!(
                    "interface {interface} not found, selecting configuration {configuration}"
                );
                device.set_configuration(configuration).wait()?;
                Self::claim(&device, interface)
            }
            other => other,
        }
    }

    /// Opens `info` and selects `configuration` before claiming `interface`, as firmware
    /// uploads need.
    pub fn open_configured(
        info: &nusb::DeviceInfo,
        configuration: u8,
        interface: u8,
    ) -> Result<Self, UsbError> {
        let device = info.open().wait()?;
        if let Err(error) = device.set_configuration(configuration).wait() {
            // Some platforms refuse to set the configuration that is already active.
            log::debug!("set_configuration({configuration}) failed: {error}");
        }
        Self::claim(&device, interface)
    }

    fn claim(device: &nusb::Device, interface: u8) -> Result<Self, UsbError> {
        let interface = device.detach_and_claim_interface(interface).wait()?;
        Ok(Self {
            interface,
            bulk_out: Mutex::new(None),
        })
    }
}

impl UsbTransport for NusbTransport {
    fn control_out(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), UsbError> {
        let transfer = ControlOut {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request,
            value,
            index,
            data,
        };
        Ok(self.interface.control_out(transfer, timeout).wait()?)
    }

    fn control_in(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        timeout: Duration,
    ) -> Result<Vec<u8>, UsbError> {
        let transfer = ControlIn {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request,
            value,
            index,
            length,
        };
        Ok(self.interface.control_in(transfer, timeout).wait()?)
    }

    fn bulk_out(&self, endpoint: u8, data: &[u8], timeout: Duration) -> Result<(), UsbError> {
        let mut cached = self
            .bulk_out
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cached
            .as_ref()
            .is_none_or(|(address, _)| *address != endpoint)
        {
            *cached = Some((endpoint, self.interface.endpoint::<Bulk, Out>(endpoint)?));
        }
        let Some((_, out)) = cached.as_mut() else {
            unreachable!("the endpoint was just opened");
        };
        let completion = out.transfer_blocking(Buffer::from(data), timeout);
        completion.status?;
        if completion.actual_len != data.len() {
            return Err(UsbError::Short {
                expected: data.len(),
                actual: completion.actual_len,
            });
        }
        Ok(())
    }

    fn bulk_in(&self, endpoint: u8) -> Result<Box<dyn BulkIn>, UsbError> {
        let endpoint = self.interface.endpoint::<Bulk, In>(endpoint)?;
        Ok(Box::new(NusbBulkIn { endpoint }))
    }
}

struct NusbBulkIn {
    endpoint: Endpoint<Bulk, In>,
}

impl BulkIn for NusbBulkIn {
    fn submit(&mut self, length: usize) {
        self.endpoint.submit(self.endpoint.allocate(length));
    }

    fn next(&mut self, timeout: Duration) -> Option<Result<Vec<u8>, UsbError>> {
        if self.endpoint.pending() == 0 {
            return None;
        }
        let completion = self.endpoint.wait_next_complete(timeout)?;
        Some(received(completion))
    }

    fn pending(&self) -> usize {
        self.endpoint.pending()
    }

    fn cancel_all(&mut self) {
        self.endpoint.cancel_all();
    }
}
