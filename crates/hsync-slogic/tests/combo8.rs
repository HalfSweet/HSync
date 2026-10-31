// SPDX-License-Identifier: Apache-2.0 OR GPL-3.0-or-later
//! Combo 8 capabilities and acquisition through the same public device API as U3.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{ComboDevice, sample_bytes};
use crossbeam_channel::{Receiver, unbounded};
use hsync_capture::LogicCapture;
use hsync_core::TriggerCondition;
use hsync_driver::mock::{MockTransport, UsbCall};
use hsync_driver::usb::UsbError;
use hsync_driver::{CapturePlan, Device, EndReason, OptionKey, OptionValue, Packet};
use hsync_slogic::protocol::{CaptureSettings, Link};
use hsync_slogic::{SLOGIC_COMBO8, SlogicDevice, UsbSpeed};
use nusb::transfer::TransferError;

fn open() -> (MockTransport<ComboDevice>, SlogicDevice) {
    let transport = MockTransport::new(ComboDevice::default());
    let device = SlogicDevice::open(
        Link::for_model(Arc::new(transport.clone()), &SLOGIC_COMBO8),
        &SLOGIC_COMBO8,
        UsbSpeed::High,
        "combo8".into(),
    )
    .expect("open Combo 8 without any control transfers");
    assert_eq!(transport.device().events, ["drain"]);
    transport.clear_calls();
    transport.device().events.clear();
    (transport, device)
}

fn collect(receiver: &Receiver<Packet>) -> Vec<Packet> {
    let mut packets = Vec::new();
    loop {
        let packet = receiver
            .recv_timeout(Duration::from_secs(6))
            .expect("bounded acquisition");
        let ended = matches!(packet, Packet::End(_));
        packets.push(packet);
        if ended {
            return packets;
        }
    }
}

fn configure(device: &mut SlogicDevice, channels: &[u16]) {
    device.set_enabled_channels(channels).expect("channels");
    device
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(5_000_000))
        .expect("rate");
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(128))
        .expect("depth");
}

fn run(device: &mut SlogicDevice, plan: CapturePlan) -> Vec<Packet> {
    let (sink, receiver) = unbounded();
    let acquisition = device.start(plan, sink).expect("start");
    let packets = collect(&receiver);
    acquisition.wait();
    packets
}

fn assert_samples(packets: &[Packet], channels: &[u16], samples: &[u32]) {
    assert_eq!(
        packets.first(),
        Some(&Packet::Start {
            channels: channels.to_vec(),
            samplerate: 5_000_000,
            analog: vec![],
        })
    );
    let mut capture = LogicCapture::new(5_000_000, channels.iter().copied());
    let mut deinterleaver = capture.deinterleaver();
    for packet in packets {
        if let Packet::Logic(bytes) = packet {
            capture.append_cross(&mut deinterleaver, bytes);
        }
    }
    assert_eq!(capture.len(), samples.len() as u64);
    for &channel in channels {
        let data = capture.channel(channel).expect("selected channel");
        for (tick, &sample) in samples.iter().enumerate() {
            assert_eq!(
                data.sample(tick as u64),
                sample & (1 << channel) != 0,
                "D{channel} at {tick}"
            );
        }
    }
}

fn assert_teardown(transport: &MockTransport<ComboDevice>) {
    let state = transport.device();
    assert!(state.events.contains(&"cancel"));
    let drain = state
        .events
        .iter()
        .position(|&event| event == "drain")
        .expect("drain");
    assert!(state.events[..drain].contains(&"cancel"));
    assert!(state.events[drain..].iter().all(|&event| event == "drain"));
}

#[test]
fn exposes_only_supported_capabilities_and_reconciles_sparse_channels() {
    let (_, mut device) = open();
    assert_eq!(device.info().model, "SLogic Combo 8");
    assert!(!device.info().slow_port);
    assert_eq!(device.channel_count(), 8);
    assert!(device.can_loop());
    assert!(device.analog_inputs().is_empty());
    assert!(!device.options().iter().any(|o| o.key == OptionKey::Vth));
    let before = device.settings().clone();
    assert!(
        device
            .set_option(OptionKey::Vth, &OptionValue::Number(1.7))
            .is_err()
    );
    assert_eq!(device.settings(), &before);
    assert_eq!(device.settings().samplerate, 40_000_000);
    let tiers = device.rate_tiers();
    for (index, (channels, maximum)) in [(2, 160_000_000), (4, 80_000_000), (8, 40_000_000)]
        .into_iter()
        .enumerate()
    {
        let tier = &tiers[index];
        assert_eq!(
            (tier.channels, tier.leading, tier.span, tier.max_samplerate),
            (channels, true, channels, maximum)
        );
        assert_eq!(tier.samplerates.first(), Some(&1_000_000));
        assert_eq!(tier.samplerates.last(), Some(&maximum));
        device
            .select_rate_tier(index, &[channels - 1])
            .expect("sparse tier");
        assert_eq!(device.settings().channels(), channels as u8);
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(maximum))
            .expect("maximum");
    }
    device.select_rate_tier(0, &[1]).expect("D1");
    device
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(160_000_000))
        .expect("160 MHz");
    device.set_channel_enabled(3, true).expect("D3");
    assert_eq!(device.settings().samplerate, 80_000_000);
    device.set_channel_enabled(7, true).expect("D7");
    assert_eq!(device.settings().samplerate, 40_000_000);
    assert!(device.set_channel_enabled(8, true).is_err());
}

