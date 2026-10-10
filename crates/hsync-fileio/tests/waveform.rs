//! Waveform imports: hand-written VCD files, the exports read back, and what goes wrong.

use std::collections::BTreeMap;
use std::io::Cursor;

use hsync_capture::LogicCapture;
use hsync_core::ChannelIndex;
use hsync_fileio::{
    ExportFormat, ExportRequest, FileError, ImportRequest, ImportedCapture, Waveform,
    WaveformFormat, WaveformItemKind, export, read_waveform,
};

/// The VCD a logic analyzer inside an FPGA exports: every change on a 100 MHz clock, in
/// picoseconds, with a vector and a signal that never changes.
const ILA: &str = "$date today $end
$version vendor tool $end
$timescale 1 ps $end
$scope module top $end
$var wire 1 ! clk $end
$var wire 1 \" valid $end
$var wire 4 # data [3:0] $end
$scope module uart $end
$var wire 1 $ tx $end
$var real 64 % temperature $end
$upscope $end
$upscope $end
$enddefinitions $end
#0
$dumpvars
0!
0\"
b0000 #
1$
r21.5 %
$end
#10000
1!
#20000
0!
1\"
b1010 #
#30000
1!
0$
#40000
0!
bxz01 #
#50000
1!
";

fn read(text: &str) -> Waveform {
    read_waveform(Cursor::new(text.as_bytes().to_vec()), &mut |_| {}).expect("waveform reads")
}

fn import_all(waveform: &mut Waveform) -> ImportedCapture {
    let signals: Vec<u32> = waveform
        .summary()
        .items
        .iter()
        .filter_map(|item| item.signal)
        .collect();
    waveform
        .import(
            &ImportRequest {
                signals,
                samplerate: None,
            },
            &mut |_| true,
        )
        .expect("import")
}

/// The levels of channel `index` as a string of `0`/`1`.
fn levels(capture: &LogicCapture, index: ChannelIndex) -> String {
    let data = capture.channel(index).expect("channel");
    (0..data.len())
        .map(|sample| if data.sample(sample) { '1' } else { '0' })
        .collect()
}

#[test]
fn lists_the_hierarchy_and_the_grid() {
    let waveform = read(ILA);
    let summary = waveform.summary();
    assert_eq!(summary.format, WaveformFormat::Vcd);
    assert_eq!(summary.samplerate, 100_000_000);
    assert_eq!(summary.sample_count, 6);
    assert!(!summary.coarsened);
    let listed: Vec<(u32, &str, WaveformItemKind, u32)> = summary
        .items
        .iter()
        .map(|item| (item.depth, item.name.as_str(), item.kind, item.width))
        .collect();
    assert_eq!(
        listed,
        [
            (0, "top", WaveformItemKind::Scope, 0),
            (1, "clk", WaveformItemKind::Logic, 1),
            (1, "valid", WaveformItemKind::Logic, 1),
            (1, "data", WaveformItemKind::Logic, 4),
            (1, "uart", WaveformItemKind::Scope, 0),
            (2, "tx", WaveformItemKind::Logic, 1),
            (2, "temperature", WaveformItemKind::Other, 0),
        ]
    );
    assert_eq!(summary.items[3].range, Some((3, 0)));
    assert!(summary.items[6].signal.is_none());
}

#[test]
fn lays_signals_onto_the_clock_grid() {
    let mut waveform = read(ILA);
    let imported = import_all(&mut waveform);
    let capture = &imported.capture;
    assert_eq!(capture.samplerate(), 100_000_000);
    assert_eq!(capture.len(), 6);
    assert_eq!(capture.trigger_sample(), None);
    let names: Vec<&str> = imported
        .channel_names
        .values()
        .map(String::as_str)
        .collect();
    assert_eq!(
        names,
        [
            "clk", "valid", "data[0]", "data[1]", "data[2]", "data[3]", "tx"
        ]
    );
    assert_eq!(levels(capture, 0), "010101");
    assert_eq!(levels(capture, 1), "001111");
    // `b1010` from sample 2, then `bxz01`: x and z read as low.
    assert_eq!(levels(capture, 2), "000011");
    assert_eq!(levels(capture, 3), "001100");
    assert_eq!(levels(capture, 4), "000000");
    assert_eq!(levels(capture, 5), "001100");
    assert_eq!(levels(capture, 6), "111000");
}

