//! What the timeline and data stream show of the protocol demo, at the design's viewport
//! (1.86–2.6 ms after the trigger at 25 MHz) and over the whole capture.

use std::path::PathBuf;

use hsync_decode::{DataLine, I2cMeaning, LaneItem, Presentation, Spec, StreamMode, StreamRow};

fn demo() -> hsync_fileio::DslDocument {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/demo/protocol.dsl");
    hsync_fileio::read_dsl_file(&path).expect("demo capture")
}

/// The demo's decoders, presented, by DSView id.
fn presentations() -> Vec<(String, Presentation)> {
    let document = demo();
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

const VIEW: (u64, u64) = (46_500, 65_000);
/// The design's 1236 px track over the viewport.
const SPP: f64 = (VIEW.1 - VIEW.0) as f64 / 1236.0;

#[test]
fn i2c_lanes_show_register_reads() {
    let all = presentations();
    let view = find(&all, "1:i2c").view(VIEW.0, VIEW.1, SPP);
    assert_eq!(view.address, Some(0x62));
    let header = &view.lanes[0];
    assert_eq!(header.lane, "header");
    let meanings: Vec<&I2cMeaning> = header
        .items
        .iter()
        .filter_map(|item| match item {
            LaneItem::I2cTransfer { meaning, .. } => Some(meaning),
            _ => None,
        })
        .collect();
    assert_eq!(
        meanings,
        vec![
            &I2cMeaning::ReadRegister {
                register: 0x04,
                data: vec![0x4F]
            },
            &I2cMeaning::ReadRegister {
                register: 0x02,
                data: vec![0x32, 0xBB]
            },
        ]
    );
    let bytes = &view.lanes[1];
    assert!(bytes.items.iter().any(|item| matches!(
        item,
        LaneItem::I2cAddress {
            address: 0x62,
            read: true,
            ..
        }
    )));
    assert!(
        bytes
            .items
            .iter()
            .any(|item| matches!(item, LaneItem::I2cAck { ack: false, .. }))
    );
}

#[test]
fn uart_lane_shows_frames() {
    let all = presentations();
    let view = find(&all, "1:uart").view(VIEW.0, VIEW.1, SPP);
    let text: String = view.lanes[0]
        .items
        .iter()
        .filter_map(|item| match item {
            LaneItem::UartFrame { value, .. } => Some(char::from(*value as u8)),
            _ => None,
        })
        .collect();
    // The frame before 'S' just reaches into the viewport's left edge.
    assert_eq!(text, "USB-based ");
}

#[test]
fn spi_lanes_show_text_and_runs() {
    let all = presentations();
    let spi = find(&all, "1:spi");
    let view = spi.view(VIEW.0, VIEW.1, SPP);
    let [
        LaneItem::SpiTransfer {
            cut: false,
            first,
            last,
            ..
        },
    ] = view.lanes[0].items.as_slice()
    else {
        panic!("one transfer in view");
    };
    // The design's viewport shows bytes 81–160 of the transfer (it starts slightly earlier).
    assert!(
        (79..=81).contains(first) && (159..=161).contains(last),
        "{first}–{last}"
    );
    let miso: String = view.lanes[1]
        .items
        .iter()
        .filter_map(|item| match item {
            LaneItem::SpiText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(miso.contains("HOST: 239.255.255.250:1900"), "{miso}");
    assert!(matches!(
        view.lanes[2].items.as_slice(),
        [LaneItem::SpiRun {
            value: 0xFF,
            count: 224,
            since: 16,
            ..
        }]
    ));
    // The data stream: MISO in text, then MOSI with the run folded.
    let rows = spi.rows(StreamMode::Text);
    assert!(rows.iter().any(|row| matches!(row, StreamRow::SpiText { text, line: DataLine::Miso, .. } if text == "HOST: 239.255.255.250:1900\r\n")));
    assert!(rows.iter().any(|row| matches!(
        row,
        StreamRow::Run {
            value: 0xFF,
            count: 224,
            offset: 16,
            line: Some(DataLine::Mosi),
            ..
        }
    )));
    assert_eq!(spi.counts().transfers, 4);
    assert_eq!(spi.counts().bytes, 288);
}

#[test]
fn can_lane_points_to_the_nearest_frames() {
    let all = presentations();
    let can = find(&all, "can-fd");
    let view = can.view(VIEW.0, VIEW.1, SPP);
    assert!(view.lanes.iter().all(|lane| lane.items.is_empty()));
    let previous = view.previous.expect("a frame before the viewport");
    let next = view.next.expect("a frame after the viewport");
    assert!(previous < VIEW.0 && next >= VIEW.1);
    assert_eq!(can.counts().frames, 3);
}

#[test]
fn zoomed_out_lanes_merge_narrow_items() {
    let all = presentations();
    let capture_len = 131_072;
    // 50 px for the whole capture: a byte is narrower than 3 px.
    let view = find(&all, "1:i2c").view(0, capture_len, capture_len as f64 / 50.0);
    let bytes = &view.lanes[1];
    assert!(
        bytes
            .items
            .iter()
            .all(|item| matches!(item, LaneItem::Dense { .. }))
    );
    assert!(bytes.items.len() <= 6, "{} items", bytes.items.len());
}

#[test]
fn counts_bytes_inside_a_segment() {
    let all = presentations();
    let i2c = find(&all, "1:i2c");
    assert_eq!(i2c.counts().transfers, 6);
    // The whole first transfer: address, register, address, one data byte.
    let first = i2c.rows(StreamMode::Text).iter().find_map(|row| match row {
        StreamRow::I2c { start, end, .. } => Some((*start, *end)),
        _ => None,
    });
    let (start, end) = first.expect("a transfer");
    assert_eq!(i2c.count_inside(start, end + 1), 4);
}
