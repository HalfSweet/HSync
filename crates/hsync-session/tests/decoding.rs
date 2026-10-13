//! Decoding in the workspace: decoders stored with a capture run when it opens, carry over to
//! captures taken afterwards, and decode those as they are taken.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_demo::DemoDriver;
use hsync_driver::{CapturePreset, DeviceManager};
use hsync_session::{
    CaptureRequest, DecodeState, EventSink, Instrument, LaneBlock, SessionEvent, Workspace,
};
use serde_json::{Value, json};

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

/// Waits for `count` decoders of capture `id` to finish; how many progress events came first.
fn decoded(events: &Receiver<SessionEvent>, id: u64, count: usize) -> usize {
    let mut done = 0;
    let mut progress = 0;
    while done < count {
        match events
            .recv_timeout(Duration::from_secs(20))
            .expect("decoder event")
        {
            SessionEvent::Decoded { capture_id, .. } if capture_id == id => done += 1,
            SessionEvent::DecodeProgress { capture_id, .. } if capture_id == id => progress += 1,
            _ => {}
        }
    }
    progress
}

fn uart_entry() -> Value {
    json!({
        "nodes": { "uart": { "type": "uart", "config": { "line": 5, "baud": 115_200 } } },
        "connections": [{ "from": "source", "to": "uart.logic" }],
        "expose": { "uart": "uart.events" }
    })
}

fn demo_instrument() -> Instrument {
    let mut instrument = Instrument::new(DeviceManager::new(vec![Arc::new(DemoDriver::new(
        Some(demo_path()),
    ))]));
    instrument.scan();
    instrument.open("virtual-demo").expect("demo device");
    instrument
}

#[test]
fn decodes_the_decoders_stored_with_a_capture() {
    let (mut workspace, events) = workspace();
    let summary = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, summary.id, 4);
    let decoding = workspace.decoding(summary.id).expect("decoders");
    let statuses = decoding.statuses();
    let kinds: Vec<&[String]> = statuses.iter().map(|s| s.kinds.as_slice()).collect();
    assert_eq!(
        kinds,
        [
            &["i2c".to_owned(), "eeprom24xx".to_owned()][..],
            &["uart".to_owned()][..],
            &["spi".to_owned()][..],
            &["can".to_owned()][..],
        ]
    );
    assert!(
        statuses.iter().all(|s| s.state == DecodeState::Done),
        "{statuses:?}"
    );
    assert_eq!(statuses[0].label, "I²C");
    assert_eq!(statuses[0].streams.len(), 2);
    assert_eq!(
        (
            statuses[0].streams[1].kind.as_str(),
            statuses[0].streams[1].schema.as_str()
        ),
        ("eeprom24xx", "eeprom24xx.events")
    );
    assert!(statuses[0].streams.iter().all(|s| s.progress.ended));
    assert_eq!(statuses[1].streams[0].progress.events, 47);

    // The lanes of the design's viewport: the UART's "S" of "USB-based".
    let lanes = decoding.view(46_500..65_000, 15.0);
    assert_eq!(lanes.len(), 4);
    let uart = &lanes[1].streams[0].view;
    assert!(
        uart.blocks
            .iter()
            .any(|block| matches!(block, LaneBlock::Event(row) if row.payload["value"] == 0x53))
    );

    // A page of the UART listing, and the frames in a segment.
    let page = decoding.stream(1, 0, None, false, 0, 10).expect("page");
    assert_eq!((page.total, page.events, page.rows.len()), (47, 47, 10));
    assert_eq!(page.rows[0].kind, "frame");
    let first = page.rows[0].start;
    let inside = decoding
        .stream(1, 0, Some((first + 10_000, first)), true, 0, 100)
        .expect("segment");
    assert!(inside.total >= 1 && inside.total < 47);
    assert_eq!(inside.inside.map(|n| n >= 1), Some(true));
    assert_eq!(decoding.row_at(1, 0, first + 1), Some(0));
    assert_eq!(decoding.row_at(1, 0, 0), Some(0));
    assert_eq!(decoding.stream(9, 0, None, false, 0, 10), None);
    assert_eq!(decoding.stream(1, 3, None, false, 0, 10), None);

    // Every decoder has data somewhere in the capture.
    for index in 0..4 {
        let coverage = decoding
            .coverage(index, 0..summary.info.sample_count, 1000)
            .expect("coverage");
        assert!(!coverage.is_empty(), "decoder {index}");
    }
}

#[test]
fn captures_decode_as_they_are_taken() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let whole = workspace
        .decoding(opened.id)
        .expect("decoders")
        .stream(1, 0, None, false, 0, 100)
        .expect("page")
        .rows;

    // The capture replaces the opened one; its decoders start with it and follow the samples.

    let mut instrument = demo_instrument();
    let sink: EventSink = Arc::new(|_event: SessionEvent| {});
    let summary = instrument
        .start(&mut workspace, CaptureRequest::default(), sink)
        .expect("start");
    assert!(summary.decoders.is_some(), "the decoders carry over");
    decoded(&events, summary.id, 4);
    let decoding = workspace.decoding(summary.id).expect("decoders");
    let statuses = decoding.statuses();
    assert!(
        statuses.iter().all(|s| s.state == DecodeState::Done),
        "{statuses:?}"
    );
    // The demo device replays the same capture, so the same frames decode.
    assert_eq!(statuses[1].streams[0].progress.events, 47);
    assert_eq!(
        decoding
            .stream(1, 0, None, false, 0, 100)
            .expect("page")
            .rows,
        whole
    );
}

