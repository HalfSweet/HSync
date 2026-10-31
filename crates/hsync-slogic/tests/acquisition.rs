// SPDX-License-Identifier: GPL-3.0-or-later
//! Captures through the public driver surface against emulated SLogic U3 analyzers.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{AuxDevice, Event, sample_bytes};
use crossbeam_channel::{Receiver, bounded, unbounded};
use hsync_capture::LogicCapture;
use hsync_core::{ANALOG_BASE, TriggerCondition, UiError};
use hsync_driver::mock::{MockTransport, UsbCall};
use hsync_driver::usb::UsbError;
use hsync_driver::{CapturePlan, Device, EndReason, OptionKey, OptionValue, Packet};
use hsync_slogic::protocol::Link;
use hsync_slogic::{Model, SLOGIC16U3, SLOGIC32U3, SlogicDevice, UsbSpeed, adc};
use nusb::transfer::TransferError;

const WAIT: Duration = Duration::from_secs(5);

fn open_model(model: &'static Model, speed: UsbSpeed) -> (MockTransport<AuxDevice>, SlogicDevice) {
    let transport = MockTransport::new(AuxDevice::new());
    let link = Link::new(Arc::new(transport.clone()));
    let device = SlogicDevice::open(link, model, speed, "test".to_owned()).expect("open");
    transport.clear_calls();
    transport.device().events.clear();
    (transport, device)
}

fn open() -> (MockTransport<AuxDevice>, SlogicDevice) {
    open_model(&SLOGIC16U3, UsbSpeed::Super)
}

/// Packs SLogic16U3 samples, whose firmware sends a four-byte head first.
fn sample_bytes16(channels: u8, samples: &[u32]) -> Vec<u8> {
    sample_bytes(&SLOGIC16U3, channels, samples)
}

fn configure(device: &mut SlogicDevice, enabled: u32, limit: u64) {
    for channel in 0..device.channel_count() {
        device
            .set_channel_enabled(channel, enabled & (1 << channel) != 0)
            .expect("channel selection");
    }
    device
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(5_000_000))
        .expect("sample rate");
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
        .expect("sample limit");
}

fn collect(packets: &Receiver<Packet>) -> Vec<Packet> {
    let mut out = Vec::new();
    loop {
        let packet = packets.recv_timeout(WAIT).expect("the acquisition ends");
        let ended = matches!(packet, Packet::End(_));
        out.push(packet);
        if ended {
            return out;
        }
    }
}

fn run(device: &mut SlogicDevice, plan: CapturePlan) -> Vec<Packet> {
    let (sink, packets) = unbounded();
    let acquisition = device.start(plan, sink).expect("start");
    let all = collect(&packets);
    acquisition.wait();
    all
}

fn assert_capture(packets: &[Packet], enabled: u32, expected: &[u32]) {
    let channels: Vec<_> = (0..32)
        .filter(|channel| enabled & (1 << channel) != 0)
        .collect();
    assert_eq!(
        packets.first(),
        Some(&Packet::Start {
            channels: channels.clone(),
            samplerate: 5_000_000,
            analog: Vec::new(),
        })
    );
    let mut capture = LogicCapture::new(5_000_000, channels.iter().copied());
    let mut deinterleaver = capture.deinterleaver();
    for packet in packets {
        if let Packet::Logic(bytes) = packet {
            capture.append_cross(&mut deinterleaver, bytes);
        }
    }
    assert_eq!(capture.len(), expected.len() as u64);
    for channel in channels {
        let data = capture.channel(channel).expect("enabled input");
        for (tick, sample) in expected.iter().enumerate() {
            assert_eq!(
                data.sample(tick as u64),
                sample & (1 << channel) != 0,
                "D{channel} at sample {tick}"
            );
        }
    }
}

fn assert_stopped_before_cancelling(events: &[Event]) {
    assert!(events.contains(&Event::Run));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Cancel { .. }))
    );
    let mut run_since_stop = false;
    for event in events {
        match event {
            Event::Run => run_since_stop = true,
            Event::Stop => run_since_stop = false,
            Event::Cancel { running } => {
                assert!(
                    !run_since_stop,
                    "STOP must precede cancellation: {events:?}"
                );
                assert!(!running, "the firmware must be stopped before cancellation");
            }
            Event::Read => {}
        }
    }
}

