//! Exporting samples to other tools: DSView's CSV, Value Change Dump, GTKWave's FST, sigrok's
//! session format and plain interleaved bytes. Callers export part of a capture by slicing it
//! first ([`LogicCapture::slice`]).
//!
//! The CSV layout follows DSView's `libsigrok4DSL/output/csv.c`; the sigrok archive what
//! libsigrok's `srzip` input reads.

use std::fs::File;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fst_writer::{
    FstFileType, FstInfo, FstScopeType, FstSignalId, FstSignalType, FstVarDirection, FstVarType,
};
use hsync_capture::{ChannelData, LogicCapture};
use hsync_core::ChannelIndex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::FileError;
use crate::progress::Progress;

/// Samples per `logic-1-N` entry of a sigrok archive and per write of the interleaved formats.
const SAMPLES_PER_CHUNK: u64 = 1 << 22;

/// What to export to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// DSView's CSV: a row where any channel changes, or with `every_sample` one per sample.
    Csv { every_sample: bool },
    /// Value Change Dump.
    Vcd,
    /// GTKWave's FST, a compact binary waveform.
    Fst,
    /// sigrok's session archive (`.sr`), which PulseView opens.
    Sigrok,
    /// Each sample as ⌈channels / 8⌉ bytes, the `k`-th channel in bit `k`, little endian.
    Binary,
}

/// What an export writes besides the samples.
#[derive(Debug, Clone)]
pub struct ExportRequest<'a> {
    pub format: ExportFormat,
    /// Name of each channel of the capture, in export order.
    pub channels: &'a [(ChannelIndex, String)],
    /// Channels of the device the capture came from, for the CSV and VCD comments.
    pub total_channels: usize,
    /// Position of the exported samples in the capture they were cut from: CSV times count from
    /// the capture's first sample, as DSView's do.
    pub first_sample: u64,
    /// Creation time for the file's comments, e.g. `2026-10-10 06:30:00 UTC`.
    pub date: &'a str,
    /// Program and version for the file's comments.
    pub generator: &'a str,
}

/// Writes the samples of `capture` in `request.format`.
pub fn export<W: Write + Seek>(
    writer: W,
    capture: &LogicCapture,
    request: &ExportRequest<'_>,
    progress: Progress<'_>,
) -> Result<W, FileError> {
    let channels: Vec<&ChannelData> = request
        .channels
        .iter()
        .filter_map(|(index, _)| capture.channel(*index))
        .collect();
    let mut writer = writer;
    match request.format {
        ExportFormat::Csv { every_sample } => {
            write_csv(
                &mut writer,
                capture,
                &channels,
                request,
                every_sample,
                progress,
            )?;
        }
        ExportFormat::Vcd => write_vcd(&mut writer, capture, &channels, request, progress)?,
        ExportFormat::Fst => write_fst(&mut writer, capture, &channels, request, progress)?,
        ExportFormat::Binary => {
            let mut sink = |bytes: &[u8]| writer.write_all(bytes);
            interleave(capture.len(), &channels, progress, &mut sink)?;
        }
        ExportFormat::Sigrok => return write_sigrok(writer, capture, &channels, request, progress),
    }
    writer.flush()?;
    Ok(writer)
}

/// `%0.15g` of C's printf, which DSView's CSV times use.
fn general(value: f64) -> String {
    const PRECISION: i32 = 15;
    if value == 0.0 {
        return "0".into();
    }
    let exponent = format!("{value:.*e}", (PRECISION - 1) as usize);
    let (mantissa, power) = exponent.split_once('e').unwrap_or((&exponent, "0"));
    let power: i32 = power.parse().unwrap_or(0);
    let trim = |text: &str| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text.to_owned()
        }
    };
    if (-4..PRECISION).contains(&power) {
        let decimals = (PRECISION - 1 - power).max(0) as usize;
        trim(&format!("{value:.decimals$}"))
    } else {
        format!(
            "{}e{}{:02}",
            trim(mantissa),
            if power < 0 { '-' } else { '+' },
            power.abs()
        )
    }
}

/// A sample rate as libsigrok writes one, e.g. `25 MHz` or `12.5 MHz`.
pub fn samplerate_text(rate: u64) -> String {
    let (unit, scale) = match rate {
        r if r >= 1_000_000_000 => ("GHz", 1_000_000_000),
        r if r >= 1_000_000 => ("MHz", 1_000_000),
        r if r >= 1_000 => ("kHz", 1_000),
        _ => ("Hz", 1),
    };
    let whole = rate / scale;
    let rest = rate % scale;
    if rest == 0 {
        format!("{whole} {unit}")
    } else {
        let digits = scale.ilog10() as usize;
        let fraction = format!("{rest:0digits$}");
        format!("{whole}.{} {unit}", fraction.trim_end_matches('0'))
    }
}

