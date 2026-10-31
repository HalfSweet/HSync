//! Exact control traces executed by the official Sipeed C implementation.

use std::collections::VecDeque;
use std::sync::Arc;

use hsync_driver::DriverError;
use hsync_driver::mock::{MockDevice, MockTransport, UsbCall};
use hsync_driver::usb::UsbError;
use hsync_slogic::SLOGIC32U3;
use hsync_slogic::protocol::{CaptureSettings, Link, MIN_IN_FLIGHT, TransferPlan, threshold_dac};

mod common;

use common::AuxDevice;

struct Script(VecDeque<(UsbCall, Vec<u8>)>);

impl Script {
    fn answer(&mut self, call: UsbCall) -> Vec<u8> {
        let (expected, reply) = self.0.pop_front().expect("an expected control request");
        assert_eq!(call, expected);
        reply
    }
}

impl MockDevice for Script {
    fn control_out(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        self.answer(UsbCall::ControlOut {
            request,
            value,
            index,
            data: data.to_vec(),
        });
        Ok(())
    }

    fn control_in(
        &mut self,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        Ok(self.answer(UsbCall::ControlIn {
            request,
            value,
            index,
            length,
        }))
    }

    fn bulk_out(&mut self, _endpoint: u8, _data: &[u8]) -> Result<(), UsbError> {
        panic!("the register protocol never uses bulk OUT")
    }

    fn bulk_in(&mut self, _endpoint: u8, _length: usize) -> Option<Result<Vec<u8>, UsbError>> {
        panic!("control traces contain no sample reads")
    }
}

fn hex<T: TryFrom<u32>>(value: &str) -> T
where
    T::Error: std::fmt::Debug,
{
    T::try_from(u32::from_str_radix(value, 16).expect("hexadecimal fixture value"))
        .expect("in-range fixture value")
}

#[test]
fn replays_the_official_control_sequences() {
    let mut lines = include_str!("fixtures/protocol.txt")
        .lines()
        .filter(|line| !line.starts_with('#'));
    let mut cases = 0;
    while let Some(header) = lines.next() {
        let fields: Vec<_> = header.split_whitespace().collect();
        assert_eq!(fields[0], "CASE");
        let settings = CaptureSettings {
            channels: fields[2].parse().expect("channel count"),
            samplerate: fields[3].parse().expect("sample rate"),
            threshold: fields[4].parse().expect("threshold"),
            clock_mhz: None,
        };
        let mut script = VecDeque::new();
        for line in lines.by_ref() {
            if line == "END" {
                break;
            }
            let fields: Vec<_> = line.split_whitespace().collect();
            let request = hex(fields[1]);
            let value = hex(fields[2]);
            let index = hex(fields[3]);
            let bytes: Vec<u8> = (0..fields[4].len())
                .step_by(2)
                .map(|at| hex(&fields[4][at..at + 2]))
                .collect();
            let call = if fields[0] == "W" {
                UsbCall::ControlOut {
                    request,
                    value,
                    index,
                    data: bytes.clone(),
                }
            } else {
                assert_eq!(fields[0], "R");
                UsbCall::ControlIn {
                    request,
                    value,
                    index,
                    length: bytes.len() as u16,
                }
            };
            script.push_back((call, bytes));
        }
        let transport = MockTransport::new(Script(script));
        let link = Link::new(Arc::new(transport.clone()));
        link.reset().expect("RESET");
        link.configure(&settings).expect("configure");
        link.run(&settings).expect("RUN");
        link.stop().expect("STOP");
        assert!(
            transport.device().0.is_empty(),
            "{} consumed every transfer",
            fields[1]
        );
        cases += 1;
    }
    assert_eq!(cases, 13);
}

fn settings() -> CaptureSettings {
    CaptureSettings {
        channels: 16,
        samplerate: 200_000_000,
        threshold: 3.3,
        clock_mhz: None,
    }
}

fn assert_localized(error: DriverError, expected: &str) {
    let DriverError::Localized(error) = error else {
        panic!("expected a localized protocol error");
    };
    assert_eq!(error.code, expected);
}

