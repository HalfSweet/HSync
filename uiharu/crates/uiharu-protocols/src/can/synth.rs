// SPDX-License-Identifier: Apache-2.0

//! Making CAN waveforms: for tests, for tools and for benchmarks.
//!
//! [`CanWaveform`] sends frames bit by bit, as a transmitter does: it lays out the fields,
//! inserts the stuff bits, computes the CRC with a shift register of its own (written apart
//! from the decoder's, cell by cell as the specification draws it), adds the stuff count and
//! the fixed stuff bits of FD frames, and times every bit in whole samples at the nominal or
//! the data bit rate; the two bits where FD frames switch rates last up to the sample point at
//! one rate and the rest of the bit at the other. [`Faults`] break a frame on purpose.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::CanConfig;

/// The terms of the CRC-15 polynomial, highest first.
const CRC15: [u32; 8] = [15, 14, 10, 8, 7, 4, 3, 0];
/// The terms of the CRC-17 polynomial, highest first.
const CRC17: [u32; 10] = [17, 16, 14, 13, 11, 6, 4, 3, 1, 0];
/// The terms of the CRC-21 polynomial, highest first.
const CRC21: [u32; 8] = [21, 20, 13, 11, 7, 4, 3, 0];
/// The data bytes each DLC stands for in FD frames.
const FD_LENGTHS: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/// A frame to send.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanFrame {
    /// The identifier: 11 bits in the base format, 29 in the extended format.
    pub id: u32,
    /// Whether the frame is in the extended format.
    pub extended: bool,
    /// Whether it is a remote frame; classic frames only.
    pub remote: bool,
    /// Whether it is an FD frame.
    pub fd: bool,
    /// Whether an FD frame switches to the data bit rate.
    pub brs: bool,
    /// Whether the transmitter of an FD frame is error passive.
    pub esi: bool,
    /// The data length code.
    pub dlc: u8,
    /// The data field: as many bytes as the code stands for; none in a remote frame.
    pub data: Vec<u8>,
}

impl CanFrame {
    /// A classic data frame in the base format, its DLC the length of `data` (up to 8 bytes).
    pub fn base(id: u32, data: &[u8]) -> Self {
        Self {
            id,
            dlc: u8::try_from(data.len()).unwrap_or(u8::MAX),
            data: data.to_vec(),
            ..Self::default()
        }
    }

    /// A classic data frame in the extended format.
    pub fn extended(id: u32, data: &[u8]) -> Self {
        Self {
            extended: true,
            ..Self::base(id, data)
        }
    }

    /// A classic remote frame requesting `dlc`.
    pub fn remote(id: u32, extended: bool, dlc: u8) -> Self {
        Self {
            id,
            extended,
            remote: true,
            dlc,
            ..Self::default()
        }
    }

    /// An FD frame in the base format. `data` has a length a DLC stands for: 0 to 8, 12, 16,
    /// 20, 24, 32, 48 or 64 bytes; other lengths get the next code up, without padding.
    pub fn fd(id: u32, data: &[u8]) -> Self {
        let dlc = FD_LENGTHS
            .iter()
            .position(|length| *length >= data.len())
            .unwrap_or(15);
        Self {
            id,
            fd: true,
            dlc: u8::try_from(dlc).unwrap_or(15),
            data: data.to_vec(),
            ..Self::default()
        }
    }

    /// The same frame in the extended format.
    pub fn in_extended_format(self) -> Self {
        Self {
            extended: true,
            ..self
        }
    }

    /// The same FD frame with its data phase at the data bit rate.
    pub fn with_brs(self) -> Self {
        Self { brs: true, ..self }
    }

    /// The same FD frame from an error-passive transmitter.
    pub fn with_esi(self) -> Self {
        Self { esi: true, ..self }
    }

    /// The same frame with another data length code, such as 9 to 15 for the 8 bytes of a
    /// classic frame.
    pub fn with_dlc(self, dlc: u8) -> Self {
        Self { dlc, ..self }
    }
}