#[test]
fn captures_every_wire_width_in_order_without_dropping_a_head() {
    let samples: Vec<_> = (0u32..320).map(|n| n.wrapping_mul(7919)).collect();
    for (wire, channels) in [(2, &[1][..]), (4, &[0, 3]), (8, &[0, 7])] {
        let (transport, mut device) = open();
        configure(&mut device, channels);
        assert_eq!(device.settings().channels(), wire);
        for _ in 0..2 {
            transport.device().samples.clear();
            transport.device().events.clear();
            transport.device().samples.extend(
                sample_bytes(&SLOGIC_COMBO8, wire, &samples)
                    .chunks(3)
                    .map(|b| Ok(b.to_vec())),
            );
            let packets = run(&mut device, CapturePlan::default());
            assert_samples(&packets, channels, &samples[..128]);
            assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
            assert_teardown(&transport);
        }
        assert_eq!(transport.device().commands, vec![vec![5, 0, wire, 0]; 2]);
        let calls = transport.calls();
        let start = calls
            .iter()
            .position(|c| matches!(c, UsbCall::ControlOut { .. }))
            .expect("START");
        assert_eq!(
            calls[..start]
                .iter()
                .filter(|c| matches!(c, UsbCall::BulkInSubmit { endpoint: 0x81, .. }))
                .count(),
            16
        );
    }
}

#[test]
fn software_trigger_handles_two_bit_samples_and_pretrigger_history() {
    let (transport, mut device) = open();
    configure(&mut device, &[0, 1]);
    let samples: Vec<_> = (0..320)
        .map(|tick| if tick >= 173 { 3 } else { 2 })
        .collect();
    transport.device().samples.extend(
        sample_bytes(&SLOGIC_COMBO8, 2, &samples)
            .chunks(7)
            .map(|b| Ok(b.to_vec())),
    );
    let packets = run(
        &mut device,
        CapturePlan {
            triggers: vec![(0, TriggerCondition::Rise), (1, TriggerCondition::High)],
            trigger_position: 25,
            ..CapturePlan::default()
        },
    );
    assert!(packets.contains(&Packet::Trigger { sample: 32 }));
    assert_samples(&packets, &[0, 1], &samples[141..269]);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
}

#[test]
fn loop_ignores_finite_depth_and_stop_cancels_before_draining() {
    let (transport, mut device) = open();
    configure(&mut device, &[0, 1]);
    transport.device().samples.push_back(Ok(vec![0xe4; 64]));
    let (sink, receiver) = unbounded();
    let acquisition = device
        .start(
            CapturePlan {
                looping: true,
                ..CapturePlan::default()
            },
            sink,
        )
        .expect("loop");
    let mut packets = Vec::new();
    loop {
        let packet = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("loop samples");
        let data = matches!(packet, Packet::Logic(_));
        assert!(!matches!(packet, Packet::End(_)));
        packets.push(packet);
        if data {
            break;
        }
    }
    acquisition.stop();
    packets.extend(collect(&receiver));
    acquisition.wait();
    assert_samples(
        &packets,
        &[0, 1],
        &(0..256).map(|n| n % 4).collect::<Vec<_>>(),
    );
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Stopped)));
    assert_teardown(&transport);
    device
        .set_channel_enabled(0, false)
        .expect("idle after stop");
}

#[test]
fn unplug_is_reported_without_draining_a_detached_endpoint() {
    let (transport, mut device) = open();
    configure(&mut device, &[0]);
    transport
        .device()
        .samples
        .push_back(Err(UsbError::Transfer(TransferError::Disconnected)));
    let packets = run(&mut device, CapturePlan::default());
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Detached)));
    assert!(!transport.device().events.contains(&"drain"));
    device
        .set_channel_enabled(1, true)
        .expect("idle after unplug");
}

#[test]
fn failed_start_cleans_up_and_a_swallowed_start_retries_once() {
    let (transport, mut device) = open();
    configure(&mut device, &[0, 1]);
    transport.device().start_failure = true;
    assert!(device.start(CapturePlan::default(), unbounded().0).is_err());
    assert_teardown(&transport);
    device
        .set_channel_enabled(0, true)
        .expect("idle after failed start");
    transport.device().start_failure = false;
    transport.device().swallow_runs = 1;
    transport.device().samples.push_back(Ok(vec![0xe4; 32]));
    let packets = run(&mut device, CapturePlan::default());
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    assert_eq!(transport.device().commands.len(), 3);
}

#[test]
fn drain_is_bounded_and_invalid_start_never_reaches_usb() {
    let transport = MockTransport::new(ComboDevice::default());
    transport
        .device()
        .drains
        .extend((0..40).map(|_| Ok(vec![0xff; 8])));
    let link = Link::for_model(Arc::new(transport.clone()), &SLOGIC_COMBO8);
    link.initialize()
        .expect("bounded drain follows the reference even with continuous data");
    assert_eq!(transport.device().drains.len(), 8);
    for (channels, samplerate) in [
        (0, 1_000_000),
        (3, 1_000_000),
        (16, 5_000_000),
        (2, 0),
        (2, 1_500_000),
        (4, 160_000_000),
        (8, 80_000_000),
    ] {
        transport.clear_calls();
        let settings = CaptureSettings {
            channels,
            samplerate,
            threshold: 1.7,
            clock_mhz: None,
        };
        assert!(link.run(&settings).is_err());
        assert!(transport.calls().is_empty());
    }
}
