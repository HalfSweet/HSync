// SPDX-License-Identifier: Apache-2.0

//! The symbols of the USB PD physical layer (USB Power Delivery Specification revision 3.1,
//! §5.3 and §5.4): the 4b5b code, its K-codes and the ordered sets made of them.
//!
//! A symbol is five bits sent least significant bit first; the table of §5.3 prints each with
//! its last bit on the left, so `11000` (Sync-1) is sent 0, 0, 0, 1, 1 and is the number
//! `0b11000` here, its first bit in bit 0.

use super::events::OrderedSet;

/// A K-code: a symbol that is no data, used for ordered sets and the end of packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KCode {
    /// Sync-1, `11000`: start synchronisation 1.
    Sync1,
    /// Sync-2, `10001`: start synchronisation 2.
    Sync2,
    /// Sync-3, `00110`: start synchronisation 3.
    Sync3,
    /// RST-1, `00111`: reset 1.
    Rst1,
    /// RST-2, `11001`: reset 2.
    Rst2,
    /// EOP, `01101`: end of packet.
    Eop,
}

impl KCode {
    /// Every K-code.
    pub const ALL: [KCode; 6] = [
        KCode::Sync1,
        KCode::Sync2,
        KCode::Sync3,
        KCode::Rst1,
        KCode::Rst2,
        KCode::Eop,
    ];

    /// The five-bit symbol, its first bit in bit 0.
    pub const fn code(self) -> u8 {
        match self {
            KCode::Sync1 => 0b11000,
            KCode::Sync2 => 0b10001,
            KCode::Sync3 => 0b00110,
            KCode::Rst1 => 0b00111,
            KCode::Rst2 => 0b11001,
            KCode::Eop => 0b01101,
        }
    }

    /// The K-code a five-bit symbol is, if any.
    pub const fn from_code(code: u8) -> Option<KCode> {
        match code {
            0b11000 => Some(KCode::Sync1),
            0b10001 => Some(KCode::Sync2),
            0b00110 => Some(KCode::Sync3),
            0b00111 => Some(KCode::Rst1),
            0b11001 => Some(KCode::Rst2),
            0b01101 => Some(KCode::Eop),
            _ => None,
        }
    }
}

/// The K-codes of Hard Reset signalling: RST-1 RST-1 RST-1 RST-2.
pub const HARD_RESET: [KCode; 4] = [KCode::Rst1, KCode::Rst1, KCode::Rst1, KCode::Rst2];

/// The K-codes of Cable Reset signalling: RST-1 Sync-1 RST-1 Sync-3.
pub const CABLE_RESET: [KCode; 4] = [KCode::Rst1, KCode::Sync1, KCode::Rst1, KCode::Sync3];

impl OrderedSet {
    /// The four K-codes of the set, in the order they are sent.
    pub const fn kcodes(self) -> [KCode; 4] {
        use KCode::{Rst2, Sync1, Sync2, Sync3};
        match self {
            OrderedSet::Sop => [Sync1, Sync1, Sync1, Sync2],
            OrderedSet::SopPrime => [Sync1, Sync1, Sync3, Sync3],
            OrderedSet::SopDoublePrime => [Sync1, Sync3, Sync1, Sync3],
            OrderedSet::SopPrimeDebug => [Sync1, Rst2, Rst2, Sync3],
            OrderedSet::SopDoublePrimeDebug => [Sync1, Rst2, Sync3, Sync2],
        }
    }
}

/// The five-bit symbols of the data nibbles 0 to F, their first bit in bit 0.
const DATA: [u8; 16] = [
    0b11110, 0b01001, 0b10100, 0b10101, 0b01010, 0b01011, 0b01110, 0b01111, 0b10010, 0b10011,
    0b10110, 0b10111, 0b11010, 0b11011, 0b11100, 0b11101,
];

/// What a five-bit symbol stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Symbol {
    /// A data nibble.
    Data(u8),
    /// A K-code.
    K(KCode),
    /// A code the table reserves: an error.
    Invalid,
}

/// Decodes a five-bit symbol, its first bit in bit 0.
pub(super) fn decode(code: u8) -> Symbol {
    if let Some(nibble) = DATA.iter().position(|data| *data == code) {
        return Symbol::Data(nibble as u8);
    }
    match KCode::from_code(code) {
        Some(kcode) => Symbol::K(kcode),
        None => Symbol::Invalid,
    }
}

/// The five-bit symbol of a data nibble.
#[cfg(test)]
pub(super) const fn encode(nibble: u8) -> u8 {
    DATA[(nibble & 0x0f) as usize]
}

