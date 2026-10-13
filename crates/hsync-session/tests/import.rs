//! Waveform imports as the workspace sees them: the capture they open, and saving it again.

use std::path::PathBuf;

use hsync_session::{CaptureSource, ExportKind, ExportRequest, ImportRequest, Workspace};

/// A logic analyzer's VCD export: changes on a 100 MHz clock, in picoseconds.
const ILA: &str = "$timescale 1 ps $end
$scope module top $end
$var wire 1 ! clk $end
$var wire 2 \" data [1:0] $end
$upscope $end
$enddefinitions $end
#0
0!
b00 \"
#10000
1!
b11 \"
#20000
0!
#30000
1!
b10 \"
";

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("hsync-import-{}-{name}", std::process::id()))
}

fn demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl")
}

#[test]
fn opens_an_imported_waveform_as_a_file_and_saves_it() {
    let vcd = temp("ila.vcd");
    std::fs::write(&vcd, ILA).expect("fixture written");
    let mut waveform = hsync_session::read_waveform_file(&vcd, &mut |_| {}).expect("reads");
    let signals: Vec<u32> = waveform
        .summary()
        .items
        .iter()
        .filter_map(|item| item.signal)
        .collect();
    let imported = waveform
        .import(
            &ImportRequest {
                signals,
                samplerate: None,
            },
            &mut |_| true,
        )
        .expect("imports");

    let mut workspace = Workspace::new();
    let summary = workspace.open_imported(&vcd, imported).expect("opens");
    assert_eq!(
        summary.source,
        CaptureSource::File {
            path: vcd.to_string_lossy().into_owned()
        }
    );
    assert_eq!(summary.info.samplerate, 100_000_000);
    assert_eq!(summary.info.sample_count, 4);
    assert_eq!(summary.info.trigger_sample, None);
    let names: Vec<&str> = summary
        .channels
        .iter()
        .map(|channel| channel.name.as_str())
        .collect();
    assert_eq!(names, ["clk", "data[0]", "data[1]"]);
    assert!(summary.decoders.is_none());

    let dsl = temp("ila.hsync");
    let saved = workspace
        .save(summary.id, &dsl, serde_json::json!({}))
        .expect("saves");
    assert_eq!(
        saved.source,
        CaptureSource::File {
            path: dsl.to_string_lossy().into_owned()
        }
    );
    let reopened = workspace.open_file(&dsl).expect("reopens");
    assert_eq!(reopened.info.samplerate, 100_000_000);
    assert_eq!(reopened.info.sample_count, 4);
    let names: Vec<&str> = reopened
        .channels
        .iter()
        .map(|channel| channel.name.as_str())
        .collect();
    assert_eq!(names, ["clk", "data[0]", "data[1]"]);
    let capture = workspace.capture(reopened.id).expect("capture");
    let capture = capture.read();
    let clk = capture.channel(0).expect("clk");
    assert_eq!(
        (0..4).map(|sample| clk.sample(sample)).collect::<Vec<_>>(),
        [false, true, false, true]
    );
    let _ = std::fs::remove_file(&vcd);
    let _ = std::fs::remove_file(&dsl);
}

#[test]
fn exports_fst_that_reads_back() {
    let mut workspace = Workspace::new();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let source = workspace.export_source(opened.id).expect("source");
    let path = temp("demo.fst");
    source
        .export(
            &path,
            &ExportRequest {
                kind: ExportKind::Fst,
                every_sample: false,
                range: Some((26_000, 27_100)),
                channels: vec![13, 1],
            },
            &mut |_| true,
        )
        .expect("export");
    let mut waveform = hsync_session::read_waveform_file(&path, &mut |_| {}).expect("reads");
    let summary = waveform.summary();
    // Only CS# changes in the segment, so the timestamps alone suggest a slower clock; the
    // import names the real one.
    assert_eq!(summary.samplerate, 8_333_333);
    assert_eq!(
        (summary.unit, summary.span, summary.finest),
        (1e-8, 4_404, 12)
    );
    let names: Vec<&str> = summary
        .items
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    assert_eq!(names, ["HSync", "CS#", "SCL"]);
    let imported = waveform
        .import(
            &ImportRequest {
                signals: summary
                    .items
                    .iter()
                    .filter_map(|item| item.signal)
                    .collect(),
                samplerate: Some(25_000_000),
            },
            &mut |_| true,
        )
        .expect("imports");
    assert_eq!(imported.capture.samplerate(), 25_000_000);
    assert_eq!(imported.capture.len(), 1_102);
    // CS# falls at sample 26,999 of the demo: sample 999 of the segment.
    let cs = imported.capture.channel(0).expect("CS#");
    assert!(cs.sample(998));
    assert!(!cs.sample(999));
    let _ = std::fs::remove_file(&path);
}
