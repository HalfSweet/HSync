//! What to decode: a bus, its channels and options, read from DSView's `decoders` entries
//! (`.dsl` files and sessions) and run over a capture.

use std::collections::BTreeMap;
use std::ops::Range;

use hsync_capture::LogicCapture;
use hsync_core::ChannelIndex;
use serde::Serialize;
use serde_json::Value;

use crate::{can, i2c, spi, uart};

/// How a decoder's results are presented: the buses with native decoders, and `Other` for the
/// annotations of any other decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Bus {
    I2c,
    Uart,
    Spi,
    Can,
    Other,
}

/// Decoder settings by bus.
#[derive(Debug, Clone, PartialEq)]
pub enum Settings {
    I2c {
        /// Show 7-bit addresses (DSView's `address_format: shifted`).
        shifted: bool,
    },
    Uart(uart::Options),
    Spi(spi::Options),
    Can(can::Options),
}

/// One decoder of a capture.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// DSView's decoder id, e.g. `1:i2c`.
    pub id: String,
    pub bus: Bus,
    /// Channel by role (`sda`, `scl`, `rxtx`, `clk`, `miso`, `mosi`, `cs`, `can_rx`).
    pub channels: BTreeMap<String, ChannelIndex>,
    pub settings: Settings,
}

fn number(options: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    match options.get(key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn text<'a>(options: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    options.get(key)?.as_str()
}

/// The bus of a DSView decoder id, ignoring its version prefix (`1:i2c` → I²C).
pub fn bus_of(id: &str) -> Option<Bus> {
    let base = id.rsplit(':').next().unwrap_or(id);
    match base {
        "i2c" => Some(Bus::I2c),
        "uart" => Some(Bus::Uart),
        "spi" => Some(Bus::Spi),
        "can" | "can-fd" => Some(Bus::Can),
        _ => None,
    }
}

impl Spec {
    /// Reads one entry of DSView's `decoders` list; `None` for buses without a native decoder.
    pub fn from_dsview(entry: &Value) -> Option<Self> {
        let id = entry.get("id")?.as_str()?.to_owned();
        let bus = bus_of(&id)?;
        let mut channels = BTreeMap::new();
        for item in entry.get("channel")?.as_array()? {
            for (role, index) in item.as_object()? {
                if let Some(index) = index.as_u64().and_then(|i| ChannelIndex::try_from(i).ok()) {
                    channels.insert(role.clone(), index);
                }
            }
        }
        let empty = serde_json::Map::new();
        let options = entry
            .get("options")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let settings = match bus {
            Bus::Other => return None,
            Bus::I2c => Settings::I2c {
                shifted: text(options, "address_format") == Some("shifted"),
            },
            Bus::Uart => {
                let defaults = uart::Options::default();
                Settings::Uart(uart::Options {
                    baudrate: number(options, "baudrate").unwrap_or(defaults.baudrate),
                    data_bits: number(options, "num_data_bits")
                        .map_or(defaults.data_bits, |bits| bits.clamp(4.0, 16.0) as u8),
                    parity: match text(options, "parity_type") {
                        Some("odd") => uart::Parity::Odd,
                        Some("even") => uart::Parity::Even,
                        Some("zero") => uart::Parity::Zero,
                        Some("one") => uart::Parity::One,
                        _ => uart::Parity::None,
                    },
                    stop_bits: number(options, "num_stop_bits").unwrap_or(defaults.stop_bits),
                    msb_first: text(options, "bit_order") == Some("msb-first"),
                    invert: text(options, "invert") == Some("yes"),
                })
            }
            Bus::Spi => Settings::Spi(spi::Options {
                cpol: number(options, "cpol").is_some_and(|v| v != 0.0),
                cpha: number(options, "cpha").is_some_and(|v| v != 0.0),
                msb_first: text(options, "bitorder") != Some("lsb-first"),
                word_size: number(options, "wordsize").map_or(8, |w| w.clamp(1.0, 32.0) as u8),
                cs_active_low: text(options, "cs_polarity") != Some("active-high"),
            }),
            Bus::Can => {
                let defaults = can::Options::default();
                let nominal = number(options, "nominal_bitrate")
                    .or_else(|| number(options, "bitrate"))
                    .unwrap_or(defaults.nominal_bitrate);
                Settings::Can(can::Options {
                    nominal_bitrate: nominal,
                    fast_bitrate: number(options, "fast_bitrate")
                        .unwrap_or(nominal.max(defaults.fast_bitrate)),
                    sample_point: number(options, "sample_point").unwrap_or(defaults.sample_point),
                })
            }
        };
        Some(Self {
            id,
            bus,
            channels,
            settings,
        })
    }

    fn channel<'a>(
        &self,
        capture: &'a LogicCapture,
        role: &str,
    ) -> Option<&'a hsync_capture::ChannelData> {
        capture.channel(*self.channels.get(role)?)
    }

    /// Runs the decoder over `samples` of `capture` (DSView's decode start and end); `None`
    /// when a required channel is missing.
    pub fn run(&self, capture: &LogicCapture, samples: Range<u64>) -> Option<Decoded> {
        let samplerate = capture.samplerate();
        Some(match &self.settings {
            Settings::I2c { shifted } => Decoded::I2c {
                trace: i2c::decode(
                    self.channel(capture, "scl")?,
                    self.channel(capture, "sda")?,
                    samples,
                ),
                shifted: *shifted,
            },
            Settings::Uart(options) => {
                let line = self
                    .channel(capture, "rxtx")
                    .or_else(|| self.channel(capture, "rx"))
                    .or_else(|| self.channel(capture, "tx"))?;
                Decoded::Uart(uart::decode(line, samples, samplerate, options))
            }
            Settings::Spi(options) => {
                let lines = spi::Lines {
                    clk: self.channel(capture, "clk")?,
                    miso: self.channel(capture, "miso"),
                    mosi: self.channel(capture, "mosi"),
                    cs: self.channel(capture, "cs"),
                };
                if lines.miso.is_none() && lines.mosi.is_none() {
                    return None;
                }
                Decoded::Spi(spi::decode(&lines, samples, options))
            }
            Settings::Can(options) => {
                let line = self
                    .channel(capture, "can_rx")
                    .or_else(|| self.channel(capture, "rx"))?;
                Decoded::Can(can::decode(line, samples, samplerate, options))
            }
        })
    }
}

/// The result of one decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    I2c { trace: i2c::Trace, shifted: bool },
    Uart(uart::Trace),
    Spi(spi::Trace),
    Can(can::Trace),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_dsview_entries() {
        let entry = json!({
            "id": "1:uart",
            "channel": [{"rxtx": 5}],
            "options": {"baudrate": 115200, "num_data_bits": 8, "parity_type": "none"},
        });
        let spec = Spec::from_dsview(&entry).expect("uart");
        assert_eq!(spec.bus, Bus::Uart);
        assert_eq!(spec.channels.get("rxtx"), Some(&5));
        let Settings::Uart(options) = spec.settings else {
            panic!("uart settings");
        };
        assert_eq!(options.baudrate, 115_200.0);
        assert!(Spec::from_dsview(&json!({"id": "1:jtag", "channel": []})).is_none());
    }
}
