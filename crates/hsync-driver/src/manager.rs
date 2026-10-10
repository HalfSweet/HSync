//! Finding devices: every driver's virtual devices, then the USB devices a driver recognises.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use nusb::MaybeFuture;
use nusb::hotplug::HotplugEvent;

use crate::device::{Candidate, Device, Driver};
use crate::error::DriverError;

/// The registered drivers.
#[derive(Clone, Default)]
pub struct DeviceManager {
    drivers: Vec<Arc<dyn Driver>>,
}

impl std::fmt::Debug for DeviceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.drivers.iter().map(|driver| driver.name()).collect();
        f.debug_struct("DeviceManager")
            .field("drivers", &names)
            .finish()
    }
}

impl DeviceManager {
    pub fn new(drivers: Vec<Arc<dyn Driver>>) -> Self {
        Self { drivers }
    }

    /// Devices available now: virtual ones first, then USB devices in system order.
    pub fn scan(&self) -> Vec<Candidate> {
        let mut found: Vec<Candidate> = self
            .drivers
            .iter()
            .flat_map(|driver| driver.virtual_devices())
            .collect();
        match nusb::list_devices().wait() {
            Ok(devices) => {
                for usb in devices {
                    if let Some(candidate) =
                        self.drivers.iter().find_map(|driver| driver.identify(&usb))
                    {
                        found.push(candidate);
                    }
                }
            }
            Err(error) => log::warn!("listing USB devices failed: {error}"),
        }
        found
    }

    /// Opens a candidate with the driver that found it.
    pub fn open(&self, candidate: &Candidate) -> Result<Box<dyn Device>, DriverError> {
        let driver = self
            .drivers
            .iter()
            .find(|driver| driver.name() == candidate.driver)
            .ok_or(DriverError::Unsupported)?;
        driver.open(candidate)
    }

    /// Calls `changed` whenever USB devices come or go, after a pause of `settle` without
    /// further changes (devices re-enumerate after a firmware upload). Stops with the handle.
    pub fn watch(
        &self,
        settle: Duration,
        changed: impl Fn() + Send + 'static,
    ) -> Option<WatchHandle> {
        let watch = match nusb::watch_devices() {
            Ok(watch) => watch,
            Err(error) => {
                log::warn!("watching USB devices failed: {error}");
                return None;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("usb-hotplug".into())
            .spawn(move || {
                let (events, receive) = crossbeam_channel::unbounded();
                // The hotplug stream is blocking; forward its events so this thread can debounce.
                std::thread::spawn(move || {
                    for event in futures_lite::stream::block_on(watch) {
                        if events.send(event).is_err() {
                            break;
                        }
                    }
                });
                while !stopping.load(Ordering::Relaxed) {
                    match receive.recv_timeout(Duration::from_millis(200)) {
                        Ok(event) => {
                            log::debug!("usb hotplug: {}", describe(&event));
                            while receive.recv_timeout(settle).is_ok() {}
                            changed();
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .ok()?;
        Some(WatchHandle {
            stop,
            thread: Some(thread),
        })
    }
}

fn describe(event: &HotplugEvent) -> String {
    match event {
        HotplugEvent::Connected(info) => {
            format!(
                "connected {:04x}:{:04x}",
                info.vendor_id(),
                info.product_id()
            )
        }
        HotplugEvent::Disconnected(_) => "disconnected".to_owned(),
    }
}

/// Stops watching for device changes when dropped.
#[derive(Debug)]
pub struct WatchHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