/// The samples where any of `channels` changes, from `0`, in order.
struct Changes<'a> {
    channels: &'a [&'a ChannelData],
    /// Each channel's next change after the current sample.
    next: Vec<Option<u64>>,
    levels: Vec<bool>,
    started: bool,
    len: u64,
}

impl<'a> Changes<'a> {
    fn new(channels: &'a [&'a ChannelData], len: u64) -> Self {
        Self {
            channels,
            next: vec![None; channels.len()],
            levels: vec![false; channels.len()],
            started: false,
            len,
        }
    }

    /// The next sample with a change and every channel's level there.
    fn advance(&mut self) -> Option<(u64, &[bool])> {
        if self.len == 0 {
            return None;
        }
        if !self.started {
            self.started = true;
            for (index, data) in self.channels.iter().enumerate() {
                self.levels[index] = data.sample(0);
                self.next[index] = data.next_change(0).filter(|&at| at < self.len);
            }
            return Some((0, &self.levels));
        }
        let at = self.next.iter().flatten().copied().min()?;
        for (index, data) in self.channels.iter().enumerate() {
            if self.next[index] == Some(at) {
                self.levels[index] = !self.levels[index];
                self.next[index] = data.next_change(at).filter(|&next| next < self.len);
            }
        }
        Some((at, &self.levels))
    }
}

fn cancelled(progress: &mut dyn FnMut(f64) -> bool, done: u64, total: u64) -> bool {
    !progress(if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    })
}

fn write_csv<W: Write>(
    out: &mut W,
    capture: &LogicCapture,
    channels: &[&ChannelData],
    request: &ExportRequest<'_>,
    every_sample: bool,
    progress: Progress<'_>,
) -> Result<(), FileError> {
    let len = capture.len();
    let rate = capture.samplerate().max(1) as f64;
    writeln!(
        out,
        "; CSV, generated by {}, sampled on {}",
        request.generator, request.date
    )?;
    writeln!(
        out,
        "; Channels ({}/{})",
        channels.len(),
        request.total_channels.max(channels.len())
    )?;
    writeln!(
        out,
        "; Sample rate: {}",
        samplerate_text(capture.samplerate())
    )?;
    writeln!(out, "; Sample count: {len}")?;
    // DSView puts a space before each channel name.
    write!(out, "Time(s),")?;
    let names: Vec<String> = request
        .channels
        .iter()
        .map(|(_, name)| {
            let field = format!(" {name}");
            if field.contains([',', '"', '\r', '\n']) {
                format!("\"{}\"", field.replace('"', "\"\""))
            } else {
                field
            }
        })
        .collect();
    writeln!(out, "{}", names.join(","))?;

    let mut row = String::new();
    let mut emit = |out: &mut W, sample: u64, levels: &mut dyn Iterator<Item = bool>| {
        row.clear();
        row.push_str(&general((request.first_sample + sample) as f64 / rate));
        for level in levels {
            row.push(',');
            row.push(if level { '1' } else { '0' });
        }
        row.push('\n');
        out.write_all(row.as_bytes())
    };
    if every_sample {
        let words = len.div_ceil(64);
        for word in 0..words {
            if word.is_multiple_of(4096) && cancelled(progress, word * 64, len) {
                return Err(FileError::Cancelled);
            }
            let values: Vec<u64> = channels
                .iter()
                .map(|data| data.word(word as usize))
                .collect();
            for bit in 0..(len - word * 64).min(64) {
                let mut levels = values.iter().map(|value| value >> bit & 1 != 0);
                emit(out, word * 64 + bit, &mut levels)?;
            }
        }
    } else {
        let mut changes = Changes::new(channels, len);
        let mut rows = 0u64;
        while let Some((sample, levels)) = changes.advance() {
            rows += 1;
            if rows.is_multiple_of(65_536) && cancelled(progress, sample, len) {
                return Err(FileError::Cancelled);
            }
            emit(out, sample, &mut levels.iter().copied())?;
        }
    }
    progress(1.0);
    Ok(())
}

