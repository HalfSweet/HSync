//! Searching the protocol demo's decoded data, as the search box does.

use std::path::PathBuf;

use hsync_decode::{Needle, Presentation, Spec, StreamMode, StreamRow};

fn presentations() -> Vec<(String, Presentation)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/demo/protocol.dsl");
    let document = hsync_fileio::read_dsl_file(&path).expect("demo capture");
    let decoders = document.decoders.expect("decoders");
    decoders
        .as_array()
        .expect("list")
        .iter()
        .filter_map(Spec::from_dsview)
        .map(|spec| {
            let decoded = spec
                .run(&document.capture, 0..document.capture.len())
                .expect("channels");
            (spec.id.clone(), Presentation::new(&decoded))
        })
        .collect()
}

fn find<'a>(all: &'a [(String, Presentation)], id: &str) -> &'a Presentation {
    &all.iter().find(|(key, _)| key == id).expect(id).1
}

fn needle(query: &str) -> Needle {
    Needle::parse(query).expect("query")
}

#[test]
fn finds_i2c_bytes_in_their_transfers() {
    let all = presentations();
    let i2c = find(&all, "1:i2c");
    let found = i2c.search(&needle("4F"), StreamMode::Text);
    assert_eq!(found.len(), 1);
    let row = found[0].row.expect("row");
    let StreamRow::I2c { start, end, .. } = &i2c.rows(StreamMode::Text)[row as usize] else {
        panic!("an I2C row");
    };
    assert!(*start <= found[0].start && found[0].end <= *end);
    // The register reads of 32 BB: three transfers.
    assert_eq!(i2c.search(&needle("32 BB"), StreamMode::Text).len(), 3);
}

#[test]
fn finds_text_in_both_listings() {
    let all = presentations();
    let spi = find(&all, "1:spi");
    for mode in [StreamMode::Text, StreamMode::Hex] {
        let found = spi.search(&needle("host"), mode);
        assert!(!found.is_empty(), "{mode:?}");
        assert!(found.iter().all(|hit| hit.row.is_some()), "{mode:?}");
        assert!(found.windows(2).all(|pair| pair[0].start <= pair[1].start));
    }
    let text = spi.search(&needle("HOST: 239"), StreamMode::Text);
    let row = text[0].row.expect("row") as usize;
    assert!(
        matches!(&spi.rows(StreamMode::Text)[row], StreamRow::SpiText { text, .. } if text.starts_with("HOST: 239"))
    );
    let uart = find(&all, "1:uart");
    assert_eq!(
        uart.search(&needle("dreamsourcelab"), StreamMode::Text)
            .len(),
        1
    );
}

#[test]
fn finds_can_identifiers() {
    let all = presentations();
    let can = find(&all, "can-fd");
    let found = can.search(&needle("0x100"), StreamMode::Text);
    assert_eq!(found.len(), 3);
    assert!(found.iter().all(|hit| hit.row.is_some()));
}

#[test]
fn level_patterns_find_the_i2c_start_conditions() {
    use hsync_capture::{PatternCondition, PatternLevel};
    use hsync_decode::Decoded;
    use hsync_decode::i2c::Kind;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/demo/protocol.dsl");
    let document = hsync_fileio::read_dsl_file(&path).expect("demo capture");
    let decoders = document.decoders.expect("decoders");
    let spec = decoders
        .as_array()
        .expect("list")
        .iter()
        .filter_map(Spec::from_dsview)
        .find(|spec| spec.id == "1:i2c")
        .expect("i2c");
    let Some(Decoded::I2c { trace, .. }) = spec.run(&document.capture, 0..document.capture.len())
    else {
        panic!("an I2C trace");
    };
    let expected: Vec<u64> = trace
        .items
        .iter()
        .filter(|item| matches!(item.kind, Kind::Start | Kind::RepeatedStart))
        .map(|item| item.start)
        .collect();
    // SDA (D0) falls while SCL (D1) is high.
    let pattern = [
        PatternCondition {
            channel: 0,
            level: PatternLevel::Falling,
        },
        PatternCondition {
            channel: 1,
            level: PatternLevel::High,
        },
    ];
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = document.capture.find_pattern(&pattern, from, true) {
        found.push(at);
        from = at + 1;
    }
    assert_eq!(found, expected);
    // And back from the end.
    let last = document.capture.find_pattern(&pattern, u64::MAX, false);
    assert_eq!(last, expected.last().copied());
}

#[test]
fn presents_and_searches_plain_annotations() {
    use hsync_decode::{Annotation, LaneItem};

    let annotation = |start: u64, row: u32, text: &str| Annotation {
        start,
        end: start + 10,
        class: row,
        row,
        texts: vec![text.to_owned(), text[..1].to_owned()],
        value: None,
    };
    let rows = ["bits".to_owned(), "fields".to_owned()];
    let presentation = Presentation::annotations(
        &rows,
        vec![
            annotation(30, 1, "Temperature: 21.5"),
            annotation(0, 0, "1"),
            annotation(10, 0, "0"),
        ],
    );
    let view = presentation.view(0, 100, 0.1);
    assert_eq!(view.lanes.len(), 2);
    assert_eq!(view.lanes[0].items.len(), 2);
    assert!(
        matches!(&view.lanes[1].items[0], LaneItem::Annotation { texts, .. } if texts[0] == "Temperature: 21.5")
    );
    let listed: Vec<u64> = presentation
        .rows(StreamMode::Text)
        .iter()
        .map(StreamRow::start)
        .collect();
    assert_eq!(listed, [0, 10, 30]);
    let found = presentation.search(&needle("temperature"), StreamMode::Text);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].row, Some(2));
}
