//! HDL waveform files: VCD, FST and GHW, as simulators and the logic analyzers inside FPGAs
//! (Vivado's ILA, Quartus' Signal Tap) write them. wellen reads them; the signals are then laid
//! onto the sample grid of a [`LogicCapture`], one channel per bit.
//!
//! A waveform is a list of value changes at arbitrary times, a capture a level per sample at a
//! fixed rate. Left to itself, the grid is the coarsest that lands every timestamp of the file
//! on a sample: one sample every greatest common divisor of the time steps. Logic analyzer
//! exports sample on a clock, so their grid is that clock, or a multiple of it when few
//! signals change; the user may name the rate instead. Simulations with events at odd times
//! get a finer grid, coarsened to [`MAX_SAMPLES`] samples when it would be too fine to hold.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, Seek};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use hsync_capture::{ChannelData, LogicCapture};
use hsync_core::{ChannelIndex, SampleIndex};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use wellen::viewers::{self, HeaderResult, ProgressCount};
use wellen::{
    FileFormat, Hierarchy, Item, ItemRef, LoadOptions, Signal, SignalEncoding, SignalRef,
    SignalSource, SignalValueRef, Time, TimeTable, Timescale, Var, VarIndex, VarRef, WellenError,
};

use crate::FileError;
use crate::progress::Progress;

/// Most samples an imported capture holds per channel; a finer time grid is coarsened to fit.
pub const MAX_SAMPLES: SampleIndex = 1 << 30;

/// The shortest sample period, as the VCD export's finest unit: 1 fs.
const MIN_PERIOD_S: f64 = 1e-15;

/// Seconds of one time unit when the file names none: Verilog's default time unit.
const DEFAULT_UNIT_S: f64 = 1e-9;

/// Signals loaded at once: their changes are held in memory until laid out.
const SIGNAL_BATCH: usize = 16;

/// Bits of a vector laid out in one pass over its changes.
const BIT_GROUP: usize = 64;

/// Samples laid out between two progress reports.
const REPORT_EVERY: SampleIndex = 1 << 20;

/// Told how far a read got, from 0 to 1. Reading cannot be stopped, so it says nothing back.
pub type ReadProgress<'a> = &'a mut dyn FnMut(f64);

/// The formats read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WaveformFormat {
    /// Value Change Dump, the text format every tool writes.
    Vcd,
    /// GTKWave's compact binary format.
    Fst,
    /// GHDL's waveform format.
    Ghw,
}

/// What an entry of a waveform file's hierarchy is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum WaveformItemKind {
    /// A module, block or other scope: the items after it with a greater depth lie inside it.
    Scope,
    /// A logic signal; each of its bits becomes a channel.
    Logic,
    /// A real, string or event signal, which has no logic level to import.
    Other,
}

/// A scope or signal of a waveform file, listed in the file's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WaveformItem {
    /// How many scopes the item lies inside.
    pub depth: u32,
    pub name: String,
    pub kind: WaveformItemKind,
    /// Names a logic signal in [`ImportRequest::signals`].
    pub signal: Option<u32>,
    /// Bits of a logic signal.
    pub width: u32,
    /// Declared bit range of a vector, most significant bit first (`[7:0]`).
    pub range: Option<(i64, i64)>,
}

/// What the import dialog shows of a waveform file.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WaveformSummary {
    pub format: WaveformFormat,
    /// Samples per second of the grid the signals are laid on when no rate is named.
    pub samplerate: u64,
    /// Samples every channel will hold at that rate.
    pub sample_count: u64,
    /// The grid is coarser than the file's time steps, to hold at most [`MAX_SAMPLES`] samples:
    /// changes closer together than one sample fall together.
    pub coarsened: bool,
    /// Seconds of one time unit of the file.
    pub unit: f64,
    /// Time units from the first timestamp to the last.
    pub span: u64,
    /// The greatest common divisor of the time steps, in time units: the finest grid every
    /// timestamp falls on.
    pub finest: u64,
    /// The scopes and signals of the file, in its order.
    pub items: Vec<WaveformItem>,
}