/// The VCD time unit: the largest of 1, 10 and 100 s, ms, µs, ns, ps or fs that divides the
/// sample period, in femtoseconds, with its name. Rates that do not divide 10^15 fs use 1 ps.
fn vcd_timescale(rate: u64) -> (u64, &'static str) {
    const UNITS: [(u64, &str); 16] = [
        (1_000_000_000_000_000, "1 s"),
        (100_000_000_000_000, "100 ms"),
        (10_000_000_000_000, "10 ms"),
        (1_000_000_000_000, "1 ms"),
        (100_000_000_000, "100 us"),
        (10_000_000_000, "10 us"),
        (1_000_000_000, "1 us"),
        (100_000_000, "100 ns"),
        (10_000_000, "10 ns"),
        (1_000_000, "1 ns"),
        (100_000, "100 ps"),
        (10_000, "10 ps"),
        (1_000, "1 ps"),
        (100, "100 fs"),
        (10, "10 fs"),
        (1, "1 fs"),
    ];
    const SECOND: u64 = 1_000_000_000_000_000;
    if rate == 0 || !SECOND.is_multiple_of(rate) {
        return (1_000, "1 ps");
    }
    let period = SECOND / rate;
    UNITS
        .iter()
        .copied()
        .find(|(unit, _)| period.is_multiple_of(*unit))
        .unwrap_or((1, "1 fs"))
}

fn write_vcd<W: Write>(
    out: &mut W,
    capture: &LogicCapture,
    channels: &[&ChannelData],
    request: &ExportRequest<'_>,
    progress: Progress<'_>,
) -> Result<(), FileError> {
    let len = capture.len();
    let rate = capture.samplerate();
    let (unit_fs, unit_name) = vcd_timescale(rate);
    let period_fs = 1_000_000_000_000_000f64 / rate.max(1) as f64;
    let time = |sample: u64| (sample as f64 * period_fs / unit_fs as f64).round() as u64;
    writeln!(out, "$date {} $end", request.date)?;
    writeln!(out, "$version {} $end", request.generator)?;
    writeln!(
        out,
        "$comment\n  Acquisition with {}/{} channels at {}\n$end",
        channels.len(),
        request.total_channels.max(channels.len()),
        samplerate_text(rate)
    )?;
    writeln!(out, "$timescale {unit_name} $end")?;
    writeln!(out, "$scope module HSync $end")?;
    for (position, (_, name)) in request.channels.iter().enumerate() {
        writeln!(out, "$var wire 1 {} {name} $end", vcd_id(position))?;
    }
    writeln!(out, "$upscope $end\n$enddefinitions $end")?;
    let mut changes = Changes::new(channels, len);
    let mut previous: Vec<Option<bool>> = vec![None; channels.len()];
    let mut line = String::new();
    let mut rows = 0u64;
    while let Some((sample, levels)) = changes.advance() {
        rows += 1;
        if rows.is_multiple_of(65_536) && cancelled(progress, sample, len) {
            return Err(FileError::Cancelled);
        }
        line.clear();
        line.push('#');
        line.push_str(&time(sample).to_string());
        for (position, &level) in levels.iter().enumerate() {
            if previous[position] != Some(level) {
                line.push(' ');
                line.push(if level { '1' } else { '0' });
                line.push_str(&vcd_id(position));
                previous[position] = Some(level);
            }
        }
        line.push('\n');
        out.write_all(line.as_bytes())?;
    }
    // The final timestamp marks the length, as DSView writes it.
    writeln!(out, "#{}", time(len))?;
    progress(1.0);
    Ok(())
}

/// The FST time unit: the largest power of ten of a second, down to 1 fs, that divides the
/// sample period, as a power of ten of a second and in femtoseconds. Rates that do not divide
/// 10^15 fs use 1 ps.
fn fst_timescale(rate: u64) -> (i8, u64) {
    const SECOND: u64 = 1_000_000_000_000_000;
    if rate == 0 || !SECOND.is_multiple_of(rate) {
        return (-12, 1_000);
    }
    let period = SECOND / rate;
    (-15..=0)
        .rev()
        .map(|exponent: i8| (exponent, 10u64.pow((exponent + 15) as u32)))
        .find(|(_, unit)| period.is_multiple_of(*unit))
        .unwrap_or((-15, 1))
}

/// Temporary FST files written so far by this process, to name them apart.
static FST_TEMP_FILES: AtomicU64 = AtomicU64::new(0);

