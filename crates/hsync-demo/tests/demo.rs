//! The demo device end to end: packets, real-time trigger search and replaying the bundled
//! capture.

use std::path::PathBuf;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_capture::LogicCapture;
use hsync_core::TriggerCondition;
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
            } => {
                let new = LogicCapture::new(samplerate, channels);
                deinterleaver = Some(new.deinterleaver());
                capture = Some(new);
            }
            Packet::Trigger { sample } => trigger = Some(sample),
            Packet::Logic(bytes) => {
                let capture = capture.as_mut().expect("start first");
                capture.append_cross(deinterleaver.as_mut().expect("start first"), &bytes);
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
