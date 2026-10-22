// SPDX-License-Identifier: Apache-2.0

//! The `ds18b20.events` contract: the function commands sent to a DS18B20 and what they carry.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

use crate::onewire::TxId;

/// The schema of DS18B20 event streams.
pub const DS18B20_EVENTS: SchemaId = SchemaId::new("ds18b20.events", 1);

/// The DS18B20's family code, the first byte of its ROM code.
pub const FAMILY_CODE: u8 = 0x28;

/// Convert T: start a temperature conversion.
pub const CONVERT_T: u8 = 0x44;
/// Write Scratchpad: write TH, TL and the configuration register.
pub const WRITE_SCRATCHPAD: u8 = 0x4e;
/// Read Scratchpad: read the nine scratchpad bytes.
pub const READ_SCRATCHPAD: u8 = 0xbe;
/// Copy Scratchpad: copy TH, TL and the configuration to EEPROM.
pub const COPY_SCRATCHPAD: u8 = 0x48;
/// Recall E²: reload TH, TL and the configuration from EEPROM.
pub const RECALL_E2: u8 = 0xb8;
/// Read Power Supply: ask whether a device is parasite-powered.
pub const READ_POWER_SUPPLY: u8 = 0xb4;

/// The function commands the decoder knows.
pub const COMMANDS: [u8; 6] = [
    CONVERT_T,
    WRITE_SCRATCHPAD,
    READ_SCRATCHPAD,
    COPY_SCRATCHPAD,
    RECALL_E2,
    READ_POWER_SUPPLY,
];

/// The bytes of the scratchpad: temperature (LSB, MSB), TH, TL, configuration, three reserved
/// bytes and the CRC of the first eight.
pub const SCRATCHPAD_LEN: usize = 9;

/// The resolution the configuration register sets, 9 to 12 bits: bits 5 and 6 are R0 and R1.
pub const fn resolution(config: u8) -> u8 {
    9 + (config >> 5 & 0b11)
}

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ds18b20End {
    /// The transaction ended at a reset after what the command carries was whole.
    Complete,
    /// The transaction ended at a reset before what the command carries was whole.
    Incomplete,
    /// The transaction was cut off: by a bad slot, a gap or the end of the samples.
    Aborted,
}

/// What travels on a DS18B20 event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Ds18b20Event {
    /// A function command the decoder knows; an `end` follows.
    Command {
        /// The 1-Wire transaction it rides on.
        tx: TxId,
        /// The command's code, such as `0x44` for Convert T.
        code: u8,
    },
    /// The nine bytes Read Scratchpad read, and the registers they hold.
    Scratchpad {
        /// The transaction.
        tx: TxId,
        /// The scratchpad as read.
        bytes: [u8; SCRATCHPAD_LEN],
        /// Whether the last byte is the CRC-8 of the first eight.
        crc_ok: bool,
        /// The temperature register, two's complement in sixteenths of a degree Celsius (at
        /// lower resolutions the lowest bits are undefined).
        temperature_raw: i16,
        /// The TH register (the high alarm trigger), or user byte 1.
        th: i8,
        /// The TL register (the low alarm trigger), or user byte 2.
        tl: i8,
        /// The configuration register.
        config: u8,
        /// The resolution it sets, 9 to 12 bits.
        resolution: u8,
    },
    /// The bytes of a Read Scratchpad that ended before the ninth.
    PartialScratchpad {
        /// The transaction.
        tx: TxId,
        /// The bytes read, 1 to 8.
        bytes: Vec<u8>,
        /// The temperature register, when both its bytes were read.
        temperature_raw: Option<i16>,
    },
    /// The three bytes Write Scratchpad wrote.
    WriteScratchpad {
        /// The transaction.
        tx: TxId,
        /// The TH register, or user byte 1.
        th: i8,
        /// The TL register, or user byte 2.
        tl: i8,
        /// The configuration register.
        config: u8,
    },
    /// The read slot after Read Power Supply.
    PowerSupply {
        /// The transaction.
        tx: TxId,
        /// Whether a device pulled the slot low: it is parasite-powered.
        parasite: bool,
    },
    /// A function command the decoder does not know; the rest of the transaction is skipped.
    UnknownCommand {
        /// The transaction.
        tx: TxId,
        /// The command's code.
        code: u8,
    },
    /// The command ended.
    End {
        /// The transaction.
        tx: TxId,
        /// How it ended.
        reason: Ds18b20End,
    },
}

impl EventSchema for Ds18b20Event {
    const SCHEMA: SchemaId = DS18B20_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &Ds18b20Event) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn the_configuration_sets_the_resolution() {
        assert_eq!(resolution(0x1f), 9);
        assert_eq!(resolution(0x3f), 10);
        assert_eq!(resolution(0x5f), 11);
        assert_eq!(resolution(0x7f), 12);
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let tx = 7;
        assert_eq!(
            json(&Ds18b20Event::Command { tx, code: 0x44 }),
            r#"{"kind":"command","tx":7,"code":68}"#
        );
        assert_eq!(
            json(&Ds18b20Event::Scratchpad {
                tx,
                bytes: [0x50, 0x05, 0x4b, 0x46, 0x7f, 0xff, 0x0c, 0x10, 0x1c],
                crc_ok: true,
                temperature_raw: 0x0550,
                th: 75,
                tl: 70,
                config: 0x7f,
                resolution: 12
            }),
            concat!(
                r#"{"kind":"scratchpad","tx":7,"bytes":[80,5,75,70,127,255,12,16,28],"#,
                r#""crcOk":true,"temperatureRaw":1360,"th":75,"tl":70,"config":127,"#,
                r#""resolution":12}"#
            )
        );
        assert_eq!(
            json(&Ds18b20Event::PartialScratchpad {
                tx,
                bytes: vec![0x91, 0x01],
                temperature_raw: Some(0x0191)
            }),
            r#"{"kind":"partial-scratchpad","tx":7,"bytes":[145,1],"temperatureRaw":401}"#
        );
        assert_eq!(
            json(&Ds18b20Event::WriteScratchpad {
                tx,
                th: -10,
                tl: -55,
                config: 0x1f
            }),
            r#"{"kind":"write-scratchpad","tx":7,"th":-10,"tl":-55,"config":31}"#
        );
        assert_eq!(
            json(&Ds18b20Event::PowerSupply { tx, parasite: true }),
            r#"{"kind":"power-supply","tx":7,"parasite":true}"#
        );
        assert_eq!(
            json(&Ds18b20Event::UnknownCommand { tx, code: 0xaa }),
            r#"{"kind":"unknown-command","tx":7,"code":170}"#
        );
        assert_eq!(
            json(&Ds18b20Event::End {
                tx,
                reason: Ds18b20End::Incomplete
            }),
            r#"{"kind":"end","tx":7,"reason":"incomplete"}"#
        );
    }
}