/// Moves edges off their places by a number of samples, as an asymmetric transceiver delay
/// does; positive is late. A tenth of a nominal bit is `nominal / 10` samples. The start of
/// frame is never moved, and an edge never passes the one before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeShift {
    /// Recessive-to-dominant edges.
    pub to_dominant: i64,
    /// Dominant-to-recessive edges.
    pub to_recessive: i64,
}

/// Ways to break a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Faults {
    /// XORed into the CRC sequence before it is sent.
    pub crc_xor: u32,
    /// XORed into the stuff count of an FD frame before it is Gray-coded.
    pub stuff_count_xor: u8,
    /// Leaves out the dynamic stuff bit with this index, counted from 0, so that six equal
    /// bits are sent.
    pub drop_stuff_bit: Option<usize>,
    /// No receiver acknowledges the frame: the ACK slot stays recessive.
    pub no_ack: bool,
    /// Inverts the bit with this index in the bits as sent, counted from SOF as 0, stuff bits
    /// included.
    pub flip_bit: Option<usize>,
    /// From the bit with this index on, an active error flag (six dominant bits) and an error
    /// delimiter (eight recessive bits), at the nominal bit rate, take the place of the rest of
    /// the frame.
    pub error_flag_at: Option<usize>,
    /// Moves the frame's edges.
    pub shift: EdgeShift,
}

/// What a sent bit is part of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BitKind {
    /// Start of frame.
    Sof,
    /// The base identifier.
    Id,
    /// Substitute remote request.
    Srr,
    /// Identifier extension flag.
    Ide,
    /// The identifier extension.
    IdExtension,
    /// Remote transmission request, or RRS in FD frames.
    Rtr,
    /// FD format indicator, or the reserved bit in its place.
    Fdf,
    /// The reserved bit of FD frames.
    Res,
    /// Bit rate switch.
    Brs,
    /// Error state indicator.
    Esi,
    /// The reserved bit `r0` of classic extended frames.
    Reserved,
    /// The data length code.
    Dlc,
    /// The data field.
    Data,
    /// A dynamic stuff bit.
    Stuff,
    /// A fixed stuff bit of the FD CRC field.
    FixedStuff,
    /// The Gray-coded stuff count of an FD frame.
    StuffCount,
    /// The parity bit of the stuff count.
    Parity,
    /// The CRC sequence.
    Crc,
    /// The CRC delimiter.
    CrcDelimiter,
    /// The ACK slot.
    AckSlot,
    /// The ACK delimiter.
    AckDelimiter,
    /// End of frame.
    Eof,
    /// An active error flag.
    ErrorFlag,
    /// An error delimiter.
    ErrorDelimiter,
}

/// A bit as it was sent, where its edges would be without [`EdgeShift`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SentBit {
    /// Whether the bit is recessive.
    pub recessive: bool,
    /// What it is part of.
    pub kind: BitKind,
    /// Its first sample.
    pub start: Tick,
    /// The sample after it.
    pub end: Tick,
}

/// The bit rate a bit is sent at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rate {
    Nominal,
    Data,
    /// Nominal up to the sample point, data after it: BRS.
    ToData,
    /// Data up to the sample point, nominal after it: the CRC delimiter.
    ToNominal,
}

#[derive(Clone, Copy, Debug)]
struct Planned {
    recessive: bool,
    kind: BitKind,
    rate: Rate,
}

/// The CRC of `bits`, computed in a register of cells as the specification draws it: the
/// next bit, XORed with the highest cell, feeds back into the cells the polynomial's lower
/// terms name as the register shifts up. `terms` lists the polynomial's terms, highest first.
fn shift_register_crc(bits: &[bool], terms: &[u32], start_with_one: bool) -> u32 {
    let width = terms[0] as usize;
    let mut cells = vec![false; width];
    if start_with_one {
        cells[width - 1] = true;
    }
    for bit in bits {
        let feedback = *bit ^ cells[width - 1];
        for cell in (1..width).rev() {
            cells[cell] = cells[cell - 1];
        }
        cells[0] = false;
        if feedback {
            for term in &terms[1..] {
                cells[*term as usize] ^= true;
            }
        }
    }
    cells
        .iter()
        .rev()
        .fold(0, |value, cell| value << 1 | u32::from(*cell))
}

