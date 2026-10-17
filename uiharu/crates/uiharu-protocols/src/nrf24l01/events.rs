// SPDX-License-Identifier: Apache-2.0

//! The `nrf24l01.events` contract: what the nRF24L01+ decoder emits.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

use crate::spi::TxId;

/// The schema of nRF24L01+ event streams.
pub const NRF24L01_EVENTS: SchemaId = SchemaId::new("nrf24l01.events", 1);

/// The most bytes one `data` event carries: a whole payload.
pub const DATA_BATCH: usize = 32;

/// A command, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// The SPI commands, named after the datasheet's mnemonics in kebab-case; the command byte
/// is given with each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    /// R_REGISTER, `000A AAAA`: reads a register, 1 to 5 bytes, least significant byte first.
    RRegister,
    /// W_REGISTER, `001A AAAA`: writes a register, 1 to 5 bytes, least significant byte first.
    WRegister,
    /// R_RX_PAYLOAD, `0110 0001`: reads the received payload, 1 to 32 bytes.
    RRxPayload,
    /// W_TX_PAYLOAD, `1010 0000`: writes the payload to send, 1 to 32 bytes.
    WTxPayload,
    /// FLUSH_TX, `1110 0001`: empties the transmit FIFO.
    FlushTx,
    /// FLUSH_RX, `1110 0010`: empties the receive FIFO.
    FlushRx,
    /// REUSE_TX_PL, `1110 0011`: sends the last payload again.
    ReuseTxPl,
    /// R_RX_PL_WID, `0110 0000`: reads the width of the payload on top of the receive FIFO,
    /// one byte.
    RRxPlWid,
    /// W_ACK_PAYLOAD, `1010 1PPP`: writes a payload to send with the acknowledgement on pipe
    /// PPP, 1 to 32 bytes.
    WAckPayload,
    /// W_TX_PAYLOAD_NOACK, `1011 0000`: writes a payload to send without asking for an
    /// acknowledgement, 1 to 32 bytes.
    WTxPayloadNoack,
    /// NOP, `1111 1111`: does nothing; the STATUS register comes back as for every command.
    Nop,
    /// ACTIVATE, `0101 0000` followed by `0x73`: the nRF24L01's (not plus) switch for the
    /// features that are always on in the nRF24L01+.
    Activate,
}

/// The registers, named after the datasheet's register map in kebab-case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Register {
    /// CONFIG, `0x00`.
    Config,
    /// EN_AA, `0x01`: auto acknowledgement per pipe.
    EnAa,
    /// EN_RXADDR, `0x02`: enabled receive pipes.
    EnRxaddr,
    /// SETUP_AW, `0x03`: address width.
    SetupAw,
    /// SETUP_RETR, `0x04`: automatic retransmission.
    SetupRetr,
    /// RF_CH, `0x05`: RF channel.
    RfCh,
    /// RF_SETUP, `0x06`.
    RfSetup,
    /// STATUS, `0x07`.
    Status,
    /// OBSERVE_TX, `0x08`: transmit observation.
    ObserveTx,
    /// RPD, `0x09`: received power detector.
    Rpd,
    /// RX_ADDR_P0, `0x0A`: up to 5 bytes.
    RxAddrP0,
    /// RX_ADDR_P1, `0x0B`: up to 5 bytes.
    RxAddrP1,
    /// RX_ADDR_P2, `0x0C`: the least significant byte.
    RxAddrP2,
    /// RX_ADDR_P3, `0x0D`: the least significant byte.
    RxAddrP3,
    /// RX_ADDR_P4, `0x0E`: the least significant byte.
    RxAddrP4,
    /// RX_ADDR_P5, `0x0F`: the least significant byte.
    RxAddrP5,
    /// TX_ADDR, `0x10`: up to 5 bytes.
    TxAddr,
    /// RX_PW_P0, `0x11`: payload width of pipe 0.
    RxPwP0,
    /// RX_PW_P1, `0x12`.
    RxPwP1,
    /// RX_PW_P2, `0x13`.
    RxPwP2,
    /// RX_PW_P3, `0x14`.
    RxPwP3,
    /// RX_PW_P4, `0x15`.
    RxPwP4,
    /// RX_PW_P5, `0x16`.
    RxPwP5,
    /// FIFO_STATUS, `0x17`.
    FifoStatus,
    /// DYNPD, `0x1C`: dynamic payload length per pipe.
    Dynpd,
    /// FEATURE, `0x1D`.
    Feature,
}

