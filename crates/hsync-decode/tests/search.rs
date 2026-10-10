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
                .run(&document.capture, document.capture.len())
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
