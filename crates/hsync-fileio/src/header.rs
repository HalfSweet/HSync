//! The `header` entry of a `.dsl` file: a GKeyFile (INI) document.
//!
//! Written by DSView's `StoreSession::meta_gen` and read by `sr_load_virtual_device_session`
//!.

use std::fmt::Write as _;

use hsync_core::ChannelIndex;

use crate::FileError;

/// The newest header format; HSync always writes it.
pub const HEADER_VERSION: u32 = 3;

/// Device mode of logic captures; DSO (1) and DAQ (2) files are not supported in v1.
pub const LOGIC_MODE: i32 = 0;

/// Parsed `header` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DslHeader {
    /// Format version: 1 keeps interleaved samples in one `data` entry, 2 and 3 keep one
    /// `L-<channel>/<block>` entry per channel and block.
    pub version: u32,
    /// Driver that produced the capture, e.g. `DSLogic` or `virtual-demo`.
    pub driver: String,
    /// 0 logic, 1 oscilloscope, 2 data acquisition.
    pub device_mode: i32,
    /// Number of samples per channel.
    pub total_samples: u64,
    /// Number of channels stored in the file.
    pub total_probes: u32,
    /// Number of blocks per channel (version 2 and later).
    pub total_blocks: Option<u32>,
    /// Samples per second.
    pub samplerate: u64,
    /// When the trigger fired, in milliseconds since the Unix epoch.
    pub trigger_time_ms: Option<i64>,
    /// Sample at which the trigger fired.
    pub trigger_pos: u64,
    /// Stored channels as `(channel index, name)`, ascending.
    pub probes: Vec<(ChannelIndex, String)>,
}

impl DslHeader {
    /// Parses the text of a `header` entry.
    pub fn parse(text: &str) -> Result<Self, FileError> {
        let mut header = Self {
            version: 1,
            driver: String::new(),
            device_mode: LOGIC_MODE,
            total_samples: 0,
            total_probes: 0,
            total_blocks: None,
            samplerate: 0,
            trigger_time_ms: None,
            trigger_pos: 0,
            probes: Vec::new(),
        };
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                name.clone_into(&mut section);
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(FileError::InvalidHeader(line.to_owned()));
            };
            let (key, value) = (key.trim(), value.trim());
            let number = || -> Result<u64, FileError> {
                value
                    .parse()
                    .map_err(|_| FileError::InvalidHeader(line.to_owned()))
            };
            match (section.as_str(), key) {
                ("version", "version") => {
                    header.version = u32::try_from(number()?)
                        .map_err(|_| FileError::InvalidHeader(line.to_owned()))?;
                }
                ("header", "driver") => value.clone_into(&mut header.driver),
                ("header", "device mode") => {
                    header.device_mode = value
                        .parse()
                        .map_err(|_| FileError::InvalidHeader(line.to_owned()))?;
                }
                ("header", "total samples") => header.total_samples = number()?,
                ("header", "total probes") => {
                    header.total_probes = u32::try_from(number()?)
                        .map_err(|_| FileError::InvalidHeader(line.to_owned()))?;
                }
                ("header", "total blocks") => {
                    header.total_blocks = Some(
                        u32::try_from(number()?)
                            .map_err(|_| FileError::InvalidHeader(line.to_owned()))?,
                    );
                }
                ("header", "samplerate") => {
                    header.samplerate = parse_samplerate(value)
                        .ok_or_else(|| FileError::InvalidHeader(line.to_owned()))?;
                }
                ("header", "trigger time") => {
                    let ms: i64 = value
                        .parse()
                        .map_err(|_| FileError::InvalidHeader(line.to_owned()))?;
                    // DSView writes the local-time epoch (e.g. -28800000 in UTC+8) for captures
                    // that never had a real time; treat anything before 1971 as unknown.
                    header.trigger_time_ms = (ms > 365 * 24 * 3600 * 1000).then_some(ms);
                }
                ("header", "trigger pos") => header.trigger_pos = number()?,
                ("header", key) if key.starts_with("probe") => {
                    let index: ChannelIndex = key["probe".len()..]
                        .parse()
                        .map_err(|_| FileError::InvalidHeader(line.to_owned()))?;
                    header.probes.push((index, value.to_owned()));
                }
                // Per-probe analog settings and keys this version does not use.
                _ => {}
            }
        }
        header.probes.sort_by_key(|(index, _)| *index);
        header.probes.dedup_by_key(|(index, _)| *index);
        Ok(header)
    }

    /// Formats the header the way DSView writes it.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "[version]");
        let _ = writeln!(text, "version = {}", self.version);
        let _ = writeln!(text, "[header]");
        let _ = writeln!(text, "driver = {}", self.driver);
        let _ = writeln!(text, "device mode = {}", self.device_mode);
        let _ = writeln!(text, "capturefile = data");
        let _ = writeln!(text, "total samples = {}", self.total_samples);
        let _ = writeln!(text, "total probes = {}", self.total_probes);
        if let Some(blocks) = self.total_blocks {
            let _ = writeln!(text, "total blocks = {blocks}");
        }
        let _ = writeln!(text, "samplerate = {}", format_samplerate(self.samplerate));
        let _ = writeln!(text, "trigger time = {}", self.trigger_time_ms.unwrap_or(0));
        let _ = writeln!(text, "trigger pos = {}", self.trigger_pos);
        for (index, name) in &self.probes {
            let _ = writeln!(text, "probe{index} = {name}");
        }
        text
    }
}

