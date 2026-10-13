//! DSView's decoder entries, as `.dsl` files and `.dsc` sessions carry them, turned into
//! HSync's. HSync decodes with uiharu, so the buses both know — I²C (with a 24xx EEPROM
//! stacked on it), UART, SPI and CAN — are translated when a file opens; other decoders are
//! left out.

use serde_json::{Value, json};

/// DSView's decoder id without the `1:`/`0:` prefix of its variants.
fn base_id(id: &str) -> &str {
    match id.split_once(':') {
        Some((prefix, rest)) if prefix.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => id,
    }
}

/// The channel DSView's entry maps `role` to.
fn channel(entry: &Value, role: &str) -> Option<u64> {
    entry
        .get("channel")?
        .as_array()?
        .iter()
        .find_map(|mapping| mapping.get(role)?.as_u64())
}

fn option<'a>(entry: &'a Value, name: &str) -> Option<&'a Value> {
    entry.get("options")?.get(name)
}

fn option_u64(entry: &Value, name: &str) -> Option<u64> {
    option(entry, name).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|number| number as u64))
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    })
}

fn option_str<'a>(entry: &'a Value, name: &str) -> Option<&'a str> {
    option(entry, name).and_then(Value::as_str)
}

/// Builds an entry of one bus node, and the device nodes stacked on it.
fn entry(
    label: Option<&str>,
    bus: &str,
    output: &str,
    config: Value,
    stacked: Vec<(&str, &str, &str, Value)>,
) -> Value {
    let mut nodes = serde_json::Map::new();
    let mut connections = vec![json!({ "from": "source", "to": format!("{bus}.logic") })];
    let mut expose = serde_json::Map::new();
    nodes.insert(bus.to_owned(), json!({ "type": bus, "config": config }));
    expose.insert(bus.to_owned(), json!(format!("{bus}.{output}")));
    for (kind, input, output, config) in stacked {
        nodes.insert(kind.to_owned(), json!({ "type": kind, "config": config }));
        connections.push(json!({
            "from": format!("{bus}.{}", expose[bus].as_str().unwrap_or_default().rsplit('.').next().unwrap_or("events")),
            "to": format!("{kind}.{input}")
        }));
        expose.insert(kind.to_owned(), json!(format!("{kind}.{output}")));
    }
    let mut entry = json!({ "nodes": nodes, "connections": connections, "expose": expose });
    if let Some(label) = label.map(base_id).filter(|label| !label.is_empty()) {
        entry["label"] = json!(label);
    }
    entry
}

/// Address bytes of a 24xx EEPROM DSView names (`24c02`, `24c256`, `generic`): parts of
/// 32 kbit and up take two.
fn eeprom_address_bytes(chip: &str) -> u64 {
    let digits: String = chip
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .trim_start_matches("24")
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    match digits.parse::<u64>() {
        Ok(kbit) if kbit >= 32 => 2,
        _ => 1,
    }
}