#[test]
fn captures_all_wire_modes_and_preserves_sparse_physical_channels() {
    let samples: Vec<_> = (0u32..320).map(|n| n.wrapping_mul(0x9e37_79b9)).collect();
    for (model, channels, enabled) in [
        (&SLOGIC16U3, 4, 0xf),
        (&SLOGIC16U3, 8, 0x81),
        (&SLOGIC16U3, 16, 0x8001),
        (&SLOGIC32U3, 4, 0x9),
        (&SLOGIC32U3, 8, 0x81),
        (&SLOGIC32U3, 16, 0x8001),
        (&SLOGIC32U3, 32, 0x8001_0001),
    ] {
        let (transport, mut device) = open_model(model, UsbSpeed::SuperPlus);
        configure(&mut device, enabled, 256);
        assert_eq!(device.settings().channels(), channels);
        let raw = sample_bytes(model, channels, &samples);
        // Exercise first-head removal and wire words spanning USB completions.
        transport
            .device()
            .samples
            .extend(raw.chunks(3).map(|part| Ok(part.to_vec())));
        let packets = run(&mut device, CapturePlan::default());
        assert_capture(&packets, enabled, &samples[..256]);
        assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
        assert_stopped_before_cancelling(&transport.device().events);
        assert_eq!(
            transport.device().channel_mask,
            u32::MAX >> (32 - channels),
            "{} {channels}-channel mode",
            model.name
        );
    }
}

#[test]
fn consecutive_captures_discard_their_own_partial_head() {
    for (model, enabled) in [(&SLOGIC16U3, 0xffff), (&SLOGIC32U3, u32::MAX)] {
        let (transport, mut device) = open_model(model, UsbSpeed::SuperPlus);
        configure(&mut device, enabled, 128);
        for seed in [1u32, 0x8000_0507] {
            let samples: Vec<_> = (0u32..128)
                .map(|n| seed.wrapping_add(n.wrapping_mul(0x0101_0047)) & enabled)
                .collect();
            transport.device().samples.clear();
            transport
                .device()
                .samples
                .push_back(Ok(sample_bytes(model, model.channels, &samples)));
            transport.device().events.clear();
            let packets = run(&mut device, CapturePlan::default());
            assert_capture(&packets, enabled, &samples);
            assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
            assert_stopped_before_cancelling(&transport.device().events);
        }
        assert_eq!(transport.device().starts, 2);
    }
}

#[test]
fn simple_software_trigger_keeps_history_and_ignores_disabled_conditions() {
    let (transport, mut device) = open();
    configure(&mut device, 0x81, 128);
    let samples: Vec<_> = (0..320)
        .map(|tick| if tick >= 173 { 0x81 } else { 0x80 })
        .collect();
    transport.device().samples.extend(
        sample_bytes16(8, &samples)
            .chunks(41)
            .map(|part| Ok(part.to_vec())),
    );
    let packets = run(
        &mut device,
        CapturePlan {
            triggers: vec![
                (0, TriggerCondition::Rise),
                (7, TriggerCondition::High),
                (5, TriggerCondition::High),
            ],
            trigger_position: 25,
            ..CapturePlan::default()
        },
    );
    assert!(packets.contains(&Packet::Trigger { sample: 32 }));
    assert_capture(&packets, 0x81, &samples[141..269]);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
}

