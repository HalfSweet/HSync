//! HSync's own capture file: write and read round trips, what it keeps, and what it refuses.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use hsync_capture::{CHUNK_SAMPLES, LogicCapture};
use hsync_fileio::{
    FileError, HSYNC_FORMAT, HSYNC_VERSION, HsyncSource, HsyncWriteRequest, is_hsync_path,
    read_dsl_file, read_hsync, write_hsync, write_hsync_with,
};
use serde_json::json;

fn demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl")
}

fn names(capture: &LogicCapture) -> BTreeMap<u16, String> {
    capture
        .channels()
        .map(|(index, _)| (index, format!("D{index}")))
        .collect()
}

fn assert_same_samples(a: &LogicCapture, b: &LogicCapture) {
    assert_eq!(a.len(), b.len());
    for ((index, left), (_, right)) in a.channels().zip(b.channels()) {
        let mut at = 0;
        loop {
            assert_eq!(
                left.sample(at),
                right.sample(at),
                "channel {index} sample {at}"
            );
            match (left.next_change(at), right.next_change(at)) {
                (Some(x), Some(y)) if x == y => at = x,
                (None, None) => break,
                other => panic!("channel {index}: edges differ after {at}: {other:?}"),
            }
        }
    }
}

#[test]
fn round_trips_the_demo_capture_with_its_documents() {
    let original = read_dsl_file(&demo_path()).expect("demo").capture;
    let names = names(&original);
    let decoders = json!([{ "nodes": { "uart": { "type": "uart", "config": { "line": 5 } } } }]);
    let session = json!({ "format": "hsync-session", "version": 1, "colours": { "0": 3 } });
    let request = HsyncWriteRequest {
        channel_names: &names,
        source: Some(HsyncSource {
            driver: "DSLogic".into(),
            model: "DSLogic U3Pro16".into(),
        }),
        app_version: "0.1.4",
        decoders: Some(&decoders),
        session: Some(&session),
    };
    let bytes = write_hsync(Cursor::new(Vec::new()), &original, &request)
        .expect("writes")
        .into_inner();
    let document = read_hsync(Cursor::new(bytes)).expect("reads back");
    assert_same_samples(&original, &document.capture);
    assert_eq!(document.capture.samplerate(), original.samplerate());
    assert_eq!(document.capture.trigger_sample(), original.trigger_sample());
    assert_eq!(document.decoders, Some(decoders));
    assert_eq!(document.session, Some(session));
    let manifest = &document.manifest;
    assert_eq!(
        (manifest.format.as_str(), manifest.version),
        (HSYNC_FORMAT, HSYNC_VERSION)
    );
    assert_eq!(manifest.app_version.as_deref(), Some("0.1.4"));
    assert_eq!(manifest.samples, original.len());
    assert_eq!(manifest.channels.len(), 16);
    assert_eq!(manifest.channels[5].name, "D5");
    assert_eq!(
        manifest.source.as_ref().map(|s| s.model.as_str()),
        Some("DSLogic U3Pro16")
    );
}

#[test]
fn round_trips_multi_block_captures_with_constant_blocks() {
    // 2.25 blocks: channel 0 toggles every 1000 samples, channel 3 stays high, 7 stays low.
    let samples = CHUNK_SAMPLES * 2 + CHUNK_SAMPLES / 4 + 17;
    let mut capture = LogicCapture::new(100_000_000, [0, 3, 7]);
    let words = samples.div_ceil(64);
    for (channel, pattern) in [(0u16, 0u8), (3, 1), (7, 2)] {
        let data: Vec<u64> = (0..words)
            .map(|word| match pattern {
                0 => (0..64).fold(0u64, |acc, bit| {
                    let sample = word * 64 + bit;
                    acc | (u64::from((sample / 1000) % 2 == 1) << bit)
                }),
                1 => u64::MAX,
                _ => 0,
            })
            .collect();
        capture
            .channel_mut(channel)
            .expect("the channel")
            .append_words(&data);
    }
    capture.truncate(samples);
    capture.set_trigger(Some(4321), Some(1_700_000_000_000));
    let request = HsyncWriteRequest {
        channel_names: &BTreeMap::new(),
        source: None,
        app_version: "test",
        decoders: None,
        session: None,
    };
    let bytes = write_hsync(Cursor::new(Vec::new()), &capture, &request)
        .expect("writes")
        .into_inner();
    let document = read_hsync(Cursor::new(bytes)).expect("reads back");
    assert_same_samples(&capture, &document.capture);
    assert_eq!(document.capture.trigger_sample(), Some(4321));
    assert_eq!(
        document.manifest.trigger.as_ref().and_then(|t| t.time_ms),
        Some(1_700_000_000_000)
    );
    assert_eq!(document.manifest.channels[1].name, "3");
    assert_eq!(document.decoders, None);
    assert_eq!(document.session, None);
}

#[test]
fn an_empty_capture_round_trips() {
    let capture = LogicCapture::new(1_000_000, [0, 1]);
    let request = HsyncWriteRequest {
        channel_names: &BTreeMap::new(),
        source: None,
        app_version: "test",
        decoders: None,
        session: None,
    };
    let bytes = write_hsync(Cursor::new(Vec::new()), &capture, &request)
        .expect("writes")
        .into_inner();
    let document = read_hsync(Cursor::new(bytes)).expect("reads back");
    assert_eq!(document.capture.len(), 0);
    assert_eq!(document.capture.channels().count(), 2);
}

#[test]
fn stops_when_cancelled_and_refuses_what_it_is_not() {
    let original = read_dsl_file(&demo_path()).expect("demo").capture;
    let names = names(&original);
    let request = HsyncWriteRequest {
        channel_names: &names,
        source: None,
        app_version: "test",
        decoders: None,
        session: None,
    };
    let mut calls = 0;
    let cancelled = write_hsync_with(Cursor::new(Vec::new()), &original, &request, &mut |_| {
        calls += 1;
        calls < 3
    });
    assert!(matches!(cancelled, Err(FileError::Cancelled)));

    // A .dsl file is a zip archive too, but not an HSync capture.
    let dsl = std::fs::read(demo_path()).expect("the demo file");
    assert!(matches!(
        read_hsync(Cursor::new(dsl)),
        Err(FileError::NotHsync)
    ));
    assert!(matches!(
        read_hsync(Cursor::new(b"not a zip".to_vec())),
        Err(FileError::Zip(_))
    ));

    // A version from the future is refused by its number.
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .expect("entry");
    zip.write_all(
        serde_json::to_string(&json!({
            "format": HSYNC_FORMAT, "version": HSYNC_VERSION + 1, "samplerate": 1, "samples": 0,
            "channels": [], "blocks": { "words": 1, "compression": "lz4" }
        }))
        .expect("json")
        .as_bytes(),
    )
    .expect("writes");
    let bytes = zip.finish().expect("archive").into_inner();
    assert!(matches!(
        read_hsync(Cursor::new(bytes)),
        Err(FileError::UnsupportedVersion(v)) if v == HSYNC_VERSION + 1
    ));

    assert!(is_hsync_path(std::path::Path::new("/tmp/a.HSYNC")));
    assert!(!is_hsync_path(std::path::Path::new("/tmp/a.dsl")));
}
