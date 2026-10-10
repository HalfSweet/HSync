//! CAN and CAN FD, after DSView's `can-fd` decoder (`decoders/can-fd/pd.py`): bits are sampled
//! at the sample point from the last recessive-to-dominant edge, the bit numbering follows that
//! decoder, and stuff bits are removed the same way.
//!
//! Where DSView takes shortcuts this follows ISO 11898-1:2015 instead: the bit rate switches at
//! the sample points of BRS and the CRC delimiter, CRC-15/17/21 are checked, frames wait for
//! bus integration (11 recessive bits), and six equal bits are a stuff error.

use hsync_capture::ChannelData;

/// Bit timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub nominal_bitrate: f64,
    /// Bit rate of the data phase of FD frames with BRS set.
    pub fast_bitrate: f64,
    /// Sample point in percent of a bit.
    pub sample_point: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            nominal_bitrate: 1_000_000.0,
            fast_bitrate: 2_000_000.0,
            sample_point: 70.0,
        }
    }
}

/// Why a frame was not received as sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// Six equal bits where stuffing applies.
    Stuff,
    /// A delimiter or the end of frame was not recessive.
    Form,
}

/// One frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The start-of-frame edge.
    pub start: u64,
    /// The end of the last bit received.
    pub end: u64,
    pub id: u32,
    pub extended: bool,
    pub fd: bool,
    pub brs: bool,
    pub esi: bool,
    pub remote: bool,
    pub dlc: u8,
    pub data: Vec<u8>,
    /// Whether the CRC sequence matches; `false` too when the frame ended before it.
    pub crc_ok: bool,
    /// A receiver drove the ACK slot dominant.
    pub acked: bool,
    pub error: Option<FrameError>,
}

/// The decoded bus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    pub frames: Vec<Frame>,
    /// The bus was busy when the capture began: activity before this sample was skipped.
    pub cut_head: Option<u64>,
    /// A frame starting here was still on the bus when the capture ended.
    pub cut_tail: Option<u64>,
}

/// Payload bytes for a data length code.
pub fn dlc_len(dlc: u8) -> usize {
    const LENGTHS: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];
    LENGTHS[usize::from(dlc & 0x0f)]
}

/// An MSB-first CRC register over a stream of bits.
struct Crc {
    width: u32,
    poly: u32,
    value: u32,
}

impl Crc {
    fn new(width: u32, poly: u32, init: u32) -> Self {
        Self {
            width,
            poly,
            value: init,
        }
    }

    fn push(&mut self, bit: bool) {
        let top = (self.value >> (self.width - 1)) & 1 == 1;
        self.value = (self.value << 1) & ((1 << self.width) - 1);
        if bit != top {
            self.value ^= self.poly;
        }
    }

    fn of(mut self, bits: impl IntoIterator<Item = bool>) -> u32 {
        for bit in bits {
            self.push(bit);
        }
        self.value
    }
}

fn pack(bits: &[bool]) -> u32 {
    bits.iter()
        .fold(0, |value, &bit| (value << 1) | u32::from(bit))
}

/// Gray code of the stuff bit count modulo 8.
fn gray(count: u32) -> u32 {
    let count = count % 8;
    count ^ (count >> 1)
}

/// Bit timing in samples.
#[derive(Debug, Clone, Copy)]
struct Timing {
    width: f64,
    sample: f64,
}

/// Where the bits of a frame are, once the control field told us.
#[derive(Debug, Clone, Copy)]
struct Layout {
    dlc_start: usize,
    last_data_bit: usize,
    /// Destuffed index of the CRC delimiter.
    crc_delimiter: usize,
}

/// Decodes the bus line over its first `end` samples at `samplerate`.
pub fn decode(line: &ChannelData, end: u64, samplerate: u64, options: &Options) -> Trace {
    let end = end.min(line.len());
    let mut trace = Trace::default();
    let timing = |bitrate: f64| {
        let width = samplerate as f64 / bitrate;
        Timing {
            width,
            sample: width * options.sample_point / 100.0,
        }
    };
    let nominal = timing(options.nominal_bitrate);
    let fast = timing(options.fast_bitrate);
    if end == 0 || !nominal.width.is_finite() || nominal.width < 3.0 {
        return trace;
    }
    let level = |sample: u64| line.sample(sample);
    // The first recessive-to-dominant edge at or after `from`.
    let falling = |from: u64| -> Option<u64> {
        let mut at = from;
        loop {
            let edge = if at == 0 {
                line.next_change(0)
            } else {
                line.next_change(at - 1)
            }?;
            if edge >= end {
                return None;
            }
            if !level(edge) {
                return Some(edge);
            }
            at = edge + 1;
        }
    };
    let integration = (11.0 * nominal.width).ceil() as u64;

    // Bus integration: a frame counts only after 11 recessive bits, or right after the
    // intermission of a clean frame.
    let mut from = 0u64;
    let mut idle_since = level(0).then_some(0u64);
    let mut integrated = false;
    loop {
        let since = match idle_since {
            Some(since) => since,
            None => {
                let Some(rise) = line.next_change(from).filter(|&rise| rise < end) else {
                    return trace;
                };
                from = rise;
                rise
            }
        };
        let Some(sof) = falling(from) else {
            return trace;
        };
        if !integrated && sof - since < integration {
            // Part of a frame that started before we could see it.
            trace.cut_head.get_or_insert(if level(0) { sof } else { 0 });
            idle_since = None;
            from = sof;
            continue;
        }
        match decode_frame(line, end, sof, nominal, fast) {
            Decoded::Frame(frame, resume) => {
                integrated = frame.error.is_none();
                trace.frames.push(frame);
                from = resume.min(end - 1);
                idle_since = level(from).then_some(from);
            }
            Decoded::CutOff => {
                trace.cut_tail = Some(sof);
                return trace;
            }
        }
    }
}

