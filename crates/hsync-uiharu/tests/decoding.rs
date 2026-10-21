//! Decoding the demo capture: as a whole, and as a device would deliver it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hsync_capture::LogicCapture;
use hsync_uiharu::{DecodeError, Decoder, EventRow, LaneBlock, Needle, Position, Update};
use parking_lot::RwLock;
use serde_json::json;
use uiharu_core::{Tick, TickRange};
use uiharu_registry::{Catalog, Description};

/// Where the demo capture's buses are.
const SDA: u8 = 0;
const SCL: u8 = 1;
const UART: u8 = 5;

fn demo() -> LogicCapture {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl");
    hsync_fileio::read_dsl_file(&path)
        .expect("the demo capture reads")
        .capture
}

fn description(rate: u64, node: serde_json::Value) -> Description {
    serde_json::from_value(json!({
        "source": { "channels": 16, "rate": rate },
        "nodes": { "bus": node },
        "connections": [{ "from": "source", "to": "bus.logic" }],
        "expose": { "events": "bus.events" }
    }))
    .expect("the description parses")
}

fn uart(rate: u64) -> Description {
    description(
        rate,
        json!({ "type": "uart", "config": { "line": UART, "baud": 115_200 } }),
    )
}

fn i2c(rate: u64) -> Description {
    description(
        rate,
        json!({ "type": "i2c", "config": { "scl": SCL, "sda": SDA } }),
    )
}

/// Starts a decoder and the channel its updates arrive on.
fn start(
    description: Description,
    capture: Arc<RwLock<LogicCapture>>,
    position: Position,
) -> Result<(Decoder, crossbeam_channel::Receiver<Update>), DecodeError> {
    let (sender, receiver) = crossbeam_channel::unbounded();
    let decoder = Decoder::start(
        description,
        &Catalog::new(),
        capture,
        position,
        Arc::new(move |update| {
            let _ = sender.send(update);
        }),
    )?;
    Ok((decoder, receiver))
}

fn wait_finished(updates: &crossbeam_channel::Receiver<Update>) {
    loop {
        match updates.recv_timeout(Duration::from_secs(60)) {
            Ok(Update::Finished) => return,
            Ok(Update::Progress) => {}
            Ok(Update::Failed(error)) => panic!("decoding failed: {error}"),
            Err(error) => panic!("no end in time: {error}"),
        }
    }
}

fn rows(decoder: &Decoder) -> Vec<EventRow> {
    decoder.streams()[0].store().page(None, 0, usize::MAX).rows
}

/// Decodes the whole capture in one go.
fn decode_whole(description: Description, capture: LogicCapture) -> Decoder {
    let len = capture.len();
    let (decoder, updates) = start(
        description,
        Arc::new(RwLock::new(capture)),
        Position {
            len,
            dropped: 0,
            finishing: true,
        },
    )
    .expect("the decoder starts");
    wait_finished(&updates);
    decoder
}

