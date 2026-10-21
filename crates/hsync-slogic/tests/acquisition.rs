// SPDX-License-Identifier: GPL-3.0-or-later
//! Captures through the public driver surface against an emulated SLogic16U3.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{AuxDevice, Event, sample_bytes};
use crossbeam_channel::{Receiver, bounded, unbounded};
use hsync_capture::LogicCapture;
use hsync_core::TriggerCondition;
use hsync_driver::mock::MockTransport;
use hsync_driver::usb::UsbError;
use hsync_driver::{CapturePlan, Device, EndReason, OptionKey, OptionValue, Packet};
use hsync_slogic::protocol::Link;
use hsync_slogic::{SlogicDevice, UsbSpeed};
use nusb::transfer::TransferError;

const WAIT: Duration = Duration::from_secs(5);

fn open() -> (MockTransport<AuxDevice>, SlogicDevice) {
    let transport = MockTransport::new(AuxDevice::new());
    let link = Link::new(Arc::new(transport.clone()));
    let device = SlogicDevice::open(link, UsbSpeed::Super, "test".to_owned()).expect("open");
    transport.clear_calls();
    transport.device().events.clear();
    (transport, device)
}

fn configure(device: &mut SlogicDevice, enabled: u16, limit: u64) {
    for channel in 0..16 {
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

fn assert_capture(packets: &[Packet], enabled: u16, expected: &[u16]) {
    let channels: Vec<_> = (0..16)
        .filter(|channel| enabled & (1 << channel) != 0)
        .collect();
    assert_eq!(
        packets.first(),
        Some(&Packet::Start {
            channels: channels.clone(),
            samplerate: 5_000_000,
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
    let samples: Vec<_> = (0u16..320).map(|n| n.wrapping_mul(7919)).collect();
    for (channels, enabled) in [(4, 0xf), (8, 0x81), (16, 0x8001)] {
        let (transport, mut device) = open();
        configure(&mut device, enabled, 256);
        assert_eq!(device.settings().channels(), channels);
        let raw = sample_bytes(channels, &samples);
        // Exercise first-head removal and wire words spanning USB completions.
        transport
            .device()
            .samples
            .extend(raw.chunks(3).map(|part| Ok(part.to_vec())));
        let packets = run(&mut device, CapturePlan::default());
        assert_capture(&packets, enabled, &samples[..256]);
        assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
        assert_stopped_before_cancelling(&transport.device().events);
    }
}

#[test]
fn consecutive_captures_discard_their_own_partial_head() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 128);
    for seed in [1u16, 507] {
        let samples: Vec<_> = (0u16..128)
            .map(|n| seed.wrapping_add(n.wrapping_mul(47)))
            .collect();
        transport.device().samples.clear();
        transport
            .device()
            .samples
            .push_back(Ok(sample_bytes(16, &samples)));
        transport.device().events.clear();
        let packets = run(&mut device, CapturePlan::default());
        assert_capture(&packets, 0xffff, &samples);
        assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
        assert_stopped_before_cancelling(&transport.device().events);
    }
    assert_eq!(transport.device().starts, 2);
}

#[test]
fn simple_software_trigger_keeps_history_and_ignores_disabled_conditions() {
    let (transport, mut device) = open();
    configure(&mut device, 0x81, 128);
    let samples: Vec<_> = (0..320)
        .map(|tick| if tick >= 173 { 0x81 } else { 0x80 })
        .collect();
    transport.device().samples.extend(
        sample_bytes(8, &samples)
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
fn looping_runs_past_the_finite_limit_then_stops_the_gate_before_cancellation() {
    let (transport, mut device) = open();
    configure(&mut device, 0xf, 64);
    transport
        .device()
        .samples
        .push_back(Ok(sample_bytes(4, &[0xf; 256])));
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

#[test]
fn retries_a_swallowed_first_run_and_discards_the_new_partial_head() {
    let (transport, mut device) = open();
    configure(&mut device, 0xffff, 128);
    let samples: Vec<_> = (0u16..128).map(|n| n.wrapping_mul(7919)).collect();
    transport.device().swallow_runs = 1;
    transport
        .device()
        .samples
        .push_back(Ok(sample_bytes(16, &samples)));
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
        .push_back(Ok(sample_bytes(16, &[0xabcd; 64])));
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
        .push_back(Ok(sample_bytes(16, &[0xabcd; 128])));
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