/// Which signals to import.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportRequest {
    /// Logic signals to import, by [`WaveformItem::signal`], in channel order; the bits of a
    /// vector follow each other, least significant first.
    pub signals: Vec<u32>,
    /// Samples per second to lay the signals on, rounded to whole time units of the file;
    /// without one, the rate of [`WaveformSummary::samplerate`].
    pub samplerate: Option<u64>,
}

/// A capture laid out from a waveform file.
#[derive(Debug)]
pub struct ImportedCapture {
    pub capture: LogicCapture,
    /// Name of every channel: the signal's name, `name[k]` for bit `k` of a vector, and the
    /// full hierarchical name where two signals share a name.
    pub channel_names: BTreeMap<ChannelIndex, String>,
}

/// The timestamps of a file: where they start, how far they reach, their unit and the finest
/// grid they all fall on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Timing {
    first: Time,
    /// Time units from the first timestamp to the last.
    span: u64,
    /// Seconds of one time unit.
    unit: f64,
    /// The greatest common divisor of the time steps, in time units; 1 for a lone timestamp.
    finest: u64,
}

impl Timing {
    fn new(time_table: &[Time], timescale: Option<Timescale>) -> Self {
        let first = time_table.first().copied().unwrap_or(0);
        let last = time_table.last().copied().unwrap_or(first);
        let finest = time_table
            .windows(2)
            .fold(0, |step, pair| gcd(step, pair[1] - pair[0]))
            .max(1);
        let unit = timescale
            .and_then(|scale| {
                scale
                    .unit
                    .to_exponent()
                    .map(|exponent| f64::from(scale.factor) * 10f64.powi(i32::from(exponent)))
            })
            .unwrap_or(DEFAULT_UNIT_S);
        Self {
            first,
            span: last - first,
            unit,
            finest,
        }
    }

    /// The coarsest grid every timestamp falls on, within [`MAX_SAMPLES`].
    fn grid(&self) -> Grid {
        self.grid_of(self.finest)
    }

    /// The grid of `samplerate` samples per second, rounded to whole time units and kept
    /// within [`MAX_SAMPLES`].
    fn grid_at(&self, samplerate: u64) -> Grid {
        let step = (1.0 / (samplerate.max(1) as f64 * self.unit)).round();
        // Rates beyond the time unit get a sample per unit; absurdly slow ones one sample.
        self.grid_of(if step.is_finite() {
            step.max(1.0) as u64
        } else {
            1
        })
    }

    fn grid_of(&self, step: u64) -> Grid {
        let mut step = step.max(1);
        // Too many samples: the smallest multiple of the step that fits.
        let needed = self.span.div_ceil(MAX_SAMPLES - 1).max(1);
        if needed > step {
            step *= needed.div_ceil(step);
        }
        // Below a femtosecond no unit names the period; coarsen to it.
        let period = step as f64 * self.unit;
        if period < MIN_PERIOD_S {
            step *= (MIN_PERIOD_S / period).ceil() as u64;
        }
        let period = step as f64 * self.unit;
        Grid {
            first: self.first,
            step,
            len: self.span / step + 1,
            samplerate: (1.0 / period).round().max(1.0) as u64,
            coarsened: !self.finest.is_multiple_of(step),
        }
    }
}

/// A sample grid over a file's timestamps: one sample every `step` time units from `first`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Grid {
    first: Time,
    step: u64,
    len: SampleIndex,
    samplerate: u64,
    /// Some timestamps fall between samples: changes closer together than one fall together.
    coarsened: bool,
}