/// Inserts dynamic stuff bits.
#[derive(Debug, Default)]
struct Stuffer {
    last: Option<bool>,
    same: u32,
    /// Stuff bits due so far, sent or left out.
    due: usize,
    /// Stuff bits sent.
    sent: usize,
    drop: Option<usize>,
}

impl Stuffer {
    /// Sends `input`, a stuff bit after every five equal bits; after the last bit only when
    /// `trailing`.
    fn run(&mut self, input: &[(bool, BitKind)], out: &mut Vec<(bool, BitKind)>, trailing: bool) {
        for (index, (bit, kind)) in input.iter().enumerate() {
            out.push((*bit, *kind));
            if self.last == Some(*bit) {
                self.same += 1;
            } else {
                self.last = Some(*bit);
                self.same = 1;
            }
            if self.same == 5 && (trailing || index + 1 < input.len()) {
                if self.drop != Some(self.due) {
                    out.push((!*bit, BitKind::Stuff));
                    self.sent += 1;
                }
                self.due += 1;
                self.last = Some(!*bit);
                self.same = 1;
            }
        }
    }
}

fn push_value(bits: &mut Vec<(bool, BitKind)>, value: u32, width: u32, kind: BitKind) {
    for bit in (0..width).rev() {
        bits.push((value >> bit & 1 != 0, kind));
    }
}