#[test]
fn initializes_in_reference_order_and_retries_stop() {
    let mut device = AuxDevice::new();
    device.stop_failures = 1;
    device
        .drains
        .extend([Ok(vec![1; 1024]), Ok(vec![2; 16]), Ok(Vec::new())]);
    let transport = MockTransport::new(device);
    Link::new(Arc::new(transport.clone()))
        .initialize()
        .expect("initialize");
    let stop = UsbCall::ControlOut {
        request: 1,
        value: 4,
        index: 0,
        data: vec![0; 4],
    };
    let read = UsbCall::BulkRead {
        endpoint: 0x82,
        length: 64 * 1024,
    };
    assert_eq!(
        transport.calls(),
        vec![
            stop.clone(),
            stop.clone(),
            UsbCall::ClearHalt { endpoint: 0x82 },
            read.clone(),
            read.clone(),
            read,
            UsbCall::ControlOut {
                request: 1,
                value: 4,
                index: 0,
                data: vec![2, 0, 0, 0]
            },
            stop,
        ]
    );
}

#[test]
fn drain_and_aux_readiness_are_bounded() {
    let mut device = AuxDevice::new();
    device.drains.extend((0..40).map(|_| Ok(vec![1; 4])));
    let transport = MockTransport::new(device);
    let link = Link::new(Arc::new(transport.clone()));
    assert_localized(
        link.initialize().expect_err("drain budget"),
        "error.device.slogic.drain",
    );
    assert_eq!(transport.device().drains.len(), 8);

    transport.device().never_ready = true;
    transport.clear_calls();
    assert_localized(
        link.configure(&settings())
            .expect_err("AUX readiness budget"),
        "error.device.slogic.auxReady",
    );
    assert_eq!(
        transport
            .calls()
            .iter()
            .filter(|call| matches!(call, UsbCall::ControlIn { value: 0xc, .. }))
            .count(),
        8
    );
    assert!(
        !transport
            .calls()
            .iter()
            .any(|call| matches!(call, UsbCall::ControlIn { value: 0x10, .. }))
    );
}

#[test]
fn rejects_short_register_reads_and_invalid_settings() {
    let mut device = AuxDevice::new();
    device.short_read = true;
    let transport = MockTransport::new(device);
    let link = Link::new(Arc::new(transport.clone()));
    assert!(matches!(
        link.configure(&settings()),
        Err(DriverError::Usb(UsbError::Short {
            expected: 4,
            actual: 3
        }))
    ));
    for invalid in [
        CaptureSettings {
            channels: 3,
            ..settings()
        },
        CaptureSettings {
            channels: 64,
            ..settings()
        },
        CaptureSettings {
            samplerate: 0,
            ..settings()
        },
        CaptureSettings {
            threshold: f64::NAN,
            ..settings()
        },
        CaptureSettings {
            threshold: f64::INFINITY,
            ..settings()
        },
        CaptureSettings {
            threshold: -0.1,
            ..settings()
        },
        CaptureSettings {
            threshold: 6.1,
            ..settings()
        },
    ] {
        transport.clear_calls();
        assert_localized(
            link.configure(&invalid).expect_err("invalid settings"),
            "error.device.slogic.captureSettings",
        );
        assert!(transport.calls().is_empty());
    }
}

#[test]
fn encodes_every_advertised_rate_on_the_800_mhz_clock() {
    for mhz in [
        5, 8, 10, 16, 20, 25, 32, 40, 50, 80, 100, 160, 200, 400, 800,
    ] {
        let transport = MockTransport::new(AuxDevice::new());
        let link = Link::new(Arc::new(transport.clone()));
        link.configure(&CaptureSettings {
            channels: 4,
            samplerate: mhz * 1_000_000,
            ..settings()
        })
        .expect("supported rate");
        assert_eq!(transport.device().divider, (800 / mhz - 1) as u32);
        assert_eq!(transport.device().channel_mask, 0xf);
    }
}

#[test]
fn slogic32u3_keeps_every_rate_but_1400_mhz_on_its_800_mhz_clock() {
    for &rate in SLOGIC32U3.samplerates {
        let mut device = AuxDevice::new();
        device.clock_mhz = vec![1400, 800];
        let transport = MockTransport::new(device);
        let link = Link::new(Arc::new(transport.clone()));
        link.configure(&CaptureSettings {
            channels: 32,
            samplerate: rate,
            ..settings()
        })
        .expect("supported rate");
        let device = transport.device();
        let (slot, base) = if rate == 1_400_000_000 {
            (0, 1_400_000_000)
        } else {
            (1, 800_000_000)
        };
        assert_eq!(
            (device.clock_index, device.divider),
            (slot, (base / rate - 1) as u32),
            "{rate} Hz"
        );
        assert_eq!(device.channel_mask, u32::MAX);
    }
}

