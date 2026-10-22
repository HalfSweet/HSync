//! DSView's decoder entries, as `.dsl` files and `.dsc` sessions carry them, turned into
//! HSync's. Supported buses and device overlays are translated when a file opens. Originals remain
//! alongside the translation, including decoder families/options that cannot run yet.

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
            .or_else(|| {
                value
                    .as_f64()
                    .filter(|number| {
                        number.is_finite()
                            && *number >= 0.0
                            && *number < u64::MAX as f64
                            && number.fract() == 0.0
                    })
                    .map(|number| number as u64)
            })
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    })
}

/// Defaults apply only to omitted fields; malformed supplied values stay unsupported.
fn integer_option(entry: &Value, name: &str, default: u64) -> Option<u64> {
    match option(entry, name) {
        None => Some(default),
        Some(_) => option_u64(entry, name),
    }
}

fn spi_mode(entry: &Value) -> Option<u64> {
    let polarity = integer_option(entry, "cpol", 0)?;
    let phase = integer_option(entry, "cpha", 0)?;
    (polarity <= 1 && phase <= 1).then(|| polarity * 2 + phase)
}

fn option_str<'a>(entry: &'a Value, name: &str) -> Option<&'a str> {
    option(entry, name).and_then(Value::as_str)
}

/// A supplied non-string is never an omitted option.
fn string_option<'a>(entry: &'a Value, name: &str, default: &'a str) -> Option<&'a str> {
    match option(entry, name) {
        None => Some(default),
        Some(value) => value.as_str(),
    }
}

/// Preserve only declared enum choices, using defaults solely for omitted values.
fn enum_option<'a>(
    entry: &'a Value,
    name: &str,
    default: &'a str,
    allowed: &[&str],
) -> Option<&'a str> {
    let value = string_option(entry, name, default)?;
    allowed.contains(&value).then_some(value)
}

/// DSView stores DAC reference voltages in volts; the DSL uses exact integer microvolts.
fn reference_microvolts(entry: &Value, name: &str, default: f64) -> Option<u64> {
    let volts = match option(entry, name) {
        None => default,
        Some(value) => value.as_f64().or_else(|| value.as_str()?.parse().ok())?,
    };
    (volts.is_finite() && (0.0..=10.0).contains(&volts))
        .then(|| (volts * 1_000_000.0).round() as u64)
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
    for (kind, input, device_output, config) in stacked {
        let mut id = kind.to_owned();
        let mut suffix = 2;
        while nodes.contains_key(&id) {
            id = format!("{kind}-{suffix}");
            suffix += 1;
        }
        nodes.insert(id.clone(), json!({ "type": kind, "config": config }));
        connections.push(json!({
            "from": format!("{bus}.{output}"),
            "to": format!("{id}.{input}")
        }));
        expose.insert(id.clone(), json!(format!("{id}.{device_output}")));
    }
    let mut entry = json!({ "nodes": nodes, "connections": connections, "expose": expose });
    if let Some(label) = label.map(base_id).filter(|label| !label.is_empty()) {
        entry["label"] = json!(label);
    }
    entry
}

/// Adds a node to an existing graph without losing repeated instances or intermediate layers.
fn attach(
    entry: &mut Value,
    from: &str,
    kind: &str,
    input: &str,
    output: &str,
    config: Value,
) -> String {
    let nodes = entry["nodes"].as_object_mut().expect("constructed graph");
    let mut id = kind.to_owned();
    let mut suffix = 2;
    while nodes.contains_key(&id) {
        id = format!("{kind}-{suffix}");
        suffix += 1;
    }
    nodes.insert(id.clone(), json!({"type":kind,"config":config}));
    entry["connections"]
        .as_array_mut()
        .expect("constructed graph")
        .push(json!({
            "from":from,"to":format!("{id}.{input}")
        }));
    let endpoint = format!("{id}.{output}");
    entry["expose"][&id] = json!(endpoint);
    endpoint
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

/// Geometry facts from the pinned DSView chip profiles. Keep this import mapping
/// in HSync: the portable decoder has an independent configurable memory model.
fn eeprom_memory_config(layer: &Value) -> Option<Value> {
    let chip = option_str(layer, "chip")?;
    let (capacity, page, bytes, blocks) = match chip {
        "microchip_24aa65" | "microchip_24lc65" | "microchip_24c65" => (8192, 64, 2, 0),
        "microchip_24aa64" | "microchip_24lc64" => (8192, 32, 2, 0),
        "microchip_24aa02uid" | "siemens_slx_24c02" => (256, 8, 1, 0),
        "microchip_24aa025uid" | "microchip_24aa025uid_sot23" | "st_m24c02" => (256, 16, 1, 0),
        "onsemi_cat24c256" => (32768, 64, 2, 0),
        "onsemi_cat24m01" => (131072, 256, 2, 1),
        "siemens_slx_24c01" => (128, 8, 1, 0),
        "st_m24c01" => (128, 16, 1, 0),
        "xicor_x24c02" => (256, 4, 1, 0),
        _ => return None,
    };
    let counter = integer_option(layer, "addr_counter", 0)?;
    if counter >= capacity {
        return None;
    }
    Some(
        json!({"chip":"generic", "capacity":capacity, "page_bytes":page,
        "address_bytes":bytes, "block_bits":blocks, "initial_counter":counter}),
    )
}

/// Only direct I²C overlays whose original device profile can be translated.
/// The source option names are from DSView's pinned decoder declarations; unknown
/// options remain in the original metadata and receive a diagnostic.
fn i2c_layer(layer: &Value) -> Option<(&'static str, &'static str, &'static str, Value)> {
    let id = base_id(layer.get("id")?.as_str()?);
    let (name, output, config) = match id {
        "eeprom24xx" => {
            if string_option(layer, "chip", "generic")? == "generic" {
                // Preserve the established generic import and its saved stream identity.
                (
                    "eeprom24xx",
                    "operations",
                    json!({"address_bytes":eeprom_address_bytes("generic")}),
                )
            } else {
                ("eeprom24xx-memory", "events", eeprom_memory_config(layer)?)
            }
        }
        "ds1307" => ("ds1307", "events", json!({})),
        "rtc8564" => ("rtc8564", "events", json!({})),
        "lm75" => {
            if string_option(layer, "sensor", "lm75")? != "lm75" {
                return None;
            }
            let resolution = integer_option(layer, "resolution", 9)?;
            if !(9..=12).contains(&resolution) {
                return None;
            }
            ("lm75", "events", json!({ "resolution": resolution }))
        }
        "mlx90614" => ("mlx90614", "events", json!({})),
        "pca9571" => ("pca9571", "events", json!({})),
        "tca6408a" => ("tca6408a", "events", json!({})),
        "edid" => ("edid", "events", json!({})),
        "atsha204a" => ("atsha204a", "events", json!({})),
        "mxc6225xu" => ("mxc6225xu", "events", json!({})),
        "nunchuk" => ("nunchuk", "events", json!({})),
        "xfp" => ("xfp", "events", json!({})),
        "ltc26x7" => {
            let model = string_option(layer, "chip", "ltc2607")?;
            if !["ltc2607", "ltc2617", "ltc2627"].contains(&model) {
                return None;
            }
            let reference = match option(layer, "vref") {
                None => 1.5,
                Some(value) => value.as_f64().or_else(|| value.as_str()?.parse().ok())?,
            };
            if !reference.is_finite() || !(0.000_001..=5.5).contains(&reference) {
                return None;
            }
            (
                "ltc26x7",
                "events",
                json!({
                    "model": model,
                    "reference_microvolts": (reference * 1_000_000.0).round() as u64
                }),
            )
        }
        _ => return None,
    };
    Some((name, "bus", output, config))
}

/// The byte-oriented SPI overlays that share the source's framing. Non-byte
/// devices need a separately configured bit stream and are retained for review.
fn spi_layer(layer: &Value) -> Option<(&'static str, &'static str, &'static str, Value)> {
    let (name, config) = match base_id(layer.get("id")?.as_str()?) {
        "a7105" => ("a7105", json!({})),
        "adns5020" => ("adns5020", json!({})),
        "adxl345" => ("adxl345-spi", json!({})),
        "cc1101" => ("cc1101", json!({})),
        "enc28j60" => ("enc28j60", json!({})),
        "max7219" => ("max7219", json!({})),
        "max72xx" => ("max72xx", json!({})),
        "mrf24j40" => ("mrf24j40", json!({})),
        "nrf24l01" => match string_option(layer, "chip", "nrf24l01")? {
            "nrf24l01" => ("nrf24l01", json!({})),
            // The reference's alternate map is largely XN297L, despite its
            // `xn297` name. HSync follows the identified manufacturer's map.
            "xn297" => ("xn297", json!({"model":"xn297l"})),
            _ => return None,
        },
        "nrf905" => ("nrf905", json!({})),
        "rfm12" => ("rfm12", json!({})),
        "spiflash" => ("spi-nor", json!({"address_bytes":"auto"})),
        "sdcard_spi" => ("sdcard-spi", json!({})),
        "avr_isp" => ("avr-isp", json!({})),
        "st25r39xx_spi" => ("st25r39xx-spi", json!({})),
        "spi_tpm" => {
            let version = match string_option(layer, "tpm_version", "2.0")? {
                "1.2" => "tpm12",
                "2.0" => "tpm20",
                _ => return None,
            };
            ("spi-tpm", json!({"tpm_version": version}))
        }
        _ => return None,
    };
    Some((name, "bus", "events", config))
}

/// These reference decoders read both sides of a JTAG scan. Keep a partial
/// capture runnable as JTAG alone rather than wiring an invalid semantic layer.
fn jtag_layer(layer: &Value) -> Option<(&'static str, &'static str, &'static str, Value)> {
    let name = match base_id(layer.get("id")?.as_str()?) {
        "jtag_stm32" => "jtag-stm32",
        "jtag_ejtag" => "jtag-ejtag",
        _ => return None,
    };
    Some((name, "bus", "events", json!({})))
}

fn layers(dsview: &Value) -> impl Iterator<Item = &Value> {
    fn collect<'a>(entry: &'a Value, result: &mut Vec<&'a Value>) {
        if let Some(children) = entry.get("stacked decoders").and_then(Value::as_array) {
            for child in children {
                result.push(child);
                // Only a translated transparent network layer may introduce nested children.
                // An unknown parent's children must not be reattached to a different bus.
                if child.get("id").and_then(Value::as_str).map(base_id) == Some("onewire_network") {
                    collect(child, result);
                }
            }
        }
    }
    let mut result = Vec::new();
    collect(dsview, &mut result);
    result.into_iter()
}

