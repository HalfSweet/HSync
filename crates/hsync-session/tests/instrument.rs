//! Devices through the instrument: opening the demo device, changing its options and capturing
//! into the workspace.

use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_core::TriggerCondition;
use hsync_demo::DemoDriver;
use hsync_driver::{DeviceManager, OptionKey, OptionValue};
use hsync_session::{
    CaptureRequest, CaptureSource, EndKind, Instrument, LivePhase, LiveStatus, SessionEvent,
    Workspace,
};

fn instrument() -> Instrument {
    let mut instrument = Instrument::new(DeviceManager::new(vec![Arc::new(DemoDriver::new(None))]));
    let devices = instrument.scan();
    assert!(devices.iter().any(|device| device.id == "virtual-demo"));
    instrument
        .open("virtual-demo")
        .expect("open the demo device");
    instrument
        .set_option(OptionKey::Pattern, &OptionValue::Choice("counter".into()))
        .expect("pattern");
    instrument
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(100_000_000))
        .expect("rate");
    instrument
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(65_536))
        .expect("limit");
    instrument
}

fn events() -> (hsync_session::EventSink, Receiver<LiveStatus>) {
    let (sender, receiver) = crossbeam_channel::unbounded();
    let sink = Arc::new(move |event: SessionEvent| {
        if let SessionEvent::Live(status) = event {
            let _ = sender.send(status);
        }
    });
    (sink, receiver)
}

fn until_ended(statuses: &Receiver<LiveStatus>) -> Vec<LiveStatus> {
    let mut all = Vec::new();
    loop {
        let status = statuses
            .recv_timeout(Duration::from_secs(10))
            .expect("status");
        let ended = status.phase == LivePhase::Ended;
        all.push(status);
        if ended {
            return all;
        }
    }
}

#[test]
fn captures_into_the_workspace() {
    let mut instrument = instrument();
    let mut workspace = Workspace::new();
    let (sink, statuses) = events();
    let summary = instrument
        .start(&mut workspace, CaptureRequest::default(), sink)
        .expect("start");
    assert_eq!(
        summary.source,
        CaptureSource::Device {
            driver: "virtual-demo".into(),
            model: "Demo".into()
        }
    );
    assert_eq!(summary.info.samplerate, 100_000_000);
    let all = until_ended(&statuses);
    let last = all.last().expect("status");
    assert_eq!(last.end, Some(EndKind::Completed));
    assert_eq!(last.samples, 65_536);
    assert_eq!(last.limit, 65_536);
    let capture = workspace.capture(summary.id).expect("live capture");
    assert_eq!(capture.read().len(), 65_536);
    assert!(!instrument.is_capturing());
    assert_eq!(
        workspace.summary().expect("summary").info.sample_count,
        65_536
    );
}

#[test]
fn waits_for_the_trigger() {
    let mut instrument = instrument();
    let mut workspace = Workspace::new();
    let (sink, statuses) = events();
    let request = CaptureRequest {
        triggers: vec![(10, TriggerCondition::Rise)],
        trigger_position: 25,
        instant: false,
        advanced: None,
    };
    let summary = instrument
        .start(&mut workspace, request, sink)
        .expect("start");
    let all = until_ended(&statuses);
    assert_eq!(all[0].phase, LivePhase::Waiting);
    let last = all.last().expect("status");
    assert_eq!(last.trigger_sample, Some(16_384));
    assert_eq!(
        workspace
            .capture(summary.id)
            .expect("capture")
            .read()
            .trigger_sample(),
        Some(16_384)
    );
}

#[test]
fn stops_and_locks_settings_while_capturing() {
    let mut instrument = instrument();
    instrument
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(1_000))
        .expect("slow rate");
    let mut workspace = Workspace::new();
    let (sink, statuses) = events();
    instrument
        .start(&mut workspace, CaptureRequest::default(), sink)
        .expect("start");
    assert!(instrument.is_capturing());
    assert!(
        instrument
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(2_000))
            .is_err()
    );
    assert!(instrument.open("virtual-demo").is_err());
    instrument.stop();
    let last = until_ended(&statuses).pop().expect("status");
    assert_eq!(last.end, Some(EndKind::Stopped));
    assert!(!instrument.is_capturing());
    instrument
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(2_000))
        .expect("unlocked");
}
