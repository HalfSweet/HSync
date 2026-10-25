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
        looping: false,
        hold: false,
        keep_stopped: false,
        preset: None,
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
fn loops_until_stopped_keeping_the_newest_samples() {
    let mut instrument = instrument();
    assert!(instrument.state().expect("state").can_loop);
    let mut workspace = Workspace::new();
    let (sink, statuses) = events();
    let request = CaptureRequest {
        looping: true,
        ..CaptureRequest::default()
    };
    let summary = instrument
        .start(&mut workspace, request, sink)
        .expect("start");
    // Run on well past the limit.
    loop {
        let status = statuses
            .recv_timeout(Duration::from_secs(10))
            .expect("status");
        assert_ne!(status.phase, LivePhase::Ended, "a loop capture runs on");
        assert!(status.samples <= 65_536);
        if status.dropped > 4 * 65_536 {
            break;
        }
    }
    instrument.stop();
    let last = until_ended(&statuses).pop().expect("status");
    assert_eq!(last.end, Some(EndKind::Stopped));
    assert_eq!(last.samples, 65_536);
    assert!(last.dropped > 4 * 65_536);

    // The newest samples, counted on from where the dropped ones ended.
    let capture = workspace.capture(summary.id).expect("capture");
    let capture = capture.read();
    assert_eq!(capture.len(), 65_536);
    let first = last.dropped;
    for channel in 0..6u16 {
        let data = capture.channel(channel).expect("channel");
        assert!(data.is_compact());
        for sample in [0, 1, 63, 64, 1000, 65_535] {
            assert_eq!(
                data.sample(sample),
                ((first + sample) >> channel) & 1 == 1,
                "D{channel} sample {sample}"
            );
        }
    }
}

/// Takes a capture to completion and returns its id.
fn complete(instrument: &mut Instrument, workspace: &mut Workspace) -> u64 {
    let (sink, statuses) = events();
    let summary = instrument
        .start(workspace, CaptureRequest::default(), sink)
        .expect("start");
    until_ended(&statuses);
    summary.id
}

#[test]
fn repeated_captures_hold_the_shown_one_until_they_complete() {
    let mut instrument = instrument();
    let mut workspace = Workspace::new();
    let first = complete(&mut instrument, &mut workspace);

    let (sink, statuses) = events();
    let hold = CaptureRequest {
        hold: true,
        ..CaptureRequest::default()
    };
    let second = instrument
        .start(&mut workspace, hold, sink)
        .expect("start")
        .id;
    assert_eq!(workspace.summary().expect("shown").id, first);
    assert!(workspace.capture(second).is_ok(), "the capture behind");
    until_ended(&statuses);
    assert_eq!(workspace.summary().expect("shown").id, second);

    // The next capture drops the one it replaced.
    let third = complete(&mut instrument, &mut workspace);
    assert_eq!(workspace.summary().expect("shown").id, third);
    assert!(workspace.capture(first).is_err());
}

#[test]
fn a_held_back_capture_stopped_early_is_dropped_unless_kept() {
    for keep_stopped in [false, true] {
        let mut instrument = instrument();
        let mut workspace = Workspace::new();
        let first = complete(&mut instrument, &mut workspace);
        instrument
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(1_000))
            .expect("slow rate");
        let (sink, statuses) = events();
        let request = CaptureRequest {
            hold: true,
            keep_stopped,
            ..CaptureRequest::default()
        };
        let second = instrument
            .start(&mut workspace, request, sink)
            .expect("start")
            .id;
        instrument.stop();
        let last = until_ended(&statuses).pop().expect("status");
        assert_eq!(last.end, Some(EndKind::Stopped));
        let shown = workspace.summary().expect("shown").id;
        assert_eq!(shown, if keep_stopped { second } else { first });
        // Whatever is shown stays usable.
        assert!(workspace.capture(shown).is_ok());
        assert!(workspace.decoding(shown).is_ok());
    }
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
    assert!(instrument.set_enabled_channels(&[0]).is_err());
    instrument.stop();
    let last = until_ended(&statuses).pop().expect("status");
    assert_eq!(last.end, Some(EndKind::Stopped));
    assert!(!instrument.is_capturing());
    instrument
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(2_000))
        .expect("unlocked");
}

#[test]
fn sets_every_enabled_channel_at_once() {
    let mut instrument = instrument();
    let state = instrument
        .set_enabled_channels(&[2, 0, 9])
        .expect("three channels");
    assert_eq!(state.enabled_channels, [0, 2, 9]);
    // The demo device's rate does not depend on its channels.
    assert!(state.rate_tiers.is_empty());
    assert_eq!(state.rate_tier, None);
    assert!(instrument.select_rate_tier(0, &[0]).is_err());
    assert!(instrument.set_enabled_channels(&[16]).is_err());
    assert_eq!(
        instrument.state().expect("open").enabled_channels,
        [0, 2, 9]
    );
}