#[test]
fn software_trigger_watches_the_upper_inputs_of_the_32_channel_mode() {
    let (transport, mut device) = open_model(&SLOGIC32U3, UsbSpeed::SuperPlus);
    let enabled = 1 << 31 | 1;
    configure(&mut device, enabled, 128);
    assert_eq!(device.settings().channels(), 32);
    let samples: Vec<_> = (0..320)
        .map(|tick| if tick >= 173 { enabled } else { 1 })
        .collect();
    transport.device().samples.extend(
        sample_bytes(&SLOGIC32U3, 32, &samples)
            .chunks(41)
            .map(|part| Ok(part.to_vec())),
    );
    let packets = run(
        &mut device,
        CapturePlan {
            triggers: vec![(31, TriggerCondition::Rise), (0, TriggerCondition::High)],
            trigger_position: 25,
            ..CapturePlan::default()
        },
    );
    assert!(packets.contains(&Packet::Trigger { sample: 32 }));
    assert_capture(&packets, enabled, &samples[141..269]);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
}

#[test]
fn reads_the_adc_module_on_its_clock_beside_the_logic_inputs() {
    let (transport, mut device) = open_model(&SLOGIC32U3, UsbSpeed::SuperPlus);
    transport.device().clock_mhz = vec![1400, 800];
    device
        .set_enabled_channels(&[0, 9, 16])
        .expect("D0, D9 and D16");
    device
        .set_analog_enabled(ANALOG_BASE + 2, true)
        .expect("A2 on inputs 16–23");
    assert_eq!(device.enabled_channels(), [0, 9]);
    device
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(200_000_000))
        .expect("200 MHz");
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(128))
        .expect("sample limit");
    // A2 counts up; D0 toggles each sample and D9 every other one.
    let samples: Vec<u32> = (0u32..256)
        .map(|n| u32::from((n as u8).reverse_bits()) << 16 | (n & 1) | (n >> 1 & 1) << 9)
        .collect();
    transport.device().samples.extend(
        sample_bytes(&SLOGIC32U3, 32, &samples)
            .chunks(100)
            .map(|part| Ok(part.to_vec())),
    );
    let packets = run(
        &mut device,
        CapturePlan {
            // A2's inputs are not logic, so a condition on one is dropped.
            triggers: vec![(16, TriggerCondition::High)],
            ..CapturePlan::default()
        },
    );
    let analog = vec![(ANALOG_BASE + 2, adc::SCALE)];
    assert_eq!(
        packets.first(),
        Some(&Packet::Start {
            channels: vec![0, 9],
            samplerate: 200_000_000,
            analog: analog.clone(),
        })
    );
    // The sample clock comes from the 800 MHz PLL, in slot 1.
    assert_eq!(transport.device().clock_index, 1);
    assert_eq!(transport.device().divider, 3);
    let mut capture = LogicCapture::new(200_000_000, [0, 9]).with_analog(analog);
    let mut deinterleaver = capture.deinterleaver();
    for packet in &packets {
        match packet {
            Packet::Logic(bytes) => capture.append_cross(&mut deinterleaver, bytes),
            Packet::Analog(codes) => capture.append_analog(codes),
            _ => {}
        }
    }
    assert_eq!(capture.len(), 128);
    let a2 = capture.analog_channel(ANALOG_BASE + 2).expect("A2");
    let d0 = capture.channel(0).expect("D0");
    let d9 = capture.channel(9).expect("D9");
    for n in 0..128 {
        assert_eq!(a2.code(n), n as u16, "A2 at sample {n}");
        assert_eq!(d0.sample(n), n & 1 != 0, "D0 at sample {n}");
        assert_eq!(d9.sample(n), n >> 1 & 1 != 0, "D9 at sample {n}");
    }
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
}

#[test]
fn looping_runs_past_the_finite_limit_then_stops_the_gate_before_cancellation() {
    let (transport, mut device) = open();
    configure(&mut device, 0xf, 64);
    transport
        .device()
        .samples
        .push_back(Ok(sample_bytes16(4, &[0xf; 256])));
    let (sink, receiver) = unbounded();
    let acquisition = device
        .start(
            CapturePlan {
                looping: true,
                ..CapturePlan::default()
            },
            sink,
        )
        .expect("start looping");
    let mut packets = Vec::new();
    loop {
        let packet = receiver.recv_timeout(WAIT).expect("samples arrive");
        let data = matches!(packet, Packet::Logic(_));
        assert!(
            !matches!(packet, Packet::End(_)),
            "loop ended early: {packet:?}"
        );
        packets.push(packet);
        if data {
            break;
        }
    }
    acquisition.stop();
    packets.extend(collect(&receiver));
    acquisition.wait();
    assert_capture(&packets, 0xf, &[0xf; 256]);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Stopped)));
    assert_stopped_before_cancelling(&transport.device().events);
}

