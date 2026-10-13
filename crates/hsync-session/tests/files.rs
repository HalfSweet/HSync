//! Saving and exporting the demo capture: whole or in part, in each format, and cancelled.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_session::{ExportKind, ExportRequest, SessionError, SessionEvent, Workspace};

fn demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl")
}

fn fresh() -> (Workspace, Receiver<SessionEvent>) {
    let (sender, receiver) = crossbeam_channel::unbounded();
    let sink = Arc::new(move |event: SessionEvent| {
        let _ = sender.send(event);
    });
    (Workspace::with_events(sink), receiver)
}

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("hsync-files-{}-{name}", std::process::id()))
}

fn request(kind: ExportKind, range: Option<(u64, u64)>, channels: &[u16]) -> ExportRequest {
    ExportRequest {
        kind,
        every_sample: false,
        range,
        channels: channels.to_vec(),
    }
}

fn decoders_done(events: &Receiver<SessionEvent>, id: u64, count: usize) {
    let mut done = 0;
    while done < count {
        if let SessionEvent::Decoded { capture_id, .. } = events
            .recv_timeout(Duration::from_secs(10))
            .expect("decoder event")
            && capture_id == id
        {
            done += 1;
        }
    }
}

#[test]
fn exports_a_segment_as_a_capture_with_the_decoders_it_can_keep() {
    let (mut workspace, _) = fresh();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let source = workspace.export_source(opened.id).expect("source");
    let path = temp("segment.dsl");
    source
        .export(
            &path,
            &request(ExportKind::Dsl, Some((40_000, 70_000)), &[1, 0]),
            &mut |_| true,
        )
        .expect("export");

    let (mut other, events) = fresh();
    let reopened = other.open_file(&path).expect("reopen");
    assert_eq!(reopened.info.sample_count, 30_001);
    assert_eq!(reopened.info.channels, vec![0, 1]);
    // Only I2C uses nothing but D0 and D1.
    decoders_done(&events, reopened.id, 1);
    let statuses = other.decoding(reopened.id).expect("decoders").statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].kinds, ["i2c", "eeprom24xx"]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn exports_csv_vcd_sigrok_and_bytes() {
    let (mut workspace, _) = fresh();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let source = workspace.export_source(opened.id).expect("source");
    for (kind, name) in [
        (ExportKind::Csv, "out.csv"),
        (ExportKind::Vcd, "out.vcd"),
        (ExportKind::Sigrok, "out.sr"),
        (ExportKind::Binary, "out.bin"),
    ] {
        let path = temp(name);
        let mut last = 0.0;
        source
            .export(&path, &request(kind, None, &[0, 1]), &mut |done| {
                assert!(done >= last, "progress goes forward");
                last = done;
                true
            })
            .expect("export");
        assert_eq!(last, 1.0, "{name}");
        let bytes = std::fs::read(&path).expect("written");
        match kind {
            ExportKind::Csv => {
                let text = String::from_utf8(bytes).expect("utf-8");
                assert!(
                    text.contains("Time(s), SDA, SCL\n0,1,1\n"),
                    "{}",
                    &text[..200]
                );
            }
            ExportKind::Vcd => {
                let text = String::from_utf8(bytes).expect("utf-8");
                assert!(text.contains("$timescale 10 ns $end"));
                assert!(text.ends_with(&format!("#{}\n", 131_072 * 4)));
            }
            ExportKind::Binary => assert_eq!(bytes.len(), 131_072),
            _ => assert_eq!(&bytes[..2], b"PK"),
        }
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn cancelling_leaves_no_file() {
    let (mut workspace, _) = fresh();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let source = workspace.export_source(opened.id).expect("source");
    let path = temp("cancelled.dsl");
    let result = source.save(&path, &mut |_| false);
    assert!(matches!(
        result,
        Err(SessionError::File(hsync_fileio::FileError::Cancelled))
    ));
    assert!(!Path::new(&path).exists());
    assert!(
        !path
            .with_file_name(format!(
                "{}.part",
                path.file_name().expect("name").to_string_lossy()
            ))
            .exists()
    );
    let empty = source.export(&path, &request(ExportKind::Csv, None, &[42]), &mut |_| true);
    assert!(matches!(empty, Err(SessionError::NothingToExport)));
}

#[test]
fn writes_text_files() {
    let path = temp("stream.csv");
    hsync_session::write_text(&path, "a,b\n1,2\n").expect("write");
    assert_eq!(
        hsync_session::read_text(&path, 1 << 20).expect("read"),
        "a,b\n1,2\n"
    );
    assert!(hsync_session::read_text(&path, 3).is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn renamed_channels_are_saved_with_their_names() {
    let (mut workspace, _) = fresh();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let renamed = workspace
        .rename_channel(opened.id, 2, "  RESET ")
        .expect("rename");
    assert_eq!(renamed.channels[2].name, "RESET");
    assert!(workspace.rename_channel(opened.id, 99, "x").is_err());
    let back = workspace.rename_channel(opened.id, 0, "").expect("clear");
    assert_eq!(back.channels[0].name, "0");
    let path = temp("renamed.dsl");
    workspace.save(opened.id, &path).expect("save");
    let (mut other, _) = fresh();
    let reopened = other.open_file(&path).expect("reopen");
    assert_eq!(reopened.channels[2].name, "RESET");
    assert_eq!(reopened.channels[0].name, "0");
    let _ = std::fs::remove_file(path);
}
