//! A scriptable [`UsbTransport`] for driver tests: a [`MockDevice`] answers every request and
//! the transport records the calls in order.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::usb::{BulkIn, UsbError, UsbTransport};

/// One request a driver made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbCall {
    ControlOut {
        request: u8,
        value: u16,
        index: u16,
        data: Vec<u8>,
    },
    ControlIn {
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    },
    BulkOut {
        endpoint: u8,
        data: Vec<u8>,
    },
    BulkRead {
        endpoint: u8,
        length: usize,
    },
    BulkInSubmit {
        endpoint: u8,
        length: usize,
    },
    ClearHalt {
        endpoint: u8,
    },
}

/// The device behind a [`MockTransport`].
pub trait MockDevice: Send {
    fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError>;
    fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError>;
    fn bulk_out(&mut self, endpoint: u8, data: &[u8]) -> Result<(), UsbError>;
    /// Completes a submitted IN transfer of up to `length` bytes, or leaves it pending (`None`).
    fn bulk_in(&mut self, endpoint: u8, length: usize) -> Option<Result<Vec<u8>, UsbError>>;

    /// Answers a single read of up to `length` bytes; by default as a submitted transfer,
    /// one left pending timing out with nothing.
    fn bulk_read(&mut self, endpoint: u8, length: usize) -> Result<Vec<u8>, UsbError> {
        self.bulk_in(endpoint, length)
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    /// What a pending IN transfer of up to `length` bytes had received when it was cancelled;
    /// nothing by default.
    fn cancel_in(&mut self, _endpoint: u8, _length: usize) -> Vec<u8> {
        Vec::new()
    }

    /// Clears a halt on an endpoint; nothing to do by default.
    fn clear_halt(&mut self, _endpoint: u8) -> Result<(), UsbError> {
        Ok(())
    }

    /// The transport's [`UsbTransport::transfer_budget`]; no limit by default.
    fn transfer_budget(&self) -> Option<usize> {
        None
    }
}

/// A transport whose device is a [`MockDevice`].
pub struct MockTransport<D> {
    device: Arc<Mutex<D>>,
    calls: Arc<Mutex<Vec<UsbCall>>>,
}

impl<D> std::fmt::Debug for MockTransport<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockTransport").finish_non_exhaustive()
    }
}

impl<D> Clone for MockTransport<D> {
    fn clone(&self) -> Self {
        Self {
            device: Arc::clone(&self.device),
            calls: Arc::clone(&self.calls),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl<D: MockDevice> MockTransport<D> {
    pub fn new(device: D) -> Self {
        Self {
            device: Arc::new(Mutex::new(device)),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The device, to inspect or change its state.
    pub fn device(&self) -> MutexGuard<'_, D> {
        lock(&self.device)
    }

    /// Every call so far, in order.
    pub fn calls(&self) -> Vec<UsbCall> {
        lock(&self.calls).clone()
    }

    /// Forgets the recorded calls.
    pub fn clear_calls(&self) {
        lock(&self.calls).clear();
    }

    fn record(&self, call: UsbCall) {
        lock(&self.calls).push(call);
    }
}

impl<D: MockDevice + 'static> UsbTransport for MockTransport<D> {
    fn control_out(
        &self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        _timeout: Duration,
    ) -> Result<(), UsbError> {
        self.record(UsbCall::ControlOut {
            request,
            value,
            index,
            data: data.to_vec(),
        });
        self.device().control_out(request, value, index, data)
    }

    fn control_in(
        &self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
        _timeout: Duration,
    ) -> Result<Vec<u8>, UsbError> {
        self.record(UsbCall::ControlIn {
            request,
            value,
            index,
            length,
        });
        self.device().control_in(request, value, index, length)
    }

    fn bulk_out(&self, endpoint: u8, data: &[u8], _timeout: Duration) -> Result<(), UsbError> {
        self.record(UsbCall::BulkOut {
            endpoint,
            data: data.to_vec(),
        });
        self.device().bulk_out(endpoint, data)
    }

    fn bulk_read(
        &self,
        endpoint: u8,
        length: usize,
        _timeout: Duration,
    ) -> Result<Vec<u8>, UsbError> {
        self.record(UsbCall::BulkRead { endpoint, length });
        self.device().bulk_read(endpoint, length)
    }

    fn clear_halt(&self, endpoint: u8) -> Result<(), UsbError> {
        self.record(UsbCall::ClearHalt { endpoint });
        self.device().clear_halt(endpoint)
    }

    fn bulk_in(&self, endpoint: u8) -> Result<Box<dyn BulkIn>, UsbError> {
        Ok(Box::new(MockBulkIn {
            transport: self.clone(),
            endpoint,
            queue: VecDeque::new(),
        }))
    }

    fn transfer_budget(&self) -> Option<usize> {
        self.device().transfer_budget()
    }
}

/// A transfer of a [`MockBulkIn`].
enum Queued {
    /// Pending, of up to this many bytes.
    Pending(usize),
    /// Cancelled with what it had received.
    Cancelled(Vec<u8>),
}

struct MockBulkIn<D> {
    transport: MockTransport<D>,
    endpoint: u8,
    queue: VecDeque<Queued>,
}

impl<D: MockDevice + 'static> BulkIn for MockBulkIn<D> {
    fn submit(&mut self, length: usize) {
        self.transport.record(UsbCall::BulkInSubmit {
            endpoint: self.endpoint,
            length,
        });
        self.queue.push_back(Queued::Pending(length));
    }

    fn next(&mut self, timeout: Duration) -> Option<Result<Vec<u8>, UsbError>> {
        let length = match self.queue.front()? {
            Queued::Pending(length) => *length,
            Queued::Cancelled(_) => {
                let Some(Queued::Cancelled(data)) = self.queue.pop_front() else {
                    unreachable!("the front was just matched");
                };
                return Some(Ok(data));
            }
        };
        let completed = self.transport.device().bulk_in(self.endpoint, length);
        match completed {
            Some(result) => {
                self.queue.pop_front();
                Some(result)
            }
            None => {
                std::thread::sleep(timeout.min(Duration::from_millis(1)));
                None
            }
        }
    }

    fn pending(&self) -> usize {
        self.queue.len()
    }

    fn cancel_all(&mut self) {
        let mut device = self.transport.device();
        for queued in &mut self.queue {
            if let Queued::Pending(length) = *queued {
                *queued = Queued::Cancelled(device.cancel_in(self.endpoint, length));
            }
        }
    }
}