#[test]
fn imports_the_chosen_signals_in_their_order() {
    let mut waveform = read(ILA);
    let summary = waveform.summary();
    let tx = summary.items[5].signal.expect("tx");
    let clk = summary.items[1].signal.expect("clk");
    let imported = waveform
        .import(
            &ImportRequest {
                signals: vec![tx, clk, tx],
                samplerate: None,
            },
            &mut |_| true,
        )
        .expect("import");
    let names: Vec<&str> = imported
        .channel_names
        .values()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["tx", "clk"]);
    assert_eq!(levels(&imported.capture, 0), "111000");
    assert_eq!(levels(&imported.capture, 1), "010101");
}

#[test]
fn names_shared_names_in_full_and_descending_ranges_by_index() {
    let text = "$timescale 1 ns $end
$scope module a $end
$var wire 1 ! clk $end
$var wire 3 \" bus [0:2] $end
$scope module b $end
$var wire 1 # clk $end
$upscope $end
$upscope $end
$enddefinitions $end
#0
1!
b100 \"
0#
#5
0!
";
    let mut waveform = read(text);
    let imported = import_all(&mut waveform);
    let names: Vec<&str> = imported
        .channel_names
        .values()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["a.clk", "bus[2]", "bus[1]", "bus[0]", "a.b.clk"]);
    // `b100` for `[0:2]`: bit 0 is the leftmost.
    assert_eq!(levels(&imported.capture, 1), "00");
    assert_eq!(levels(&imported.capture, 3), "11");
}

#[test]
fn takes_the_timescale_factor_and_short_vectors() {
    let text = "$timescale 10 ns $end
$scope module m $end
$var reg 8 ! q [7:0] $end
$upscope $end
$enddefinitions $end
#0
b1 !
#4
b11 !
#6
";
    let mut waveform = read(text);
    assert_eq!(waveform.samplerate(), 50_000_000);
    assert_eq!(waveform.sample_count(), 4);
    let imported = import_all(&mut waveform);
    assert_eq!(levels(&imported.capture, 0), "1111");
    assert_eq!(levels(&imported.capture, 1), "0011");
    assert_eq!(levels(&imported.capture, 7), "0000");
}

#[test]
fn starts_at_the_first_timestamp() {
    let text = "$timescale 1 us $end
$scope module m $end
$var wire 1 ! a $end
$upscope $end
$enddefinitions $end
#100
1!
#101
0!
#103
1!
";
    let mut waveform = read(text);
    let imported = import_all(&mut waveform);
    assert_eq!(imported.capture.samplerate(), 1_000_000);
    assert_eq!(levels(&imported.capture, 0), "1001");
}

/// A capture whose channels hold the given levels, one string of `0`/`1` each.
fn build(rate: u64, levels: &[(ChannelIndex, &str)]) -> LogicCapture {
    let mut capture = LogicCapture::new(rate, levels.iter().map(|(index, _)| *index));
    for (index, text) in levels {
        let bits: Vec<bool> = text.bytes().map(|b| b == b'1').collect();
        let words: Vec<u64> = bits
            .chunks(64)
            .map(|chunk| {
                chunk
                    .iter()
                    .enumerate()
                    .fold(0, |word, (bit, &high)| word | (u64::from(high) << bit))
            })
            .collect();
        let data = capture.channel_mut(*index).expect("channel");
        data.append_words(&words);
        data.truncate(bits.len() as u64);
    }
    capture
}

fn export_bytes(capture: &LogicCapture, format: ExportFormat) -> Vec<u8> {
    let names: Vec<(ChannelIndex, String)> = capture
        .channels()
        .map(|(index, _)| (index, format!("D{index}")))
        .collect();
    let request = ExportRequest {
        format,
        channels: &names,
        total_channels: 16,
        first_sample: 0,
        date: "2026-10-10 06:30:00 UTC",
        generator: "HSync 0.1.0",
    };
    export(Cursor::new(Vec::new()), capture, &request, &mut |_| true)
        .expect("export")
        .into_inner()
}

