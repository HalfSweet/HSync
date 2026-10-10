//! UART, after DSView's `1:uart` decoder (`decoders/1-uart/pd.py`): frames start at a falling
//! edge, bits are sampled at `start + (w - 1) / 2 + n·w` for a bit width of `w` samples.
//!
//! Unlike DSView, a frame already under way when the capture began is not decoded but
//! reported, and so is a frame the capture ends in.

use hsync_capture::ChannelData;

/// Parity of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
    Zero,
    One,
}

/// Frame format and timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub baudrate: f64,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: f64,
    pub msb_first: bool,
    /// The line idles low.
    pub invert: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            baudrate: 115_200.0,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1.0,
            msb_first: false,
            invert: false,
        }
    }
}

/// One character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// The start bit's falling edge.
    pub start: u64,
    /// Half a bit after the (last) stop bit's sample point.
    pub end: u64,
    pub value: u16,
    /// `None` without a parity bit.
    pub parity_ok: Option<bool>,
    /// The stop bit was not at the idle level.
    pub framing_error: bool,
}

/// The decoded line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    pub frames: Vec<Frame>,
    /// The line was busy when the capture began; frames before this sample were skipped.
    pub cut_head: Option<u64>,
    /// A frame starting here was still being sent when the capture ended.
    pub cut_tail: Option<u64>,
}

/// Decodes `line` over its first `end` samples at `samplerate`.
pub fn decode(line: &ChannelData, end: u64, samplerate: u64, options: &Options) -> Trace {
    let end = end.min(line.len());
    let width = samplerate as f64 / options.baudrate;
    let mut trace = Trace::default();
    if end == 0 || !width.is_finite() || width < 2.0 {
        return trace;
    }
    let idle = !options.invert;
    let level = |sample: u64| line.sample(sample) != options.invert;
    let next_edge = |from: u64| -> Option<u64> {
        let edge = if from == 0 {
            line.next_change(0)
        } else {
            line.next_change(from - 1)
        }?;
        (edge < end).then_some(edge)
    };
    let parity_bits = u64::from(options.parity != Parity::None);
    let data_bits = u64::from(options.data_bits);

    // A line not idle at the start is in the middle of a frame: wait for a full frame of idle
    // level before trusting falling edges again.
    let mut from = 0;
    if level(0) != idle {
        trace.cut_head = Some(0);
        let quiet = ((1 + data_bits + parity_bits) as f64 * width).ceil() as u64;
        let mut at = 0;
        loop {
            let Some(rise) = next_edge(at + 1) else {
                return trace;
            };
            let fall = next_edge(rise + 1);
            if fall.is_none_or(|fall| fall - rise >= quiet) {
                from = rise;
                break;
            }
            at = fall.unwrap_or(end);
        }
    }

    let sample_point = |frame: u64, bit: u64| -> u64 {
        (frame as f64 + (width - 1.0) / 2.0 + bit as f64 * width).ceil() as u64
    };
    loop {
        // The start bit: a change away from the idle level.
        let mut search = from;
        let start = loop {
            let Some(edge) = next_edge(search) else {
                return trace;
            };
            if level(edge) != idle {
                break edge;
            }
            search = edge + 1;
        };
        let stop_bit = 1 + data_bits + parity_bits;
        let last = sample_point(start, stop_bit);
        if last >= end {
            trace.cut_tail = Some(start);
            return trace;
        }
        if level(sample_point(start, 0)) == idle {
            // A glitch rather than a start bit.
            from = sample_point(start, 0);
            continue;
        }
        let mut value = 0u16;
        let mut ones = 0;
        for bit in 0..data_bits {
            let high = level(sample_point(start, 1 + bit));
            ones += u32::from(high);
            let position = if options.msb_first {
                data_bits - 1 - bit
            } else {
                bit
            };
            value |= u16::from(high) << position;
        }
        let parity_ok = (options.parity != Parity::None).then(|| {
            let bit = level(sample_point(start, 1 + data_bits));
            match options.parity {
                Parity::Odd => (ones + u32::from(bit)) % 2 == 1,
                Parity::Even => (ones + u32::from(bit)) % 2 == 0,
                Parity::Zero => !bit,
                Parity::One | Parity::None => bit,
            }
        });
        let framing_error = level(last) != idle;
        let half = (width / 2.0).ceil() as u64;
        let extra_stop = ((options.stop_bits - 1.0).max(0.0) * width).round() as u64;
        trace.frames.push(Frame {
            start,
            end: (last + half + extra_stop).min(end),
            value,
            parity_ok,
            framing_error,
        });
        from = last;
    }
}
