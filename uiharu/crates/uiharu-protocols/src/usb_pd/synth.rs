// SPDX-License-Identifier: Apache-2.0

//! Making USB PD waveforms — BMC packets on the CC line — for tests, the command line's
//! generator and benchmarks.
//!
//! [`UsbPdWaveform`] sends a packet bit by bit as a transmitter does: the 64-bit preamble, the
//! ordered set, the 4b5b symbols of the header, the data and a CRC-32, the EOP, and the final
//! transition, after which the line is held low and released to its idle level. The CRC comes
//! from [`crc32`], a bit-serial register of its own that shifts the other way from the
//! decoder's, and the 4b5b table is written out as the specification prints it. [`Faults`]
//! break a packet on purpose.

use uiharu_core::{Levels, LogicBlock, LogicBlockBuilder, Tick};

use super::decoder::UsbPdConfig;
use super::events::OrderedSet;
use super::symbols::{CABLE_RESET, HARD_RESET, KCode};

/// The 4b5b symbols of the nibbles 0 to F as the specification's table prints them: the bit
/// sent last on the left.
const TABLE: [&str; 16] = [
    "11110", "01001", "10100", "10101", "01010", "01011", "01110", "01111", "10010", "10011",
    "10110", "10111", "11010", "11011", "11100", "11101",
];

/// The bits of the preamble.
const PREAMBLE_BITS: usize = 64;

/// A K-code as the specification's table prints it.
fn printed_kcode(kcode: KCode) -> &'static str {
    match kcode {
        KCode::Sync1 => "11000",
        KCode::Sync2 => "10001",
        KCode::Sync3 => "00110",
        KCode::Rst1 => "00111",
        KCode::Rst2 => "11001",
        KCode::Eop => "01101",
    }
}

/// The number a printed symbol stands for: its rightmost bit, sent first, in bit 0.
fn number(printed: &str) -> u8 {
    printed
        .bytes()
        .fold(0, |value, bit| value << 1 | u8::from(bit == b'1'))
}

/// The five-bit symbol of a data nibble, its first bit in bit 0.
pub fn nibble_symbol(nibble: u8) -> u8 {
    number(TABLE[usize::from(nibble & 0x0f)])
}

/// The five-bit symbol of a K-code, its first bit in bit 0.
pub fn kcode_symbol(kcode: KCode) -> u8 {
    number(printed_kcode(kcode))
}

/// The CRC-32 of `bytes`, each sent least significant bit first, as a packet's CRC field is
/// sent: polynomial 0x04C11DB7, register preset to all ones, result inverted, run bit by bit in
/// its reflected form — the register shifted right, the reversed polynomial 0xEDB88320 added
/// when the bit shifted out differs from the bit coming in.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut register = u32::MAX;
    for byte in bytes {
        for i in 0..8 {
            let incoming = u32::from(byte >> i & 1);
            let outgoing = register & 1;
            register >>= 1;
            if incoming != outgoing {
                register ^= 0xedb8_8320;
            }
        }
    }
    !register
}

/// A message header to send.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    /// Message Type, bits 4–0.
    pub message_type: u8,
    /// Bit 5: Port Data Role in SOP messages, set for a DFP.
    pub data_role: bool,
    /// Bit 8: Port Power Role in SOP messages, set for a source; Cable Plug in SOP' and SOP''
    /// messages, set when a cable plug sends it.
    pub power_role: bool,
    /// Specification Revision, bits 7–6: 2 for revision 3.x.
    pub revision: u8,
    /// MessageID, bits 11–9.
    pub message_id: u8,
    /// Number of Data Objects, bits 14–12.
    pub objects: u8,
    /// Extended, bit 15.
    pub extended: bool,
}

impl Header {
    /// The 16 bits of the header.
    pub fn value(&self) -> u16 {
        u16::from(self.message_type & 0x1f)
            | u16::from(self.data_role) << 5
            | u16::from(self.revision & 0x3) << 6
            | u16::from(self.power_role) << 8
            | u16::from(self.message_id & 0x7) << 9
            | u16::from(self.objects & 0x7) << 12
            | u16::from(self.extended) << 15
    }
}