fn assert_round_trip(format: ExportFormat, expected: WaveformFormat) {
    let original = build(
        25_000_000,
        &[(0, "0011001110"), (3, "1111111111"), (5, "0000000001")],
    );
    let bytes = export_bytes(&original, format);
    let mut waveform =
        read_waveform(Cursor::new(bytes), &mut |_| {}).expect("the export reads back");
    assert_eq!(waveform.format(), expected);
    let imported = import_all(&mut waveform);
    assert_eq!(imported.capture.samplerate(), 25_000_000);
    // The final timestamp marks the length; the sample it starts holds the last levels.
    assert_eq!(imported.capture.len(), 11);
    let names: Vec<&str> = imported
        .channel_names
        .values()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["D0", "D3", "D5"]);
    assert_eq!(levels(&imported.capture, 0), "00110011100");
    assert_eq!(levels(&imported.capture, 1), "11111111111");
    assert_eq!(levels(&imported.capture, 2), "00000000011");
}

#[test]
fn reads_back_the_vcd_export() {
    assert_round_trip(ExportFormat::Vcd, WaveformFormat::Vcd);
}

#[test]
fn reads_back_the_fst_export() {
    assert_round_trip(ExportFormat::Fst, WaveformFormat::Fst);
}

#[test]
fn reports_progress_and_stops_when_cancelled() {
    let mut waveform = read(ILA);
    let signals: Vec<u32> = waveform
        .summary()
        .items
        .iter()
        .filter_map(|item| item.signal)
        .collect();
    let mut reports = Vec::new();
    let imported = waveform
        .import(
            &ImportRequest {
                signals: signals.clone(),
                samplerate: None,
            },
            &mut |done| {
                reports.push(done);
                true
            },
        )
        .expect("import");
    assert_eq!(imported.capture.len(), 6);
    assert_eq!(reports.last().copied(), Some(1.0));
    assert!(reports.windows(2).all(|pair| pair[0] <= pair[1]));
    let cancelled = waveform.import(
        &ImportRequest {
            signals,
            samplerate: None,
        },
        &mut |_| false,
    );
    assert!(matches!(cancelled, Err(FileError::Cancelled)));
}

#[test]
fn refuses_what_is_not_a_waveform() {
    let text = "PK\x03\x04 not a waveform at all";
    let error = read_waveform(Cursor::new(text.as_bytes().to_vec()), &mut |_| {})
        .map(|waveform| waveform.format())
        .expect_err("refused");
    assert!(matches!(error, FileError::NotWaveform), "{error:?}");
}

#[test]
fn refuses_a_waveform_without_timestamps() {
    let text = "$timescale 1 ns $end
$scope module m $end
$var wire 1 ! a $end
$upscope $end
$enddefinitions $end
";
    let error = read_waveform(Cursor::new(text.as_bytes().to_vec()), &mut |_| {})
        .map(|waveform| waveform.format())
        .expect_err("refused");
    assert!(matches!(error, FileError::EmptyWaveform), "{error:?}");
}

#[test]
fn refuses_unknown_and_missing_signals() {
    let mut waveform = read(ILA);
    let unknown = waveform.import(
        &ImportRequest {
            signals: vec![99],
            samplerate: None,
        },
        &mut |_| true,
    );
    assert!(matches!(unknown, Err(FileError::UnknownSignal(99))));
    let none = waveform.import(
        &ImportRequest {
            signals: vec![],
            samplerate: None,
        },
        &mut |_| true,
    );
    assert!(matches!(none, Err(FileError::NoSignal)));
}

#[test]
fn keeps_the_names_of_the_imported_channels_by_index() {
    let mut waveform = read(ILA);
    let imported = import_all(&mut waveform);
    let expected: BTreeMap<ChannelIndex, String> = [
        (0, "clk"),
        (1, "valid"),
        (2, "data[0]"),
        (3, "data[1]"),
        (4, "data[2]"),
        (5, "data[3]"),
        (6, "tx"),
    ]
    .into_iter()
    .map(|(index, name)| (index, name.to_owned()))
    .collect();
    assert_eq!(imported.channel_names, expected);
    assert_eq!(
        imported.capture.info().channels,
        (0..7).collect::<Vec<ChannelIndex>>()
    );
}
