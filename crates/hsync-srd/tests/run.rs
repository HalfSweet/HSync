//! DSView's own Python decoders on the protocol demo, against what the native decoders find.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use hsync_srd::{DecoderHost, Stack, run_stack};
use parking_lot::RwLock;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn demo() -> hsync_fileio::DslDocument {
    hsync_fileio::read_dsl_file(&root().join("src-tauri/resources/demo/protocol.dsl"))
        .expect("demo")
}

fn entry(document: &hsync_fileio::DslDocument, id: &str) -> serde_json::Value {
    document
        .decoders
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .expect("decoders")
        .iter()
        .find(|entry| entry["id"] == id)
        .expect(id)
        .clone()
}

#[test]
fn python_i2c_and_its_eeprom_decode_the_demo() {
    let document = demo();
    let stack = Stack::from_dsview(&entry(&document, "1:i2c")).expect("stack");
    assert_eq!(stack.layers.len(), 2, "1:i2c with eeprom24xx stacked");
    let host = DecoderHost::new(root().join("src-tauri/resources/decoders"));
    let len = document.capture.len();
    let capture = Arc::new(RwLock::new(document.capture));
    let outcome = run_stack(
        &host,
        &capture,
        len,
        &stack,
        &Arc::new(AtomicBool::new(false)),
    )
    .expect("run");
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let results = outcome.layers;
    let i2c = &results[0];
    // Address and data bytes: the same 4 + 3 × 5 + 4 + 4 = 27 bytes as the native decoder.
    let bytes = i2c
        .annotations
        .iter()
        .filter(|annotation| (6..=9).contains(&annotation.class))
        .count();
    assert_eq!(bytes, 27, "{:?}", &i2c.annotations[..8]);
    assert!(
        i2c.annotations
            .iter()
            .any(|annotation| annotation.texts.iter().any(|text| text.contains("Start")))
    );
    let eeprom = &results[1];
    assert!(!eeprom.annotations.is_empty(), "eeprom24xx put out nothing");
}

#[test]
fn python_uart_reads_the_demo_text() {
    let document = demo();
    let stack = Stack::from_dsview(&entry(&document, "1:uart")).expect("stack");
    let host = DecoderHost::new(root().join("src-tauri/resources/decoders"));
    let len = document.capture.len();
    let capture = Arc::new(RwLock::new(document.capture));
    let outcome = run_stack(
        &host,
        &capture,
        len,
        &stack,
        &Arc::new(AtomicBool::new(false)),
    )
    .expect("run");
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let results = outcome.layers;
    let rows = &results[0].rows;
    assert!(
        rows.iter()
            .any(|(id, _)| id == "rxtx" || id.contains("data")),
        "{rows:?}"
    );
    // 1:uart puts each frame's value as '@XX' for the user's number format.
    let text: String = results[0]
        .annotations
        .iter()
        .filter(|annotation| annotation.class == 0)
        .filter_map(|annotation| annotation.value.as_deref())
        .filter_map(|hex| u8::from_str_radix(hex, 16).ok())
        .map(char::from)
        .collect();
    assert!(
        text.contains("DSLogic series USB-based LA from DreamSourceLab"),
        "{text}"
    );
}