/// The bits of `frame` as a transmitter sends them, with the faults applied.
fn plan(frame: &CanFrame, faults: &Faults) -> Vec<Planned> {
    let mut fields = Vec::new();
    fields.push((false, BitKind::Sof));
    let remote = frame.remote && !frame.fd;
    if frame.extended {
        push_value(&mut fields, frame.id >> 18 & 0x7ff, 11, BitKind::Id);
        fields.push((true, BitKind::Srr));
        fields.push((true, BitKind::Ide));
        push_value(&mut fields, frame.id & 0x3_ffff, 18, BitKind::IdExtension);
        fields.push((remote, BitKind::Rtr));
    } else {
        push_value(&mut fields, frame.id & 0x7ff, 11, BitKind::Id);
        fields.push((remote, BitKind::Rtr));
        fields.push((false, BitKind::Ide));
    }
    fields.push((frame.fd, BitKind::Fdf));
    if frame.fd {
        fields.push((false, BitKind::Res));
        fields.push((frame.brs, BitKind::Brs));
        fields.push((frame.esi, BitKind::Esi));
    } else if frame.extended {
        fields.push((false, BitKind::Reserved));
    }
    push_value(&mut fields, u32::from(frame.dlc & 0xf), 4, BitKind::Dlc);
    if !remote {
        for byte in &frame.data {
            push_value(&mut fields, u32::from(*byte), 8, BitKind::Data);
        }
    }

    let mut stuffer = Stuffer {
        drop: faults.drop_stuff_bit,
        ..Stuffer::default()
    };
    let mut bits = Vec::new();
    if frame.fd {
        // Dynamic stuffing ends with the data field: the fixed stuff bit that opens the CRC
        // field takes the place of a stuff bit due after it.
        stuffer.run(&fields, &mut bits, false);
        let count = (stuffer.sent % 8) as u8 ^ (faults.stuff_count_xor & 7);
        let gray = count ^ count >> 1;
        let parity = gray.count_ones() % 2 == 1;
        let mut crc_field = Vec::new();
        push_value(&mut crc_field, u32::from(gray), 3, BitKind::StuffCount);
        crc_field.push((parity, BitKind::Parity));
        // The DLC, not the bytes sent, picks the polynomial.
        let terms: &[u32] = if FD_LENGTHS[usize::from(frame.dlc & 0xf)] > 16 {
            &CRC21
        } else {
            &CRC17
        };
        let mut protected: Vec<bool> = bits.iter().map(|(bit, _)| *bit).collect();
        protected.extend(crc_field.iter().map(|(bit, _)| *bit));
        let crc = shift_register_crc(&protected, terms, true) ^ faults.crc_xor;
        push_value(&mut crc_field, crc, terms[0], BitKind::Crc);
        let mut before = bits.last().is_some_and(|(bit, _)| *bit);
        for (index, (bit, kind)) in crc_field.into_iter().enumerate() {
            if index % 4 == 0 {
                bits.push((!before, BitKind::FixedStuff));
            }
            bits.push((bit, kind));
            before = bit;
        }
    } else {
        let protected: Vec<bool> = fields.iter().map(|(bit, _)| *bit).collect();
        let crc = shift_register_crc(&protected, &CRC15, false) ^ faults.crc_xor;
        push_value(&mut fields, crc & 0x7fff, 15, BitKind::Crc);
        // Stuffing runs through the CRC sequence, a stuff bit after its last bit included.
        stuffer.run(&fields, &mut bits, true);
    }
    bits.push((true, BitKind::CrcDelimiter));
    bits.push((faults.no_ack, BitKind::AckSlot));
    bits.push((true, BitKind::AckDelimiter));
    bits.extend(std::iter::repeat_n((true, BitKind::Eof), 7));

    let mut rate = Rate::Nominal;
    let mut planned: Vec<Planned> = bits
        .into_iter()
        .map(|(recessive, kind)| {
            let this = match kind {
                BitKind::Brs if frame.fd && frame.brs => Rate::ToData,
                BitKind::CrcDelimiter if rate == Rate::Data => Rate::ToNominal,
                _ => rate,
            };
            rate = match this {
                Rate::ToData => Rate::Data,
                Rate::ToNominal => Rate::Nominal,
                other => other,
            };
            Planned {
                recessive,
                kind,
                rate: this,
            }
        })
        .collect();
    if let Some(index) = faults.flip_bit
        && let Some(bit) = planned.get_mut(index)
    {
        bit.recessive = !bit.recessive;
    }
    if let Some(index) = faults.error_flag_at
        && index < planned.len()
    {
        planned.truncate(index);
        let flag = |recessive, kind| Planned {
            recessive,
            kind,
            rate: Rate::Nominal,
        };
        planned.extend(std::iter::repeat_n(flag(false, BitKind::ErrorFlag), 6));
        planned.extend(std::iter::repeat_n(flag(true, BitKind::ErrorDelimiter), 8));
    }
    planned
}

/// Builds the samples of a CAN bus one frame at a time.
#[derive(Debug)]
pub struct CanWaveform {
    builder: LogicBlockBuilder,
    rx: u8,
    dominant_low: bool,
    /// The sample point, in percent.
    sample_point: u64,
    /// Samples per nominal bit.
    nominal: u64,
    /// Samples per data bit.
    data: u64,
}