enum Decoded {
    /// A frame, and where to look for the next one.
    Frame(Frame, u64),
    /// The capture ended inside the frame.
    CutOff,
}

#[allow(clippy::too_many_lines)]
fn decode_frame(line: &ChannelData, end: u64, sof: u64, nominal: Timing, fast: Timing) -> Decoded {
    let mut timing = nominal;
    // Bit `reference_bit` (raw count, stuff bits included) starts at sample `reference`.
    let mut reference = sof as f64;
    let mut reference_bit = 0usize;
    let mut raw: Vec<bool> = Vec::with_capacity(128);
    let mut bits: Vec<bool> = Vec::with_capacity(128);
    let mut stuff_count = 0u32;
    let mut last_sample = sof;
    let mut layout: Option<Layout> = None;
    let mut frame = Frame {
        start: sof,
        end: sof,
        id: 0,
        extended: false,
        fd: false,
        brs: false,
        esi: false,
        remote: false,
        dlc: 0,
        data: Vec::new(),
        crc_ok: false,
        acked: false,
        error: None,
    };
    // Raw index of the last data bit, for the FD CRC.
    let mut raw_last_data = 0usize;

    loop {
        let bit_index = raw.len();
        let position =
            (reference + timing.width * (bit_index - reference_bit) as f64 + timing.sample) as u64;
        if position >= end {
            return Decoded::CutOff;
        }
        // Hard synchronisation on recessive-to-dominant edges before the sample point.
        let mut at = last_sample + 1;
        let mut resynced = false;
        while at <= position {
            let Some(edge) = line.next_change(at - 1).filter(|&edge| edge <= position) else {
                break;
            };
            if !line.sample(edge) && edge > last_sample {
                reference = edge as f64;
                reference_bit = bit_index;
                resynced = true;
                break;
            }
            at = edge + 1;
        }
        if resynced {
            let resampled = (reference + timing.sample) as u64;
            if resampled != position {
                continue;
            }
        }
        let bit = line.sample(position);
        last_sample = position;
        raw.push(bit);
        frame.end = (position as f64 + timing.width - timing.sample) as u64;

        // Dynamic stuffing applies up to the data field (FD) or the CRC sequence (classic).
        let stuffed = match layout {
            None => true,
            Some(layout) if frame.fd => bits.len() <= layout.last_data_bit,
            // Up to the CRC delimiter's place, which a stuff bit may take after the CRC.
            Some(layout) => bits.len() <= layout.crc_delimiter,
        };
        if stuffed && raw.len() >= 6 {
            let window = &raw[raw.len() - 6..];
            let run = window[..5].iter().all(|&b| b == window[0]);
            if run && window[5] != window[0] {
                stuff_count += 1;
                continue;
            }
            if run {
                frame.error = Some(FrameError::Stuff);
                return Decoded::Frame(frame, position);
            }
        }
        bits.push(bit);
        let n = bits.len() - 1;

        if n == 0 && bit {
            // Not a start of frame after all.
            frame.error = Some(FrameError::Form);
            return Decoded::Frame(frame, position);
        }
        if n == 11 {
            frame.id = pack(&bits[1..=11]);
        }
        if n == 13 {
            frame.extended = bits[13];
        }
        if layout.is_none() {
            let (fdf, brs, esi) = if frame.extended {
                (33, 35, 36)
            } else {
                (14, 16, 17)
            };
            if frame.extended && n == 31 {
                frame.id = (pack(&bits[1..=11]) << 18) | pack(&bits[14..=31]);
            }
            if n == fdf {
                frame.fd = bit;
                let rtr = if frame.extended { bits[32] } else { bits[12] };
                frame.remote = !frame.fd && rtr;
            }
            if frame.fd && n == brs {
                frame.brs = bit;
                if bit {
                    // The data phase starts at this sample point.
                    reference = position as f64 + fast.width - fast.sample;
                    reference_bit = raw.len();
                    timing = fast;
                }
            }
            if frame.fd && n == esi {
                frame.esi = bit;
            }
            let dlc_start = match (frame.extended, frame.fd) {
                (false, false) => 15,
                (false, true) => 18,
                (true, false) => 35,
                (true, true) => 37,
            };
            if n > fdf && n == dlc_start + 3 {
                let dlc = pack(&bits[dlc_start..=dlc_start + 3]) as u8;
                frame.dlc = dlc;
                let payload = if frame.remote {
                    0
                } else if frame.fd {
                    dlc_len(dlc)
                } else {
                    usize::from(dlc.min(8))
                };
                let last_data_bit = dlc_start + 3 + payload * 8;
                let crc_len = if !frame.fd {
                    15
                } else if payload <= 16 {
                    27
                } else {
                    32
                };
                layout = Some(Layout {
                    dlc_start,
                    last_data_bit,
                    crc_delimiter: last_data_bit + crc_len + 1,
                });
                raw_last_data = raw.len() - 1;
            }
            continue;
        }
        let Some(fields) = layout else {
            continue;
        };
        if n <= fields.last_data_bit {
            if n == fields.last_data_bit {
                raw_last_data = raw.len() - 1;
                frame.data = bits[fields.dlc_start + 4..=fields.last_data_bit]
                    .chunks(8)
                    .map(|byte| pack(byte) as u8)
                    .collect();
            }
            continue;
        }
        if n == fields.crc_delimiter {
            frame.crc_ok = check_crc(&frame, &raw, &bits, &fields, raw_last_data, stuff_count);
            if !bit {
                frame.error = Some(FrameError::Form);
            }
            if frame.fd && frame.brs {
                // Back to the nominal rate at this sample point.
                reference = position as f64 + nominal.width - nominal.sample;
                reference_bit = raw.len();
                timing = nominal;
            }
        } else if n == fields.crc_delimiter + 1 {
            frame.acked = !bit;
        } else if n == fields.crc_delimiter + 2 {
            if !bit {
                frame.error.get_or_insert(FrameError::Form);
            }
        } else if n > fields.crc_delimiter + 2 {
            if !bit {
                frame.error.get_or_insert(FrameError::Form);
                return Decoded::Frame(frame, position);
            }
            if n == fields.crc_delimiter + 9 {
                // Seven recessive end-of-frame bits; three more of intermission follow.
                let resume = (position as f64 + 3.0 * nominal.width) as u64;
                return Decoded::Frame(frame, resume.min(end));
            }
        }
    }
}

