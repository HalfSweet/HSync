//! The native decoders on DSView's protocol demo capture, which the design's artboards show:
//! I²C on D0/D1, UART on D5, SPI on D12–D15 and CAN FD on D9, at 25 MHz.

use std::path::PathBuf;

use hsync_capture::LogicCapture;
use hsync_decode::{can, i2c, spi, uart};

const SAMPLERATE: u64 = 25_000_000;

fn demo() -> LogicCapture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/demo/protocol.dsl");
    hsync_fileio::read_dsl_file(&path)
        .expect("demo capture")
        .capture
}

fn ms(sample: u64) -> f64 {
    sample as f64 * 1000.0 / SAMPLERATE as f64
}

fn i2c_words(items: &[i2c::Item]) -> Vec<String> {
    items
        .iter()
        .map(|item| match item.kind {
            i2c::Kind::Start => "S".to_owned(),
            i2c::Kind::RepeatedStart => "Sr".to_owned(),
            i2c::Kind::Stop => "P".to_owned(),
            i2c::Kind::AddressWrite => format!("W{:02X}", item.address()),
            i2c::Kind::AddressRead => format!("R{:02X}", item.address()),
            i2c::Kind::DataWrite | i2c::Kind::DataRead => format!("{:02X}", item.byte),
            i2c::Kind::Ack => "A".to_owned(),
            i2c::Kind::Nack => "N".to_owned(),
        })
        .collect()
}

#[test]
fn decodes_i2c_register_reads() {
    let capture = demo();
    let trace = i2c::decode(
        capture.channel(1).expect("SCL"),
        capture.channel(0).expect("SDA"),
        capture.len(),
    );
    assert_eq!(trace.cut_head, None);
    let words = i2c_words(&trace.items).join(" ");
    let transfers: Vec<&str> = words
        .split(" P")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    assert_eq!(transfers.len(), 6);
    // The two transfers in the design's viewport (1.86–2.6 ms).
    assert_eq!(transfers[0], "S W62 A 04 A Sr R62 A 4F N");
    assert_eq!(transfers[1], "S W62 A 02 A Sr R62 A 32 A BB N");
    let first = trace.items.first().expect("start");
    assert!((ms(first.start) - 1.8059).abs() < 1e-3);
}

#[test]
fn decodes_uart_text() {
    let capture = demo();
    let trace = uart::decode(
        capture.channel(5).expect("RX"),
        capture.len(),
        SAMPLERATE,
        &uart::Options::default(),
    );
    assert_eq!((trace.cut_head, trace.cut_tail), (None, None));
    assert!(trace.frames.iter().all(|frame| !frame.framing_error));
    let text: String = trace
        .frames
        .iter()
        .map(|frame| char::from(frame.value as u8))
        .collect();
    assert_eq!(text, "DSLogic series USB-based LA from DreamSourceLab");
    let in_view: String = trace
        .frames
        .iter()
        .filter(|frame| (1.85..2.6).contains(&ms(frame.start)))
        .map(|frame| char::from(frame.value as u8))
        .collect();
    assert_eq!(in_view, "SB-based ");
}

#[test]
fn decodes_spi_transfers() {
    let capture = demo();
    let lines = spi::Lines {
        clk: capture.channel(12).expect("CLK"),
        miso: capture.channel(15),
        mosi: capture.channel(14),
        cs: capture.channel(13),
    };
    let trace = spi::decode(&lines, capture.len(), &spi::Options::default());
    let sizes: Vec<usize> = trace.transfers.iter().map(|t| t.words.len()).collect();
    assert_eq!(sizes, vec![16, 240, 16, 16]);
    let long = &trace.transfers[1];
    assert!(!long.cut_head && long.end.is_some());
    let words = &trace.words[long.words.clone()];
    let miso: Vec<u8> = words.iter().map(|w| w.miso.expect("MISO") as u8).collect();
    let text = String::from_utf8_lossy(&miso);
    assert!(text.contains("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\n"));
    let mosi: Vec<u8> = words.iter().map(|w| w.mosi.expect("MOSI") as u8).collect();
    assert_eq!(
        &mosi[..16],
        &[
            0x00, 0x21, 0x00, 0x00, 0x79, 0x02, 0x00, 0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB,
            0xCD, 0xEF
        ]
    );
    assert!(mosi[16..].iter().all(|&b| b == 0xFF));
    assert_eq!(mosi.len() - 16, 224);
}

#[test]
fn decodes_can_fd_frames_with_valid_crcs() {
    let capture = demo();
    let options = can::Options {
        nominal_bitrate: 500_000.0,
        fast_bitrate: 2_000_000.0,
        sample_point: 70.0,
    };
    let trace = can::decode(
        capture.channel(9).expect("CAN"),
        capture.len(),
        SAMPLERATE,
        &options,
    );
    assert_eq!((trace.cut_head, trace.cut_tail), (None, None));
    assert_eq!(trace.frames.len(), 3);
    for frame in &trace.frames {
        assert_eq!(frame.id, 0x100);
        assert!(frame.fd && frame.brs && !frame.extended);
        assert_eq!(frame.dlc, 15);
        assert_eq!(frame.data.len(), 64);
        assert_eq!(
            &frame.data[..8],
            &[0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]
        );
        assert!(
            frame.crc_ok,
            "CRC-21 of the frame at {:.4} ms",
            ms(frame.start)
        );
        assert!(frame.acked);
        assert_eq!(frame.error, None);
    }
    // None in the design's viewport.
    assert!(
        trace
            .frames
            .iter()
            .all(|f| ms(f.end) < 1.86 || ms(f.start) > 2.6)
    );
}
