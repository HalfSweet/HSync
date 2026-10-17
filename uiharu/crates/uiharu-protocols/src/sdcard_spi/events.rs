// SPDX-License-Identifier: Apache-2.0

//! The `sdcard.events` contract: what the SD card decoder emits.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

use crate::spi::TxId;

/// The schema of SD card event streams.
pub const SDCARD_EVENTS: SchemaId = SchemaId::new("sdcard.events", 1);

/// The most bytes one `data` event carries.
pub const DATA_BATCH: usize = 32;

/// An operation — a command and what it brings about — numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// The format of a response, as the SPI-mode chapter of the specification names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResponseFormat {
    /// One byte, the card status flags.
    R1,
    /// R1 followed by busy bytes while the card works.
    R1b,
    /// R1 and a second status byte (CMD13, ACMD13).
    R2,
    /// R1 and the operation conditions register, four bytes (CMD58).
    R3,
    /// R1 and four bytes of interface conditions (CMD8).
    R7,
}

/// A control token of a data transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Token {
    /// `0xFE`: a data block follows (reads, and single-block writes).
    StartBlock,
    /// `0xFC`: a block of a multiple-block write follows.
    StartMultiBlock,
    /// `0xFD`: the host ends a multiple-block write.
    StopTran,
    /// `xxx0sss1`: the card's answer to a written block; `sss` is `010` when it was accepted,
    /// `101` for a CRC error, `110` for a write error.
    DataResponse,
    /// `0000xxxx`: the card sends error flags instead of a data block.
    Error,
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdEnd {
    /// The exchange the command calls for was whole.
    Complete,
    /// Chip select was released, or the host started another command, before the response
    /// or the data the command calls for was whole.
    Incomplete,
    /// The samples ended or were lost while the operation was under way.
    Aborted,
}

/// What travels on an SD card event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SdEvent {
    /// A command frame from the host: it begins an operation.
    Command {
        /// The operation that begins.
        op: OpId,
        /// The SPI transaction the frame rides on.
        tx: TxId,
        /// The command index, 0 to 63.
        index: u8,
        /// Whether the card takes it as an application command: it follows a CMD55 the card
        /// did not reject, and its index names an application command.
        acmd: bool,
        /// The 32-bit argument.
        argument: u32,
        /// The CRC7 field as sent.
        crc7: u8,
        /// Whether the CRC7 field matches the frame.
        crc_ok: bool,
    },
    /// The card's response.
    Response {
        /// The operation.
        op: OpId,
        /// Its format; `r1` when the card answered a command it refused with R1 alone.
        format: ResponseFormat,
        /// The response bytes, R1 first: 1 to 5.
        bytes: Vec<u8>,
        /// How many busy bytes followed an R1b response (saturating at 65535); 0 otherwise.
        busy_bytes: u16,
    },
    /// A control token.
    Token {
        /// The operation.
        op: OpId,
        /// Which token.
        token: Token,
        /// The byte on the bus.
        value: u8,
    },
    /// Bytes of a data block, in order.
    Data {
        /// The operation.
        op: OpId,
        /// The position of the first byte, counted from the operation's first data byte.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The end of a data block: its CRC16.
    BlockEnd {
        /// The operation.
        op: OpId,
        /// The CRC16 field as sent.
        crc16: u16,
        /// Whether it matches the block's bytes.
        crc_ok: bool,
        /// The block's length in bytes.
        length: u16,
    },
    /// The operation ended.
    End {
        /// The operation.
        op: OpId,
        /// How it ended.
        reason: SdEnd,
    },
    /// A transaction whose first byte on MOSI that is not fill does not begin a command
    /// frame; its words were skipped.
    UnknownCommand {
        /// The SPI transaction.
        tx: TxId,
        /// That byte.
        byte: u8,
    },
}

impl EventSchema for SdEvent {
    const SCHEMA: SchemaId = SDCARD_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &SdEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let op = OpId(2);
        assert_eq!(
            json(&SdEvent::Command {
                op,
                tx: 1,
                index: 8,
                acmd: false,
                argument: 0x1aa,
                crc7: 0x43,
                crc_ok: true,
            }),
            r#"{"kind":"command","op":2,"tx":1,"index":8,"acmd":false,"argument":426,"crc7":67,"crcOk":true}"#
        );
        assert_eq!(
            json(&SdEvent::Response {
                op,
                format: ResponseFormat::R1b,
                bytes: vec![0],
                busy_bytes: 3,
            }),
            r#"{"kind":"response","op":2,"format":"r1b","bytes":[0],"busyBytes":3}"#
        );
        assert_eq!(
            json(&SdEvent::Token {
                op,
                token: Token::StartMultiBlock,
                value: 0xfc,
            }),
            r#"{"kind":"token","op":2,"token":"start-multi-block","value":252}"#
        );
        assert_eq!(
            json(&SdEvent::Data {
                op,
                offset: 512,
                bytes: vec![1, 2],
            }),
            r#"{"kind":"data","op":2,"offset":512,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&SdEvent::BlockEnd {
                op,
                crc16: 0x7fa1,
                crc_ok: true,
                length: 512,
            }),
            r#"{"kind":"block-end","op":2,"crc16":32673,"crcOk":true,"length":512}"#
        );
        assert_eq!(
            json(&SdEvent::End {
                op,
                reason: SdEnd::Incomplete,
            }),
            r#"{"kind":"end","op":2,"reason":"incomplete"}"#
        );
        assert_eq!(
            json(&SdEvent::UnknownCommand { tx: 5, byte: 0xfe }),
            r#"{"kind":"unknown-command","tx":5,"byte":254}"#
        );
    }

    #[test]
    fn names_follow_the_specification() {
        let name = |value: serde_json::Value| value.as_str().map(str::to_owned);
        let format = |format| name(serde_json::to_value(format).expect("serializes"));
        assert_eq!(format(ResponseFormat::R1).as_deref(), Some("r1"));
        assert_eq!(format(ResponseFormat::R1b).as_deref(), Some("r1b"));
        assert_eq!(format(ResponseFormat::R7).as_deref(), Some("r7"));
        let token = |token| name(serde_json::to_value(token).expect("serializes"));
        assert_eq!(token(Token::StopTran).as_deref(), Some("stop-tran"));
        assert_eq!(token(Token::DataResponse).as_deref(), Some("data-response"));
        assert_eq!(token(Token::Error).as_deref(), Some("error"));
    }
}
