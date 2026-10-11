// SPDX-License-Identifier: Apache-2.0

//! Integers wider than JSON readers keep exactly.
//!
//! JSON has no integer type: readers such as JavaScript's `JSON.parse` turn every number into
//! a double and lose the bits above 2^53. A field that may hold such a value — a 1-Wire ROM
//! code, the word of a parallel bus of more than 53 lines — is marked
//! `#[serde(with = "uiharu_core::hex_u64")]`: human-readable formats such as JSON get a
//! hexadecimal string (`"0x28ab0000c0ffee1d"`), binary formats the integer itself. Reading
//! accepts both, so a number written by hand still parses.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserializer, Serializer};

/// Writes `value` as `"0x…"` for human-readable formats, as an integer otherwise.
pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    if serializer.is_human_readable() {
        serializer.serialize_str(&format!("0x{value:x}"))
    } else {
        serializer.serialize_u64(*value)
    }
}

/// Reads an integer written as `"0x…"`, as a bare hexadecimal string, or as a number.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    struct Wide;

    impl Visitor<'_> for Wide {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an unsigned 64-bit integer or a hexadecimal string")
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<u64, E> {
            Ok(value)
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<u64, E> {
            u64::try_from(value).map_err(|_| E::custom("a negative number is not a u64"))
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<u64, E> {
            let digits = text
                .strip_prefix("0x")
                .or_else(|| text.strip_prefix("0X"))
                .unwrap_or(text);
            u64::from_str_radix(digits, 16)
                .map_err(|_| E::custom(format!("{text:?} is not a hexadecimal integer")))
        }
    }

    if deserializer.is_human_readable() {
        deserializer.deserialize_any(Wide)
    } else {
        deserializer.deserialize_u64(Wide)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "super")]
        wide: u64,
    }

    #[test]
    fn json_carries_wide_integers_as_hex_strings() {
        let holder = Holder {
            wide: 0x28ab_0000_c0ff_ee1d,
        };
        let json = serde_json::to_string(&holder).expect("serializes");
        assert_eq!(json, r#"{"wide":"0x28ab0000c0ffee1d"}"#);
        let back: Holder = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, holder);
    }

    #[test]
    fn json_still_reads_numbers_and_bare_hex() {
        let back: Holder = serde_json::from_str(r#"{"wide":40}"#).expect("parses");
        assert_eq!(back.wide, 40);
        let back: Holder = serde_json::from_str(r#"{"wide":"ff"}"#).expect("parses");
        assert_eq!(back.wide, 255);
        assert!(serde_json::from_str::<Holder>(r#"{"wide":"0xzz"}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"wide":-1}"#).is_err());
    }

    #[test]
    fn zero_and_the_maximum_round_trip() {
        for wide in [0, 1, u64::MAX] {
            let json = serde_json::to_string(&Holder { wide }).expect("serializes");
            let back: Holder = serde_json::from_str(&json).expect("parses");
            assert_eq!(back.wide, wide);
        }
    }
}
