//! Workspace behaviour on DSView's demo capture.

use std::path::PathBuf;

use hsync_session::{CaptureSource, Direction, SessionError, Workspace, edge, pulse};

fn demo_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/demo/protocol.dsl")
}

#[test]
fn opens_the_demo_and_names_its_channels() {
    let mut workspace = Workspace::new();
    assert!(workspace.summary().is_none());
    let summary = workspace.open_demo(&demo_path()).expect("demo opens");
    assert_eq!(summary.source, CaptureSource::Demo);
    assert_eq!(summary.info.samplerate, 25_000_000);
    assert_eq!(summary.channels.len(), 16);
    assert_eq!(summary.channels[1].name, "SCL");
    assert_eq!(summary.channels[2].name, "2");
    assert!(summary.decoders.is_some());
}

#[test]
fn queries_fail_for_replaced_captures() {
    let mut workspace = Workspace::new();
    let first = workspace.open_demo(&demo_path()).expect("demo opens");
    let second = workspace.open_demo(&demo_path()).expect("demo opens again");
    assert_ne!(first.id, second.id);
    assert!(matches!(
        workspace.capture(first.id),
        Err(SessionError::StaleCapture)
    ));
    assert!(workspace.capture(second.id).is_ok());
}

#[test]
fn reads_pulses_and_edges_of_the_i2c_clock() {
    let mut workspace = Workspace::new();
    let summary = workspace.open_demo(&demo_path()).expect("demo opens");
    let capture = workspace.capture(summary.id).expect("open");
    let capture = capture.read();

    // SCL idles high until the first read starts at 1.8192 ms.
    let first_fall = edge(&capture, 1, 0, Direction::Next)
        .expect("SCL exists")
        .expect("SCL toggles");
    assert_eq!(first_fall, 45_479);
    let idle = pulse(&capture, 1, 10)
        .expect("SCL exists")
        .expect("in range");
    assert!(idle.high && !idle.start_is_edge && idle.end_is_edge);
    assert_eq!(idle.end, first_fall);

    // Inside the read, clock pulses are 5 µs (125 samples) wide and alternate regularly.
    let low = pulse(&capture, 1, first_fall + 10)
        .expect("SCL exists")
        .expect("in range");
    assert!(!low.high);
    let period = low.next_end.expect("clock continues") - low.start;
    assert!((240..=260).contains(&period), "period {period}");
    assert_eq!(
        edge(&capture, 1, first_fall + 10, Direction::Previous).expect("SCL exists"),
        Some(first_fall)
    );
}