fn uart_layer(
    dsview: &Value,
    layer: &Value,
) -> Option<(&'static str, &'static str, &'static str, Value)> {
    if integer_option(dsview, "num_data_bits", 8)? != 8
        || option_str(dsview, "bit_order").is_some_and(|order| order != "lsb-first")
    {
        return None;
    }
    let name = base_id(layer.get("id")?.as_str()?);
    match name {
        "pan1321" => Some(("pan1321", "bus", "events", json!({}))),
        "midi"
            if integer_option(dsview, "baudrate", 115_200)? == 31_250
                && option_str(dsview, "parity_type").unwrap_or("none") == "none"
                && integer_option(dsview, "num_stop_bits", 1)? == 1 =>
        {
            Some(("midi", "uart", "events", json!({})))
        }
        "lin"
            if option_str(dsview, "parity_type").unwrap_or("none") == "none"
                && integer_option(dsview, "num_stop_bits", 1)? == 1 =>
        {
            let version = integer_option(layer, "version", 2)?;
            if ![1, 2].contains(&version) {
                return None;
            }
            Some((
                "lin-profile",
                "bus",
                "events",
                json!({
                    "baud":integer_option(dsview,"baudrate",115_200)?,"version":version.to_string()
                }),
            ))
        }
        _ => None,
    }
}

fn onewire_layer(layer: &Value) -> Option<(&'static str, Value)> {
    match base_id(layer.get("id")?.as_str()?) {
        "ds2408" => Some(("ds2408", json!({}))),
        "ds243x" => Some(("ds243x", json!({"model":"ds2432"}))),
        "ds28ea00" => Some(("ds28ea00", json!({}))),
        _ => None,
    }
}

fn microwire_layer(layer: &Value) -> Option<(&'static str, &'static str, &'static str, Value)> {
    if base_id(layer.get("id")?.as_str()?) != "eeprom93xx" {
        return None;
    }
    let address_bits = integer_option(layer, "addresssize", 8)?;
    if !(2..=16).contains(&address_bits) {
        return None;
    }
    let organisation = match integer_option(layer, "wordsize", 16)? {
        8 => "x8",
        16 => "x16",
        _ => return None,
    };
    Some((
        "eeprom93xx",
        "microwire",
        "events",
        json!({"address_bits":address_bits,"organisation":organisation}),
    ))
}

/// Reference SPI decoders receive a separate bit stream even for byte framing.
/// HSync expresses that with a parallel one-bit transport over the same channels.
fn spi_bit_layer(dsview: &Value, layer: &Value) -> Option<(&'static str, Value)> {
    if channel(dsview, "cs").is_none() || option_str(dsview, "bitorder") == Some("lsb-first") {
        return None;
    }
    let (name, config, mosi, miso) = match base_id(layer.get("id")?.as_str()?) {
        "ad5626" => ("ad5626", json!({}), true, false),
        "adf435x" => ("adf435x", json!({}), true, false),
        "ad79x0" => (
            "ad79x0",
            json!({"reference_microvolts":reference_microvolts(layer,"vref",1.5)?}),
            false,
            true,
        ),
        "ltc242x" => (
            "ltc242x",
            json!({"reference_microvolts":reference_microvolts(layer,"vref",1.5)?}),
            false,
            true,
        ),
        "x2444m" => ("x2444m", json!({}), true, true),
        _ => return None,
    };
    if (mosi && channel(dsview, "mosi").is_none()) || (miso && channel(dsview, "miso").is_none()) {
        return None;
    }
    Some((name, config))
}

/// Use only an actual connected CS and MSB-first byte frames for the byte profiles.
fn compatible_spi_layers(dsview: &Value, require_miso: bool) -> bool {
    integer_option(dsview, "wordsize", 8) == Some(8)
        && option_str(dsview, "bitorder") != Some("lsb-first")
        && channel(dsview, "cs").is_some()
        && channel(dsview, "mosi").is_some()
        && (!require_miso || channel(dsview, "miso").is_some())
}

/// Continuous byte profiles do not require a captured chip-select signal.
fn spi_layer_for(
    dsview: &Value,
    layer: &Value,
) -> Option<(&'static str, &'static str, &'static str, Value)> {
    let id = base_id(layer.get("id")?.as_str()?);
    if ["rgb_led_spi", "ade77xx"].contains(&id) {
        let mode = spi_mode(dsview)?;
        if integer_option(dsview, "wordsize", 8)? != 8
            || option_str(dsview, "bitorder") == Some("lsb-first")
            || ![0, 3].contains(&mode)
            || channel(dsview, "mosi").is_none()
            || (id == "ade77xx" && channel(dsview, "miso").is_none())
        {
            return None;
        }
        return Some(if id == "ade77xx" {
            (
                "ade77xx",
                "bus",
                "events",
                json!({"model":"ade7758", "framed":false}),
            )
        } else {
            (
                "rgb-led-spi",
                "bus",
                "events",
                json!({"order":"rgb", "framed":false}),
            )
        });
    }
    let translated = spi_layer(layer)?;
    // Command-only consumers can decode a captured host line without a slave line.
    // Keep full-duplex requirements for consumers whose generated contract needs both.
    let require_miso = !matches!(translated.0, "max7219" | "max72xx" | "adns5020");
    compatible_spi_layers(dsview, require_miso).then_some(translated)
}

