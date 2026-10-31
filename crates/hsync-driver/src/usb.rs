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
    Transfer(TransferError),
    /// The system refused memory for a transfer, which never reached the device: Linux charges
    /// the transfers of every program to one pool, usbcore's `usbfs_memory_mb`.
    #[error("usb transfer: out of memory")]
    NoMemory,
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

/// The `errno` usbfs answers when its pool cannot hold a transfer, which nusb passes on as
/// [`TransferError::Unknown`].
const ENOMEM: u32 = 12;

impl From<TransferError> for UsbError {
    fn from(error: TransferError) -> Self {
        match error {
            // Other systems report their own codes in `Unknown`, where 12 means something else.
            TransferError::Unknown(ENOMEM) if cfg!(target_os = "linux") => Self::NoMemory,
            error => Self::Transfer(error),
        }
    }
}

/// How many of `count` transfers of `size` bytes fit in `budget` (see
/// [`UsbTransport::transfer_budget`]); always at least one.
pub fn transfers_within(budget: Option<usize>, size: usize, count: usize) -> usize {
    budget.map_or(count, |budget| count.min(budget / size.max(1)).max(1))
}

/// The pool of usbfs, in MiB.
const USBFS_MEMORY_MB: &str = "/sys/module/usbcore/parameters/usbfs_memory_mb";

/// [`UsbTransport::transfer_budget`] under usbfs, from what [`USBFS_MEMORY_MB`] holds (the
/// kernel's default, 16, when it cannot be read): three quarters of the pool, leaving room for
/// other programs and the kernel's bookkeeping of each transfer; no limit for a pool of 0, which
/// usbfs leaves unlimited.
fn usbfs_budget(limit: Option<&str>) -> Option<usize> {
    let mib = limit
        .and_then(|text| text.trim().parse::<usize>().ok())
        .unwrap_or(16);
    (mib > 0).then(|| mib.saturating_mul(1 << 20) / 4 * 3)
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

    /// Reads once from a bulk IN endpoint, as `libusb_bulk_transfer` does: up to `length` bytes,
    /// or whatever arrived before `timeout`, possibly nothing. `length` must be a multiple of the
    /// endpoint's maximum packet size.
    fn bulk_read(
        &self,
        endpoint: u8,
        length: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, UsbError>;

    /// A queue of bulk IN transfers on `endpoint`.
    fn bulk_in(&self, endpoint: u8) -> Result<Box<dyn BulkIn>, UsbError>;

    /// Clears a halt on a bulk endpoint, as `libusb_clear_halt` does: the device gets a
    /// `CLEAR_FEATURE(ENDPOINT_HALT)` and both sides restart the endpoint's data toggle. No
    /// transfers may be pending on it.
    fn clear_halt(&self, endpoint: u8) -> Result<(), UsbError>;

    /// How many bytes the transfers queued on bulk IN endpoints may hold together, or `None`
    /// without a limit. Linux charges the transfers of every program to one pool, usbcore's
    /// `usbfs_memory_mb` (16 MiB by default), and fails a transfer that does not fit with
    /// [`UsbError::NoMemory`]; other systems have no such pool.
    fn transfer_budget(&self) -> Option<usize> {
        None
    }
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
    /// The endpoint of the last [`UsbTransport::bulk_read`], kept for the next one; a queue
    /// from [`UsbTransport::bulk_in`] on the same endpoint takes it over.
    bulk_read: Mutex<Option<(u8, Endpoint<Bulk, In>)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            bulk_read: Mutex::new(None),
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
        let mut cached = lock(&self.bulk_out);
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

    fn bulk_read(
        &self,
        endpoint: u8,
        length: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, UsbError> {
        let mut cached = lock(&self.bulk_read);
        if cached
            .as_ref()
            .is_none_or(|(address, _)| *address != endpoint)
        {
            *cached = Some((endpoint, self.interface.endpoint::<Bulk, In>(endpoint)?));
        }
        let Some((_, input)) = cached.as_mut() else {
            unreachable!("the endpoint was just opened");
        };
        received(input.transfer_blocking(Buffer::new(length), timeout))
    }

    fn clear_halt(&self, endpoint: u8) -> Result<(), UsbError> {
        // An endpoint is claimed once: one kept for single reads or writes clears its own halt.
        if endpoint & 0x80 != 0 {
            if let Some((address, input)) = lock(&self.bulk_read).as_mut()
                && *address == endpoint
            {
                return Ok(input.clear_halt().wait()?);
            }
            let mut input = self.interface.endpoint::<Bulk, In>(endpoint)?;
            Ok(input.clear_halt().wait()?)
        } else {
            if let Some((address, out)) = lock(&self.bulk_out).as_mut()
                && *address == endpoint
            {
                return Ok(out.clear_halt().wait()?);
            }
            let mut out = self.interface.endpoint::<Bulk, Out>(endpoint)?;
            Ok(out.clear_halt().wait()?)
        }
    }

    fn bulk_in(&self, endpoint: u8) -> Result<Box<dyn BulkIn>, UsbError> {
        // An endpoint is claimed once; the queue takes it over from single reads.
        let cached = lock(&self.bulk_read).take_if(|(address, _)| *address == endpoint);
        let endpoint = match cached {
            Some((_, endpoint)) => endpoint,
            None => self.interface.endpoint::<Bulk, In>(endpoint)?,
        };
        Ok(Box::new(NusbBulkIn { endpoint }))
    }

    fn transfer_budget(&self) -> Option<usize> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        usbfs_budget(std::fs::read_to_string(USBFS_MEMORY_MB).ok().as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_usbfs_memory_exhaustion_on_linux_only() {
        let error = UsbError::from(TransferError::Unknown(ENOMEM));
        assert_eq!(
            matches!(error, UsbError::NoMemory),
            cfg!(target_os = "linux")
        );
        assert!(matches!(
            UsbError::from(TransferError::Stall),
            UsbError::Transfer(TransferError::Stall)
        ));
    }

    #[test]
    fn budgets_three_quarters_of_the_usbfs_pool() {
        assert_eq!(usbfs_budget(Some("16\n")), Some(12 << 20));
        assert_eq!(usbfs_budget(Some("256")), Some(192 << 20));
        assert_eq!(usbfs_budget(Some("0\n")), None);
        // An unreadable or garbled limit is taken as the kernel's default.
        assert_eq!(usbfs_budget(None), Some(12 << 20));
        assert_eq!(usbfs_budget(Some("lots")), Some(12 << 20));
    }

    #[test]
    fn fits_transfers_in_the_budget() {
        assert_eq!(transfers_within(None, 1 << 20, 16), 16);
        assert_eq!(transfers_within(Some(12 << 20), 1 << 20, 16), 12);
        assert_eq!(transfers_within(Some(12 << 20), 64 << 10, 16), 16);
        assert_eq!(transfers_within(Some(1 << 20), 4 << 20, 4), 1);
    }
}