/// Turns one of DSView's entries into HSync's, or `None` when uiharu has no decoder for it.
pub fn convert(dsview: &Value) -> Option<Value> {
    let id = dsview.get("id")?.as_str()?;
    let label = dsview.get("label").and_then(Value::as_str);
    match base_id(id) {
        "i2c" => {
            let config = json!({ "scl": channel(dsview, "scl")?, "sda": channel(dsview, "sda")? });
            let stacked = dsview
                .get("stacked decoders")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|layer| layer.get("id").and_then(Value::as_str) == Some("eeprom24xx"))
                .map(|layer| {
                    let chip = option_str(layer, "chip").unwrap_or("generic");
                    (
                        "eeprom24xx",
                        "bus",
                        "operations",
                        json!({ "address_bytes": eeprom_address_bytes(chip) }),
                    )
                })
                .collect();
            Some(entry(label, "i2c", "events", config, stacked))
        }
        "uart" => {
            let stop_bits = match option(dsview, "num_stop_bits").and_then(Value::as_f64) {
                Some(bits) if bits >= 2.0 => "two",
                Some(bits) if bits >= 1.5 => "one-and-a-half",
                _ => "one",
            };
            let parity = match option_str(dsview, "parity_type") {
                Some("odd") => "odd",
                Some("even") => "even",
                Some("zero") => "space",
                Some("one") => "mark",
                _ => "none",
            };
            let config = json!({
                "line": channel(dsview, "rxtx")?,
                "baud": option_u64(dsview, "baudrate").unwrap_or(115_200),
                "data_bits": option_u64(dsview, "num_data_bits").unwrap_or(8),
                "parity": parity,
                "stop_bits": stop_bits,
                "bit_order": if option_str(dsview, "bit_order") == Some("msb-first") { "msb-first" } else { "lsb-first" },
                "inverted": option_str(dsview, "invert") == Some("yes"),
            });
            Some(entry(label, "uart", "events", config, Vec::new()))
        }
        "spi" => {
            let word_bits = option_u64(dsview, "wordsize").unwrap_or(8);
            if !(1..=8).contains(&word_bits) {
                log::info!("dropping DSView decoder {id}: {word_bits}-bit words");
                return None;
            }
            let mut config = json!({
                "clk": channel(dsview, "clk")?,
                "mode": option_u64(dsview, "cpol").unwrap_or(0) * 2 + option_u64(dsview, "cpha").unwrap_or(0),
                "word_bits": word_bits,
                "bit_order": if option_str(dsview, "bitorder") == Some("lsb-first") { "lsb-first" } else { "msb-first" },
                "cs_active_low": option_str(dsview, "cs_polarity") != Some("active-high"),
            });
            for role in ["cs", "mosi", "miso"] {
                if let Some(index) = channel(dsview, role) {
                    config[role] = json!(index);
                }
            }
            if config.get("mosi").is_none() && config.get("miso").is_none() {
                return None;
            }
            Some(entry(label, "spi", "events", config, Vec::new()))
        }
        "can" | "can-fd" => {
            let fd_bitrate = option_u64(dsview, "fast_bitrate").filter(|&rate| rate > 0);
            let config = json!({
                "rx": channel(dsview, "can_rx")?,
                "bitrate": option_u64(dsview, "nominal_bitrate")
                    .or_else(|| option_u64(dsview, "bitrate"))
                    .unwrap_or(500_000),
                "fd_bitrate": fd_bitrate,
                "sample_point": option_u64(dsview, "sample_point").unwrap_or(70),
                "fd": fd_bitrate.is_some(),
            });
            Some(entry(label, "can", "events", config, Vec::new()))
        }
        other => {
            log::info!("dropping DSView decoder {other}: uiharu has no decoder for it");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::DecoderEntry;

    fn demo_entries() -> Vec<Value> {
        serde_json::from_str(r#"[
            {"channel": [{"sda": 0}, {"scl": 1}], "id": "1:i2c", "label": "1:I²C", "options": {"address_format": "shifted"}, "stacked decoders": [{"id": "eeprom24xx", "options": {"addr_counter": 0, "chip": "generic"}}], "version": 2, "view_index": 0},
            {"channel": [{"rxtx": 5}], "id": "1:uart", "label": "1:UART", "options": {"anno_startstop": "yes", "baudrate": 115200, "bit_order": "lsb-first", "format": "hex", "invert": "no", "num_data_bits": 8, "num_stop_bits": 1, "parity_check": "yes", "parity_type": "none"}, "stacked decoders": [], "version": 2},
            {"channel": [{"mosi": 14}, {"clk": 12}, {"cs": 13}, {"miso": 15}], "id": "1:spi", "label": "1:SPI", "options": {"bitorder": "msb-first", "cpha": 0, "cpol": 0, "cs_polarity": "active-low", "frame": "yes", "wordsize": 8}, "stacked decoders": [], "version": 2},
            {"channel": [{"can_rx": 9}], "id": "can-fd", "label": "CAN-FD", "options": {"fast_bitrate": 2000000, "nominal_bitrate": 500000, "sample_point": 70}, "stacked decoders": [], "version": 2}
        ]"#).expect("entries")
    }

    #[test]
    fn converts_the_demo_captures_decoders() {
        let converted: Vec<Value> = demo_entries().iter().filter_map(convert).collect();
        assert_eq!(converted.len(), 4);
        let entries: Vec<DecoderEntry> = converted
            .iter()
            .map(|value| DecoderEntry::parse(value).expect("an entry"))
            .collect();
        assert_eq!(entries[0].kinds(), ["i2c", "eeprom24xx"]);
        assert_eq!(entries[0].label(), "I²C");
        assert_eq!(
            entries[0].expose.keys().collect::<Vec<_>>(),
            ["i2c", "eeprom24xx"]
        );
        assert_eq!(
            converted[0]["connections"][1],
            json!({ "from": "i2c.events", "to": "eeprom24xx.bus" })
        );
        assert_eq!(
            converted[0]["nodes"]["i2c"]["config"],
            json!({ "scl": 1, "sda": 0 })
        );
        assert_eq!(
            converted[1]["nodes"]["uart"]["config"],
            json!({ "line": 5, "baud": 115200, "data_bits": 8, "parity": "none", "stop_bits": "one", "bit_order": "lsb-first", "inverted": false })
        );
        assert_eq!(
            converted[2]["nodes"]["spi"]["config"],
            json!({ "clk": 12, "cs": 13, "mosi": 14, "miso": 15, "mode": 0, "word_bits": 8, "bit_order": "msb-first", "cs_active_low": true })
        );
        assert_eq!(
            converted[3]["nodes"]["can"]["config"],
            json!({ "rx": 9, "bitrate": 500000, "fd_bitrate": 2000000, "sample_point": 70, "fd": true })
        );
        assert_eq!(entries[3].label(), "CAN-FD");
        for entry in &entries {
            uiharu_registry::build(&entry.description(16, 25_000_000)).expect("builds");
        }
    }

    #[test]
    fn maps_uart_framing_spi_modes_and_eeprom_sizes() {
        let uart = convert(&json!({
            "id": "0:uart", "channel": [{"rxtx": 3}],
            "options": {"baudrate": 9600, "num_data_bits": 7, "parity_type": "zero", "num_stop_bits": 1.5, "bit_order": "msb-first", "invert": "yes"}
        }))
        .expect("uart");
        assert_eq!(
            uart["nodes"]["uart"]["config"],
            json!({ "line": 3, "baud": 9600, "data_bits": 7, "parity": "space", "stop_bits": "one-and-a-half", "bit_order": "msb-first", "inverted": true })
        );
        assert_eq!(uart.get("label"), None);
        let spi = convert(&json!({
            "id": "1:spi", "channel": [{"clk": 0}, {"miso": 1}],
            "options": {"cpol": 1, "cpha": 1, "bitorder": "lsb-first", "cs_polarity": "active-high", "wordsize": 4}
        }))
        .expect("spi");
        assert_eq!(
            spi["nodes"]["spi"]["config"],
            json!({ "clk": 0, "miso": 1, "mode": 3, "word_bits": 4, "bit_order": "lsb-first", "cs_active_low": false })
        );
        assert_eq!(eeprom_address_bytes("generic"), 1);
        assert_eq!(eeprom_address_bytes("24c16"), 1);
        assert_eq!(eeprom_address_bytes("24c32"), 2);
        assert_eq!(eeprom_address_bytes("24c1024"), 2);
        assert_eq!(eeprom_address_bytes("microchip-24aa512"), 2);
    }

    #[test]
    fn leaves_out_what_uiharu_cannot_decode() {
        assert_eq!(
            convert(&json!({ "id": "counter", "channel": [{"data": 1}] })),
            None
        );
        assert_eq!(
            convert(
                &json!({ "id": "1:spi", "channel": [{"clk": 0}, {"mosi": 1}], "options": {"wordsize": 16} })
            ),
            None
        );
        assert_eq!(
            convert(&json!({ "id": "1:i2c", "channel": [{"sda": 0}] })),
            None
        );
        assert_eq!(convert(&json!({ "nodes": {} })), None);
    }
}