#[test]
fn a_required_clock_is_the_only_one_taken() {
    for (rate, slot) in [(200_000_000, Some(1)), (1_400_000_000, None)] {
        let mut device = AuxDevice::new();
        device.clock_mhz = vec![1400, 800];
        let transport = MockTransport::new(device);
        let link = Link::new(Arc::new(transport.clone()));
        let result = link.configure(&CaptureSettings {
            channels: 32,
            samplerate: rate,
            clock_mhz: Some(800),
            ..settings()
        });
        match slot {
            Some(slot) => {
                result.expect("a rate of the 800 MHz clock");
                assert_eq!(transport.device().clock_index, slot);
                assert_eq!(transport.device().divider, (800_000_000 / rate - 1) as u32);
            }
            None => assert_localized(
                result.expect_err("1400 MHz is not of the 800 MHz clock"),
                "error.device.slogic.sampleClock",
            ),
        }
    }
    // Without the 800 MHz clock the search gives up after both laps.
    let mut device = AuxDevice::new();
    device.clock_mhz = vec![1400, 400];
    let transport = MockTransport::new(device);
    let link = Link::new(Arc::new(transport.clone()));
    assert_localized(
        link.configure(&CaptureSettings {
            samplerate: 200_000_000,
            clock_mhz: Some(800),
            ..settings()
        })
        .expect_err("no 800 MHz clock"),
        "error.device.slogic.sampleClock",
    );
}

#[test]
fn rejects_clocks_that_cannot_generate_the_rate_after_bounded_search() {
    let transport = MockTransport::new(AuxDevice::new());
    let link = Link::new(Arc::new(transport.clone()));
    assert_localized(
        link.configure(&CaptureSettings {
            samplerate: 7_000_000,
            ..settings()
        })
        .expect_err("incompatible base"),
        "error.device.slogic.sampleClock",
    );
    // One channel-payload write followed by exactly 16 rate-slot probes.
    assert_eq!(
        transport
            .calls()
            .iter()
            .filter(|call| matches!(call, UsbCall::ControlOut { value: 0x10, .. }))
            .count(),
        17
    );
}

#[test]
fn rounds_and_clamps_thresholds_like_the_reference() {
    for (volts, dac) in [
        (0.0, 0),
        (0.3, 46),
        (1.0, 154),
        (1.8, 277),
        (2.5, 384),
        (3.3, 507),
        (5.0, 769),
        (6.0, 923),
        (-1.0, 0),
        (7.0, 923),
    ] {
        assert_eq!(threshold_dac(volts), dac, "{volts} V");
    }
}

#[test]
fn keeps_the_transfer_ring_bounded_and_aligned_for_every_mode() {
    // About 5 ms of data per request: 12.5 kB at the slowest rate, raised to the 32 KiB minimum.
    let slowest = TransferPlan::new(4, 5_000_000, None);
    assert_eq!((slowest.size, slowest.count), (32 * 1024, 16));
    assert_eq!(slowest.expected_rate_bytes, 2_500_000);
    // 200 kB on USB 2.0, rounded up to whole 32 KiB.
    let usb2 = TransferPlan::new(16, 20_000_000, None);
    assert_eq!((usb2.size, usb2.count), (224 * 1024, 16));
    for (channels, rate, bytes) in [
        (4, 800_000_000, 400_000_000),
        (8, 400_000_000, 400_000_000),
        (16, 200_000_000, 400_000_000),
        (4, 1_400_000_000, 700_000_000),
        (8, 800_000_000, 800_000_000),
        (32, 200_000_000, 800_000_000),
    ] {
        let plan = TransferPlan::new(channels, rate, None);
        assert_eq!(plan.size, 1024 * 1024);
        assert_eq!(plan.count, 16);
        assert_eq!(plan.expected_rate_bytes, bytes);
        assert_eq!(plan.timeout.as_millis(), 50);
    }
}

#[test]
fn keeps_the_transfer_ring_within_the_usbfs_budget() {
    // The default 16 MiB usbfs pool leaves 12 MiB: twelve 1 MiB requests at full rate, where
    // sixteen failed after 15 MiB.
    let full = TransferPlan::new(16, 200_000_000, Some(12 << 20));
    assert_eq!((full.size, full.count), (1 << 20, 12));
    // Slower captures fit the whole ring.
    let usb2 = TransferPlan::new(16, 20_000_000, Some(12 << 20));
    assert_eq!((usb2.size, usb2.count), (224 * 1024, 16));
    // In a pool too small for MIN_IN_FLIGHT requests of 1 MiB, the requests shrink instead.
    let tight = TransferPlan::new(16, 200_000_000, Some(768 * 1024));
    assert_eq!((tight.size, tight.count), (192 * 1024, MIN_IN_FLIGHT));
}