/// The 16 bits of an extended message header.
pub fn extended_header(
    data_size: u16,
    chunk_number: u8,
    request_chunk: bool,
    chunked: bool,
) -> u16 {
    (data_size & 0x1ff)
        | u16::from(request_chunk) << 10
        | u16::from(chunk_number & 0xf) << 11
        | u16::from(chunked) << 15
}

/// `value` in `unit`s, kept to the `width` bits of its field.
fn field(value: u32, unit: u32, width: u32) -> u32 {
    (value / unit) & ((1 << width) - 1)
}

/// A fixed supply power data object: the voltage in 50 mV units, the current in 10 mA units
/// and bits 29–20 as `flags`.
pub fn fixed_pdo(millivolts: u32, milliamps: u32, flags: u32) -> u32 {
    field(flags, 1, 10) << 20 | field(millivolts, 50, 10) << 10 | field(milliamps, 10, 10)
}

/// A battery power data object: the voltages in 50 mV units, the power in 250 mW units.
pub fn battery_pdo(max_millivolts: u32, min_millivolts: u32, milliwatts: u32) -> u32 {
    1 << 30
        | field(max_millivolts, 50, 10) << 20
        | field(min_millivolts, 50, 10) << 10
        | field(milliwatts, 250, 10)
}

/// A variable supply power data object: the voltages in 50 mV units, the current in 10 mA
/// units.
pub fn variable_pdo(max_millivolts: u32, min_millivolts: u32, milliamps: u32) -> u32 {
    2 << 30
        | field(max_millivolts, 50, 10) << 20
        | field(min_millivolts, 50, 10) << 10
        | field(milliamps, 10, 10)
}

/// A programmable power supply APDO: the voltages in 100 mV units, the current in 50 mA units.
pub fn pps_apdo(max_millivolts: u32, min_millivolts: u32, milliamps: u32) -> u32 {
    3 << 30
        | field(max_millivolts, 100, 8) << 17
        | field(min_millivolts, 100, 8) << 8
        | field(milliamps, 50, 7)
}

/// A request data object for a fixed or variable supply: the object position, the operating
/// and maximum currents in 10 mA units, and bits 27–23 as `flags` (GiveBack, Capability
/// Mismatch, USB Communications Capable, No USB Suspend, Unchunked Extended Messages
/// Supported, from bit 4 down).
pub fn request(position: u8, operating_milliamps: u32, max_milliamps: u32, flags: u32) -> u32 {
    field(u32::from(position), 1, 4) << 28
        | field(flags, 1, 5) << 23
        | field(operating_milliamps, 10, 10) << 10
        | field(max_milliamps, 10, 10)
}

/// Data objects as the bytes that carry them, each least significant byte first.
pub fn object_bytes(objects: &[u32]) -> Vec<u8> {
    objects
        .iter()
        .flat_map(|object| object.to_le_bytes())
        .collect()
}

/// The symbols of a message after its preamble: the ordered set, the header, `payload` (the
/// bytes after the header), the CRC with `crc_xor` applied and the EOP.
pub fn message_symbols(set: OrderedSet, header: u16, payload: &[u8], crc_xor: u32) -> Vec<u8> {
    let mut symbols: Vec<u8> = set.kcodes().into_iter().map(kcode_symbol).collect();
    let mut bytes = header.to_le_bytes().to_vec();
    bytes.extend_from_slice(payload);
    let crc = crc32(&bytes) ^ crc_xor;
    bytes.extend_from_slice(&crc.to_le_bytes());
    for byte in bytes {
        symbols.push(nibble_symbol(byte & 0x0f));
        symbols.push(nibble_symbol(byte >> 4));
    }
    symbols.push(kcode_symbol(KCode::Eop));
    symbols
}

/// Ways to break a packet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Faults {
    /// XORed into the CRC before it is sent.
    pub crc_xor: u32,
    /// Sends the five-bit code given instead of the symbol with this index, counted from the
    /// ordered set's first K-code as 0.
    pub symbol: Option<(usize, u8)>,
    /// Makes the bit with this index, counted from the preamble's first bit as 0, last this
    /// many percent of a bit; a 1 keeps its middle transition in its middle.
    pub stretch: Option<(usize, u32)>,
    /// Stops before the bit with this index, counted like `stretch`: the transmitter falls
    /// silent and the line holds its level; at or past the last bit, only the final transition
    /// is left out.
    pub cut: Option<usize>,
}