#[test]
fn a_first_demo_capture_brings_the_decoders_of_the_protocol_capture() {
    let (mut workspace, events) = workspace();
    let mut instrument = demo_instrument();
    let sink: EventSink = Arc::new(|_event: SessionEvent| {});
    let summary = instrument
        .start(&mut workspace, CaptureRequest::default(), sink)
        .expect("start");
    assert_eq!(summary.channels[1].name, "SCL");
    let decoders = summary.decoders.as_ref().expect("the decoders come along");
    assert_eq!(decoders.as_array().map(Vec::len), Some(4));
    assert!(decoders[0].get("nodes").is_some(), "HSync's entries");
    decoded(&events, summary.id, 4);
    let statuses = workspace.decoding(summary.id).expect("decoders").statuses();
    assert!(statuses.iter().all(|s| s.state == DecodeState::Done));
    assert_eq!(statuses[1].streams[0].progress.events, 47);
}

#[test]
fn a_requested_preset_replaces_the_devices_for_a_first_capture_only() {
    // DSView's form of the entry, as an older session may still hold it.
    let only_uart = CapturePreset {
        channel_names: [(5, "RX".to_owned())].into(),
        decoders: Some(json!([{
            "id": "1:uart",
            "channel": [{"rxtx": 5}],
            "options": {"baudrate": 115200},
        }])),
    };
    let sink: EventSink = Arc::new(|_event| {});

    // Nothing shown: the request's preset, not the protocol capture's.
    let (mut bare, events) = workspace();
    let request = CaptureRequest {
        preset: Some(only_uart.clone()),
        ..CaptureRequest::default()
    };
    let summary = demo_instrument()
        .start(&mut bare, request, Arc::clone(&sink))
        .expect("start");
    assert_eq!(summary.channels[1].name, "1");
    assert_eq!(summary.channels[5].name, "RX");
    decoded(&events, summary.id, 1);
    let statuses = bare.decoding(summary.id).expect("decoders").statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].kinds, ["uart"]);
    assert_eq!(statuses[0].streams[0].progress.events, 47);

    // An empty decoder list leaves the capture without decoders.
    let (mut empty, _events) = workspace();
    let request = CaptureRequest {
        preset: Some(CapturePreset {
            decoders: Some(json!([])),
            ..CapturePreset::default()
        }),
        ..CaptureRequest::default()
    };
    let summary = demo_instrument()
        .start(&mut empty, request, Arc::clone(&sink))
        .expect("start");
    assert_eq!(summary.decoders, None);
    assert_eq!(summary.channels[1].name, "1");

    // A shown capture passes its own on instead.
    let (mut shown, _events) = workspace();
    shown.open_demo(&demo_path()).expect("demo");
    let request = CaptureRequest {
        preset: Some(only_uart),
        ..CaptureRequest::default()
    };
    let summary = demo_instrument()
        .start(&mut shown, request, sink)
        .expect("start");
    assert_eq!(summary.channels[1].name, "SCL");
    assert_eq!(
        summary
            .decoders
            .as_ref()
            .and_then(|d| d.as_array())
            .map(Vec::len),
        Some(4)
    );
}

#[test]
fn replaces_the_decoders_and_decodes_again() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let summary = workspace
        .set_decoders(opened.id, json!([uart_entry()]))
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
    assert_eq!(statuses[0].kinds, ["uart"]);
    assert_eq!(statuses[0].streams[0].progress.events, 47);
    // An empty list removes every decoder.
    let summary = workspace.set_decoders(opened.id, json!([])).expect("clear");
    assert!(summary.decoders.is_none());
    assert!(
        workspace
            .decoding(opened.id)
            .expect("decoders")
            .statuses()
            .is_empty()
    );
}