#[test]
fn failed_run_stops_before_cancelling_and_leaves_the_device_configurable() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 64);
    transport.device().run_failure = true;
    let (sink, _receiver) = unbounded();
    assert!(device.start(CapturePlan::default(), sink).is_err());
    assert_stopped_before_cancelling(&transport.device().events);
    device
        .set_channel_enabled(15, false)
        .expect("idle after failed RUN");
}

#[test]
fn disconnected_receiver_stops_the_worker_without_blocking_teardown() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 64);
    let (sink, receiver) = unbounded();
    drop(receiver);
    let acquisition = device.start(CapturePlan::default(), sink).expect("start");
    acquisition.wait();
    assert_stopped_before_cancelling(&transport.device().events);
    assert!(!transport.device().running);
    device
        .set_channel_enabled(0, false)
        .expect("worker is idle");
}

#[test]
fn unplug_reports_detached_and_does_not_send_stop_to_the_missing_device() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 64);
    transport
        .device()
        .samples
        .push_back(Err(UsbError::Transfer(TransferError::Disconnected)));
    let packets = run(&mut device, CapturePlan::default());
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Detached)));
    let events = transport.device().events.clone();
    let run = events
        .iter()
        .position(|event| *event == Event::Run)
        .expect("RUN");
    assert!(!events[run + 1..].contains(&Event::Stop));
    device
        .set_channel_enabled(0, false)
        .expect("worker is idle");
}

fn submitted(transport: &MockTransport<AuxDevice>) -> usize {
    transport
        .calls()
        .iter()
        .filter(|call| matches!(call, UsbCall::BulkInSubmit { .. }))
        .count()
}

#[test]
fn queues_only_the_transfers_the_transport_budget_holds() {
    let (transport, mut device) = open();
    // 16 channels at 5 MHz: 64 KiB transfers, four of which fit.
    transport.device().transfer_budget = Some(256 * 1024);
    configure(&mut device, 0xffff, 128);
    let samples: Vec<_> = (0u32..128).map(|n| n.wrapping_mul(7919) & 0xffff).collect();
    transport
        .device()
        .samples
        .push_back(Ok(sample_bytes16(16, &samples)));
    let packets = run(&mut device, CapturePlan::default());
    assert_capture(&packets, 0xffff, &samples);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    // The four queued before RUN, and the one requeued after the data.
    assert_eq!(submitted(&transport), 5);
    assert!(transport.calls().iter().all(|call| match call {
        UsbCall::BulkInSubmit { length, .. } => *length == 64 * 1024,
        _ => true,
    }));
}

#[test]
fn carries_on_without_transfers_the_system_refused_memory_for() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 128);
    let samples: Vec<_> = (0u32..128).map(|n| n.wrapping_mul(7919) & 0xffff).collect();
    let bytes = sample_bytes16(16, &samples);
    let (first, rest) = bytes.split_at(100);
    // Twelve of the sixteen requests never reached the device; four stay in flight.
    let mut device_state = transport.device();
    device_state.samples.push_back(Ok(first.to_vec()));
    device_state
        .samples
        .extend((0..12).map(|_| Err(UsbError::NoMemory)));
    device_state.samples.push_back(Ok(rest.to_vec()));
    drop(device_state);
    let packets = run(&mut device, CapturePlan::default());
    assert_capture(&packets, 0xffff, &samples);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    // Refused requests are not queued again: sixteen, plus one for each completion with data.
    assert_eq!(submitted(&transport), 18);
    assert_stopped_before_cancelling(&transport.device().events);
}

