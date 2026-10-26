//! The demo device end to end: packets, real-time trigger search and replaying the bundled
//! capture.

use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_capture::LogicCapture;
use hsync_core::{ANALOG_BASE, TriggerCondition};
use hsync_demo::DemoDriver;
use hsync_driver::{CapturePlan, Device, Driver, EndReason, OptionKey, OptionValue, Packet};

fn protocol_demo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl")
}

fn open(protocol: Option<PathBuf>) -> Box<dyn Device> {
    let driver = DemoDriver::new(protocol);
    let candidates = driver.virtual_devices();
    assert_eq!(candidates.len(), 1);
    driver.open(&candidates[0]).expect("open the demo device")
}

fn set(device: &mut dyn Device, key: OptionKey, value: OptionValue) {
    device.set_option(key, &value).expect("option");
}

/// Collects a capture until its end packet.
fn capture(packets: &Receiver<Packet>) -> (LogicCapture, Option<u64>, EndReason) {
    let mut capture = None;
    let mut deinterleaver = None;
    let mut trigger = None;
    loop {
        match packets
            .recv_timeout(Duration::from_secs(10))
            .expect("packet")
        {
            Packet::Start {
                channels,
                samplerate,
                analog,
            } => {
                let new = LogicCapture::new(samplerate, channels).with_analog(analog);
                deinterleaver = Some(new.deinterleaver());
                capture = Some(new);
            }
            Packet::Trigger { sample } => trigger = Some(sample),
            Packet::Logic(bytes) => {
                let capture = capture.as_mut().expect("start first");
                capture.append_cross(deinterleaver.as_mut().expect("start first"), &bytes);
            }
            Packet::Analog(codes) => {
                capture.as_mut().expect("start first").append_analog(&codes);
            }
            Packet::Progress { .. } | Packet::Overflow => {}
            Packet::End(reason) => return (capture.expect("start first"), trigger, reason),
        }
    }
}

#[test]
fn counter_capture_completes() {
    let mut device = open(None);
    set(
        device.as_mut(),
        OptionKey::Pattern,
        OptionValue::Choice("counter".into()),
    );
    set(
        device.as_mut(),
        OptionKey::Samplerate,
        OptionValue::Integer(100_000_000),
    );
    set(
        device.as_mut(),
        OptionKey::SampleLimit,
        OptionValue::Integer(65_536),
    );
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(CapturePlan::default(), sink).expect("start");
    let (capture, trigger, reason) = capture(&packets);
    acquisition.wait();
    assert_eq!(reason, EndReason::Completed);
    assert_eq!(trigger, None);
    assert_eq!(capture.len(), 65_536);
    let channel = capture.channel(3).expect("channel 3");
    for sample in 0..200 {
        assert_eq!(channel.sample(sample), (sample >> 3) & 1 == 1);
    }
    // Two sines alongside: at 100 MHz, A0's 1 kHz one starts at 1.65 V and peaks 25,000
    // samples later.
    assert_eq!(capture.info().analog, [ANALOG_BASE, ANALOG_BASE + 1]);
    let a0 = capture.analog_channel(ANALOG_BASE).expect("A0");
    assert_eq!(a0.len(), 65_536);
    assert!((a0.volts(0) - 1.65).abs() < 0.01);
    assert!((a0.volts(25_000) - 3.3).abs() < 0.01);
}

#[test]
fn trigger_lands_after_the_pre_trigger_samples() {
    let mut device = open(None);
    set(
        device.as_mut(),
        OptionKey::Pattern,
        OptionValue::Choice("counter".into()),
    );
    set(
        device.as_mut(),
        OptionKey::Samplerate,
        OptionValue::Integer(100_000_000),
    );
    set(
        device.as_mut(),
        OptionKey::SampleLimit,
        OptionValue::Integer(65_536),
    );
    let (sink, packets) = crossbeam_channel::unbounded();
    let plan = CapturePlan {
        triggers: vec![(10, TriggerCondition::Rise)],
        trigger_position: 25,
        instant: false,
        advanced: None,
        looping: false,
    };
    let acquisition = device.start(plan, sink).expect("start");
    let (capture, trigger, reason) = capture(&packets);
    acquisition.wait();
    assert_eq!(reason, EndReason::Completed);
    // Channel 10 first rises past sample 16 384 at 17 408; the capture keeps 16 384 samples
    // before it.
    assert_eq!(trigger, Some(16_384));
    let channel = capture.channel(10).expect("channel 10");
    assert!(!channel.sample(16_383));
    assert!(channel.sample(16_384));
    assert_eq!(capture.len(), 65_536);
}

