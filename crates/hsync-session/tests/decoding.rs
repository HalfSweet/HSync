//! Decoding in the workspace: decoders stored with a capture run when it opens, and carry over
//! to captures taken afterwards, which decode once complete.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_decode::{Bus, LaneItem, StreamMode, StreamRow};
use hsync_demo::DemoDriver;
use hsync_driver::DeviceManager;
use hsync_session::{CaptureRequest, DecodeState, Instrument, SessionEvent, Workspace};

fn demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl")
}

fn workspace() -> (Workspace, Receiver<SessionEvent>) {
    let (sender, receiver) = crossbeam_channel::unbounded();
    let sink = Arc::new(move |event: SessionEvent| {
        let _ = sender.send(event);
    });
    (Workspace::with_events(sink), receiver)
}

/// Waits for `count` decoders of capture `id` to finish.
fn decoded(events: &Receiver<SessionEvent>, id: u64, count: usize) {
    let mut done = 0;
    while done < count {
        match events
            .recv_timeout(Duration::from_secs(10))
            .expect("decoder event")
        {
            SessionEvent::Decoded { capture_id, .. } if capture_id == id => done += 1,
            _ => {}
        }
    }
}

#[test]
fn decodes_the_decoders_stored_with_a_capture() {
    let (mut workspace, events) = workspace();
    let summary = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, summary.id, 4);
    let decoding = workspace.decoding(summary.id).expect("decoders");
    let statuses = decoding.statuses();
    let buses: Vec<Option<Bus>> = statuses.iter().map(|s| s.bus).collect();
    assert_eq!(
        buses,
        vec![
            Some(Bus::I2c),
            Some(Bus::Uart),
            Some(Bus::Spi),
            Some(Bus::Can)
        ]
    );
    assert!(statuses.iter().all(|s| s.state == DecodeState::Done));
    assert_eq!(statuses[0].counts.map(|c| c.transfers), Some(6));

    // The lanes of the design's viewport.
    let lanes = decoding.view(46_500, 65_000, 15.0);
    assert_eq!(lanes.len(), 4);
    assert!(
        lanes[1].view.lanes[0]
            .items
            .iter()
            .any(|item| matches!(item, LaneItem::UartFrame { value: 0x53, .. }))
    );

    // A page of the I²C stream, and the transfers in a segment.
    let page = decoding
        .stream(0, StreamMode::Text, None, false, 0, 100)
        .expect("stream");
    assert_eq!(page.total, 6);
    assert!(matches!(page.rows[0], StreamRow::I2c { .. }));
    let first = page.rows[0].start();
    let inside = decoding
        .stream(
            0,
            StreamMode::Text,
            Some((first, first + 10_000)),
            true,
            0,
            100,
        )
        .expect("segment");
    assert_eq!(inside.total, 1);
    assert_eq!(inside.inside, Some(4));
}

#[test]
fn captures_keep_the_decoders_and_decode_when_complete() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);

    let mut instrument = Instrument::new(DeviceManager::new(vec![Arc::new(DemoDriver::new(
        Some(demo_path()),
    ))]));
    instrument.scan();
    instrument.open("virtual-demo").expect("demo device");
    let sink = Arc::new(|_event: SessionEvent| {});
    let summary = instrument
        .start(&mut workspace, CaptureRequest::default(), sink)
        .expect("start");
    assert!(summary.decoders.is_some(), "the decoders carry over");
    decoded(&events, summary.id, 4);
    let decoding = workspace.decoding(summary.id).expect("decoders");
    let statuses = decoding.statuses();
    assert!(statuses.iter().all(|s| s.state == DecodeState::Done));
    // The demo device replays the same capture, so the same transfers decode.
    assert_eq!(statuses[0].counts.map(|c| c.transfers), Some(6));
}