#[test]
fn fails_with_the_usbfs_hint_when_too_few_transfers_remain() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 128);
    let mut device_state = transport.device();
    device_state
        .samples
        .push_back(Ok(sample_bytes16(16, &[0xabcd; 32])));
    device_state
        .samples
        .extend((0..13).map(|_| Err(UsbError::NoMemory)));
    drop(device_state);
    let packets = run(&mut device, CapturePlan::default());
    assert_eq!(
        packets.last(),
        Some(&Packet::End(EndReason::Failed(UiError::new(
            "error.device.usbMemory"
        ))))
    );
    assert_stopped_before_cancelling(&transport.device().events);
    device
        .set_channel_enabled(0, false)
        .expect("idle after the refused transfers");
}

#[test]
fn retries_a_swallowed_first_run_and_discards_the_new_partial_head() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 128);
    let samples: Vec<_> = (0u32..128).map(|n| n.wrapping_mul(7919) & 0xffff).collect();
    transport.device().swallow_runs = 1;
    transport
        .device()
        .samples
        .push_back(Ok(sample_bytes16(16, &samples)));
    let packets = run(&mut device, CapturePlan::default());
    assert_capture(&packets, 0xffff, &samples);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    assert_eq!(transport.device().starts, 2);
    assert_stopped_before_cancelling(&transport.device().events);
    device
        .set_channel_enabled(0, false)
        .expect("idle after rearm");
}

#[test]
fn fails_a_silent_capture_after_one_bounded_retry() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 64);
    transport.device().swallow_runs = usize::MAX;
    let packets = run(&mut device, CapturePlan::default());
    assert!(matches!(
        packets.last(),
        Some(Packet::End(EndReason::Failed(_)))
    ));
    assert_eq!(transport.device().starts, 2);
    assert_stopped_before_cancelling(&transport.device().events);
    device
        .set_channel_enabled(0, false)
        .expect("idle after stalled capture");
}

#[test]
fn rearm_configuration_failure_clears_the_running_state_without_a_queue() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 64);
    transport.device().swallow_runs = 1;
    let (sink, receiver) = unbounded();
    let acquisition = device
        .start(CapturePlan::default(), sink)
        .expect("first RUN");
    // start() configures and sends RUN synchronously. Only the second prepare
    // sees this fault, after the first transfer queue has been cancelled/dropped.
    transport.device().never_ready = true;
    let packets = collect(&receiver);
    acquisition.wait();
    assert!(matches!(
        packets.last(),
        Some(Packet::End(EndReason::Failed(_)))
    ));
    assert_eq!(transport.device().starts, 1);
    assert_stopped_before_cancelling(&transport.device().events);
    device
        .set_channel_enabled(0, false)
        .expect("idle after failed rearm");
}

#[test]
fn never_rearms_after_samples_have_arrived() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 128);
    transport
        .device()
        .samples
        .push_back(Ok(sample_bytes16(16, &[0xabcd; 64])));
    let packets = run(&mut device, CapturePlan::default());
    assert_capture(&packets, 0xffff, &[0xabcd; 64]);
    assert!(matches!(
        packets.last(),
        Some(Packet::End(EndReason::Failed(_)))
    ));
    assert_eq!(transport.device().starts, 1);
    assert_stopped_before_cancelling(&transport.device().events);
}

#[test]
fn user_stop_interrupts_a_worker_with_a_bounded_receiver() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 128);
    transport
        .device()
        .samples
        .push_back(Ok(sample_bytes16(16, &[0xabcd; 128])));
    let (sink, receiver) = bounded(0);
    let acquisition = device
        .start(
            CapturePlan {
                looping: true,
                ..CapturePlan::default()
            },
            sink,
        )
        .expect("start");
    // No channel capacity is available for a subsequent Logic packet until
    // the consumer resumes. A stop must interrupt the blocking send as well
    // as a pending USB transfer, then deliver End once the consumer is ready.
    assert!(matches!(
        receiver.recv_timeout(WAIT),
        Ok(Packet::Start { .. })
    ));
    acquisition.stop();
    let packets = collect(&receiver);
    acquisition.wait();
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Stopped)));
    assert_stopped_before_cancelling(&transport.device().events);
    device
        .set_channel_enabled(0, false)
        .expect("idle after user stop");
}
