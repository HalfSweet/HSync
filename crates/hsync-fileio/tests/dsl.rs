//! `.dsl` compatibility: DSView's own demo capture and write/read round trips.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;

use hsync_capture::{CHUNK_SAMPLES, LogicCapture};
use hsync_fileio::{DslWriteRequest, read_dsl, read_dsl_file, write_dsl};

fn demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl")
}

#[test]
fn reads_the_dsview_protocol_demo() {
    let document = read_dsl_file(&demo_path()).expect("demo capture loads");
    let header = &document.header;
    assert_eq!(header.version, 3);
    assert_eq!(header.samplerate, 25_000_000);
    assert_eq!(header.total_samples, 131_072);
    assert_eq!(header.probes.len(), 16);
    assert_eq!(header.probes[0].1, "SDA");
    assert_eq!(header.probes[15].1, "MISO");

    let capture = &document.capture;
    assert_eq!(capture.len(), 131_072);
    assert_eq!(capture.info().channels, (0..16).collect::<Vec<u16>>());

    // Six I2C reads at 100 kHz: SCL idles high and pulses 255 times.
    let scl = capture.channel(1).expect("SCL");
    assert!(scl.sample(0));
    let edges = scl.count_edges(0, capture.len());
    assert_eq!((edges.rising, edges.falling), (255, 255));
    // The long SPI transfer starts when CS# falls at sample 26999 (1.0800 ms).
    let cs = capture.channel(13).expect("CS#");
    assert_eq!(cs.next_change(26_000), Some(26_999));

    let decoders = document.decoders.expect("decoders entry");
    let ids: Vec<&str> = decoders
        .as_array()
        .expect("decoder list")
        .iter()
        .filter_map(|decoder| decoder["id"].as_str())
        .collect();
    assert_eq!(ids, ["1:i2c", "1:uart", "1:spi", "can-fd"]);
    assert!(document.session.is_some());
}

fn round_trip(capture: &LogicCapture) -> LogicCapture {
    let names: BTreeMap<u16, String> = capture
        .channels()
        .map(|(index, _)| (index, format!("D{index}")))
        .collect();
    let request = DslWriteRequest {
        driver: "HSync",
        channel_names: &names,
        session: &serde_json::json!({ "Version": 3 }),
        decoders: &serde_json::json!([]),
        hsync_decoders: None,
    };
    let bytes = write_dsl(Cursor::new(Vec::new()), capture, &request)
        .expect("writes")
        .into_inner();
    let document = read_dsl(Cursor::new(bytes)).expect("reads back");
    assert_eq!(document.header.probes.len(), names.len());
    document.capture
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
fn round_trips_the_demo_capture() {
    let original = read_dsl_file(&demo_path())
        .expect("demo capture loads")
        .capture;
    let copy = round_trip(&original);
    assert_same_samples(&original, &copy);
    assert_eq!(copy.trigger_sample(), Some(0));
}

#[test]
fn round_trips_multi_block_captures_with_constant_blocks() {
    // 2.25 blocks: channel 0 toggles every 1000 samples, channel 3 stays high.
    let words = (2 * CHUNK_SAMPLES + CHUNK_SAMPLES / 4) / 64;
    let mut capture = LogicCapture::new(100_000_000, [0, 3]);
    let toggling: Vec<u64> = (0..words)
        .map(|word| {
            (0..64).fold(0u64, |acc, bit| {
                let sample = word * 64 + bit;
                acc | (u64::from((sample / 1000) % 2 == 1) << bit)
            })
        })
        .collect();
    let words = usize::try_from(words).expect("fits");
    capture.channel_mut(0).expect("D0").append_words(&toggling);
    capture
        .channel_mut(3)
        .expect("D3")
        .append_words(&vec![!0; words]);
    capture.truncate(capture.len() - 10);
    capture.set_trigger(Some(12_345), Some(1_760_000_000_000));

    let copy = round_trip(&capture);
    assert_same_samples(&capture, &copy);
    assert_eq!(copy.trigger_sample(), Some(12_345));
    assert_eq!(copy.info().trigger_time_ms, Some(1_760_000_000_000));
}

#[test]
fn keeps_hsync_decoders_in_an_entry_of_their_own() {
    let capture = read_dsl_file(&demo_path()).expect("demo").capture;
    let names: BTreeMap<u16, String> = capture
        .channels()
        .map(|(index, _)| (index, format!("D{index}")))
        .collect();
    let entries = serde_json::json!([{ "nodes": { "uart": { "type": "uart" } } }]);
    let request = DslWriteRequest {
        driver: "HSync",
        channel_names: &names,
        session: &serde_json::json!({ "Version": 3 }),
        decoders: &serde_json::json!([]),
        hsync_decoders: Some(&entries),
    };
    let bytes = write_dsl(Cursor::new(Vec::new()), &capture, &request)
        .expect("writes")
        .into_inner();
    let document = read_dsl(Cursor::new(bytes)).expect("reads back");
    assert_eq!(document.decoders, Some(serde_json::json!([])));
    assert_eq!(document.hsync_decoders, Some(entries));
    // DSView's own files have no such entry.
    let demo = read_dsl_file(&demo_path()).expect("demo");
    assert_eq!(demo.hsync_decoders, None);
}
