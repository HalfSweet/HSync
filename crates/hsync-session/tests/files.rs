//! Saving and exporting the demo capture: whole or in part, in each format, and cancelled.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_session::{ExportKind, ExportRequest, ModuleInfo, SessionError, SessionEvent, Workspace};
use serde_json::json;

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
fn exports_a_segment_as_an_hsync_capture_with_the_decoders_it_can_keep() {
    let (mut workspace, _) = fresh();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let source = workspace.export_source(opened.id).expect("source");
    let path = temp("segment.hsync");
    source
        .export(
            &path,
            &request(ExportKind::Hsync, Some((40_000, 70_000)), &[1, 0]),
            &mut |_| true,
        )
        .expect("export");

    let (mut other, events) = fresh();
    let reopened = other.open_file(&path).expect("reopen");
    assert_eq!(reopened.info.sample_count, 30_001);
    assert_eq!(reopened.info.channels, vec![0, 1]);
    assert_eq!(reopened.channels[0].name, "SDA");
    assert_eq!(reopened.session, None);
    // Only I2C uses nothing but D0 and D1.
    decoders_done(&events, reopened.id, 1);
    let statuses = other.decoding(reopened.id).expect("decoders").statuses();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].kinds, ["i2c", "eeprom24xx"]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_capture_file_carries_the_decoder_modules_its_decoders_name() {
    let module_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../uiharu/dsl/ir/uart.json");
    assert!(
        module_path.is_file(),
        "{} is missing; run `pnpm dsl:compile` before Cargo tests",
        module_path.display()
    );
    let (mut workspace, events) = fresh();
    workspace.load_module(&module_path).expect("loads");
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    decoders_done(&events, opened.id, 4);
    let path = temp("carried.hsync");
    workspace.save(opened.id, &path, json!({})).expect("saves");
    let document = hsync_fileio::read_hsync_file(&path).expect("reads");
    assert_eq!(
        document.modules,
        BTreeMap::from([(
            "uart".to_owned(),
            std::fs::read_to_string(&module_path).expect("the module")
        )])
    );

    // Opened elsewhere, the file brings its module along, says so, and decodes with it.
    let (mut other, events) = fresh();
    let reopened = other.open_file(&path).expect("reopens");
    assert!(matches!(
        events
            .recv_timeout(Duration::from_secs(10))
            .expect("an event"),
        SessionEvent::ModulesChanged
    ));
    let carried = ModuleInfo {
        name: "uart".into(),
        path: path.to_string_lossy().into_owned(),
        category: "bus".into(),
        shadows: true,
    };
    assert_eq!(other.modules(), std::slice::from_ref(&carried));
    let uart = other.catalog();
    let uart = uart
        .iter()
        .find(|entry| entry.name == "uart")
        .expect("uart");
    assert_eq!(uart.module.as_deref(), Some(carried.path.as_str()));
    decoders_done(&events, reopened.id, 4);
    let statuses = other.decoding(reopened.id).expect("decoders").statuses();
    assert_eq!(statuses[1].kinds, ["uart"]);
    assert_eq!(statuses[1].streams[0].progress.events, 47);
    // Saved again from there, the file carries the module on.
    let again = temp("carried-again.hsync");
    other.save(reopened.id, &again, json!({})).expect("saves");
    let document = hsync_fileio::read_hsync_file(&again).expect("reads");
    assert_eq!(document.modules.keys().collect::<Vec<_>>(), ["uart"]);
    // Let go, it is not brought back until the file is opened again.
    assert!(other.unload_module("uart"));
    assert!(other.modules().is_empty());

    // A workspace with a module of that name keeps its own.
    let (mut third, _) = fresh();
    let its_own = third.load_module(&module_path).expect("loads");
    third.open_file(&path).expect("reopens");
    assert_eq!(third.modules(), [its_own]);

    // A segment without the UART line carries no module: no decoder written names it.
    let source = workspace.export_source(opened.id).expect("source");
    let segment = temp("carried-segment.hsync");
    source
        .export(
            &segment,
            &request(ExportKind::Hsync, Some((40_000, 70_000)), &[1, 0]),
            &mut |_| true,
        )
        .expect("exports");
    let document = hsync_fileio::read_hsync_file(&segment).expect("reads");
    assert!(document.modules.is_empty());
    for file in [path, again, segment] {
        let _ = std::fs::remove_file(file);
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
    let path = temp("cancelled.hsync");
    let result = source.save(&path, &serde_json::json!({}), &mut |_| false);
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
    let path = temp("renamed.hsync");
    workspace
        .save(opened.id, &path, serde_json::json!({}))
        .expect("save");
    let (mut other, _) = fresh();
    let reopened = other.open_file(&path).expect("reopen");
    assert_eq!(reopened.channels[2].name, "RESET");
    assert_eq!(reopened.channels[0].name, "0");
    let _ = std::fs::remove_file(path);
}

#[test]
fn imported_decoder_metadata_survives_edits_and_hsync_save_reopen() {
    let raw = json!([
        {"id":"future-bus","label":"Original unsupported bus", "options":{"opaque":[1,2]},
            "stacked decoders":[{"id":"future-layer","options":{"keep":true}}]},
        {"id":"1:i2c","channel":[{"scl":1},{"sda":0}],
            "options":{"address_format":"shifted","future_option":123},
            "stacked decoders":[{"id":"future-chip","options":{"other":"raw"}}]}
    ]);
    let (mut workspace, _) = fresh();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    // This is the same backend entry point used when applying a .dsc session.
    let imported = workspace
        .set_decoders(opened.id, raw.clone())
        .expect("imports session entries");
    let entries = imported.decoders.expect("preserved entries");
    assert_eq!(entries[0]["import"]["raw"], raw[0]);
    assert_eq!(entries[1]["import"]["raw"], raw[1]);
    let statuses = workspace.decoding(opened.id).expect("decoding").statuses();
    assert!(
        statuses[0].streams.is_empty(),
        "unsupported imports create no decoder worker"
    );
    assert_eq!(
        statuses[0].error.as_ref().expect("diagnostic").code,
        "decode.import.unsupportedDecoder"
    );
    // Editing the runnable entry's label/display must not erase the inert import beside it.
    let mut edited = entries;
    edited[1]["label"] = json!("Edited label");
    edited[1]["show"] = json!({"numbers":"dec"});
    let changed = workspace
        .set_decoders(opened.id, edited.clone())
        .expect("unrelated edit");
    assert_eq!(changed.decoders, Some(edited.clone()));
    let path = temp("preserved-imports.hsync");
    workspace.save(opened.id, &path, json!({})).expect("saves");
    let (mut other, _) = fresh();
    let reopened = other.open_file(&path).expect("reopens");
    assert_eq!(reopened.decoders, Some(edited));
    let _ = std::fs::remove_file(path);
}

#[test]
fn original_dsl_decoder_entries_survive_conversion_and_hsync_round_trip() {
    let source = hsync_fileio::read_dsl_file(&demo_path()).expect("demo document");
    let originals = json!([
        {"id":"future-bus","options":{"unknown":true}},
        {"id":"uart","channel":[{"rxtx":5}],"options":{"baudrate":115200,"future_option":99}}
    ]);
    let names = BTreeMap::new();
    let session = json!({});
    let request = hsync_fileio::DslWriteRequest {
        driver: "DSLogic",
        channel_names: &names,
        session: &session,
        decoders: &originals,
        hsync_decoders: None,
    };
    let dsl = temp("original-imports.dsl");
    hsync_fileio::write_dsl_file(&dsl, &source.capture, &request, &mut |_| true)
        .expect("fixture DSL");
    let (mut workspace, _) = fresh();
    let opened = workspace
        .open_file(&dsl)
        .expect("opens original DSL entries");
    let entries = opened.decoders.clone().expect("entries");
    assert_eq!(entries[0]["import"]["raw"], originals[0]);
    assert_eq!(entries[1]["import"]["raw"], originals[1]);
    let hsync = temp("original-imports.hsync");
    workspace.save(opened.id, &hsync, json!({})).expect("saves");
    let (mut other, _) = fresh();
    assert_eq!(
        other.open_file(&hsync).expect("reopens").decoders,
        Some(entries)
    );
    let _ = std::fs::remove_file(dsl);
    let _ = std::fs::remove_file(hsync);
}

#[test]
fn dsview_first_capture_preset_retains_originals_through_save_and_reopen() {
    let raw = json!([{"id":"future-bus", "options":{"keep":{"nested":[1,2]}},
        "stacked decoders":[{"id":"future-layer"}]}]);
    let source = hsync_fileio::read_dsl_file(&demo_path()).expect("samples");
    let (mut workspace, _) = fresh();
    let live = workspace.begin_live(
        hsync_session::CaptureSource::Device {
            driver: "virtual-demo".into(),
            model: "Demo".into(),
        },
        source.capture,
        false,
        Some(hsync_driver::CapturePreset {
            channel_names: BTreeMap::new(),
            decoders: Some(raw.clone()),
        }),
    );
    assert_eq!(
        live.summary.decoders.as_ref().expect("preserved")[0]["import"]["raw"],
        raw[0]
    );
    let path = temp("preset-imports.hsync");
    workspace
        .rename_channel(live.summary.id, 0, "Unrelated edit")
        .expect("renames");
    workspace
        .save(live.summary.id, &path, json!({}))
        .expect("saves");
    let (mut other, _) = fresh();
    let reopened = other.open_file(&path).expect("reopens");
    assert_eq!(
        reopened.decoders.as_ref().expect("retained")[0]["import"]["raw"],
        raw[0]
    );
    assert_eq!(reopened.channels[0].name, "Unrelated edit");
    let _ = std::fs::remove_file(path);
}

#[test]
fn changing_import_execution_state_rebuilds_even_when_the_graph_is_unchanged() {
    let (mut workspace, _) = fresh();
    let opened = workspace.open_demo(&demo_path()).expect("demo");
    let disabled = json!([{"nodes":{"i2c":{"type":"i2c"}},
        "connections":[{"from":"source","to":"i2c.logic"}],"expose":{"i2c":"i2c.events"},
        "import":{"format":"dsview","raw":{"id":"i2c"},"runnable":false,"diagnostics":[]}}]);
    workspace
        .set_decoders(opened.id, disabled.clone())
        .expect("disabled metadata");
    let before = workspace.decoding(opened.id).expect("set");
    let mut enabled = disabled;
    enabled[0]["import"]["runnable"] = json!(true);
    workspace
        .set_decoders(opened.id, enabled)
        .expect("enable translated graph");
    let after = workspace.decoding(opened.id).expect("new set");
    assert!(!Arc::ptr_eq(&before, &after));
}
