//! I²C, ported from DSView's `1:i2c` decoder (`libsigrokdecode4DSL/decoders/1-i2c/pd.py`):
//! the same conditions, states and annotation spans.

use std::ops::Range;

use hsync_capture::ChannelData;

use crate::sampler::{Pin, Sampler, Term};

/// What happened on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Start,
    RepeatedStart,
    Stop,
    AddressWrite,
    AddressRead,
    DataWrite,
    DataRead,
    Ack,
    Nack,
}

/// One event: a condition, a byte or an acknowledge bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub start: u64,
    pub end: u64,
    /// The byte as sent; for addresses including the R/W bit.
    pub byte: u8,
}

impl Item {
    /// The 7-bit address of an address byte.
    pub fn address(&self) -> u8 {
        self.byte >> 1
    }
}

/// The decoded bus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    pub items: Vec<Item>,
    /// The first clock edge before the first start condition: a transfer already under way
    /// when the capture began, which cannot be decoded.
    pub cut_head: Option<u64>,
}

const SCL: usize = 0;
const SDA: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// What the decoder waits for, as DSView's `FIND …` states.
enum State {
    Start,
    Address,
    Data,
    Ack,
}

/// Decodes `scl` and `sda` over `samples`.
pub fn decode(scl: &ChannelData, sda: &ChannelData, samples: Range<u64>) -> Trace {
    let mut sampler = Sampler::new(vec![scl, sda], samples.end).from(samples.start);
    let start_condition = vec![Term::Pin(SCL, Pin::High), Term::Pin(SDA, Pin::Falling)];
    let stop_condition = vec![Term::Pin(SCL, Pin::High), Term::Pin(SDA, Pin::Rising)];
    let clock = vec![Term::Pin(SCL, Pin::Rising)];
    let find_start = [start_condition.clone()];
    let find_bits = [clock.clone(), start_condition, stop_condition.clone()];
    let find_ack = [clock, stop_condition];

    let mut trace = Trace::default();
    let mut state = State::Start;
    let mut repeat = false;
    let mut writing = false;
    let mut bits = 0u32;
    let mut byte = 0u8;
    let mut byte_start = 0u64;
    let mut last_bit = 0u64;
    let mut bit_width = 0u64;

    loop {
        let conditions: &[Vec<Term>] = match state {
            State::Start => &find_start,
            State::Address | State::Data => &find_bits,
            State::Ack => &find_ack,
        };
        let Some(found) = sampler.wait(conditions) else {
            break;
        };
        let at = found.sample;
        let is_start = match state {
            State::Start => true,
            State::Address | State::Data => !found.has(0) && found.has(1),
            State::Ack => false,
        };
        let is_stop = match state {
            State::Start => false,
            State::Address | State::Data => !found.has(0) && !found.has(1),
            State::Ack => !found.has(0),
        };
        if is_start {
            trace.items.push(Item {
                kind: if repeat {
                    Kind::RepeatedStart
                } else {
                    Kind::Start
                },
                start: at,
                end: at,
                byte: 0,
            });
            state = State::Address;
            repeat = true;
            bits = 0;
            byte = 0;
        } else if is_stop {
            trace.items.push(Item {
                kind: Kind::Stop,
                start: at,
                end: at,
                byte: 0,
            });
            state = State::Start;
            repeat = false;
        } else if state == State::Ack {
            let nack = sampler.pin(SDA);
            trace.items.push(Item {
                kind: if nack { Kind::Nack } else { Kind::Ack },
                start: at,
                end: at + bit_width,
                byte: 0,
            });
            state = State::Data;
        } else {
            // A data or address bit, MSB first.
            byte = (byte << 1) | u8::from(sampler.pin(SDA));
            if bits == 0 {
                byte_start = at;
            }
            if bits == 7 {
                bit_width = at - last_bit;
            }
            last_bit = at;
            if bits < 7 {
                bits += 1;
                continue;
            }
            let kind = if state == State::Address {
                writing = byte & 1 == 0;
                if writing {
                    Kind::AddressWrite
                } else {
                    Kind::AddressRead
                }
            } else if writing {
                Kind::DataWrite
            } else {
                Kind::DataRead
            };
            trace.items.push(Item {
                kind,
                start: byte_start,
                end: at + bit_width,
                byte,
            });
            bits = 0;
            byte = 0;
            state = State::Ack;
        }
    }

    let first_start = trace
        .items
        .iter()
        .find(|item| matches!(item.kind, Kind::Start))
        .map_or(samples.end, |item| item.start);
    trace.cut_head = scl
        .next_change(samples.start)
        .filter(|&edge| edge < first_start);
    trace
}