/// What an ordered set starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Start {
    /// A message addressed by the set.
    Message(OrderedSet),
    /// Hard Reset signalling.
    HardReset,
    /// Cable Reset signalling.
    CableReset,
}

/// Every ordered set, with what it starts.
const SETS: [(Start, [KCode; 4]); 7] = [
    (Start::Message(OrderedSet::Sop), OrderedSet::Sop.kcodes()),
    (
        Start::Message(OrderedSet::SopPrime),
        OrderedSet::SopPrime.kcodes(),
    ),
    (
        Start::Message(OrderedSet::SopDoublePrime),
        OrderedSet::SopDoublePrime.kcodes(),
    ),
    (
        Start::Message(OrderedSet::SopPrimeDebug),
        OrderedSet::SopPrimeDebug.kcodes(),
    ),
    (
        Start::Message(OrderedSet::SopDoublePrimeDebug),
        OrderedSet::SopDoublePrimeDebug.kcodes(),
    ),
    (Start::HardReset, HARD_RESET),
    (Start::CableReset, CABLE_RESET),
];

/// Recognises the ordered set the four symbols `codes` make, as §5.4 has receivers do: all
/// four K-codes in place, or three of them, the other one corrupted. Returns what the set
/// starts and the position, 1 to 4, of the corrupted K-code; `None` when no set has three
/// K-codes in place, or two have — the sets differ in two places at least, so a single
/// corrupted K-code can leave a sequence one away from two sets.
pub(super) fn recognise(codes: [u8; 4]) -> Option<(Start, Option<u8>)> {
    let mut found = None;
    for (start, kcodes) in SETS {
        let mut wrong = kcodes
            .iter()
            .zip(codes)
            .enumerate()
            .filter(|(_, (kcode, code))| kcode.code() != *code)
            .map(|(position, _)| position);
        let first = wrong.next();
        if wrong.next().is_some() {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some((start, first.map(|position| position as u8 + 1)));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_every_code_once() {
        let mut data = 0;
        let mut kcodes = 0;
        let mut invalid = Vec::new();
        for code in 0..32u8 {
            match decode(code) {
                Symbol::Data(nibble) => {
                    assert_eq!(encode(nibble), code);
                    data += 1;
                }
                Symbol::K(kcode) => {
                    assert_eq!(kcode.code(), code);
                    kcodes += 1;
                }
                Symbol::Invalid => invalid.push(code),
            }
        }
        assert_eq!((data, kcodes), (16, 6));
        assert_eq!(
            invalid,
            vec![
                0b00000, 0b00001, 0b00010, 0b00011, 0b00100, 0b00101, 0b01000, 0b01100, 0b10000,
                0b11111
            ]
        );
    }

    #[test]
    fn ordered_sets_need_three_kcodes_in_place() {
        let codes = |kcodes: [KCode; 4]| kcodes.map(KCode::code);
        assert_eq!(
            recognise(codes(OrderedSet::Sop.kcodes())),
            Some((Start::Message(OrderedSet::Sop), None))
        );
        assert_eq!(recognise(codes(HARD_RESET)), Some((Start::HardReset, None)));
        assert_eq!(
            recognise(codes(CABLE_RESET)),
            Some((Start::CableReset, None))
        );
        // One corrupted K-code: the set the other three make, and where the corruption is.
        let mut sop_prime = codes(OrderedSet::SopPrime.kcodes());
        sop_prime[0] = 0b11111;
        assert_eq!(
            recognise(sop_prime),
            Some((Start::Message(OrderedSet::SopPrime), Some(1)))
        );
        let mut reset = codes(HARD_RESET);
        reset[3] = encode(0x7);
        assert_eq!(recognise(reset), Some((Start::HardReset, Some(4))));
        // Two corrupted K-codes make no set.
        let mut broken = codes(OrderedSet::Sop.kcodes());
        broken[1] = 0;
        broken[2] = 0;
        assert_eq!(recognise(broken), None);
        // Sync-1 Sync-1 Sync-1 Sync-3 is one K-code away from both SOP and SOP'.
        let ambiguous = codes([KCode::Sync1, KCode::Sync1, KCode::Sync1, KCode::Sync3]);
        assert_eq!(recognise(ambiguous), None);
    }

    #[test]
    fn the_sets_differ_in_two_places_at_least() {
        for (i, (_, a)) in SETS.iter().enumerate() {
            for (_, b) in &SETS[i + 1..] {
                let differ = a.iter().zip(b).filter(|(x, y)| x != y).count();
                assert!(differ >= 2, "{a:?} and {b:?}");
            }
        }
    }
}