impl Grid {
    /// The sample `time` falls in.
    fn sample(&self, time: Time) -> SampleIndex {
        time.saturating_sub(self.first) / self.step
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// A waveform file read into memory, ready to lay its signals onto the sample grid.
pub struct Waveform {
    format: WaveformFormat,
    hierarchy: Hierarchy,
    source: SignalSource,
    time_table: TimeTable,
    timing: Timing,
    /// The logic signals, by the number [`WaveformItem::signal`] gives them.
    logic: BTreeMap<u32, VarRef>,
}

impl std::fmt::Debug for Waveform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waveform")
            .field("format", &self.format)
            .field("timing", &self.timing)
            .field("signals", &self.logic.len())
            .finish_non_exhaustive()
    }
}

fn load_options() -> LoadOptions {
    LoadOptions {
        multi_thread: true,
        remove_scopes_with_empty_name: true,
    }
}

/// Reads the waveform file at `path`.
pub fn read_waveform_file(path: &Path, progress: ReadProgress<'_>) -> Result<Waveform, FileError> {
    // wellen panics on a file it cannot open; find that out first.
    File::open(path)?;
    let header = viewers::read_header_from_file(path, &load_options()).map_err(wellen_error)?;
    finish_read(header, progress)
}

/// Reads a waveform from memory or another reader.
pub fn read_waveform<R: BufRead + Seek + Send + Sync + 'static>(
    reader: R,
    progress: ReadProgress<'_>,
) -> Result<Waveform, FileError> {
    let header = viewers::read_header(reader, &load_options()).map_err(wellen_error)?;
    finish_read(header, progress)
}

fn finish_read<R: BufRead + Seek + Send + Sync + 'static>(
    header: HeaderResult<R>,
    progress: ReadProgress<'_>,
) -> Result<Waveform, FileError> {
    let HeaderResult {
        hierarchy,
        file_format,
        body_len,
        body,
    } = header;
    let format = match file_format {
        FileFormat::Vcd => WaveformFormat::Vcd,
        FileFormat::Fst => WaveformFormat::Fst,
        FileFormat::Ghw => WaveformFormat::Ghw,
        FileFormat::Unknown => return Err(FileError::NotWaveform),
    };
    // The body is read on a thread of its own so its progress can be reported meanwhile.
    let count = ProgressCount::default();
    let read = thread::scope(|scope| {
        let reader = scope.spawn(|| viewers::read_body(body, &hierarchy, Some(Arc::clone(&count))));
        while !reader.is_finished() {
            if body_len > 0 {
                progress((count.load(Ordering::Relaxed) as f64 / body_len as f64).min(1.0));
            }
            thread::sleep(Duration::from_millis(50));
        }
        reader.join()
    });
    let body = match read {
        Ok(result) => result.map_err(wellen_error)?,
        Err(_) => return Err(FileError::Waveform("the reader panicked".to_owned())),
    };
    progress(1.0);
    if body.time_table.is_empty() {
        return Err(FileError::EmptyWaveform);
    }
    let timing = Timing::new(&body.time_table, hierarchy.timescale());
    let logic = hierarchy
        .all_vars()
        .filter(|var| logic_width(&hierarchy[*var], &hierarchy).is_some())
        .map(|var| (var.index() as u32, var))
        .collect();
    Ok(Waveform {
        format,
        hierarchy,
        source: body.source,
        time_table: body.time_table,
        timing,
        logic,
    })
}

fn wellen_error(error: WellenError) -> FileError {
    match error {
        WellenError::UnknownFileFormat => FileError::NotWaveform,
        WellenError::Io(error) => FileError::Io(error),
        WellenError::FailedToLoad(_, detail) => FileError::Waveform(detail),
    }
}

/// Bits of a logic signal; `None` for reals, strings and events.
fn logic_width(var: &Var, hierarchy: &Hierarchy) -> Option<u32> {
    match var.signal_encoding(hierarchy) {
        SignalEncoding::BitVector(width) if width > 0 => Some(width),
        _ => None,
    }
}

impl Waveform {
    pub fn format(&self) -> WaveformFormat {
        self.format
    }