/// Turns one of DSView's entries into HSync's, or `None` when uiharu has no decoder for it.
fn convert_nodes(dsview: &Value) -> Option<Value> {
    let id = dsview.get("id")?.as_str()?;
    let label = dsview.get("label").and_then(Value::as_str);
    match base_id(id) {
        "i2c" => {
            enum_option(
                dsview,
                "address_format",
                "unshifted",
                &["shifted", "unshifted"],
            )?;
            let config = json!({ "scl": channel(dsview, "scl")?, "sda": channel(dsview, "sda")? });
            let stacked = layers(dsview).filter_map(i2c_layer).collect();
            Some(entry(label, "i2c", "events", config, stacked))
        }
        "uart" => {
            enum_option(dsview, "parity_check", "yes", &["yes", "no"])?;
            enum_option(dsview, "anno_startstop", "no", &["yes", "no"])?;
            let stop = match option(dsview, "num_stop_bits") {
                None => 1.0,
                Some(value) => value.as_f64().or_else(|| value.as_str()?.parse().ok())?,
            };
            let stop_bits = match stop {
                1.0 => "one",
                1.5 => "one-and-a-half",
                2.0 => "two",
                _ => return None,
            };
            let parity = match string_option(dsview, "parity_type", "none")? {
                "odd" => "odd",
                "even" => "even",
                "zero" => "space",
                "one" => "mark",
                "none" => "none",
                _ => return None,
            };
            let bits = integer_option(dsview, "num_data_bits", 8)?;
            if !(5..=9).contains(&bits) {
                return None;
            }
            let config = json!({
                "line": channel(dsview, "rxtx")?,
                "baud": integer_option(dsview, "baudrate", 115_200)?,
                "data_bits": bits,
                "parity": parity,
                "stop_bits": stop_bits,
                "bit_order": enum_option(dsview, "bit_order", "lsb-first", &["lsb-first", "msb-first"] )?,
                "inverted": enum_option(dsview, "invert", "no", &["yes", "no"])? == "yes",
            });
            let mut graph = entry(label, "uart", "events", config, Vec::new());
            for (kind, input, output, config) in
                layers(dsview).filter_map(|layer| uart_layer(dsview, layer))
            {
                let endpoint = attach(&mut graph, "uart.events", kind, input, output, config);
                if kind == "midi" {
                    attach(
                        &mut graph,
                        &endpoint,
                        "midi-semantics",
                        "bus",
                        "events",
                        json!({}),
                    );
                }
            }
            Some(graph)
        }
        "spi" => {
            enum_option(dsview, "frame", "no", &["yes", "no"])?;
            let word_bits = integer_option(dsview, "wordsize", 8)?;
            if !(1..=8).contains(&word_bits) {
                return None;
            }
            let mut config = json!({
                "clk": channel(dsview, "clk")?,
                "mode": spi_mode(dsview)?,
                "word_bits": word_bits,
                "bit_order": enum_option(dsview, "bitorder", "msb-first", &["msb-first", "lsb-first"] )?,
                "cs_active_low": enum_option(dsview, "cs_polarity", "active-low", &["active-low", "active-high"] )? == "active-low",
            });
            for role in ["cs", "mosi", "miso"] {
                // Omitted optional channels must not inherit the decoder's defaults.
                config[role] = json!(channel(dsview, role));
            }
            if config["mosi"].is_null() && config["miso"].is_null() {
                return None;
            }
            let mut graph = entry(label, "spi", "events", config, Vec::new());
            for (kind, input, output, config) in
                layers(dsview).filter_map(|layer| spi_layer_for(dsview, layer))
            {
                let endpoint = attach(&mut graph, "spi.events", kind, input, output, config);
                if kind == "avr-isp" {
                    attach(
                        &mut graph,
                        &endpoint,
                        "avr-isp-device",
                        "bus",
                        "events",
                        json!({}),
                    );
                } else if kind == "sdcard-spi" {
                    attach(
                        &mut graph,
                        "spi.events",
                        "sd-registers",
                        "bus",
                        "events",
                        json!({}),
                    );
                }
            }
            let bit_layers: Vec<_> = layers(dsview)
                .filter_map(|layer| spi_bit_layer(dsview, layer))
                .collect();
            if !bit_layers.is_empty() {
                let mut bit_config = graph["nodes"]["spi"]["config"].clone();
                bit_config["word_bits"] = json!(1);
                let bits = attach(&mut graph, "source", "spi", "logic", "events", bit_config);
                for (kind, config) in bit_layers {
                    attach(&mut graph, &bits, kind, "bus", "events", config);
                }
            }
            Some(graph)
        }
        "qspi" => {
            let data: Vec<u64> = ["io0", "io1", "io2", "io3"]
                .iter()
                .map_while(|role| channel(dsview, role))
                .collect();
            if ![1, 2, 4].contains(&data.len()) {
                return None;
            }
            // A disconnected intermediate lane must not shift the remaining lanes.
            if ["io0", "io1", "io2", "io3"]
                .iter()
                .skip(data.len())
                .any(|role| channel(dsview, role).is_some())
            {
                return None;
            }
            Some(entry(
                label,
                "qspi",
                "events",
                json!({
                    "clk": channel(dsview, "clk")?, "cs": channel(dsview, "cs"), "data": data,
                    "mode": spi_mode(dsview)?,
                    "word_bits": integer_option(dsview, "wordsize", 8)?,
                    "cs_active_low": enum_option(dsview, "cs_polarity", "active-low", &["active-low", "active-high"] )? == "active-low",
                    "lsb_first": enum_option(dsview, "bitorder", "msb-first", &["msb-first", "lsb-first"] )? == "lsb-first",
                }),
                Vec::new(),
            ))
        }
        "sdcard_sd" => {
            let four = ["dat0", "dat1", "dat2", "dat3"]
                .iter()
                .all(|role| channel(dsview, role).is_some());
            if !four
                && ["dat1", "dat2", "dat3"]
                    .iter()
                    .any(|role| channel(dsview, role).is_some())
            {
                return None;
            }
            Some(entry(
                label,
                "sd-native",
                "events",
                json!({
                    "clk": channel(dsview, "clk")?, "cmd": channel(dsview, "cmd")?,
                    "dat0": channel(dsview, "dat0"), "dat1": channel(dsview, "dat1"),
                    "dat2": channel(dsview, "dat2"), "dat3": channel(dsview, "dat3"),
                    "width": if four { "four" } else { "one" },
                }),
                Vec::new(),
            ))
        }
        "jtag" => Some(entry(
            label,
            "jtag",
            "events",
            json!({
                "tck": channel(dsview, "tck")?, "tms": channel(dsview, "tms")?,
                "tdi": channel(dsview, "tdi"), "tdo": channel(dsview, "tdo"),
                "trst": channel(dsview, "trst"),
            }),
            layers(dsview)
                .filter(|_| channel(dsview, "tdi").is_some() && channel(dsview, "tdo").is_some())
                .filter_map(jtag_layer)
                .collect(),
        )),
        "cjtag" => Some(entry(
            label,
            "cjtag",
            "events",
            json!({
                "clock":channel(dsview,"tckc")?,"data":channel(dsview,"tmsc")?
            }),
            Vec::new(),
        )),
        "swd" => {
            let strict = match string_option(dsview, "strict_start", "no")? {
                "yes" => true,
                "no" => false,
                _ => return None,
            };
            Some(entry(
                label,
                "swd",
                "events",
                json!({
                    "swclk":channel(dsview,"swclk")?,"swdio":channel(dsview,"swdio")?,
                    "strict_start":strict
                }),
                vec![("swd-dp", "bus", "events", json!({}))],
            ))
        }
        "mdio" => {
            let edge = string_option(dsview, "read_edge", "falling")?;
            if !["rising", "falling"].contains(&edge) {
                return None;
            }
            Some(entry(
                label,
                "mdio",
                "events",
                json!({
                    "mdc":channel(dsview,"mdc")?,"mdio":channel(dsview,"mdio")?,"readEdge":edge
                }),
                layers(dsview)
                    .filter(|layer| {
                        layer.get("id").and_then(Value::as_str).map(base_id) == Some("cfp")
                    })
                    .map(|_| ("cfp", "bus", "events", json!({})))
                    .collect(),
            ))
        }
        "onewire_link" => {
            let speed = match string_option(dsview, "overdrive", "no")? {
                "yes" => "overdrive",
                "no" => "standard",
                _ => return None,
            };
            let mut graph = entry(
                label,
                "onewire-link",
                "events",
                json!({
                    "data":channel(dsview,"owr")?,"speed":speed
                }),
                Vec::new(),
            );
            if layers(dsview).any(|layer| {
                base_id(layer.get("id").and_then(Value::as_str).unwrap_or_default())
                    == "onewire_network"
            }) {
                let network = attach(
                    &mut graph,
                    "onewire-link.events",
                    "onewire",
                    "link",
                    "events",
                    json!({}),
                );
                for (kind, config) in layers(dsview).filter_map(onewire_layer) {
                    attach(&mut graph, &network, kind, "bus", "events", config);
                }
            }
            Some(graph)
        }
        "ps2" => {
            if string_option(dsview, "HtoD_Clock", "rise")? != "rise"
                || string_option(dsview, "DtoH_Clock", "fall")? != "fall"
            {
                return None;
            }
            Some(entry(
                label,
                "ps2",
                "events",
                json!({
                    "clk":channel(dsview,"clk")?,"data":channel(dsview,"data")?
                }),
                Vec::new(),
            ))
        }
        "am230x" => {
            let model = match string_option(dsview, "device", "am230x")? {
                "am230x" | "am230x/rht" => "am2302",
                "dht11" => "dht11",
                _ => return None,
            };
            Some(entry(
                label,
                "am230x",
                "events",
                json!({"data":channel(dsview,"sda")?,"model":model}),
                vec![("am230x-values", "bus", "events", json!({"model":model}))],
            ))
        }
        "usb_power_delivery" => {
            enum_option(dsview, "fulltext", "no", &["yes", "no"])?;
            let mut graph = entry(
                label,
                "usb-pd",
                "events",
                json!({"cc":channel(dsview,"cc1")?}),
                Vec::new(),
            );
            if let Some(cc) = channel(dsview, "cc2") {
                attach(
                    &mut graph,
                    "source",
                    "usb-pd",
                    "logic",
                    "events",
                    json!({"cc":cc}),
                );
            }
            Some(graph)
        }
        "usb_signalling" => {
            let speed = match string_option(dsview, "signalling", "automatic")? {
                "full-speed" => "full",
                "low-speed" => "low",
                _ => return None,
            };
            let stacked = layers(dsview)
                .filter(|layer| {
                    layer.get("id").and_then(Value::as_str).map(base_id) == Some("usb_packet")
                })
                .map(|_| ("usb-transactions", "packets", "transactions", json!({})))
                .collect();
            Some(entry(
                label,
                "usb",
                "packets",
                json!({
                    "dp":channel(dsview,"dp")?,"dm":channel(dsview,"dm")?,"speed":speed
                }),
                stacked,
            ))
        }
        "dmx512" => Some(entry(
            label,
            "uart",
            "events",
            json!({
                "line":channel(dsview,"dmx")?,"baud":250000,"data_bits":8,"parity":"none",
                "stop_bits":"two","bit_order":"lsb-first","inverted":enum_option(dsview,"invert","no",&["yes","no"])? == "yes"
            }),
            vec![("dmx512-analysis", "bus", "events", json!({}))],
        )),
        "microwire" => Some(entry(
            label,
            "microwire",
            "events",
            json!({
                "cs":channel(dsview,"cs")?,"sk":channel(dsview,"sk")?,
                "di":channel(dsview,"si")?,"do":channel(dsview,"so"),"cs_active_low":false
            }),
            layers(dsview).filter_map(microwire_layer).collect(),
        )),
        "swim" => Some(entry(
            label,
            "swim-stm8",
            "events",
            json!({"data": channel(dsview, "swim")?, "calibrate_sync": false}),
            Vec::new(),
        )),
        "c2" => Some(entry(
            label,
            "c2",
            "events",
            json!({
                "clock": channel(dsview, "c2ck")?, "data": channel(dsview, "c2d")?,
            }),
            Vec::new(),
        )),
        "avr_pdi" => Some(entry(
            label,
            "avr-pdi",
            "events",
            json!({
                "clock": channel(dsview, "reset")?, "data": channel(dsview, "data")?,
            }),
            Vec::new(),
        )),
        "fsi" => Some(entry(
            label,
            "fsi",
            "events",
            json!({
                "clock": channel(dsview, "clock")?, "data": channel(dsview, "data")?,
            }),
            Vec::new(),
        )),
        "aud" => Some(entry(
            label,
            "aud",
            "events",
            json!({
                "clock": channel(dsview, "audck")?, "sync": channel(dsview, "naudsync")?,
                "data0": channel(dsview, "audata0")?, "data1": channel(dsview, "audata1")?,
                "data2": channel(dsview, "audata2")?, "data3": channel(dsview, "audata3")?,
            }),
            Vec::new(),
        )),
        "sda2506" => Some(entry(
            label,
            "sda2506",
            "events",
            json!({
                "clk": channel(dsview, "clk")?, "data": channel(dsview, "d")?, "ce": channel(dsview, "ce")?,
            }),
            Vec::new(),
        )),
        "st7735" => Some(entry(
            label,
            "st7735",
            "events",
            json!({
                "clk": channel(dsview, "clk")?, "data": channel(dsview, "mosi")?,
                "cs": channel(dsview, "cs")?, "dc": channel(dsview, "dc")?,
            }),
            Vec::new(),
        )),
        "tlc5620" => Some(entry(
            label,
            "tlc5620",
            "events",
            json!({
                "clk": channel(dsview, "clk")?, "data": channel(dsview, "data")?,
                "load": channel(dsview, "load"), "ldac": channel(dsview, "ldac"),
                "word_aligned": channel(dsview, "load").is_none(),
                "vref_a_microvolts": reference_microvolts(dsview, "vref_a", 3.3)?,
                "vref_b_microvolts": reference_microvolts(dsview, "vref_b", 3.3)?,
                "vref_c_microvolts": reference_microvolts(dsview, "vref_c", 3.3)?,
                "vref_d_microvolts": reference_microvolts(dsview, "vref_d", 3.3)?,
            }),
            Vec::new(),
        )),
        // The reference calls its clock channel `dcx` and its D/C channel `wrx`.
        "st7789" => Some(entry(
            label,
            "st7789",
            "events",
            json!({
                "clk": channel(dsview, "dcx")?, "data": channel(dsview, "sdo")?,
                "cs": channel(dsview, "csx")?, "dc": channel(dsview, "wrx")?,
            }),
            Vec::new(),
        )),
        "can" | "can-fd" => {
            let fast = integer_option(dsview, "fast_bitrate", 0)?;
            let fd_bitrate = (fast > 0).then_some(fast);
            let bitrate = if option(dsview, "nominal_bitrate").is_some() {
                integer_option(dsview, "nominal_bitrate", 500_000)?
            } else {
                integer_option(dsview, "bitrate", 500_000)?
            };
            let config = json!({
                "rx": channel(dsview, "can_rx")?,
                "bitrate": bitrate,
                "fd_bitrate": fd_bitrate,
                "sample_point": integer_option(dsview, "sample_point", 70)?,
                "fd": fd_bitrate.is_some(),
            });
            Some(entry(label, "can", "events", config, Vec::new()))
        }
        _ => None,
    }
}