#[test]
fn a_capture_delivered_in_pieces_decodes_as_a_whole_one() {
    let whole = demo();
    let rate = whole.samplerate();
    let channels: Vec<u16> = whole.channels().map(|(index, _)| index).collect();
    let expected = rows(&decode_whole(uart(rate), demo()));
    assert!(expected.len() > 10, "{} frames", expected.len());

    let live = Arc::new(RwLock::new(LogicCapture::new(
        rate,
        channels.iter().copied(),
    )));
    let (decoder, updates) =
        start(uart(rate), Arc::clone(&live), Position::default()).expect("the decoder starts");
    // Words arrive in pieces of uneven sizes, as a device delivers them.
    let words = whole.channel(0).expect("channel 0").word_count();
    let mut next = 0;
    let mut seed: u64 = 7;
    while next < words {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let piece = (1 + (seed >> 33) % 300) as usize;
        let end = (next + piece).min(words);
        {
            let mut capture = live.write();
            for &channel in &channels {
                let source = whole.channel(channel).expect("the channel");
                let words: Vec<u64> = (next..end).map(|word| source.word(word)).collect();
                capture
                    .channel_mut(channel)
                    .expect("the channel")
                    .append_words(&words);
            }
            if end == words {
                capture.truncate(whole.len());
            }
        }
        next = end;
        decoder.notify(Position {
            len: live.read().len(),
            dropped: 0,
            finishing: end == words,
        });
        if seed.is_multiple_of(5) {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
    wait_finished(&updates);
    assert_eq!(rows(&decoder), expected);
    let store = decoder.streams()[0].store();
    assert_eq!(store.ended(), Some(Tick(whole.len())));
    assert_eq!(decoder.settled(), Tick(whole.len()));
}

#[test]
fn the_streams_answer_the_interfaces_questions() {
    let capture = demo();
    let len = capture.len();
    let decoder = decode_whole(i2c(capture.samplerate()), capture);
    let stream = &decoder.streams()[0];
    assert_eq!((stream.name(), stream.kind()), ("events", "i2c"));
    assert_eq!(stream.schema().name, "i2c.events");
    let store = stream.store();
    assert!(!store.is_empty());
    let whole = TickRange::new(Tick::ZERO, Tick(len));
    let (hits, truncated) = store.search(&Needle::parse("address").expect("a query"), 10);
    assert_eq!((hits.len(), truncated), (10, true));
    assert_eq!(store.row(hits[0]).expect("a row").kind, "address");
    let page = store.page(None, 0, 3);
    assert_eq!(page.rows.len(), 3);
    assert_eq!(page.total as usize, store.len());
    assert!(
        !store
            .view(whole, len as f64 / 1440.0, 4096)
            .blocks
            .is_empty()
    );
    assert!(!store.coverage(whole, len / 1000).is_empty());
    assert!(store.row_at(Tick(len / 2)) > 0);
}

#[test]
fn channels_the_capture_lacks_are_reported() {
    let capture = demo();
    let rate = capture.samplerate();
    let partial = capture.slice(0, capture.len(), &[0, 5]);
    let (decoder, _updates) = start(
        uart(rate),
        Arc::new(RwLock::new(partial.slice(0, partial.len(), &[0, 5]))),
        Position::default(),
    )
    .expect("a decoder reading channels the capture has starts");
    drop(decoder);
    let error = start(
        i2c(rate),
        Arc::new(RwLock::new(partial)),
        Position::default(),
    )
    .expect_err("a decoder reading a missing channel does not start");
    assert!(matches!(error, DecodeError::MissingChannels(ref channels) if channels == &[1]));
}

#[test]
fn a_description_that_does_not_build_is_refused() {
    let capture = demo();
    let rate = capture.samplerate();
    let error = start(
        description(rate, json!({ "type": "no-such-bus" })),
        Arc::new(RwLock::new(capture)),
        Position::default(),
    )
    .expect_err("an unknown type does not start");
    assert!(matches!(error, DecodeError::Build(_)), "{error}");
}

#[test]
fn cancelling_ends_the_thread() {
    let capture = demo();
    let rate = capture.samplerate();
    let (mut decoder, updates) = start(
        uart(rate),
        Arc::new(RwLock::new(capture)),
        Position {
            len: 0,
            dropped: 0,
            finishing: false,
        },
    )
    .expect("the decoder starts");
    decoder.cancel();
    decoder.join();
    assert!(!decoder.is_finished());
    assert_eq!(decoder.error(), None);
    assert!(updates.try_recv().is_err());
}

#[test]
fn jtag_state_lanes_use_durations_and_the_remaining_captures_origin() {
    let mut capture = LogicCapture::new(1_000_000, 0..4);
    for channel in 0..4 {
        capture
            .channel_mut(channel)
            .expect("the channel")
            .append_words(&[0, 0]);
    }
    let (decoder, updates) = start(
        description(1_000_000, json!({
            "type": "jtag",
            "config": { "tck": 0, "tms": 1, "tdi": 2, "tdo": 3, "initial_state": "test-logic-reset" }
        })),
        Arc::new(RwLock::new(capture)),
        Position { len: 128, dropped: 500, finishing: true },
    ).expect("the decoder starts");
    wait_finished(&updates);

    let view = decoder.view(0, 40..80, 1.0, 10).expect("the stream");
    assert!(view.blocks.is_empty());
    let states = &view.annotations.expect("JTAG state annotations")[0];
    assert_eq!(states.id, "states");
    assert!(matches!(&states.blocks[..], [LaneBlock::Event(row)]
        if row.start == 0 && row.end == 128 && row.payload["state"] == "test-logic-reset"));
    let page = decoder.page(0, None, 0, 10).expect("the raw stream");
    assert_eq!(page.total, 1);
    assert_eq!((page.rows[0].start, page.rows[0].end), (0, 1));
}