    /// Samples per second of the grid the timestamps give.
    pub fn samplerate(&self) -> u64 {
        self.timing.grid().samplerate
    }

    /// Samples every imported channel holds at that rate.
    pub fn sample_count(&self) -> SampleIndex {
        self.timing.grid().len
    }

    /// What the import dialog shows.
    pub fn summary(&self) -> WaveformSummary {
        let mut items = Vec::new();
        self.list(&mut self.hierarchy.items(), 0, &mut items);
        let grid = self.timing.grid();
        WaveformSummary {
            format: self.format,
            samplerate: grid.samplerate,
            sample_count: grid.len,
            coarsened: grid.coarsened,
            unit: self.timing.unit,
            span: self.timing.span,
            finest: self.timing.finest,
            items,
        }
    }

    fn list(
        &self,
        refs: &mut dyn Iterator<Item = ItemRef>,
        depth: u32,
        out: &mut Vec<WaveformItem>,
    ) {
        let hierarchy = &self.hierarchy;
        for item in refs {
            match item.deref(hierarchy) {
                Item::Scope(scope) => {
                    out.push(WaveformItem {
                        depth,
                        name: scope.name(hierarchy).to_owned(),
                        kind: WaveformItemKind::Scope,
                        signal: None,
                        width: 0,
                        range: None,
                    });
                    self.list(&mut scope.items(hierarchy), depth + 1, out);
                }
                Item::Var(var) => {
                    let ItemRef::Var(var_ref) = item else {
                        continue;
                    };
                    let width = logic_width(var, hierarchy);
                    out.push(WaveformItem {
                        depth,
                        name: var.name(hierarchy).to_owned(),
                        kind: if width.is_some() {
                            WaveformItemKind::Logic
                        } else {
                            WaveformItemKind::Other
                        },
                        signal: width.map(|_| var_ref.index() as u32),
                        width: width.unwrap_or(0),
                        range: var.index().map(|index| (index.msb(), index.lsb())),
                    });
                }
            }
        }
    }

