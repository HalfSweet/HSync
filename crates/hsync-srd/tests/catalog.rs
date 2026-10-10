//! Reading DSView's decoders.

use std::path::PathBuf;

use hsync_srd::load_catalog;

fn decoders() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/resources/decoders")
}

#[test]
fn reads_every_decoder() {
    let catalog = load_catalog(&decoders()).expect("catalog");
    // 150 decoder packages; `common` holds helpers only.
    assert!(catalog.len() >= 145, "{} decoders", catalog.len());
    let i2c = catalog.iter().find(|one| one.id == "1:i2c").expect("1:i2c");
    assert_eq!(i2c.name, "1:I²C");
    let roles: Vec<&str> = i2c
        .channels
        .iter()
        .map(|channel| channel.id.as_str())
        .collect();
    assert_eq!(roles, ["scl", "sda"]);
    assert_eq!(i2c.options[0].id, "address_format");
    assert_eq!(i2c.options[0].default, "unshifted");
    assert_eq!(i2c.annotations[0].kind, Some(7));
    assert_eq!(i2c.annotation_rows[1].id, "addr-data");
    let eeprom = catalog
        .iter()
        .find(|one| one.id == "eeprom24xx")
        .expect("eeprom24xx");
    assert_eq!(eeprom.inputs, ["i2c"]);
    let uart = catalog
        .iter()
        .find(|one| one.id == "1:uart")
        .expect("1:uart");
    assert!(
        uart.options
            .iter()
            .any(|option| option.id == "baudrate" && option.default == 115_200)
    );
}