#[test]
fn replays_the_protocol_capture() {
    let source = hsync_fileio::read_dsl_file(&protocol_demo()).expect("demo capture");
    let mut device = open(Some(protocol_demo()));
    let options = device.options();
    let pattern = options
        .iter()
        .find(|option| option.key == OptionKey::Pattern)
        .expect("pattern option");
    assert_eq!(pattern.value, OptionValue::Choice("protocol".into()));
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(CapturePlan::default(), sink).expect("start");
    let (capture, _, reason) = capture(&packets);
    acquisition.wait();
    assert_eq!(reason, EndReason::Completed);
    assert_eq!(capture.samplerate(), 25_000_000);
    assert_eq!(capture.len(), source.capture.len());
    for (index, data) in source.capture.channels() {
        let replayed = capture.channel(index).expect("channel");
        assert_eq!(
            replayed.count_edges(0, capture.len()),
            data.count_edges(0, capture.len()),
            "channel {index}"
        );
    }
}

#[test]
fn the_protocol_pattern_brings_the_names_and_decoders_of_the_capture() {
    let source = hsync_fileio::read_dsl_file(&protocol_demo()).expect("demo capture");
    let mut device = open(Some(protocol_demo()));
    let preset = device.preset().expect("preset");
    assert_eq!(
        preset.channel_names.get(&1).map(String::as_str),
        Some("SCL")
    );
    assert_eq!(preset.channel_names.len(), source.header.probes.len());
    assert_eq!(preset.decoders, source.decoders);
    assert!(preset.decoders.is_some());

    set(
        device.as_mut(),
        OptionKey::Pattern,
        OptionValue::Choice("random".into()),
    );
    assert_eq!(device.preset(), None);
    // Without the capture there is nothing to bring.
    assert_eq!(open(None).preset(), None);
}

#[test]
fn loops_until_stopped() {
    let mut device = open(None);
    set(
        device.as_mut(),
        OptionKey::Pattern,
        OptionValue::Choice("counter".into()),
    );
    set(
        device.as_mut(),
        OptionKey::Samplerate,
        OptionValue::Integer(100_000_000),
    );
    set(
        device.as_mut(),
        OptionKey::SampleLimit,
        OptionValue::Integer(4096),
    );
    assert!(device.can_loop());
    let (sink, packets) = crossbeam_channel::unbounded();
    let plan = CapturePlan {
        looping: true,
        ..CapturePlan::default()
    };
    let acquisition = device.start(plan, sink).expect("start");
    let mut bytes = 0;
    while bytes < 10 * 4096 / 8 * 16 {
        match packets.recv_timeout(Duration::from_secs(5)).expect("data") {
            Packet::Logic(data) => bytes += data.len(),
            Packet::End(reason) => panic!("ended by itself: {reason:?}"),
            _ => {}
        }
    }
    acquisition.stop();
    acquisition.wait();
    let ended = packets
        .try_iter()
        .find_map(|packet| match packet {
            Packet::End(reason) => Some(reason),
            _ => None,
        })
        .expect("an end");
    assert_eq!(ended, EndReason::Stopped);
}

#[test]
fn stops_while_waiting_for_the_trigger() {
    let mut device = open(None);
    set(
        device.as_mut(),
        OptionKey::Pattern,
        OptionValue::Choice("counter".into()),
    );
    set(
        device.as_mut(),
        OptionKey::Samplerate,
        OptionValue::Integer(1_000),
    );
    let (sink, packets) = crossbeam_channel::unbounded();
    let plan = CapturePlan {
        triggers: vec![(0, TriggerCondition::Rise)],
        trigger_position: 50,
        instant: false,
        advanced: None,
        looping: false,
    };
    let acquisition = device.start(plan, sink).expect("start");
    loop {
        match packets
            .recv_timeout(Duration::from_secs(5))
            .expect("progress")
        {
            Packet::Progress { triggered, .. } => {
                assert!(!triggered);
                break;
            }
            Packet::Start { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(2_000))
            .is_err()
    );
    acquisition.stop();
    let reason = loop {
        match packets.recv_timeout(Duration::from_secs(5)).expect("end") {
            Packet::Progress { .. } => {}
            Packet::End(reason) => break reason,
            other => panic!("unexpected {other:?}"),
        }
    };
    acquisition.wait();
    assert_eq!(reason, EndReason::Stopped);
}