    /// Lays the signals of `request` onto the grid, in its order, telling `progress` how far it
    /// got and stopping when it says so.
    pub fn import(
        &mut self,
        request: &ImportRequest,
        progress: Progress<'_>,
    ) -> Result<ImportedCapture, FileError> {
        let mut vars: Vec<VarRef> = Vec::with_capacity(request.signals.len());
        for &signal in &request.signals {
            let var = *self
                .logic
                .get(&signal)
                .ok_or(FileError::UnknownSignal(signal))?;
            if !vars.contains(&var) {
                vars.push(var);
            }
        }
        if vars.is_empty() {
            return Err(FileError::NoSignal);
        }
        let names = channel_names(&self.hierarchy, &vars);
        let total: usize = names.iter().map(Vec::len).sum();
        let channels =
            ChannelIndex::try_from(total).map_err(|_| FileError::TooManyChannels(total))?;
        let grid = match request.samplerate {
            Some(samplerate) => self.timing.grid_at(samplerate),
            None => self.timing.grid(),
        };
        let mut capture = LogicCapture::new(grid.samplerate, 0..channels);
        let mut first_channel: ChannelIndex = 0;
        for (batch, batch_names) in vars.chunks(SIGNAL_BATCH).zip(names.chunks(SIGNAL_BATCH)) {
            let mut refs: Vec<SignalRef> = batch
                .iter()
                .map(|var| self.hierarchy[*var].signal_ref())
                .collect();
            refs.sort_unstable();
            refs.dedup();
            let signals: HashMap<SignalRef, Signal> = self
                .source
                .load_signals(&refs, &self.hierarchy, true)
                .into_iter()
                .map(|signal| (signal.signal_ref(), signal))
                .collect();
            for (var, var_names) in batch.iter().zip(batch_names) {
                let signal = signals.get(&self.hierarchy[*var].signal_ref());
                let done = usize::from(first_channel);
                let mut report = |fraction: f64| progress((done as f64 + fraction) / total as f64);
                for start in (0..var_names.len()).step_by(BIT_GROUP) {
                    let bits = start..(start + BIT_GROUP).min(var_names.len());
                    let channel = first_channel + start as ChannelIndex;
                    self.lay_out(signal, bits, grid, &mut capture, channel, &mut report)?;
                }
                first_channel += var_names.len() as ChannelIndex;
            }
        }
        progress(1.0);
        let channel_names = names
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, name)| (index as ChannelIndex, name))
            .collect();
        Ok(ImportedCapture {
            capture,
            channel_names,
        })
    }

    /// Lays bits `bits` of `signal` onto the channels from `first_channel` on; without a signal
    /// the channels stay low.
    fn lay_out(
        &self,
        signal: Option<&Signal>,
        bits: Range<usize>,
        grid: Grid,
        capture: &mut LogicCapture,
        first_channel: ChannelIndex,
        progress: &mut dyn FnMut(f64) -> bool,
    ) -> Result<(), FileError> {
        let count = bits.len();
        let mut packers: Vec<RunPacker> = (0..count).map(|_| RunPacker::default()).collect();
        let mut levels = vec![false; count];
        let mut reported = 0;
        for (time_index, value) in signal.into_iter().flat_map(Signal::iter_changes) {
            let sample = grid.sample(self.time_table[time_index as usize]);
            for (bit, packer) in packers.iter_mut().enumerate() {
                let data = channel_of(capture, first_channel + bit as ChannelIndex)?;
                packer.extend(levels[bit], sample, data);
            }
            match value {
                SignalValueRef::BitVec(vector) => {
                    for (bit, level) in levels.iter_mut().enumerate() {
                        let position = (bits.start + bit) as u32;
                        *level = position < vector.width()
                            && is_high(vector.get_bit(position).as_ascii());
                    }
                }
                _ => levels.fill(false),
            }
            if sample - reported >= REPORT_EVERY {
                reported = sample;
                if !progress(sample as f64 / grid.len as f64) {
                    return Err(FileError::Cancelled);
                }
            }
        }
        for (bit, packer) in packers.iter_mut().enumerate() {
            let data = channel_of(capture, first_channel + bit as ChannelIndex)?;
            packer.extend(levels[bit], grid.len, data);
            packer.finish(data);
        }
        if !progress(1.0) {
            return Err(FileError::Cancelled);
        }
        Ok(())
    }
}

/// Channel `index` of `capture`, which [`Waveform::import`] created.
fn channel_of(
    capture: &mut LogicCapture,
    index: ChannelIndex,
) -> Result<&mut ChannelData, FileError> {
    capture
        .channel_mut(index)
        .ok_or_else(|| FileError::Waveform(format!("channel {index} missing")))
}

/// Whether a bit of a 2-, 4- or 9-state value, as wellen letters it, counts as high: `1` and
/// the weak `h`. Unknown, high-impedance and the rest are low, as a logic analyzer sees them.
fn is_high(bit: char) -> bool {
    matches!(bit, '1' | 'h')
}

/// Names the channels of `vars`, one list per signal: its name, `name[k]` for the bits of a
/// vector, and the full hierarchical name where two signals share a name.
fn channel_names(hierarchy: &Hierarchy, vars: &[VarRef]) -> Vec<Vec<String>> {
    let mut shared: HashMap<&str, usize> = HashMap::new();
    for var in vars {
        *shared.entry(hierarchy[*var].name(hierarchy)).or_default() += 1;
    }
    vars.iter()
        .map(|&var_ref| {
            let var = &hierarchy[var_ref];
            let base = if shared.get(var.name(hierarchy)).copied().unwrap_or(0) > 1 {
                var.full_name(hierarchy)
            } else {
                var.name(hierarchy).to_owned()
            };
            let width = logic_width(var, hierarchy).unwrap_or(1);
            match var.index() {
                Some(index) => (0..width)
                    .map(|bit| format!("{base}[{}]", bit_index(index, bit)))
                    .collect(),
                None if width == 1 => vec![base],
                None => (0..width).map(|bit| format!("{base}[{bit}]")).collect(),
            }
        })
        .collect()
}