/// Builds the samples of a CC line one packet or idle stretch at a time.
#[derive(Debug)]
pub struct UsbPdWaveform {
    cc: u8,
    builder: LogicBlockBuilder,
    start: Tick,
    /// Samples per second.
    rate: u64,
    /// The transmitter's bits per second.
    bit_rate: u64,
    /// Where the line has got to, in 1 / (2 · `bit_rate`) samples from `start`: a half bit is
    /// `rate` of them.
    time: u128,
    level: bool,
    idle_level: bool,
    /// How long the line is held low after a packet, in samples.
    hold_low: u64,
}

impl UsbPdWaveform {
    /// A waveform on `channels` channels with the CC line on the channel `config` names,
    /// sampled at `rate_hz`, starting at `start` with the line idle low; the transmitter keeps
    /// the configured bit rate and holds the line low for 1 µs after each packet.
    pub fn new(config: &UsbPdConfig, channels: u8, rate_hz: u64, start: Tick) -> Self {
        Self {
            cc: config.cc,
            builder: LogicBlockBuilder::new(start, channels),
            start,
            rate: rate_hz.max(1),
            bit_rate: u64::from(config.bit_rate).max(1),
            time: 0,
            level: false,
            idle_level: false,
            hold_low: rate_hz.div_ceil(1_000_000),
        }
    }

    /// The same waveform with a transmitter running at `bit_rate_hz` instead, to test how far
    /// its clock may be off.
    pub fn with_bit_rate(mut self, bit_rate_hz: u64) -> Self {
        self.bit_rate = bit_rate_hz.max(1);
        self
    }

    /// The same waveform on a line that idles high: it starts high and is released high after
    /// each packet's low hold.
    pub fn with_idle_high(mut self) -> Self {
        self.idle_level = true;
        self.level = true;
        self
    }

    /// The same waveform with the line held low for `samples` after each packet.
    pub fn with_hold_low(mut self, samples: u64) -> Self {
        self.hold_low = samples;
        self
    }

    /// The tick the next sample will have.
    pub fn position(&self) -> Tick {
        self.builder.next_tick()
    }

    fn levels(&self, high: bool) -> Levels {
        Levels(u64::from(high) << self.cc)
    }

    /// Holds the line at `high` until `time`.
    fn hold_until(&mut self, high: bool, time: u128) {
        let offset = time / (2 * u128::from(self.bit_rate));
        let offset = u64::try_from(offset).unwrap_or(u64::MAX);
        let count = self.builder.next_tick().until(self.start.offset(offset));
        self.builder.push_repeated(self.levels(high), count);
        self.time = time;
        self.level = high;
    }

    /// Holds the line at `high` for `samples` samples.
    pub fn hold(&mut self, high: bool, samples: u64) {
        let time = self.time + u128::from(samples) * 2 * u128::from(self.bit_rate);
        self.hold_until(high, time);
    }

    /// The idle line for `samples` samples.
    pub fn idle(&mut self, samples: u64) {
        self.hold(self.idle_level, samples);
    }

    /// The idle line for `bits` bit times.
    pub fn idle_bits(&mut self, bits: u64) {
        let time = self.time + u128::from(bits) * 2 * u128::from(self.rate);
        self.hold_until(self.idle_level, time);
    }

    /// The line stays where it is for `bits` bit times, as after a packet whose transmitter
    /// fell silent.
    pub fn stay_bits(&mut self, bits: u64) {
        let time = self.time + u128::from(bits) * 2 * u128::from(self.rate);
        self.hold_until(self.level, time);
    }

    /// One BMC bit lasting `percent` percent of a bit: a transition at its start, and for a 1
    /// another in its middle.
    fn bmc(&mut self, one: bool, percent: u32) {
        let half = u128::from(self.rate) * u128::from(percent) / 100;
        let first = !self.level;
        self.hold_until(first, self.time + half);
        self.hold_until(first != one, self.time + half);
    }

    /// Sends one bit.
    pub fn bit(&mut self, one: bool) {
        self.bmc(one, 100);
    }

    /// Sends the 64 bits of the preamble, 0 and 1 alternating.
    pub fn preamble(&mut self) {
        for i in 0..PREAMBLE_BITS {
            self.bit(i % 2 == 1);
        }
    }

