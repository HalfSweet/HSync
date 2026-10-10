//! What the timeline and data stream show of the protocol demo, at the design's viewport
//! (1.86–2.6 ms after the trigger at 25 MHz) and over the whole capture.

use std::path::PathBuf;

use hsync_decode::{DataLine, LaneItem, Presentation, Spec, StreamMode, StreamRow};

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
fn i2c_title_row_shows_the_bytes() {
    let all = presentations();
    let view = find(&all, "1:i2c").view(VIEW.0, VIEW.1, SPP);
    let [header] = view.lanes.as_slice() else {
        panic!("one lane");
    };
    assert_eq!(header.lane, "header");
    let bytes: Vec<String> = header
        .items
        .iter()
        .filter_map(|item| match item {
            LaneItem::I2cAddress { address, read, .. } => {
                Some(format!("{}{address:02X}", if *read { 'R' } else { 'W' }))
            }
            LaneItem::I2cData { value, .. } => Some(format!("{value:02X}")),
            _ => None,
        })
        .collect();
    // Two register reads, 04 → 4F and 02 → 32 BB; BB lies past the right edge.
    assert_eq!(bytes, ["W62", "04", "R62", "4F", "W62", "02", "R62", "32"]);
    assert!(
        header
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
fn spi_lanes_show_each_word() {
    let all = presentations();
    let spi = find(&all, "1:spi");
    let view = spi.view(VIEW.0, VIEW.1, SPP);
    let names: Vec<&str> = view.lanes.iter().map(|lane| lane.lane.as_str()).collect();
    assert_eq!(names, ["miso", "mosi"]);
    let words = |lane: usize| -> Vec<u32> {
        view.lanes[lane]
            .items
            .iter()
            .map(|item| match item {
                LaneItem::SpiWord { value, .. } => *value,
                other => panic!("{other:?}"),
            })
            .collect()
    };
    let miso: String = words(0)
        .into_iter()
        .map(|value| char::from(value as u8))
        .collect();
    assert!(miso.contains("HOST: 239.255.255.250:1900"), "{miso}");
    let mosi = words(1);
    assert!(mosi.len() > 16 && mosi.iter().all(|&value| value == 0xFF));
    // The data stream lists hex, 16 words a row, in either mode.
    let rows = spi.rows(StreamMode::Hex);
    assert_eq!(rows, spi.rows(StreamMode::Text));
    assert!(
        rows.iter()
            .all(|row| matches!(row, StreamRow::Hex { .. } | StreamRow::Gap { .. }))
    );
    assert!(rows.iter().any(|row| matches!(
        row,
        StreamRow::Hex {
            line: Some(DataLine::Mosi),
            offset: 16,
            values,
            ..
        } if values == &[0xFF; 16]
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
    let bytes = &view.lanes[0];
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

#[test]
fn views_fetched_wide_answer_for_the_samples_in_view() {
    // An empty view inside a window with frames still points at the frames beside it.
    let all = presentations();
    let can = find(&all, "can-fd");
    let full = can.view(0, u64::MAX, SPP);
    let frames: Vec<(u64, u64)> = full.lanes[0]
        .items
        .iter()
        .map(|item| (item.start(), item.end()))
        .collect();
    let gap = frames[0].1 + 10..frames[1].0 - 10;
    let around = can.view_around(frames[0].0..frames[1].1, gap.clone(), SPP);
    assert_eq!(around.previous, Some(frames[0].0));
    assert_eq!(around.next, Some(frames[1].0));
}