/// Declared index of bit `bit` (0 the least significant) of a vector declared `[msb:lsb]`.
fn bit_index(index: VarIndex, bit: u32) -> i64 {
    if index.msb() >= index.lsb() {
        index.lsb() + i64::from(bit)
    } else {
        index.lsb() - i64::from(bit)
    }
}

/// Packs runs of one level into 64-sample words, handing complete words to a channel as they
/// pile up.
#[derive(Debug, Default)]
struct RunPacker {
    /// Complete words not handed on yet, then the word being filled.
    words: Vec<u64>,
    /// Samples packed so far.
    len: SampleIndex,
    /// Complete words handed on.
    flushed: usize,
}

impl RunPacker {
    /// Complete words kept before they are handed on.
    const FLUSH_WORDS: usize = 1 << 14;

    /// Packs samples up to `until` (exclusive) at `level`.
    fn extend(&mut self, level: bool, until: SampleIndex, data: &mut ChannelData) {
        while self.len < until {
            let offset = (self.len % 64) as u32;
            let remaining = until - self.len;
            if offset != 0 {
                // Finish the word being filled.
                let take = remaining.min(u64::from(64 - offset)) as u32;
                if level && let Some(word) = self.words.last_mut() {
                    *word |= low_bits(take) << offset;
                }
                self.len += u64::from(take);
            } else if remaining >= 64 {
                let words = (remaining / 64).min((Self::FLUSH_WORDS - self.complete()) as u64);
                let fill = if level { !0 } else { 0 };
                self.words.extend(std::iter::repeat_n(fill, words as usize));
                self.len += words * 64;
            } else {
                self.words
                    .push(if level { low_bits(remaining as u32) } else { 0 });
                self.len += remaining;
            }
            if self.complete() >= Self::FLUSH_WORDS {
                self.flush(data);
            }
        }
    }

    /// Complete words not handed on yet.
    fn complete(&self) -> usize {
        (self.len / 64) as usize - self.flushed
    }

    fn flush(&mut self, data: &mut ChannelData) {
        let complete = self.complete();
        data.append_words(&self.words[..complete]);
        self.words.drain(..complete);
        self.flushed += complete;
    }

    /// Hands the rest on, ending the channel inside a word if the samples do.
    fn finish(&mut self, data: &mut ChannelData) {
        self.flush(data);
        if !self.words.is_empty() {
            data.append_words(&self.words);
            self.words.clear();
            data.truncate(self.len);
        }
    }
}