fn check_crc(
    frame: &Frame,
    raw: &[bool],
    bits: &[bool],
    fields: &Layout,
    raw_last_data: usize,
    stuff_count: u32,
) -> bool {
    let last = fields.last_data_bit;
    if !frame.fd {
        let computed = Crc::new(15, 0x4599, 0).of(bits[..=last].iter().copied());
        return computed == pack(&bits[last + 1..=last + 15]);
    }
    // Stuff count (Gray coded) and its even parity follow a fixed stuff bit.
    let count_bits = &bits[last + 2..=last + 4];
    let parity = bits[last + 5];
    let count_ok = pack(count_bits) == gray(stuff_count)
        && (count_bits.iter().filter(|&&b| b).count() % 2 == 1) == parity;
    let crc_bits: Vec<bool> = (last + 6..fields.crc_delimiter)
        .filter(|&n| !(n - (last + 5) - 1).is_multiple_of(5))
        .map(|n| bits[n])
        .collect();
    let (width, poly) = if crc_bits.len() == 17 {
        (17, 0x1685b)
    } else {
        (21, 0x102899)
    };
    let input = raw[..=raw_last_data]
        .iter()
        .chain(count_bits)
        .chain(std::iter::once(&parity))
        .copied();
    let computed = Crc::new(width, poly, 1 << (width - 1)).of(input);
    count_ok && computed == pack(&crc_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_codes_the_stuff_count() {
        let codes: Vec<u32> = (0..8).map(gray).collect();
        assert_eq!(
            codes,
            vec![0b000, 0b001, 0b011, 0b010, 0b110, 0b111, 0b101, 0b100]
        );
    }

    #[test]
    fn computes_crc15() {
        // Standard data frame, ID 0x123, DLC 1, data 0x55: SOF, ID, RTR, IDE, r0, DLC, data.
        let mut bits = vec![false];
        bits.extend((0..11).rev().map(|i| (0x123 >> i) & 1 == 1));
        bits.extend([false, false, false]);
        bits.extend((0..4).rev().map(|i| (1 >> i) & 1 == 1));
        bits.extend((0..8).rev().map(|i| (0x55 >> i) & 1 == 1));
        let crc = Crc::new(15, 0x4599, 0).of(bits.iter().copied());
        // Appending the CRC leaves a zero remainder.
        let check = Crc::new(15, 0x4599, 0).of(bits
            .iter()
            .copied()
            .chain((0..15).rev().map(|i| (crc >> i) & 1 == 1)));
        assert_eq!(check, 0);
    }
}