/// Writes the capture as an FST file the way [`write_vcd`] writes a VCD. fst-writer writes to
/// a path of its own, so the file goes to a temporary one and is copied into `out` once
/// complete.
fn write_fst<W: Write>(
    out: &mut W,
    capture: &LogicCapture,
    channels: &[&ChannelData],
    request: &ExportRequest<'_>,
    progress: Progress<'_>,
) -> Result<(), FileError> {
    let temp: PathBuf = std::env::temp_dir().join(format!(
        "hsync-{}-{}.fst",
        std::process::id(),
        FST_TEMP_FILES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = write_fst_to(&temp, capture, channels, request, progress).and_then(|()| {
        std::io::copy(&mut File::open(&temp)?, out)?;
        Ok(())
    });
    let _ = std::fs::remove_file(&temp);
    written
}

fn write_fst_to(
    path: &Path,
    capture: &LogicCapture,
    channels: &[&ChannelData],
    request: &ExportRequest<'_>,
    progress: Progress<'_>,
) -> Result<(), FileError> {
    let len = capture.len();
    let rate = capture.samplerate();
    let (exponent, unit_fs) = fst_timescale(rate);
    let period_fs = 1_000_000_000_000_000f64 / rate.max(1) as f64;
    let time = |sample: u64| (sample as f64 * period_fs / unit_fs as f64).round() as u64;
    let info = FstInfo {
        start_time: 0,
        timescale_exponent: exponent,
        version: request.generator.to_owned(),
        date: request.date.to_owned(),
        file_type: FstFileType::Verilog,
    };
    let mut header = fst_writer::open_fst(path, &info)?;
    header.scope("HSync", "", FstScopeType::Module)?;
    let ids: Vec<FstSignalId> = request
        .channels
        .iter()
        .map(|(_, name)| {
            header.var(
                name,
                FstSignalType::bit_vec(1),
                FstVarType::Wire,
                FstVarDirection::Implicit,
                None,
            )
        })
        .collect::<Result<_, _>>()?;
    header.up_scope()?;
    let mut body = header.finish()?;
    let mut changes = Changes::new(channels, len);
    let mut previous: Vec<Option<bool>> = vec![None; channels.len()];
    let mut rows = 0u64;
    while let Some((sample, levels)) = changes.advance() {
        rows += 1;
        if rows.is_multiple_of(65_536) && cancelled(progress, sample, len) {
            return Err(FileError::Cancelled);
        }
        body.time_change(time(sample))?;
        for (position, &level) in levels.iter().enumerate() {
            if previous[position] != Some(level) {
                body.signal_change(ids[position], if level { b"1" } else { b"0" })?;
                previous[position] = Some(level);
            }
        }
    }
    // The final timestamp marks the length, as the VCD does.
    body.time_change(time(len))?;
    body.finish()?;
    progress(1.0);
    Ok(())
}

/// A VCD identifier: `!`, `"`, … for the first 94 channels, then two characters.
fn vcd_id(position: usize) -> String {
    const FIRST: u8 = b'!';
    const COUNT: usize = 94;
    if position < COUNT {
        char::from(FIRST + position as u8).to_string()
    } else {
        let high = position / COUNT - 1;
        let low = position % COUNT;
        format!(
            "{}{}",
            char::from(FIRST + high as u8),
            char::from(FIRST + low as u8)
        )
    }
}

/// Transposes an 8×8 bit matrix held in a word, row `i` in byte `i` (Hacker's Delight 7-3).
fn transpose8(mut x: u64) -> u64 {
    let mut t = (x ^ (x >> 7)) & 0x00AA_00AA_00AA_00AA;
    x ^= t ^ (t << 7);
    t = (x ^ (x >> 14)) & 0x0000_CCCC_0000_CCCC;
    x ^= t ^ (t << 14);
    t = (x ^ (x >> 28)) & 0x0000_0000_F0F0_F0F0;
    x ^= t ^ (t << 28);
    x
}

/// Hands `sink` the samples interleaved, ⌈channels / 8⌉ bytes each, a few million at a time.
fn interleave(
    len: u64,
    channels: &[&ChannelData],
    progress: Progress<'_>,
    sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<usize, FileError> {
    let unit = channels.len().div_ceil(8).max(1);
    let words = len.div_ceil(64);
    let words_per_chunk = SAMPLES_PER_CHUNK / 64;
    let mut buffer = Vec::with_capacity((words_per_chunk * 64) as usize * unit);
    let mut word = 0;
    while word < words {
        if cancelled(progress, word * 64, len) {
            return Err(FileError::Cancelled);
        }
        buffer.clear();
        let end = (word + words_per_chunk).min(words);
        for at in word..end {
            let samples = (len - at * 64).min(64) as usize;
            let start = buffer.len();
            buffer.resize(start + 64 * unit, 0);
            for (group, chunk) in channels.chunks(8).enumerate() {
                let values: Vec<u64> = chunk.iter().map(|data| data.word(at as usize)).collect();
                for byte in 0..8 {
                    // Row k: byte `byte` of channel k's word, i.e. samples 8·byte… of channel k.
                    let rows = values.iter().enumerate().fold(0u64, |rows, (k, value)| {
                        rows | ((value >> (8 * byte)) & 0xff) << (8 * k)
                    });
                    let columns = transpose8(rows);
                    for bit in 0..8 {
                        buffer[start + (8 * byte + bit) * unit + group] =
                            (columns >> (8 * bit)) as u8;
                    }
                }
            }
            buffer.truncate(start + samples * unit);
        }
        sink(&buffer)?;
        word = end;
    }
    progress(1.0);
    Ok(unit)
}

fn write_sigrok<W: Write + Seek>(
    writer: W,
    capture: &LogicCapture,
    channels: &[&ChannelData],
    request: &ExportRequest<'_>,
    progress: Progress<'_>,
) -> Result<W, FileError> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1))
        .large_file(
            capture
                .len()
                .saturating_mul(channels.len().div_ceil(8) as u64)
                > u64::from(u32::MAX),
        );
    let mut zip = ZipWriter::new(writer);
    zip.start_file("version", options)?;
    zip.write_all(b"2")?;
    let mut entry = 0u32;
    let mut failed: Option<zip::result::ZipError> = None;
    let unit = {
        let mut sink = |bytes: &[u8]| -> std::io::Result<()> {
            entry += 1;
            if let Err(error) = zip.start_file(format!("logic-1-{entry}"), options) {
                let message = error.to_string();
                failed = Some(error);
                return Err(std::io::Error::other(message));
            }
            zip.write_all(bytes)
        };
        interleave(capture.len(), channels, progress, &mut sink)
    };
    if let Some(error) = failed {
        return Err(error.into());
    }
    let unit = unit?;
    let mut metadata = String::new();
    metadata.push_str("[global]\nsigrok version=0.5.2\n\n[device 1]\n");
    metadata.push_str("capturefile=logic-1\n");
    metadata.push_str(&format!("total probes={}\n", channels.len()));
    metadata.push_str(&format!(
        "samplerate={}\n",
        samplerate_text(capture.samplerate())
    ));
    metadata.push_str("total analog=0\n");
    for (position, (_, name)) in request.channels.iter().enumerate() {
        metadata.push_str(&format!("probe{}={name}\n", position + 1));
    }
    metadata.push_str(&format!("unitsize={unit}\n"));
    zip.start_file("metadata", options)?;
    zip.write_all(metadata.as_bytes())?;
    Ok(zip.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times_like_printf() {
        assert_eq!(general(0.0), "0");
        assert_eq!(general(4e-8), "4e-08");
        assert_eq!(general(1.0 / 25e6 * 3.0), "1.2e-07");
        assert_eq!(general(0.00184), "0.00184");
        assert_eq!(general(1.5), "1.5");
        assert_eq!(general(123456.0), "123456");
        assert_eq!(general(1e20), "1e+20");
    }

    #[test]
    fn names_sample_rates() {
        assert_eq!(samplerate_text(25_000_000), "25 MHz");
        assert_eq!(samplerate_text(12_500_000), "12.5 MHz");
        assert_eq!(samplerate_text(1_000_000_000), "1 GHz");
        assert_eq!(samplerate_text(50_000), "50 kHz");
        assert_eq!(samplerate_text(999), "999 Hz");
    }

    #[test]
    fn picks_a_timescale_that_divides_the_period() {
        assert_eq!(vcd_timescale(25_000_000), (10_000_000, "10 ns"));
        assert_eq!(vcd_timescale(100_000_000), (10_000_000, "10 ns"));
        assert_eq!(vcd_timescale(400_000_000), (100_000, "100 ps"));
        assert_eq!(vcd_timescale(1_000_000), (1_000_000_000, "1 us"));
        assert_eq!(vcd_timescale(3_000_000), (1_000, "1 ps"));
    }

    #[test]
    fn picks_a_power_of_ten_fst_unit() {
        assert_eq!(fst_timescale(25_000_000), (-8, 10_000_000));
        assert_eq!(fst_timescale(100_000_000), (-8, 10_000_000));
        assert_eq!(fst_timescale(400_000_000), (-10, 100_000));
        assert_eq!(fst_timescale(1_000_000), (-6, 1_000_000_000));
        assert_eq!(fst_timescale(3_000_000), (-12, 1_000));
    }

    #[test]
    fn names_vcd_channels() {
        assert_eq!(vcd_id(0), "!");
        assert_eq!(vcd_id(93), "~");
        assert_eq!(vcd_id(94), "!!");
    }
}