impl CanWaveform {
    /// A waveform on `channels` channels for a bus wired as `config`, at `nominal` samples per
    /// nominal bit and `data` samples per data bit, starting at `start`. The receive line
    /// carries the bus; the other channels stay low.
    pub fn new(config: &CanConfig, channels: u8, nominal: u64, data: u64, start: Tick) -> Self {
        Self {
            builder: LogicBlockBuilder::new(start, channels),
            rx: config.rx,
            dominant_low: config.dominant_low,
            sample_point: u64::from(config.sample_point.clamp(1, 99)),
            nominal: nominal.max(1),
            data: data.max(1),
        }
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    fn hold(&mut self, recessive: bool, samples: u64) {
        let high = recessive == self.dominant_low;
        let levels = Levels(u64::from(high) << self.rx);
        self.builder.push_repeated(levels, samples);
    }

    /// Leaves the bus recessive for `bits` nominal bits.
    pub fn idle(&mut self, bits: u64) {
        self.hold(true, bits * self.nominal);
    }

    /// Leaves the bus recessive for `samples` samples.
    pub fn idle_samples(&mut self, samples: u64) {
        self.hold(true, samples);
    }

    /// Drives the bus dominant for `bits` nominal bits, as error and overload flags do.
    pub fn dominant(&mut self, bits: u64) {
        self.hold(false, bits * self.nominal);
    }

    /// Drives the bus dominant for `samples` samples: a glitch, when shorter than a bit.
    pub fn dominant_samples(&mut self, samples: u64) {
        self.hold(false, samples);
    }

    /// Samples in a bit sent at `rate`.
    fn duration(&self, rate: Rate) -> u64 {
        let to_sample_point = |bit: u64| bit * self.sample_point / 100;
        match rate {
            Rate::Nominal => self.nominal,
            Rate::Data => self.data,
            Rate::ToData => to_sample_point(self.nominal) + self.data - to_sample_point(self.data),
            Rate::ToNominal => {
                to_sample_point(self.data) + self.nominal - to_sample_point(self.nominal)
            }
        }
    }

    /// Sends `frame`, from SOF through the end of frame.
    pub fn frame(&mut self, frame: &CanFrame) -> Vec<SentBit> {
        self.frame_with(frame, &Faults::default())
    }

    /// Sends `frame` broken as `faults` say. Returns the bits as sent.
    pub fn frame_with(&mut self, frame: &CanFrame, faults: &Faults) -> Vec<SentBit> {
        let planned = plan(frame, faults);
        let origin = self.position();
        let mut bounds = Vec::with_capacity(planned.len() + 1);
        let mut at = 0u64;
        for bit in &planned {
            bounds.push(at);
            at += self.duration(bit.rate);
        }
        bounds.push(at);
        let total = at;
        let mut level = planned.first().is_none_or(|bit| bit.recessive);
        let mut from = 0u64;
        for (index, bit) in planned.iter().enumerate().skip(1) {
            if bit.recessive == level {
                continue;
            }
            let shift = if bit.recessive {
                faults.shift.to_recessive
            } else {
                faults.shift.to_dominant
            };
            let edge = (bounds[index] as i64 + shift).clamp(from as i64, total as i64) as u64;
            self.hold(level, edge - from);
            from = edge;
            level = bit.recessive;
        }
        self.hold(level, total - from);
        planned
            .iter()
            .enumerate()
            .map(|(index, bit)| SentBit {
                recessive: bit.recessive,
                kind: bit.kind,
                start: origin.offset(bounds[index]),
                end: origin.offset(bounds[index + 1]),
            })
            .collect()
    }

    /// The samples so far; at least one must have been produced.
    pub fn finish(self) -> LogicBlock {
        self.builder.finish().unwrap_or_else(|_| {
            LogicBlockBuilder::new(Tick::ZERO, 1)
                .finish()
                .expect("no samples were produced")
        })
    }

    /// The samples so far, if any.
    pub fn try_finish(self) -> Option<LogicBlock> {
        self.builder.finish().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_of(sent: &[SentBit]) -> String {
        sent.iter()
            .map(|bit| if bit.recessive { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn a_classic_frame_is_stuffed_and_closed() {
        let config = CanConfig::default();
        let mut waveform = CanWaveform::new(&config, 1, 10, 10, Tick::ZERO);
        let sent = waveform.frame(&CanFrame::base(0x000, &[]));
        // SOF and eleven zero identifier bits, RTR, IDE and r0: a stuff bit after every fifth
        // equal bit, the CRC of all zeros, then the trailer.
        assert_eq!(sent[0].kind, BitKind::Sof);
        assert_eq!(sent[5].kind, BitKind::Stuff);
        assert!(sent[5].recessive);
        let stuff_bits = sent.iter().filter(|bit| bit.kind == BitKind::Stuff).count();
        assert!(stuff_bits >= 3, "{}", bits_of(&sent));
        let tail: Vec<BitKind> = sent[sent.len() - 10..].iter().map(|bit| bit.kind).collect();
        assert_eq!(tail[0], BitKind::CrcDelimiter);
        assert_eq!(tail[1], BitKind::AckSlot);
        assert_eq!(tail[2], BitKind::AckDelimiter);
        assert!(tail[3..].iter().all(|kind| *kind == BitKind::Eof));
        assert_eq!(waveform.position(), Tick(10 * sent.len() as u64));
    }

    #[test]
    fn the_shift_register_divides_by_the_polynomial() {
        // The CRC of a single 1 is the generator's lower terms; of 1 followed by a 0, those
        // terms shifted once more and reduced.
        assert_eq!(shift_register_crc(&[true], &CRC15, false), 0x4599);
        assert_eq!(shift_register_crc(&[true], &CRC17, false), 0x1_685b);
        assert_eq!(shift_register_crc(&[true], &CRC21, false), 0x10_2899);
        // Starting from a one in the top cell is the same as starting from zero with the first
        // bit of the stream inverted.
        assert_eq!(
            shift_register_crc(&[false, true, true], &CRC17, true),
            shift_register_crc(&[true, true, true], &CRC17, false)
        );
    }

    #[test]
    fn fd_crc_fields_have_fixed_stuff_bits() {
        let config = CanConfig::default();
        let mut waveform = CanWaveform::new(&config, 1, 8, 4, Tick::ZERO);
        let sent = waveform.frame(&CanFrame::fd(0x123, &[0x55; 12]).with_brs());
        let field: Vec<&SentBit> = sent
            .iter()
            .skip_while(|bit| bit.kind != BitKind::FixedStuff)
            .take_while(|bit| bit.kind != BitKind::CrcDelimiter)
            .collect();
        // CRC-17: the stuff count and parity, 17 CRC bits and 6 fixed stuff bits.
        assert_eq!(field.len(), 4 + 17 + 6);
        for (index, bit) in field.iter().enumerate() {
            assert_eq!(
                bit.kind == BitKind::FixedStuff,
                index % 5 == 0,
                "bit {index}"
            );
            if bit.kind == BitKind::FixedStuff && index > 0 {
                assert_ne!(bit.recessive, field[index - 1].recessive);
            }
        }
        // The data phase is faster.
        let data = sent
            .iter()
            .find(|bit| bit.kind == BitKind::Data)
            .expect("data");
        assert_eq!(data.end.0 - data.start.0, 4);
    }

    #[test]
    fn faults_break_what_they_name() {
        let config = CanConfig::default();
        let frame = CanFrame::base(0x7ff, &[0xff]);
        let mut clean = CanWaveform::new(&config, 1, 10, 10, Tick::ZERO);
        let sent = clean.frame(&frame);
        let mut broken = CanWaveform::new(&config, 1, 10, 10, Tick::ZERO);
        let faults = Faults {
            drop_stuff_bit: Some(0),
            no_ack: true,
            ..Faults::default()
        };
        let without = broken.frame_with(&frame, &faults);
        assert_eq!(without.len() + 1, sent.len());
        let ack = without
            .iter()
            .find(|bit| bit.kind == BitKind::AckSlot)
            .expect("an ACK slot");
        assert!(ack.recessive);
        let mut flagged = CanWaveform::new(&config, 1, 10, 10, Tick::ZERO);
        let error = flagged.frame_with(
            &frame,
            &Faults {
                error_flag_at: Some(20),
                ..Faults::default()
            },
        );
        assert_eq!(error.len(), 20 + 6 + 8);
        assert!(error[20..26].iter().all(|bit| !bit.recessive));
    }
}