/// Parses a samplerate such as `25 MHz`, `100 kHz`, `1.5 MHz` or `250000`.
///
/// Accepts everything DSView's `sr_parse_sizestring` accepts, plus decimal fractions.
pub fn parse_samplerate(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, rest) = text.split_at(digits);
    let rest = rest.trim_start();
    let (multiplier, unit) = match rest.chars().next() {
        Some('k' | 'K') => (1_000u128, &rest[1..]),
        Some('m' | 'M') => (1_000_000, &rest[1..]),
        Some('g' | 'G') => (1_000_000_000, &rest[1..]),
        _ => (1, rest),
    };
    if !(unit.is_empty() || unit.eq_ignore_ascii_case("hz")) {
        return None;
    }
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let whole: u128 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    let mut value = whole.checked_mul(multiplier)?;
    if !fraction.is_empty() {
        let digits = u32::try_from(fraction.len()).ok()?;
        let scale = 10u128.checked_pow(digits)?;
        value = value.checked_add(fraction.parse::<u128>().ok()? * multiplier / scale)?;
    }
    u64::try_from(value).ok()
}

/// Formats a samplerate exactly like DSView's `sr_samplerate_string`, so the files it reads
/// back parse the same way.
pub fn format_samplerate(hz: u64) -> String {
    const G: u64 = 1_000_000_000;
    const M: u64 = 1_000_000;
    const K: u64 = 1_000;
    match hz {
        x if x >= G && x % G == 0 => format!("{} GHz", x / G),
        x if x >= G => format!("{}.{} GHz", x / G, x % G),
        x if x >= M && x % M == 0 => format!("{} MHz", x / M),
        x if x >= M => format!("{}.{} MHz", x / M, x % M),
        x if x >= K && x % K == 0 => format!("{} kHz", x / K),
        x if x >= K => format!("{}.{} KHz", x / K, x % K),
        x => format!("{x} Hz"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = "[version]\nversion = 3\n[header]\ndriver = virtual-demo\ndevice mode = 0\n\
        capturefile = data\ntotal samples = 131072\ntotal probes = 16\ntotal blocks = 1\n\
        samplerate = 25 MHz\ntrigger time = -28800000\ntrigger pos = 0\nprobe0 = SDA\n\
        probe1 = SCL\nprobe13 = CS#\n";

    #[test]
    fn parses_the_demo_header() {
        let header = DslHeader::parse(DEMO).expect("valid header");
        assert_eq!(header.version, 3);
        assert_eq!(header.driver, "virtual-demo");
        assert_eq!(header.total_samples, 131_072);
        assert_eq!(header.total_blocks, Some(1));
        assert_eq!(header.samplerate, 25_000_000);
        assert_eq!(header.trigger_time_ms, None);
        assert_eq!(
            header.probes,
            vec![(0, "SDA".into()), (1, "SCL".into()), (13, "CS#".into())]
        );
    }

    #[test]
    fn formats_what_it_parses() {
        let header = DslHeader::parse(DEMO).expect("valid header");
        assert_eq!(DslHeader::parse(&header.to_text()).ok(), Some(header));
    }

    #[test]
    fn parses_samplerates() {
        assert_eq!(parse_samplerate("25 MHz"), Some(25_000_000));
        assert_eq!(parse_samplerate("25MHz"), Some(25_000_000));
        assert_eq!(parse_samplerate("100 kHz"), Some(100_000));
        assert_eq!(parse_samplerate("1 GHz"), Some(1_000_000_000));
        assert_eq!(parse_samplerate("1.5 MHz"), Some(1_500_000));
        assert_eq!(parse_samplerate("10 Hz"), Some(10));
        assert_eq!(parse_samplerate("250000"), Some(250_000));
        assert_eq!(parse_samplerate("fast"), None);
        assert_eq!(parse_samplerate("3 MB"), None);
    }

    #[test]
    fn formats_samplerates_like_dsview() {
        assert_eq!(format_samplerate(25_000_000), "25 MHz");
        assert_eq!(format_samplerate(1_000_000_000), "1 GHz");
        assert_eq!(format_samplerate(400_000), "400 kHz");
        assert_eq!(format_samplerate(10), "10 Hz");
        for rate in [10, 1_000, 40_000, 25_000_000, 125_000_000, 1_000_000_000] {
            assert_eq!(parse_samplerate(&format_samplerate(rate)), Some(rate));
        }
    }
}
