//! SPI, ported from DSView's `1:spi` decoder (`decoders/1-spi/pd.py`): words are shifted in on
//! the sampling clock edge while chip select is active, and a chip select period is a transfer.

use std::ops::Range;

use hsync_capture::ChannelData;

use crate::sampler::{Pin, Sampler, Term};

/// Clock phase and polarity, bit order and framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub cpol: bool,
    pub cpha: bool,
    pub msb_first: bool,
    pub word_size: u8,
    pub cs_active_low: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cpol: false,
            cpha: false,
            msb_first: true,
            word_size: 8,
            cs_active_low: true,
        }
    }
}

/// The lines of the bus; only the clock is required, with at least one data line.
#[derive(Debug, Clone, Copy)]
pub struct Lines<'a> {
    pub clk: &'a ChannelData,
    pub miso: Option<&'a ChannelData>,
    pub mosi: Option<&'a ChannelData>,
    pub cs: Option<&'a ChannelData>,
}

/// One word on both data lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Word {
    pub start: u64,
    pub end: u64,
    pub miso: Option<u32>,
    pub mosi: Option<u32>,
}

/// A chip select period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub start: u64,
    /// `None` when the capture ended with chip select still active.
    pub end: Option<u64>,
    /// Chip select was already active when the capture began.
    pub cut_head: bool,
    /// The transfer's words in [`Trace::words`].
    pub words: std::ops::Range<usize>,
}

/// The decoded bus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    pub words: Vec<Word>,
    pub transfers: Vec<Transfer>,
}

const CLK: usize = 0;
const MISO: usize = 1;
const MOSI: usize = 2;
const CS: usize = 3;

/// Decodes the bus over `samples`.
pub fn decode(lines: &Lines<'_>, samples: Range<u64>, options: &Options) -> Trace {
    let mut trace = Trace::default();
    // Missing lines are stood in for by the clock; their values are never read.
    let channels = vec![
        lines.clk,
        lines.miso.unwrap_or(lines.clk),
        lines.mosi.unwrap_or(lines.clk),
        lines.cs.unwrap_or(lines.clk),
    ];
    let mut sampler = Sampler::new(channels, samples.end).from(samples.start);
    let end = sampler.len();
    if end <= sampler.first() || (lines.miso.is_none() && lines.mosi.is_none()) {
        return trace;
    }
    let sample_on_rising = options.cpol == options.cpha;
    let mut conditions = vec![vec![Term::Pin(
        CLK,
        if sample_on_rising {
            Pin::Rising
        } else {
            Pin::Falling
        },
    )]];
    let have_cs = lines.cs.is_some();
    if have_cs {
        conditions.push(vec![Term::Pin(CS, Pin::Either)]);
    }
    let asserted = |high: bool| high != options.cs_active_low;
    let word_size = u32::from(options.word_size.clamp(1, 32));

    let mut transfer: Option<(u64, bool, usize)> = None;
    let mut bits = 0u32;
    let mut word_start = 0u64;
    let mut last_bit = 0u64;
    let mut miso = 0u32;
    let mut mosi = 0u32;

    // What sigrok's first `wait({})` sees: the levels at sample 0.
    if !have_cs || asserted(sampler.level(CS, sampler.first())) {
        transfer = Some((0, true, 0));
    }
    let close = |trace: &mut Trace, transfer: (u64, bool, usize), at: Option<u64>| {
        trace.transfers.push(Transfer {
            start: transfer.0,
            end: at,
            cut_head: transfer.1,
            words: transfer.2..trace.words.len(),
        });
    };

    while let Some(found) = sampler.wait(&conditions) {
        let at = found.sample;
        if have_cs && found.has(1) {
            let active = asserted(sampler.pin(CS));
            if active {
                transfer = Some((at, false, trace.words.len()));
            } else if let Some(open) = transfer.take() {
                close(&mut trace, open, Some(at));
            }
            bits = 0;
            miso = 0;
            mosi = 0;
        }
        if have_cs && !asserted(sampler.pin(CS)) {
            continue;
        }
        if !found.has(0) {
            continue;
        }
        if bits == 0 {
            word_start = at;
        }
        let shift = if options.msb_first {
            word_size - 1 - bits
        } else {
            bits
        };
        if lines.miso.is_some() {
            miso |= u32::from(sampler.pin(MISO)) << shift;
        }
        if lines.mosi.is_some() {
            mosi |= u32::from(sampler.pin(MOSI)) << shift;
        }
        let width = if bits > 0 { at - last_bit } else { 0 };
        last_bit = at;
        bits += 1;
        if bits < word_size {
            continue;
        }
        trace.words.push(Word {
            start: word_start,
            end: at + width,
            miso: lines.miso.map(|_| miso),
            mosi: lines.mosi.map(|_| mosi),
        });
        bits = 0;
        miso = 0;
        mosi = 0;
    }
    if let Some(open) = transfer.take()
        && (have_cs || !trace.words.is_empty())
    {
        close(&mut trace, open, None);
    }
    trace
}
