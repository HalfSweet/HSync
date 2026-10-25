// SPDX-License-Identifier: Apache-2.0 OR GPL-3.0-or-later
//! Register and streaming device model shared by protocol and acquisition tests.
#![allow(dead_code)]

use std::collections::VecDeque;

use hsync_driver::mock::MockDevice;
use hsync_driver::usb::UsbError;
use hsync_slogic::Model;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Stop,
    Run,
    Read,
    /// Whether the device was still running when the transfer was cancelled.
    Cancel {
        running: bool,
    },
}

#[derive(Debug)]
pub struct AuxDevice {
    pub samples: VecDeque<Result<Vec<u8>, UsbError>>,
    pub drains: VecDeque<Result<Vec<u8>, UsbError>>,
    pub running: bool,
    pub stops: usize,
    pub starts: usize,
    pub events: Vec<Event>,
    pub clock_mhz: Vec<u16>,
    /// The clock slot the rate command selected last.
    pub clock_index: u16,
    pub busy_polls: usize,
    pub never_ready: bool,
    pub short_read: bool,
    pub stop_failures: usize,
    pub run_failure: bool,
    /// Firmware acknowledges these RUN requests without opening its sample gate.
    pub swallow_runs: usize,
    pub channel_mask: u32,
    pub threshold_dac: u32,
    pub divider: u32,
    command: u32,
    payload: [u8; 16],
    polls: usize,
}

impl Default for AuxDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl AuxDevice {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            drains: VecDeque::new(),
            running: false,
            stops: 0,
            starts: 0,
            events: Vec::new(),
            clock_mhz: vec![800],
            clock_index: 0,
            busy_polls: 0,
            never_ready: false,
            short_read: false,
            stop_failures: 0,
            run_failure: false,
            swallow_runs: 0,
            channel_mask: 0,
            threshold_dac: 0,
            divider: 0,
            command: 0,
            payload: [0; 16],
            polls: 0,
        }
    }

    fn select_clock(&mut self, index: u16) {
        let mhz = self.clock_mhz.get(usize::from(index)).copied().unwrap_or(0);
        self.clock_index = index;
        self.payload[..2].copy_from_slice(&index.to_le_bytes());
        self.payload[2..4].copy_from_slice(&mhz.to_le_bytes());
    }
}

impl MockDevice for AuxDevice {
    fn control_out(
        &mut self,
        request: u8,
        address: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        assert_eq!((request, index, data.len()), (1, 0, 4));
        let word = u32::from_le_bytes(data.try_into().expect("one register word"));
        match address {
            4 => match word {
                0 => {
                    self.events.push(Event::Stop);
                    self.stops += 1;
                    if self.stop_failures > 0 {
                        self.stop_failures -= 1;
                        return Err(UsbError::Timeout);
                    }
                    self.running = false;
                }
                1 => {
                    self.events.push(Event::Run);
                    self.starts += 1;
                    if self.run_failure {
                        return Err(UsbError::Timeout);
                    }
                    if self.swallow_runs > 0 {
                        self.swallow_runs -= 1;
                        return Ok(());
                    }
                    self.running = true;
                }
                2 => self.running = false,
                _ => panic!("unknown control value"),
            },
            0xc => {
                self.command = word;
                self.polls = 0;
                self.payload = [0; 16];
                if word == 2 {
                    self.select_clock(0);
                }
            }
            0x10..=0x1c => {
                let at = usize::from(address - 0x10);
                self.payload[at..at + 4].copy_from_slice(data);
                match (self.command, address) {
                    (1, 0x10) => self.channel_mask = word,
                    (2, 0x10) => self.select_clock(word as u16),
                    (2, 0x14) => self.divider = word,
                    (3, 0x10) => self.threshold_dac = word,
                    _ => {}
                }
            }
            _ => panic!("unknown register address"),
        }
        Ok(())
    }

    fn control_in(
        &mut self,
        request: u8,
        address: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        assert_eq!((request, index, length), (0, 0, 4));
        if self.short_read {
            return Ok(vec![0; 3]);
        }
        if address == 0xc {
            self.polls += 1;
            let size = if self.command == 2 { 8 } else { 4 };
            let header: u32 = if self.never_ready || self.polls <= self.busy_polls {
                0
            } else {
                (1 << 16) | (size << 9)
            };
            Ok(header.to_le_bytes().to_vec())
        } else {
            let at = usize::from(address - 0x10);
            Ok(self.payload[at..at + 4].to_vec())
        }
    }

    fn bulk_out(&mut self, _endpoint: u8, _data: &[u8]) -> Result<(), UsbError> {
        panic!("the SLogic U3 models have no bulk OUT commands")
    }

    fn bulk_in(&mut self, endpoint: u8, _length: usize) -> Option<Result<Vec<u8>, UsbError>> {
        assert_eq!(endpoint, 0x82);
        self.events.push(Event::Read);
        if self.running {
            self.samples.pop_front()
        } else {
            None
        }
    }

    fn bulk_read(&mut self, endpoint: u8, length: usize) -> Result<Vec<u8>, UsbError> {
        assert_eq!((endpoint, length), (0x82, 64 * 1024));
        self.drains.pop_front().unwrap_or_else(|| Ok(Vec::new()))
    }

    fn cancel_in(&mut self, _endpoint: u8, _length: usize) -> Vec<u8> {
        self.events.push(Event::Cancel {
            running: self.running,
        });
        Vec::new()
    }
}

/// Packs physical-channel masks in the official wire order, after the model's discarded head.
pub fn sample_bytes(model: &Model, channels: u8, samples: &[u32]) -> Vec<u8> {
    let mut bytes = vec![0xff; model.head_bytes];
    match channels {
        4 => {
            assert!(samples.len().is_multiple_of(2));
            bytes.extend(
                samples
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| ((pair[0] & 0xf) | ((pair[1] & 0xf) << 4)) as u8),
            );
        }
        8 => bytes.extend(samples.iter().map(|&sample| sample as u8)),
        16 => bytes.extend(
            samples
                .iter()
                .flat_map(|&sample| (sample as u16).to_le_bytes()),
        ),
        32 => bytes.extend(samples.iter().flat_map(|sample| sample.to_le_bytes())),
        _ => panic!("invalid wire mode"),
    }
    bytes
}