#[test]
fn saves_and_reopens_a_capture_with_its_decoders() {
    let (mut first, events) = workspace();
    let opened = first.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let only_spi = json!([{
        "nodes": { "spi": { "type": "spi", "config": { "clk": 12, "cs": 13, "mosi": 14, "miso": 15 } } },
        "connections": [{ "from": "source", "to": "spi.logic" }],
        "expose": { "spi": "spi.events" },
        "label": "Flash bus"
    }]);
    first
        .set_decoders(opened.id, only_spi.clone())
        .expect("decoders");
    decoded(&events, opened.id, 1);
    let path = std::env::temp_dir().join(format!("hsync-save-{}.hsync", std::process::id()));
    let session = json!({ "format": "hsync-session", "version": 1, "colours": { "12": 4 } });
    let saved = first.save(opened.id, &path, session.clone()).expect("save");
    assert!(matches!(
        saved.source,
        hsync_session::CaptureSource::File { .. }
    ));
    assert_eq!(saved.session, Some(session.clone()));

    let (mut other, other_events) = workspace();
    let reopened = other.open_file(&path).expect("reopen");
    assert_eq!(reopened.decoders, Some(only_spi));
    assert_eq!(reopened.session, Some(session));
    decoded(&other_events, reopened.id, 1);
    assert_eq!(reopened.info.sample_count, opened.info.sample_count);
    assert_eq!(reopened.channels, opened.channels);
    let statuses = other.decoding(reopened.id).expect("decoders").statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].label, "Flash bus");
    assert_eq!(
        statuses[0].streams[0].progress.events,
        first.decoding(opened.id).expect("decoders").statuses()[0].streams[0]
            .progress
            .events
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn searches_every_decoder_in_time_order() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let decoding = workspace.decoding(opened.id).expect("decoders");
    let result = decoding.search("data").expect("a query");
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
    let frames = decoding.search("frame 0x53").expect("a query");
    assert!(frames.hits.iter().all(|hit| hit.decoder == 1));
    assert!(!frames.hits.is_empty());
    let row = decoding
        .stream(1, 0, None, false, frames.hits[0].row, 1)
        .expect("page")
        .rows
        .remove(0);
    assert_eq!(row.payload["value"], 0x53);
    assert!(decoding.search("  ").is_none());
}

#[test]
fn display_changes_keep_the_results() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let before = workspace.decoding(opened.id).expect("decoders");
    let mut decoders = opened.decoders.clone().expect("decoders");
    decoders[0]["show"] = json!({ "numbers": "dec" });
    decoders[0]["label"] = json!("EEPROM bus");
    let summary = workspace
        .set_decoders(opened.id, decoders.clone())
        .expect("decoders");
    assert_eq!(summary.decoders, Some(decoders));
    let after = workspace.decoding(opened.id).expect("decoders");
    assert!(Arc::ptr_eq(&before, &after), "decoded again");
}

#[test]
fn entries_that_cannot_run_are_reported() {
    let (mut workspace, events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoded(&events, opened.id, 4);
    let mut missing = uart_entry();
    missing["nodes"]["uart"]["config"]["line"] = json!(20);
    let unknown = json!({ "nodes": { "x": { "type": "no-such-decoder" } } });
    let malformed = json!({ "nodes": {}, "channel": [] });
    let summary = workspace
        .set_decoders(
            opened.id,
            json!([missing, unknown, malformed, { "id": "counter", "channel": [{ "data": 1 }] }]),
        )
        .expect("set");
    // DSView's counter has no uiharu decoder and is left out.
    assert_eq!(
        summary
            .decoders
            .as_ref()
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(3)
    );
    let statuses = workspace.decoding(opened.id).expect("decoders").statuses();
    let states: Vec<DecodeState> = statuses.iter().map(|s| s.state).collect();
    assert_eq!(
        states,
        [
            DecodeState::MissingChannels,
            DecodeState::Invalid,
            DecodeState::Invalid
        ]
    );
    let codes: Vec<&str> = statuses
        .iter()
        .map(|s| s.error.as_ref().expect("an error").code.as_str())
        .collect();
    assert_eq!(
        codes,
        [
            "error.decode.missingChannels",
            "error.decode.invalid",
            "error.decode.invalid"
        ]
    );
    assert_eq!(
        statuses[0].error.as_ref().expect("error").params["channels"],
        json!("20")
    );
    assert!(statuses.iter().all(|s| s.streams.is_empty()));
}

#[test]
fn counts_edges_in_a_segment_as_dsview() {
    let (mut workspace, _events) = workspace();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let capture = workspace.capture(opened.id).expect("capture");
    let capture = capture.read();
    // SCL (D1) starts high; its first edge falls at 45 479.
    let first = hsync_session::edges(&capture, 0, 45_479, &[1]).expect("edges");
    assert_eq!((first[0].rising, first[0].falling), (0, 1));
    let none = hsync_session::edges(&capture, 0, 45_478, &[1]).expect("edges");
    assert_eq!((none[0].rising, none[0].falling), (0, 0));
    // The order of the ends does not matter, and every edge of the capture is counted once.
    let all =
        hsync_session::edges(&capture, opened.info.sample_count - 1, 0, &[0, 1]).expect("all");
    let halves = [
        hsync_session::edges(&capture, 0, 60_000, &[0, 1]).expect("first half"),
        hsync_session::edges(&capture, 60_000, opened.info.sample_count - 1, &[0, 1])
            .expect("second half"),
    ];
    for (index, total) in all.iter().enumerate() {
        assert_eq!(
            total.rising + total.falling,
            halves
                .iter()
                .map(|half| half[index].rising + half[index].falling)
                .sum::<u64>()
        );
    }
    assert!(hsync_session::edges(&capture, 0, 10, &[99]).is_err());
}
