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

#[test]
fn replaces_the_decoders_and_decodes_again() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let only_uart = serde_json::json!([{
        "id": "1:uart",
        "channel": [{"rxtx": 5}],
        "options": {"baudrate": 115200},
    }]);
    let summary = workspace
        .set_decoders(opened.id, only_uart, false)
        .expect("set decoders");
    assert_eq!(
        summary
            .decoders
            .as_ref()
            .and_then(|d| d.as_array())
            .map(Vec::len),
        Some(1)
    );
    decoded(&events, opened.id, 1);
    let statuses = workspace.decoding(opened.id).expect("decoders").statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].bus, Some(Bus::Uart));
    assert_eq!(statuses[0].counts.map(|c| c.frames), Some(47));
    // An empty list removes every decoder.
    let summary = workspace
        .set_decoders(opened.id, serde_json::json!([]), false)
        .expect("clear");
    assert!(summary.decoders.is_none());
    assert!(
        workspace
            .set_decoders(opened.id, serde_json::json!([]), true)
            .is_err()
    );
}

#[test]
fn saves_and_reopens_a_capture_with_its_decoders() {
    let (mut first, events) = workspace();
    let opened = first.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let only_spi = serde_json::json!([{
        "id": "1:spi",
        "channel": [{"clk": 12}, {"miso": 15}, {"mosi": 14}, {"cs": 13}],
        "options": {"cpol": 0, "cpha": 0},
    }]);
    first
        .set_decoders(opened.id, only_spi, false)
        .expect("decoders");
    let path = std::env::temp_dir().join(format!("hsync-save-{}.dsl", std::process::id()));
    let saved = first.save(opened.id, &path).expect("save");
    assert!(matches!(
        saved.source,
        hsync_session::CaptureSource::File { .. }
    ));

    let (mut other, other_events) = workspace();
    let reopened = other.open_file(&path).expect("reopen");
    decoded(&other_events, reopened.id, 1);
    assert_eq!(reopened.info.sample_count, opened.info.sample_count);
    assert_eq!(reopened.channels, opened.channels);
    let statuses = other.decoding(reopened.id).expect("decoders").statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].counts.map(|c| c.transfers), Some(4));
    let _ = std::fs::remove_file(path);
}

#[test]
fn searches_every_decoder_in_time_order() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let decoding = workspace.decoding(opened.id).expect("decoders");
    let result = decoding
        .search("s", hsync_decode::StreamMode::Text)
        .expect("a query");
    assert!(!result.truncated);
    assert!(
        result
            .hits
            .windows(2)
            .all(|pair| pair[0].start <= pair[1].start)
    );
    let decoders: std::collections::BTreeSet<u32> =
        result.hits.iter().map(|hit| hit.decoder).collect();
    assert!(decoders.len() >= 2, "{decoders:?}");
    assert!(
        decoding
            .search("  ", hsync_decode::StreamMode::Text)
            .is_none()
    );
}