    /// Sends a five-bit symbol, its bit 0 first.
    pub fn symbol(&mut self, code: u8) {
        for i in 0..5 {
            self.bit(code >> i & 1 != 0);
        }
    }

    /// Sends the final transition that ends a packet; when it leaves the line high, the line
    /// stays high for a bit time first. Then the line is held low until it is released.
    pub fn end_of_packet(&mut self) {
        if !self.level {
            self.hold_until(true, self.time + 2 * u128::from(self.rate));
        }
        self.hold(false, self.hold_low);
    }

    /// Sends a packet: the preamble, `symbols` — the ordered set and all that follows it —
    /// and the final transition, broken as `faults` say.
    pub fn frame(&mut self, symbols: &[u8], faults: &Faults) {
        let mut bits: Vec<bool> = (0..PREAMBLE_BITS).map(|i| i % 2 == 1).collect();
        for (index, code) in symbols.iter().enumerate() {
            let code = match faults.symbol {
                Some((at, replacement)) if at == index => replacement,
                _ => *code,
            };
            bits.extend((0..5).map(|i| code >> i & 1 != 0));
        }
        for (index, one) in bits.iter().enumerate() {
            if faults.cut == Some(index) {
                return;
            }
            let percent = match faults.stretch {
                Some((at, percent)) if at == index => percent,
                _ => 100,
            };
            self.bmc(*one, percent);
        }
        if faults.cut.is_none() {
            self.end_of_packet();
        }
    }

    /// Sends a message: `header` and `payload`, the bytes after it, with their CRC.
    pub fn message(&mut self, set: OrderedSet, header: u16, payload: &[u8]) {
        self.message_with(set, header, payload, &Faults::default());
    }

    /// Sends a message broken as `faults` say.
    pub fn message_with(&mut self, set: OrderedSet, header: u16, payload: &[u8], faults: &Faults) {
        let symbols = message_symbols(set, header, payload, faults.crc_xor);
        self.frame(&symbols, faults);
    }

    /// Sends Hard Reset signalling: the preamble, RST-1 RST-1 RST-1 RST-2 and the final
    /// transition.
    pub fn hard_reset(&mut self) {
        self.frame(&HARD_RESET.map(kcode_symbol), &Faults::default());
    }

    /// Sends Cable Reset signalling: the preamble, RST-1 Sync-1 RST-1 Sync-3 and the final
    /// transition.
    pub fn cable_reset(&mut self) {
        self.frame(&CABLE_RESET.map(kcode_symbol), &Faults::default());
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
    use super::super::symbols;
    use super::*;

    #[test]
    fn the_printed_table_is_the_decoders() {
        for nibble in 0..16u8 {
            assert_eq!(symbols::encode(nibble), nibble_symbol(nibble), "{nibble:x}");
        }
        for kcode in KCode::ALL {
            assert_eq!(kcode.code(), kcode_symbol(kcode), "{kcode:?}");
        }
    }

    #[test]
    fn the_crc_is_the_standard_crc32() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn headers_and_objects_lay_out_their_fields() {
        let header = Header {
            message_type: 1,
            data_role: true,
            power_role: true,
            revision: 2,
            message_id: 7,
            objects: 2,
            extended: false,
        };
        assert_eq!(header.value(), 0x2fa1);
        assert_eq!(extended_header(260, 3, true, true), 0x9d04);
        assert_eq!(fixed_pdo(5000, 3000, 0), 0x0001_912c);
        assert_eq!(request(1, 3000, 3000, 0b00100), 0x1204_b12c);
        assert_eq!(object_bytes(&[0x0102_0304]), vec![4, 3, 2, 1]);
    }

    #[test]
    fn a_bit_has_a_transition_at_its_start_and_a_one_another_in_its_middle() {
        let config = UsbPdConfig::default();
        let mut waveform = UsbPdWaveform::new(&config, 1, 1_200_000, Tick::ZERO);
        waveform.idle(2);
        waveform.bit(false);
        waveform.bit(true);
        waveform.idle(2);
        let block = waveform.finish();
        let levels: String = (0..block.len())
            .map(|tick| match block.level(0, Tick(tick)) {
                Some(true) => '1',
                _ => '0',
            })
            .collect();
        // Four samples per bit: 0 is 1111, 1 is 0011, then idle low.
        assert_eq!(levels, ["00", "1111", "0011", "00"].concat());
    }
}