/// Which way data moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Into the chip, on MOSI: register writes and payloads to send.
    In,
    /// Out of the chip, on MISO: register reads, received payloads and their width.
    Out,
}

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// Chip select was released after the command was whole.
    Complete,
    /// Chip select was released before the command's first data byte, which it requires.
    Incomplete,
    /// The transaction went on past the bytes the command takes; the rest of it was skipped.
    TooLong,
    /// The samples ended or were lost while the command was under way.
    Aborted,
}

/// What travels on an nRF24L01+ event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Nrf24Event {
    /// A command byte the decoder knows: it begins an operation.
    Command {
        /// The operation that begins.
        op: OpId,
        /// The SPI transaction it rides on.
        tx: TxId,
        /// The command.
        command: Command,
        /// The register address, for R_REGISTER and W_REGISTER.
        register: Option<u8>,
        /// The register's name, when the address is in the register map.
        register_name: Option<Register>,
        /// The pipe, for W_ACK_PAYLOAD.
        pipe: Option<u8>,
        /// The STATUS register, which the chip sends on MISO during the command byte.
        status: u8,
    },
    /// Data bytes of the command, in bus order: multi-byte registers least significant byte
    /// first.
    Data {
        /// The operation.
        op: OpId,
        /// Which way they moved.
        direction: Direction,
        /// The position of the first byte among the command's data bytes.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The operation ended.
    End {
        /// The operation.
        op: OpId,
        /// How it ended.
        reason: EndReason,
    },
    /// A transaction whose command byte names no command; the rest of it was skipped.
    Unknown {
        /// The SPI transaction.
        tx: TxId,
        /// The command byte on MOSI.
        command: u8,
        /// The STATUS register on MISO during it.
        status: u8,
    },
}

impl EventSchema for Nrf24Event {
    const SCHEMA: SchemaId = NRF24L01_EVENTS;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(event: &Nrf24Event) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let op = OpId(3);
        assert_eq!(
            json(&Nrf24Event::Command {
                op,
                tx: 2,
                command: Command::WRegister,
                register: Some(0x0a),
                register_name: Some(Register::RxAddrP0),
                pipe: None,
                status: 0x0e,
            }),
            r#"{"kind":"command","op":3,"tx":2,"command":"w-register","register":10,"registerName":"rx-addr-p0","pipe":null,"status":14}"#
        );
        assert_eq!(
            json(&Nrf24Event::Data {
                op,
                direction: Direction::Out,
                offset: 0,
                bytes: vec![0xe7, 0xe7],
            }),
            r#"{"kind":"data","op":3,"direction":"out","offset":0,"bytes":[231,231]}"#
        );
        assert_eq!(
            json(&Nrf24Event::End {
                op,
                reason: EndReason::TooLong,
            }),
            r#"{"kind":"end","op":3,"reason":"too-long"}"#
        );
        assert_eq!(
            json(&Nrf24Event::Unknown {
                tx: 4,
                command: 0x50,
                status: 0x0e,
            }),
            r#"{"kind":"unknown","tx":4,"command":80,"status":14}"#
        );
    }

    #[test]
    fn names_follow_the_datasheet_mnemonics() {
        let name = |value: serde_json::Value| value.as_str().map(str::to_owned);
        let command = |command: Command| name(serde_json::to_value(command).expect("ok"));
        assert_eq!(command(Command::RRegister).as_deref(), Some("r-register"));
        assert_eq!(
            command(Command::RRxPayload).as_deref(),
            Some("r-rx-payload")
        );
        assert_eq!(command(Command::ReuseTxPl).as_deref(), Some("reuse-tx-pl"));
        assert_eq!(command(Command::RRxPlWid).as_deref(), Some("r-rx-pl-wid"));
        assert_eq!(
            command(Command::WAckPayload).as_deref(),
            Some("w-ack-payload")
        );
        assert_eq!(
            command(Command::WTxPayloadNoack).as_deref(),
            Some("w-tx-payload-noack")
        );
        let register = |register: Register| name(serde_json::to_value(register).expect("ok"));
        assert_eq!(register(Register::EnRxaddr).as_deref(), Some("en-rxaddr"));
        assert_eq!(register(Register::SetupRetr).as_deref(), Some("setup-retr"));
        assert_eq!(register(Register::RxAddrP5).as_deref(), Some("rx-addr-p5"));
        assert_eq!(register(Register::RxPwP0).as_deref(), Some("rx-pw-p0"));
        assert_eq!(
            register(Register::FifoStatus).as_deref(),
            Some("fifo-status")
        );
        assert_eq!(register(Register::Dynpd).as_deref(), Some("dynpd"));
        let reason = |reason: EndReason| name(serde_json::to_value(reason).expect("ok"));
        assert_eq!(reason(EndReason::Incomplete).as_deref(), Some("incomplete"));
    }
}