/// A word with its low `count` bits set.
fn low_bits(count: u32) -> u64 {
    if count >= 64 { !0 } else { (1u64 << count) - 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wellen::TimescaleUnit;

    #[test]
    fn grid_takes_the_common_divisor_of_the_time_steps() {
        let scale = Some(Timescale::new(1, TimescaleUnit::PicoSeconds));
        let timing = Timing::new(&[0, 10_000, 20_000, 25_000, 40_000], scale);
        assert_eq!(
            (timing.span, timing.unit, timing.finest),
            (40_000, 1e-12, 5_000)
        );
        let grid = timing.grid();
        assert_eq!(grid.step, 5_000);
        assert_eq!(grid.len, 9);
        assert_eq!(grid.samplerate, 200_000_000);
        assert!(!grid.coarsened);
        assert_eq!(grid.sample(25_000), 5);
    }

    #[test]
    fn grid_starts_at_the_first_timestamp_and_honours_the_factor() {
        let scale = Some(Timescale::new(10, TimescaleUnit::NanoSeconds));
        let grid = Timing::new(&[300, 304, 308], scale).grid();
        assert_eq!((grid.first, grid.step, grid.len), (300, 4, 3));
        assert_eq!(grid.samplerate, 25_000_000);
        assert_eq!(grid.sample(304), 1);
    }

    #[test]
    fn grid_follows_a_named_rate() {
        let scale = Some(Timescale::new(1, TimescaleUnit::PicoSeconds));
        let timing = Timing::new(&[0, 30_000, 60_000], scale);
        assert_eq!(timing.grid().samplerate, 33_333_333);
        let grid = timing.grid_at(100_000_000);
        assert_eq!(
            (grid.step, grid.len, grid.samplerate),
            (10_000, 7, 100_000_000)
        );
        assert!(!grid.coarsened);
        // A coarser rate than the timestamps: they fall between samples.
        let coarse = timing.grid_at(25_000_000);
        assert_eq!((coarse.step, coarse.len), (40_000, 2));
        assert!(coarse.coarsened);
        // Finer than the time unit: one sample per unit.
        assert_eq!(timing.grid_at(5_000_000_000_000).step, 1);
    }

    #[test]
    fn grid_coarsens_to_the_sample_limit() {
        let scale = Some(Timescale::new(1, TimescaleUnit::PicoSeconds));
        let timing = Timing::new(&[0, 1, 3 * MAX_SAMPLES], scale);
        let grid = timing.grid();
        assert!(grid.coarsened);
        assert_eq!(grid.step, 4);
        assert!(grid.len <= MAX_SAMPLES);
        assert_eq!(grid.samplerate, 250_000_000_000);
        assert!(timing.grid_at(1_000_000_000_000).len <= MAX_SAMPLES);
    }

    #[test]
    fn grid_assumes_nanoseconds_and_a_single_timestamp() {
        let grid = Timing::new(&[7], None).grid();
        assert_eq!(
            (grid.step, grid.len, grid.samplerate),
            (1, 1, 1_000_000_000)
        );
    }

    #[test]
    fn grid_stays_at_a_femtosecond() {
        let scale = Some(Timescale::new(1, TimescaleUnit::AttoSeconds));
        let grid = Timing::new(&[0, 1, 2], scale).grid();
        assert_eq!(grid.step, 1000);
        assert!(grid.coarsened);
        assert_eq!(grid.samplerate, 1_000_000_000_000_000);
    }

    #[test]
    fn packer_fills_runs_across_words() {
        let mut data = ChannelData::new();
        let mut packer = RunPacker::default();
        packer.extend(false, 3, &mut data);
        packer.extend(true, 70, &mut data);
        packer.extend(false, 130, &mut data);
        packer.finish(&mut data);
        assert_eq!(data.len(), 130);
        assert_eq!(data.word(0), !0 << 3);
        assert_eq!(data.word(1), 0b11_1111);
        assert_eq!(data.word(2), 0);
        assert_eq!(data.next_change(0), Some(3));
        assert_eq!(data.next_change(3), Some(70));
    }

    #[test]
    fn packer_hands_long_runs_on_in_pieces() {
        let mut data = ChannelData::new();
        let mut packer = RunPacker::default();
        let samples = (RunPacker::FLUSH_WORDS as u64 * 64) * 3 + 5;
        packer.extend(true, samples, &mut data);
        packer.finish(&mut data);
        assert_eq!(data.len(), samples);
        assert!(data.sample(samples - 1));
        assert_eq!(data.next_change(0), None);
    }

    #[test]
    fn names_bits_by_their_declared_index() {
        assert_eq!(bit_index(VarIndex::new(7, 0), 0), 0);
        assert_eq!(bit_index(VarIndex::new(7, 0), 7), 7);
        assert_eq!(bit_index(VarIndex::new(0, 7), 0), 7);
        assert_eq!(bit_index(VarIndex::new(4, 1), 2), 3);
    }
}