/// Numeric display options actually declared by the pinned DSView decoders.
/// Other imported families do not acquire an option merely because HSync has a global one.
fn number_formats(id: &str) -> &'static [&'static str] {
    match base_id(id) {
        "uart" => &["hex", "dec", "oct", "bin", "ascii"],
        "spiflash" | "eeprom93xx" => &["hex", "ascii"],
        _ => &[],
    }
}

fn supported_number_format(entry: &Value) -> Option<&str> {
    let format = option_str(entry, "format")?;
    let id = entry.get("id")?.as_str()?;
    number_formats(id).contains(&format).then_some(format)
}

/// Translates the bus and overlays, retaining a faithful entry-wide numeric display choice.
/// Layer-specific choices remain import metadata unless the entry-wide format already agrees.
pub fn convert(dsview: &Value) -> Option<Value> {
    let mut entry = convert_nodes(dsview)?;
    if base_id(dsview.get("id")?.as_str()?) == "uart" && option(dsview, "format").is_some() {
        entry["show"] = json!({"numbers": supported_number_format(dsview)?});
    }
    Some(entry)
}

/// Preserves one original import alongside its runnable translation and localized diagnostics.
/// Unknown entries are inert metadata; their raw options and stacked entries remain lossless.
pub fn preserve(raw: &Value) -> Value {
    let id = raw.get("id").and_then(Value::as_str).unwrap_or_default();
    let translated = convert(raw);
    let runnable = translated.is_some();
    let mut entry = translated.unwrap_or_else(|| json!({
        "nodes": {},
        "label": raw.get("label").and_then(Value::as_str).filter(|label| !label.is_empty()).unwrap_or(id)
    }));
    let mut diagnostics = Vec::new();
    let mut number_format_notes = Vec::new();
    if let Some(format) = option(raw, "format") {
        let applied = runnable
            && supported_number_format(raw).is_some()
            && entry["show"]["numbers"] == *format;
        number_format_notes
            .push(json!({"decoder":id,"scope":"entry","value":format,"applied":applied}));
    }
    if !runnable {
        diagnostics
            .push(json!({"code":"decode.import.unsupportedDecoder", "params":{"decoder":id}}));
        if base_id(id) == "uart"
            && option(raw, "format").is_some()
            && supported_number_format(raw).is_none()
        {
            diagnostics.push(json!({"code":"decode.import.unsupportedOptions","params":{"decoder":id,"options":"format"}}));
        }
    } else {
        let recognized: &[&str] = match base_id(id) {
            "i2c" => &["address_format"],
            "uart" => &[
                "baudrate",
                "num_data_bits",
                "num_stop_bits",
                "parity_type",
                "bit_order",
                "invert",
                "anno_startstop",
                "format",
            ],
            "spi" | "qspi" => &[
                "wordsize",
                "cpol",
                "cpha",
                "bitorder",
                "cs_polarity",
                "frame",
            ],
            "can" | "can-fd" => &["fast_bitrate", "nominal_bitrate", "bitrate", "sample_point"],
            "tlc5620" => &["vref_a", "vref_b", "vref_c", "vref_d"],
            "swd" => &["strict_start"],
            "mdio" => &["read_edge"],
            "onewire_link" => &["overdrive"],
            "ps2" => &["HtoD_Clock", "DtoH_Clock"],
            "am230x" => &["device"],
            "usb_power_delivery" => &["fulltext"],
            "usb_signalling" => &["signalling"],
            "dmx512" => &["invert"],
            _ => &[],
        };
        if [
            "qspi",
            "sdcard_sd",
            "jtag",
            "swim",
            "c2",
            "avr_pdi",
            "fsi",
            "aud",
            "sda2506",
            "st7735",
            "st7789",
            "tlc5620",
            "cjtag",
            "swd",
            "mdio",
            "onewire_link",
            "ps2",
            "am230x",
            "usb_power_delivery",
            "usb_signalling",
            "dmx512",
            "microwire",
        ]
        .contains(&base_id(id))
        {
            diagnostics
                .push(json!({"code":"decode.import.deviceProfile", "params":{"decoder":id}}));
        }
        if base_id(id) == "jtag" {
            let extra: Vec<&str> = ["srst", "rtck"]
                .into_iter()
                .filter(|role| channel(raw, role).is_some())
                .collect();
            if !extra.is_empty() {
                diagnostics.push(json!({"code":"decode.import.unsupportedChannels", "params":{"decoder":id,"channels":extra.join(", ")}}));
            }
        }
        if let Some(options) = raw.get("options").and_then(Value::as_object) {
            let unmapped: Vec<&str> = options
                .keys()
                .map(String::as_str)
                .filter(|name| {
                    !recognized.contains(name)
                        && !(*name == "parity_check" && option_str(raw, name) == Some("yes"))
                })
                .collect();
            if !unmapped.is_empty() {
                diagnostics.push(json!({"code":"decode.import.unsupportedOptions", "params":{"decoder":id,"options":unmapped.join(", ")}}));
            }
        }
        {
            for (layer_index, layer) in layers(raw).enumerate() {
                let layer_id = layer.get("id").and_then(Value::as_str).unwrap_or_default();
                let mapped = match base_id(id) {
                    "i2c" => i2c_layer(layer).is_some(),
                    "spi" => {
                        spi_layer_for(raw, layer).is_some() || spi_bit_layer(raw, layer).is_some()
                    }
                    "uart" => uart_layer(raw, layer).is_some(),
                    "mdio" => base_id(layer_id) == "cfp",
                    "onewire_link" => {
                        base_id(layer_id) == "onewire_network"
                            || (onewire_layer(layer).is_some()
                                && entry["nodes"]["onewire"].is_object())
                    }
                    "usb_signalling" => base_id(layer_id) == "usb_packet",
                    "microwire" => microwire_layer(layer).is_some(),
                    "jtag" => {
                        channel(raw, "tdi").is_some()
                            && channel(raw, "tdo").is_some()
                            && jtag_layer(layer).is_some()
                    }
                    _ => false,
                };
                let effective_format = entry["show"]["numbers"].as_str().unwrap_or("hex");
                let format_applied =
                    mapped && supported_number_format(layer) == Some(effective_format);
                if let Some(format) = option(layer, "format") {
                    number_format_notes.push(json!({"decoder":layer_id,"scope":"layer","layer":layer_index,"value":format,"applied":format_applied}));
                }
                if mapped {
                    let recognized: &[&str] = match base_id(layer_id) {
                        "eeprom24xx" if eeprom_memory_config(layer).is_some() => {
                            &["chip", "addr_counter"]
                        }
                        "eeprom24xx" => &["chip"],
                        "lm75" => &["sensor", "resolution"],
                        "ltc26x7" => &["chip", "vref"],
                        "spi_tpm" => &["tpm_version"],
                        "nrf24l01" => &["chip"],
                        "lin" => &["version"],
                        "ad79x0" | "ltc242x" => &["vref"],
                        "eeprom93xx" => &["addresssize", "wordsize"],
                        _ => &[],
                    };
                    if base_id(layer_id) != "eeprom24xx" || eeprom_memory_config(layer).is_some() {
                        // Target addresses, chip variants and transaction assumptions
                        // may be narrower than the source; retain a visible profile note.
                        diagnostics.push(json!({"code":"decode.import.deviceProfile", "params":{"decoder":layer_id}}));
                    }
                    if base_id(layer_id) == "nrf24l01" && option_str(layer, "chip") == Some("xn297")
                    {
                        diagnostics.push(json!({"code":"decode.import.xn297Profile", "params":{"decoder":layer_id}}));
                    }
                    if let Some(options) = layer.get("options").and_then(Value::as_object) {
                        let unmapped: Vec<&str> = options
                            .keys()
                            .map(String::as_str)
                            .filter(|name| {
                                !recognized.contains(name) && !(*name == "format" && format_applied)
                            })
                            .collect();
                        if !unmapped.is_empty() {
                            diagnostics.push(json!({"code":"decode.import.unsupportedOptions", "params":{"decoder":layer_id,"options":unmapped.join(", ")}}));
                        }
                    }
                } else {
                    diagnostics.push(json!({"code":"decode.import.unsupportedStack", "params":{"decoder":id,"stack":layer_id}}));
                }
            }
        }
    }
    entry["import"] =
        json!({"format":"dsview","raw":raw,"runnable":runnable,"diagnostics":diagnostics});
    if !number_format_notes.is_empty() {
        entry["import"]["numberFormats"] = json!(number_format_notes);
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::DecoderEntry;

    #[test]
    fn imports_completed_bus_and_device_chains_as_valid_graphs() {
        for (raw, kinds) in [
            (
                json!({"id":"swd","channel":[{"swclk":0},{"swdio":1}]}),
                vec!["swd", "swd-dp"],
            ),
            (
                json!({"id":"cjtag","channel":[{"tckc":0},{"tmsc":1}]}),
                vec!["cjtag"],
            ),
            (
                json!({"id":"mdio","channel":[{"mdc":0},{"mdio":1}],"stacked decoders":[{"id":"cfp"}]}),
                vec!["mdio", "cfp"],
            ),
            (
                json!({"id":"onewire_link","channel":[{"owr":0}],"stacked decoders":[{"id":"onewire_network","stacked decoders":[{"id":"ds2408"},{"id":"ds243x"},{"id":"ds28ea00"}]}]}),
                vec!["onewire-link", "onewire", "ds2408", "ds243x", "ds28ea00"],
            ),
            (
                json!({"id":"am230x","channel":[{"sda":0}],"options":{"device":"dht11"}}),
                vec!["am230x", "am230x-values"],
            ),
            (
                json!({"id":"uart","channel":[{"rxtx":0}],"options":{"baudrate":31250},"stacked decoders":[{"id":"midi"}]}),
                vec!["uart", "midi", "midi-semantics"],
            ),
            (
                json!({"id":"uart","channel":[{"rxtx":0}],"options":{"baudrate":19200},"stacked decoders":[{"id":"lin","options":{"version":1}}]}),
                vec!["uart", "lin-profile"],
            ),
            (
                json!({"id":"dmx512","channel":[{"dmx":0}],"options":{"invert":"yes"}}),
                vec!["uart", "dmx512-analysis"],
            ),
            (
                json!({"id":"usb_power_delivery","channel":[{"cc1":0},{"cc2":1}]}),
                vec!["usb-pd", "usb-pd"],
            ),
            (
                json!({"id":"usb_signalling","channel":[{"dp":0},{"dm":1}],"options":{"signalling":"full-speed"},"stacked decoders":[{"id":"usb_packet"}]}),
                vec!["usb", "usb-transactions"],
            ),
            (
                json!({"id":"microwire","channel":[{"cs":0},{"sk":1},{"si":2},{"so":3}],"stacked decoders":[{"id":"eeprom93xx","options":{"addresssize":7,"wordsize":8}}]}),
                vec!["microwire", "eeprom93xx"],
            ),
            (
                json!({"id":"spi","channel":[{"clk":0},{"cs":1},{"mosi":2},{"miso":3}],"stacked decoders":[{"id":"avr_isp"},{"id":"sdcard_spi"},{"id":"spiflash"}]}),
                vec![
                    "spi",
                    "avr-isp",
                    "avr-isp-device",
                    "sdcard-spi",
                    "sd-registers",
                    "spi-nor",
                ],
            ),
            (
                json!({"id":"spi","channel":[{"clk":0},{"cs":1},{"mosi":2},{"miso":3}],"stacked decoders":[{"id":"ad5626"},{"id":"ad79x0"},{"id":"adf435x"},{"id":"ltc242x"},{"id":"x2444m"}]}),
                vec![
                    "spi", "spi", "ad5626", "ad79x0", "adf435x", "ltc242x", "x2444m",
                ],
            ),
        ] {
            let kept = preserve(&raw);
            assert_eq!(kept["import"]["raw"], raw);
            assert_eq!(kept["import"]["runnable"], true, "{raw}");
            let parsed = DecoderEntry::parse(&kept).expect("valid imported entry");
            let mut actual = parsed.kinds();
            actual.sort();
            let mut expected = kinds;
            expected.sort();
            assert_eq!(actual, expected, "{raw}");
            assert!(
                !kept["import"]["diagnostics"]
                    .as_array()
                    .expect("notes")
                    .iter()
                    .any(|d| d["code"] == "decode.import.unsupportedStack"),
                "{kept}"
            );
            uiharu_registry::build(&parsed.description(4, 100_000_000))
                .unwrap_or_else(|error| panic!("{raw}: {error}"));
        }
    }

    #[test]
    fn imports_clock_edges_and_keeps_unsupported_framing_and_stacks_explicit() {
        let mdio =
            convert(&json!({"id":"mdio","channel":[{"mdc":0},{"mdio":1}]})).expect("default MDIO");
        assert_eq!(mdio["nodes"]["mdio"]["config"]["readEdge"], "falling");
        let swd =
            convert(&json!({"id":"swd","channel":[{"swclk":0},{"swdio":1}]})).expect("default SWD");
        assert_eq!(swd["nodes"]["swd"]["config"]["strict_start"], false);
        let midi = preserve(
            &json!({"id":"uart","channel":[{"rxtx":0}],"options":{"baudrate":9600},"stacked decoders":[{"id":"midi"}]}),
        );
        assert_eq!(midi["nodes"].as_object().expect("nodes").len(), 1);
        assert!(
            midi["import"]["diagnostics"]
                .as_array()
                .expect("notes")
                .iter()
                .any(|d| d["code"] == "decode.import.unsupportedStack")
        );
        let usb = json!({"id":"usb_signalling","channel":[{"dp":0},{"dm":1}],"options":{"signalling":"automatic"}});
        assert_eq!(preserve(&usb)["import"]["runnable"], false);
        let ps2 =
            json!({"id":"ps2","channel":[{"clk":0},{"data":1}],"options":{"HtoD_Clock":"fall"}});
        assert_eq!(preserve(&ps2)["import"]["runnable"], false);
    }

    #[test]
    fn malformed_numeric_options_remain_inert_instead_of_being_rounded_or_defaulted() {
        for raw in [
            json!({"id":"uart", "channel":[{"rxtx":0}], "options":{"num_data_bits":7.5}}),
            json!({"id":"uart", "channel":[{"rxtx":0}], "options":{"baudrate":"unknown"}}),
            json!({"id":"spi", "channel":[{"clk":0},{"mosi":1}], "options":{"wordsize":-1}}),
            json!({"id":"spi", "channel":[{"clk":0},{"mosi":1}], "options":{"cpol":u64::MAX}}),
            json!({"id":"qspi", "channel":[{"clk":0},{"io0":1}], "options":{"cpha":2}}),
            json!({"id":"can", "channel":[{"can_rx":0}], "options":{"sample_point":70.5}}),
            json!({"id":"can-fd", "channel":[{"can_rx":0}], "options":{"fast_bitrate":"invalid"}}),
        ] {
            let retained = preserve(&raw);
            assert_eq!(retained["import"]["raw"], raw);
            assert_eq!(retained["import"]["runnable"], false);
            assert_eq!(retained["nodes"], json!({}));
        }
        let valid = convert(&json!({"id":"spi", "channel":[{"clk":0},{"mosi":1}],
            "options":{"wordsize":8.0,"cpol":"1","cpha":1.0}}))
        .expect("integral numeric forms remain supported");
        assert_eq!(valid["nodes"]["spi"]["config"]["mode"], 3);
        assert_eq!(valid["nodes"]["spi"]["config"]["word_bits"], 8);
    }

    #[test]
    fn supplied_invalid_enums_are_inert_and_raw_options_remain_lossless() {
        for (id, channels, option_name) in [
            ("i2c", json!([{"scl":0},{"sda":1}]), "address_format"),
            ("uart", json!([{"rxtx":0}]), "bit_order"),
            ("uart", json!([{"rxtx":0}]), "invert"),
            ("uart", json!([{"rxtx":0}]), "parity_type"),
            ("uart", json!([{"rxtx":0}]), "parity_check"),
            ("spi", json!([{"clk":0},{"mosi":1}]), "bitorder"),
            ("spi", json!([{"clk":0},{"mosi":1}]), "cs_polarity"),
            ("spi", json!([{"clk":0},{"mosi":1}]), "frame"),
            ("qspi", json!([{"clk":0},{"io0":1}]), "bitorder"),
            ("qspi", json!([{"clk":0},{"io0":1}]), "cs_polarity"),
            ("swd", json!([{"swclk":0},{"swdio":1}]), "strict_start"),
            ("mdio", json!([{"mdc":0},{"mdio":1}]), "read_edge"),
            ("onewire_link", json!([{"owr":0}]), "overdrive"),
            ("ps2", json!([{"clk":0},{"data":1}]), "HtoD_Clock"),
            ("am230x", json!([{"sda":0}]), "device"),
            ("dmx512", json!([{"dmx":0}]), "invert"),
            ("usb_power_delivery", json!([{"cc1":0}]), "fulltext"),
        ] {
            for invalid in [json!("unknown"), json!(7), Value::Null] {
                let mut raw = json!({"id":id,"channel":channels,"options":{}});
                raw["options"][option_name] = invalid;
                let kept = preserve(&raw);
                assert_eq!(kept["import"]["raw"], raw);
                assert_eq!(kept["import"]["runnable"], false, "{raw}");
                assert_eq!(kept["nodes"], json!({}), "{raw}");
            }
        }
        for (id, channels, layer) in [
            (
                "i2c",
                json!([{"scl":0},{"sda":1}]),
                json!({"id":"lm75","options":{"sensor":7}}),
            ),
            (
                "i2c",
                json!([{"scl":0},{"sda":1}]),
                json!({"id":"eeprom24xx","options":{"chip":7}}),
            ),
            (
                "spi",
                json!([{"clk":0},{"cs":1},{"mosi":2},{"miso":3}]),
                json!({"id":"nrf24l01","options":{"chip":7}}),
            ),
            (
                "spi",
                json!([{"clk":0},{"cs":1},{"mosi":2},{"miso":3}]),
                json!({"id":"spi_tpm","options":{"tpm_version":7}}),
            ),
        ] {
            let raw = json!({"id":id,"channel":channels,"stacked decoders":[layer]});
            let kept = preserve(&raw);
            assert_eq!(kept["import"]["raw"], raw);
            assert_eq!(kept["nodes"].as_object().expect("bus only").len(), 1);
            assert!(
                kept["import"]["diagnostics"]
                    .as_array()
                    .expect("notes")
                    .iter()
                    .any(|d| d["code"] == "decode.import.unsupportedStack")
            );
        }
    }

    #[test]
    fn imports_each_declared_uart_number_format_as_an_entry_display_setting() {
        for format in ["hex", "dec", "oct", "bin", "ascii"] {
            let raw = json!({"id":"1:uart","channel":[{"rxtx":0}],"options":{"format":format}});
            let converted = convert(&raw).expect("supported numeric display");
            assert_eq!(converted["show"], json!({"numbers":format}));
            assert!(converted["nodes"]["uart"]["config"].get("format").is_none());
            let kept = preserve(&raw);
            assert_eq!(kept["import"]["raw"], raw);
            assert_eq!(kept["import"]["runnable"], true);
            assert_eq!(
                kept["import"]["numberFormats"][0],
                json!({"decoder":"1:uart","scope":"entry","value":format,"applied":true})
            );
            assert_eq!(
                DecoderEntry::parse(&kept).expect("entry").show,
                json!({"numbers":format})
            );
        }
        for invalid in [json!("float"), json!(7), Value::Null] {
            let raw = json!({"id":"uart","channel":[{"rxtx":0}],"options":{"format":invalid}});
            let kept = preserve(&raw);
            assert_eq!(kept["import"]["raw"], raw);
            assert_eq!(kept["import"]["runnable"], false);
            assert_eq!(kept["nodes"], json!({}));
            assert!(
                kept["import"]["diagnostics"]
                    .as_array()
                    .expect("notes")
                    .iter()
                    .any(|d| d["code"] == "decode.import.unsupportedOptions"
                        && d["params"]["options"] == "format")
            );
            assert_eq!(kept["import"]["numberFormats"][0]["applied"], false);
        }
    }

    #[test]
    fn preserves_layer_formats_individually_without_changing_unrelated_bus_lanes() {
        let raw = json!({"id":"spi","channel":[{"clk":0},{"cs":1},{"mosi":2},{"miso":3}],
            "stacked decoders":[{"id":"spiflash","options":{"format":"hex"}},
                {"id":"spiflash","options":{"format":"ascii"}},
                {"id":"spiflash","options":{"format":"dec"}}]});
        let kept = preserve(&raw);
        assert_eq!(kept["import"]["raw"], raw);
        assert_eq!(kept["import"]["runnable"], true);
        assert!(kept.get("show").is_none());
        assert_eq!(
            kept["nodes"]
                .as_object()
                .expect("bus and three flash layers")
                .len(),
            4
        );
        assert_eq!(
            kept["import"]["numberFormats"],
            json!([
                {"decoder":"spiflash","scope":"layer","layer":0,"value":"hex","applied":true},
                {"decoder":"spiflash","scope":"layer","layer":1,"value":"ascii","applied":false},
                {"decoder":"spiflash","scope":"layer","layer":2,"value":"dec","applied":false}
            ])
        );
        assert_eq!(
            kept["import"]["diagnostics"]
                .as_array()
                .expect("notes")
                .iter()
                .filter(|d| d["code"] == "decode.import.unsupportedOptions"
                    && d["params"]["options"] == "format")
                .count(),
            2
        );
        let raw = json!({"id":"microwire","channel":[{"cs":0},{"sk":1},{"si":2},{"so":3}],
            "stacked decoders":[{"id":"eeprom93xx","options":{"format":"ascii"}}]});
        let kept = preserve(&raw);
        assert_eq!(kept["import"]["numberFormats"][0]["applied"], false);
        assert_eq!(kept["import"]["numberFormats"][0]["decoder"], "eeprom93xx");
        assert!(kept.get("show").is_none());
    }

    #[test]
    fn does_not_invent_reference_format_options_for_other_decoders() {
        let raw = json!({"id":"spi","channel":[{"clk":0},{"mosi":1}],"options":{"format":"dec"}});
        let kept = preserve(&raw);
        assert_eq!(kept["import"]["raw"], raw);
        assert!(kept.get("show").is_none());
        assert_eq!(kept["import"]["numberFormats"][0]["applied"], false);
        assert!(
            kept["import"]["diagnostics"]
                .as_array()
                .expect("notes")
                .iter()
                .any(|d| d["code"] == "decode.import.unsupportedOptions"
                    && d["params"]["options"] == "format")
        );
    }

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
            json!({ "clk": 0, "miso": 1, "mosi": null, "cs": null, "mode": 3, "word_bits": 4, "bit_order": "lsb-first", "cs_active_low": false })
        );
        assert_eq!(eeprom_address_bytes("generic"), 1);
        assert_eq!(eeprom_address_bytes("24c16"), 1);
        assert_eq!(eeprom_address_bytes("24c32"), 2);
        assert_eq!(eeprom_address_bytes("24c1024"), 2);
        assert_eq!(eeprom_address_bytes("microchip-24aa512"), 2);
    }

    #[test]
    fn original_options_and_unknown_layers_remain_lossless() {
        let raw = json!({"id":"1:i2c", "label":"My bus", "channel":[{"scl":1},{"sda":0}],
            "options":{"address_format":"shifted", "custom_mode":17},
            "stacked decoders":[{"id":"future-chip", "options":{"opaque":[1,2,3]}}],
            "future_metadata":{"keep":true}});
        let kept = preserve(&raw);
        assert_eq!(kept["import"]["raw"], raw);
        assert_eq!(kept["import"]["runnable"], true);
        assert_eq!(
            kept["import"]["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .len(),
            2
        );
        assert!(crate::entry::DecoderEntry::parse(&kept).is_ok());
        assert_eq!(kept["nodes"]["i2c"]["config"], json!({"scl":1,"sda":0}));
        assert_eq!(kept["label"], "My bus");
    }

    #[test]
    fn imports_i2c_devices_and_keeps_repeated_instances_and_unknown_options() {
        let raw = json!({
            "id": "1:i2c", "channel": [{"scl": 7}, {"sda": 3}],
            "stacked decoders": [
                {"id": "ds1307", "options": {}},
                {"id": "rtc8564", "options": {}},
                {"id": "mlx90614", "options": {}},
                {"id": "pca9571", "options": {}},
                {"id": "tca6408a", "options": {}},
                {"id": "ds1307", "options": {"future": 12}}
            ]
        });
        let kept = preserve(&raw);
        let entry = DecoderEntry::parse(&kept).expect("an entry");
        assert_eq!(entry.nodes.len(), 7);
        assert_eq!(kept["nodes"]["ds1307-2"]["type"], "ds1307");
        assert_eq!(kept["connections"][6]["to"], "ds1307-2.bus");
        assert_eq!(kept["import"]["raw"], raw);
        assert!(
            kept["import"]["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .any(|note| note["code"] == "decode.import.unsupportedOptions"
                    && note["params"]["options"] == "future")
        );
        assert_eq!(kept["nodes"]["i2c"]["config"], json!({"scl":7,"sda":3}));
        uiharu_registry::build(&entry.description(8, 1_000_000))
            .expect("all translated layers connect");
    }

    #[test]
    fn imports_supported_temperature_widths_without_guessing_unknown_models() {
        let raw = json!({
            "id": "i2c", "channel": [{"scl": 1}, {"sda": 0}],
            "stacked decoders": [
                {"id": "lm75", "options": {"sensor": "lm75", "resolution": 12}},
                {"id": "lm75", "options": {"sensor": "other-sensor", "resolution": 9}},
                {"id": "lm75", "options": {"resolution": 16}}
            ]
        });
        let kept = preserve(&raw);
        assert_eq!(kept["nodes"].as_object().expect("nodes").len(), 2);
        assert_eq!(kept["nodes"]["lm75"]["config"]["resolution"], 12);
        assert_eq!(kept["import"]["raw"], raw);
        let notes = kept["import"]["diagnostics"]
            .as_array()
            .expect("diagnostics");
        assert_eq!(
            notes
                .iter()
                .filter(|note| note["code"] == "decode.import.unsupportedStack")
                .count(),
            2
        );
        assert!(
            notes
                .iter()
                .any(|note| note["code"] == "decode.import.deviceProfile")
        );
    }

    #[test]
    fn maps_dac_models_and_volts_to_integer_microvolts() {
        let layer = json!({"id":"ltc26x7", "options":{"chip":"ltc2627", "vref":2.048}});
        let (_, _, _, config) = i2c_layer(&layer).expect("supported DAC");
        assert_eq!(
            config,
            json!({"model":"ltc2627", "reference_microvolts":2_048_000})
        );
        assert!(i2c_layer(&json!({"id":"ltc26x7", "options":{"chip":"future-dac"}})).is_none());
        assert!(i2c_layer(&json!({"id":"ltc26x7", "options":{"vref":-1}})).is_none());
        assert!(i2c_layer(&json!({"id":"ltc26x7", "options":{"vref":"invalid"}})).is_none());
    }

    #[test]
    fn imports_eeprom_geometry_and_initial_counter_without_guessing_chips() {
        let raw = json!({"id":"i2c", "channel":[{"scl":0},{"sda":1}], "stacked decoders":[
            {"id":"eeprom24xx", "options":{"chip":"onsemi_cat24m01","addr_counter":65536}},
            {"id":"eeprom24xx", "options":{"chip":"microchip_24lc65"}},
            {"id":"eeprom24xx", "options":{"chip":"unknown-chip"}}
        ]});
        let kept = preserve(&raw);
        let config = &kept["nodes"]["eeprom24xx-memory"]["config"];
        assert_eq!(config["capacity"], 131072);
        assert_eq!(config["block_bits"], 1);
        assert_eq!(config["initial_counter"], 65536);
        assert_eq!(
            kept["nodes"]["eeprom24xx-memory-2"]["config"]["page_bytes"],
            64
        );
        assert_eq!(kept["nodes"].as_object().expect("nodes").len(), 3);
        assert_eq!(kept["import"]["raw"], raw);
        uiharu_registry::build(
            &DecoderEntry::parse(&kept)
                .expect("entry")
                .description(2, 1_000_000),
        )
        .expect("EEPROM geometry builds");
        assert!(
            eeprom_memory_config(&json!({"options":{"chip":"st_m24c01","addr_counter":128}}))
                .is_none()
        );
    }

    #[test]
    fn imports_debug_sd_and_display_channels_without_default_connections() {
        for (raw, kind) in [
            (
                json!({"id":"qspi","channel":[{"clk":7},{"io0":4},{"io1":2}],"options":{"cpha":1,"cpol":1}}),
                "qspi",
            ),
            (
                json!({"id":"sdcard_sd","channel":[{"clk":7},{"cmd":1}]}),
                "sd-native",
            ),
            (
                json!({"id":"sdcard_sd","channel":[{"clk":7},{"cmd":1},{"dat0":0},{"dat1":2},{"dat2":3},{"dat3":4}]}),
                "sd-native",
            ),
            (
                json!({"id":"jtag","channel":[{"tck":7},{"tms":1},{"tdi":2},{"tdo":3},{"srst":6}]}),
                "jtag",
            ),
            (json!({"id":"swim","channel":[{"swim":7}]}), "swim-stm8"),
            (json!({"id":"c2","channel":[{"c2ck":7},{"c2d":2}]}), "c2"),
            (
                json!({"id":"avr_pdi","channel":[{"reset":7},{"data":2}]}),
                "avr-pdi",
            ),
            (
                json!({"id":"fsi","channel":[{"clock":7},{"data":2}]}),
                "fsi",
            ),
            (
                json!({"id":"aud","channel":[{"audck":7},{"naudsync":6},{"audata0":1},{"audata1":2},{"audata2":3},{"audata3":4}]}),
                "aud",
            ),
            (
                json!({"id":"sda2506","channel":[{"clk":7},{"d":2},{"ce":3}]}),
                "sda2506",
            ),
            (
                json!({"id":"st7735","channel":[{"clk":7},{"mosi":2},{"cs":3},{"dc":4}]}),
                "st7735",
            ),
            (
                json!({"id":"st7789","channel":[{"dcx":7},{"sdo":2},{"csx":3},{"wrx":4}]}),
                "st7789",
            ),
            (
                json!({"id":"tlc5620","channel":[{"clk":7},{"data":2},{"load":3},{"ldac":4}],"options":{"vref_a":2.048}}),
                "tlc5620",
            ),
        ] {
            let kept = preserve(&raw);
            assert_eq!(kept["import"]["raw"], raw);
            assert_eq!(kept["nodes"][kind]["type"], kind);
            let parsed = DecoderEntry::parse(&kept).expect("entry");
            uiharu_registry::build(&parsed.description(8, 100_000_000))
                .unwrap_or_else(|error| panic!("{kind}: {error}"));
            if kind == "jtag" {
                assert!(kept["nodes"][kind]["config"]["trst"].is_null());
                assert!(
                    kept["import"]["diagnostics"]
                        .as_array()
                        .expect("notes")
                        .iter()
                        .any(|note| note["code"] == "decode.import.unsupportedChannels")
                );
            }
            if kind == "tlc5620" {
                assert_eq!(
                    kept["nodes"][kind]["config"]["vref_a_microvolts"],
                    2_048_000
                );
                assert_eq!(kept["nodes"][kind]["config"]["word_aligned"], false);
            }
            if kind == "swim-stm8" {
                assert_eq!(kept["nodes"][kind]["config"]["calibrate_sync"], false);
            }
        }
        for raw in [
            json!({"id":"qspi","channel":[{"clk":0},{"io0":1},{"io2":2}]}),
            json!({"id":"sdcard_sd","channel":[{"clk":0},{"cmd":1},{"dat0":2},{"dat2":3}]}),
        ] {
            assert_eq!(preserve(&raw)["import"]["runnable"], false);
        }
        let aligned = preserve(&json!({"id":"tlc5620","channel":[{"clk":0},{"data":1}]}));
        assert_eq!(aligned["nodes"]["tlc5620"]["config"]["word_aligned"], true);
        assert!(aligned["nodes"]["tlc5620"]["config"]["load"].is_null());
        uiharu_registry::build(
            &DecoderEntry::parse(&aligned)
                .expect("entry")
                .description(2, 1_000_000),
        )
        .expect("explicit no-LOAD alignment imports");
    }

    #[test]
    fn imports_spi_profiles_only_with_compatible_framing_and_preserves_repeats() {
        let raw = json!({"id":"spi","channel":[{"clk":0},{"cs":1},{"mosi":2},{"miso":3}],
            "stacked decoders":[{"id":"adxl345"},{"id":"max7219"},{"id":"max7219"},{"id":"spi_tpm","options":{"tpm_version":"1.2"}},{"id":"future"}]});
        let kept = preserve(&raw);
        assert_eq!(kept["nodes"]["spi-tpm"]["config"]["tpm_version"], "tpm12");
        assert_eq!(kept["nodes"]["adxl345-spi"]["type"], "adxl345-spi");
        assert_eq!(kept["nodes"]["max7219-2"]["type"], "max7219");
        assert_eq!(kept["import"]["raw"], raw);
        let parsed = DecoderEntry::parse(&kept).expect("entry");
        uiharu_registry::build(&parsed.description(4, 10_000_000)).expect("SPI profiles connect");
        let mut incompatible = raw;
        incompatible["options"] = json!({"wordsize":4});
        assert_eq!(
            preserve(&incompatible)["nodes"]
                .as_object()
                .expect("nodes")
                .len(),
            1
        );
        let missing = preserve(&json!({"id":"spi","channel":[{"clk":0},{"miso":1}]}));
        assert!(missing["nodes"]["spi"]["config"]["cs"].is_null());
        assert!(missing["nodes"]["spi"]["config"]["mosi"].is_null());
        uiharu_registry::build(
            &DecoderEntry::parse(&missing)
                .expect("entry")
                .description(2, 10_000_000),
        )
        .expect("only real channels required");
    }

    #[test]
    fn imports_command_only_spi_consumers_without_inventing_a_miso_line() {
        let raw = json!({"id":"spi","channel":[{"clk":0},{"cs":1},{"mosi":2}],
            "stacked decoders":[{"id":"max7219"},{"id":"max72xx"},{"id":"adns5020"},
                {"id":"adxl345"},{"id":"nrf24l01"}]});
        let kept = preserve(&raw);
        for name in ["max7219", "max72xx", "adns5020"] {
            assert_eq!(kept["nodes"][name]["type"], name);
        }
        assert!(kept["nodes"]["spi"]["config"]["miso"].is_null());
        assert!(kept["nodes"]["adxl345-spi"].is_null());
        assert!(kept["nodes"]["nrf24l01"].is_null());
        let diagnostics = kept["import"]["diagnostics"].as_array().expect("notes");
        for name in ["adxl345", "nrf24l01"] {
            assert!(
                diagnostics
                    .iter()
                    .any(|d| d["code"] == "decode.import.unsupportedStack"
                        && d["params"]["stack"] == name)
            );
        }
        let parsed = DecoderEntry::parse(&kept).expect("entry");
        uiharu_registry::build(&parsed.description(3, 10_000_000))
            .expect("command-only profiles build");
        // Omitting MOSI does not turn command-only consumers into receive-only decoders.
        let mut receive_only = raw;
        receive_only["channel"] = json!([{"clk":0},{"cs":1},{"miso":2}]);
        let kept = preserve(&receive_only);
        assert_eq!(kept["nodes"].as_object().expect("nodes").len(), 1);
    }

    #[test]
    fn imports_jtag_device_profiles_with_both_scan_directions() {
        let raw = json!({"id":"jtag", "channel":[{"tck":0},{"tms":1},{"tdi":2},{"tdo":3}],
            "stacked decoders":[{"id":"jtag_stm32"},{"id":"jtag_ejtag"}]});
        let kept = preserve(&raw);
        for name in ["jtag-stm32", "jtag-ejtag"] {
            assert_eq!(kept["nodes"][name]["type"], name);
        }
        assert_eq!(kept["import"]["raw"], raw);
        uiharu_registry::build(
            &DecoderEntry::parse(&kept)
                .expect("entry")
                .description(4, 16_000_000),
        )
        .expect("JTAG profiles connect");
        let mut partial = raw;
        partial["channel"] = json!([{"tck":0},{"tms":1},{"tdi":2}]);
        let kept = preserve(&partial);
        assert_eq!(kept["nodes"].as_object().expect("nodes").len(), 1);
        assert!(
            kept["import"]["diagnostics"]
                .as_array()
                .expect("notes")
                .iter()
                .any(|note| note["code"] == "decode.import.unsupportedStack")
        );
    }

    #[test]
    fn imports_radio_variant_with_explicit_register_map_note() {
        let raw = json!({"id":"spi","channel":[{"clk":0},{"cs":1},{"mosi":2},{"miso":3}],
            "stacked decoders":[{"id":"nrf24l01"},{"id":"nrf24l01","options":{"chip":"xn297"}},
                {"id":"nrf24l01","options":{"chip":"unknown"}}]});
        let kept = preserve(&raw);
        assert_eq!(kept["nodes"]["nrf24l01"]["type"], "nrf24l01");
        assert_eq!(kept["nodes"]["xn297"]["config"]["model"], "xn297l");
        let notes = kept["import"]["diagnostics"].as_array().expect("notes");
        assert!(
            notes
                .iter()
                .any(|note| note["code"] == "decode.import.xn297Profile")
        );
        assert!(
            notes
                .iter()
                .any(|note| note["code"] == "decode.import.unsupportedStack")
        );
        assert_eq!(kept["import"]["raw"], raw);
        uiharu_registry::build(
            &DecoderEntry::parse(&kept)
                .expect("entry")
                .description(4, 10_000_000),
        )
        .expect("radio variants connect");
    }

    #[test]
    fn imports_continuous_spi_devices_without_chip_select() {
        for (source, target, channels) in [
            ("rgb_led_spi", "rgb-led-spi", json!([{"clk":0},{"mosi":1}])),
            (
                "ade77xx",
                "ade77xx",
                json!([{"clk":0},{"mosi":1},{"miso":2}]),
            ),
        ] {
            let raw = json!({"id":"spi", "channel":channels, "stacked decoders":[{"id":source}]});
            let kept = preserve(&raw);
            assert_eq!(kept["import"]["raw"], raw);
            assert_eq!(kept["nodes"][target]["type"], target);
            assert_eq!(kept["nodes"][target]["config"]["framed"], false);
            uiharu_registry::build(
                &DecoderEntry::parse(&kept)
                    .expect("entry")
                    .description(3, 10_000_000),
            )
            .expect("continuous SPI profile connects");
            let mut unsupported = raw;
            unsupported["options"] = json!({"cpha":1});
            let kept = preserve(&unsupported);
            assert_eq!(kept["nodes"].as_object().expect("nodes").len(), 1);
            assert!(
                kept["import"]["diagnostics"]
                    .as_array()
                    .expect("notes")
                    .iter()
                    .any(|note| note["code"] == "decode.import.unsupportedStack")
            );
        }
    }

    #[test]
    fn unsupported_imports_are_metadata_with_no_runnable_nodes() {
        let raw = json!({"id":"future-bus", "options":{"binary":[0,255]}, "channel":[{"bus":3}]});
        let kept = preserve(&raw);
        assert_eq!(kept["import"]["raw"], raw);
        assert_eq!(kept["import"]["runnable"], false);
        assert_eq!(kept["label"], "future-bus");
        assert!(
            crate::entry::DecoderEntry::parse(&kept)
                .expect("preserved metadata")
                .nodes
                .is_empty()
        );
    }

    #[test]
    fn leaves_out_what_uiharu_cannot_decode() {
        for stop in [0.0, 0.5, 2.5] {
            let raw = json!({"id":"uart","channel":[{"rxtx":0}],"options":{"num_stop_bits":stop}});
            let kept = preserve(&raw);
            assert_eq!(kept["import"]["runnable"], false);
            assert_eq!(kept["import"]["raw"], raw);
        }
        assert!(
            convert(&json!({"id":"uart","channel":[{"rxtx":0}],"options":{"num_data_bits":128}}))
                .is_none()
        );
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
