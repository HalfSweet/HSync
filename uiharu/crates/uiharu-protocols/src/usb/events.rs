// SPDX-License-Identifier: Apache-2.0

//! The `usb.packets` contract: what the USB packet decoder emits and what decoders built on USB
//! packets read.

use serde::{Deserialize, Serialize};
use uiharu_core::{EventSchema, SchemaId};

/// The schema of USB packet streams.
pub const USB_PACKETS: SchemaId = SchemaId::new("usb.packets", 1);

/// The most bytes one `data` event carries.
pub const DATA_BATCH: usize = 32;

/// A packet, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PacketId(pub u64);

/// A packet identifier: the four-bit type of a PID field, by the name USB 2.0 Table 8-1 gives
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pid {
    /// Token `0001`: host to function.
    Out,
    /// Token `1001`: function to host.
    In,
    /// Token `0101`: start of frame.
    Sof,
    /// Token `1101`: host to function, to a control endpoint.
    Setup,
    /// Data `0011`.
    Data0,
    /// Data `1011`.
    Data1,
    /// Data `0111`.
    Data2,
    /// Data `1111`.
    Mdata,
    /// Handshake `0010`: received without error.
    Ack,
    /// Handshake `1010`: not ready.
    Nak,
    /// Handshake `1110`: halted, or the request is not supported.
    Stall,
    /// Handshake `0110`: received, not ready for more.
    Nyet,
    /// Special `1100`: the full-speed preamble before low-speed traffic.
    Pre,
    /// Special `1100`: a split transaction error, a high-speed handshake that shares the code
    /// of PRE.
    Err,
    /// Special `1000`: a split transaction token.
    Split,
    /// Special `0100`: a flow control probe, with a token's fields.
    Ping,
    /// `0000`, which the specification reserves.
    Reserved,
}

impl Pid {
    /// The PID a four-bit type stands for; code `1100` reads as PRE, the meaning it has on a
    /// low- or full-speed bus.
    pub const fn from_code(code: u8) -> Pid {
        match code & 0x0f {
            0x1 => Pid::Out,
            0x9 => Pid::In,
            0x5 => Pid::Sof,
            0xd => Pid::Setup,
            0x3 => Pid::Data0,
            0xb => Pid::Data1,
            0x7 => Pid::Data2,
            0xf => Pid::Mdata,
            0x2 => Pid::Ack,
            0xa => Pid::Nak,
            0xe => Pid::Stall,
            0x6 => Pid::Nyet,
            0xc => Pid::Pre,
            0x8 => Pid::Split,
            0x4 => Pid::Ping,
            _ => Pid::Reserved,
        }
    }

    /// The four-bit type.
    pub const fn code(self) -> u8 {
        match self {
            Pid::Out => 0x1,
            Pid::In => 0x9,
            Pid::Sof => 0x5,
            Pid::Setup => 0xd,
            Pid::Data0 => 0x3,
            Pid::Data1 => 0xb,
            Pid::Data2 => 0x7,
            Pid::Mdata => 0xf,
            Pid::Ack => 0x2,
            Pid::Nak => 0xa,
            Pid::Stall => 0xe,
            Pid::Nyet => 0x6,
            Pid::Pre | Pid::Err => 0xc,
            Pid::Split => 0x8,
            Pid::Ping => 0x4,
            Pid::Reserved => 0x0,
        }
    }

    /// The PID byte as it is sent: the type in the low nibble, its complement in the high one.
    pub const fn byte(self) -> u8 {
        let code = self.code();
        code | ((!code & 0x0f) << 4)
    }

    /// Whether the PID starts a token packet with an address and an endpoint: OUT, IN, SETUP
    /// and PING.
    pub const fn is_token(self) -> bool {
        matches!(self, Pid::Out | Pid::In | Pid::Setup | Pid::Ping)
    }

    /// Whether the PID starts a data packet.
    pub const fn is_data(self) -> bool {
        matches!(self, Pid::Data0 | Pid::Data1 | Pid::Data2 | Pid::Mdata)
    }

    /// Whether the PID is a handshake: ACK, NAK, STALL or NYET.
    pub const fn is_handshake(self) -> bool {
        matches!(self, Pid::Ack | Pid::Nak | Pid::Stall | Pid::Nyet)
    }
}

/// How a packet ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndReason {
    /// An end of packet: SE0, then the bus left it.
    Eop,
    /// The packet was a preamble, which ends after its PID without an EOP.
    Preamble,
    /// A seventh 1 in a row where bit stuffing calls for a 0.
    BitStuffError,
    /// The PID's check nibble is not the complement of its type.
    PidError,
    /// The bus went to SE1.
    Se1,
    /// The samples ended or were lost while the packet was under way.
    Aborted,
}

/// What travels on a USB packet stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum UsbEvent {
    /// A SYNC pattern: a packet begins.
    Begin {
        /// The packet that begins.
        pkt: PacketId,
    },
    /// The packet's PID.
    Pid {
        /// The packet.
        pkt: PacketId,
        /// The type, from the low nibble.
        pid: Pid,
        /// Whether the high nibble is its complement.
        ok: bool,
    },
    /// The fields of a token packet (OUT, IN, SETUP, PING).
    Token {
        /// The packet.
        pkt: PacketId,
        /// ADDR, 7 bits.
        address: u8,
        /// ENDP, 4 bits.
        endpoint: u8,
        /// The CRC5 field as it was sent, read least significant bit first.
        crc5: u8,
        /// Whether the CRC5 matches.
        ok: bool,
    },
    /// The fields of a start-of-frame packet.
    Sof {
        /// The packet.
        pkt: PacketId,
        /// The 11-bit frame number.
        frame: u16,
        /// The CRC5 field as it was sent, read least significant bit first.
        crc5: u8,
        /// Whether the CRC5 matches.
        ok: bool,
    },
    /// Payload bytes of a data packet, in order.
    Data {
        /// The packet.
        pkt: PacketId,
        /// The position of the first byte in the payload.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The end of a data packet's payload: its CRC16 and length.
    DataEnd {
        /// The packet.
        pkt: PacketId,
        /// The CRC16 field, the packet's last two bytes read little-endian; `None` when the
        /// packet ended before two whole bytes.
        crc16: Option<u16>,
        /// Whether the CRC16 matches.
        ok: bool,
        /// The payload's length in bytes, the CRC16 not counted.
        length: u16,
    },
    /// The 24 bits that follow a SPLIT PID, first bit in bit 0.
    Split {
        /// The packet.
        pkt: PacketId,
        /// The bits.
        raw: u32,
    },
    /// The packet ended.
    End {
        /// The packet.
        pkt: PacketId,
        /// How it ended.
        reason: EndReason,
    },
    /// A reset: SE0 for 2.5 µs or more, reported when it ends.
    Reset {
        /// How long the SE0 lasted.
        ticks: u64,
    },
    /// The bus has been idle for 3 ms.
    Suspend,
    /// Resume signalling: K for 20 ms or more, ended by an end of packet.
    Resume,
    /// A low-speed keep-alive: an end of packet on the idle bus.
    KeepAlive,
}

impl UsbEvent {
    /// The packet the event belongs to; `None` for the events of the bus outside packets.
    pub fn pkt(&self) -> Option<PacketId> {
        match self {
            UsbEvent::Begin { pkt }
            | UsbEvent::Pid { pkt, .. }
            | UsbEvent::Token { pkt, .. }
            | UsbEvent::Sof { pkt, .. }
            | UsbEvent::Data { pkt, .. }
            | UsbEvent::DataEnd { pkt, .. }
            | UsbEvent::Split { pkt, .. }
            | UsbEvent::End { pkt, .. } => Some(*pkt),
            UsbEvent::Reset { .. } | UsbEvent::Suspend | UsbEvent::Resume | UsbEvent::KeepAlive => {
                None
            }
        }
    }
}

impl EventSchema for UsbEvent {
    const SCHEMA: SchemaId = USB_PACKETS;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_codes_round_trip() {
        for code in 0..16u8 {
            assert_eq!(Pid::from_code(code).code(), code);
        }
        assert_eq!(Pid::Setup.byte(), 0x2d);
        assert_eq!(Pid::In.byte(), 0x69);
        assert_eq!(Pid::Data0.byte(), 0xc3);
        assert_eq!(Pid::Ack.byte(), 0xd2);
        assert_eq!(Pid::Err.code(), Pid::Pre.code());
        assert!(Pid::Ping.is_token() && !Pid::Sof.is_token());
        assert!(Pid::Mdata.is_data() && Pid::Nyet.is_handshake());
    }

    fn json(event: &UsbEvent) -> String {
        serde_json::to_string(event).expect("serializes")
    }

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let pkt = PacketId(2);
        assert_eq!(
            json(&UsbEvent::Begin { pkt }),
            r#"{"kind":"begin","pkt":2}"#
        );
        assert_eq!(
            json(&UsbEvent::Pid {
                pkt,
                pid: Pid::Data0,
                ok: true
            }),
            r#"{"kind":"pid","pkt":2,"pid":"data0","ok":true}"#
        );
        assert_eq!(
            json(&UsbEvent::Token {
                pkt,
                address: 3,
                endpoint: 1,
                crc5: 0x1d,
                ok: false
            }),
            r#"{"kind":"token","pkt":2,"address":3,"endpoint":1,"crc5":29,"ok":false}"#
        );
        assert_eq!(
            json(&UsbEvent::Sof {
                pkt,
                frame: 0x7ff,
                crc5: 1,
                ok: true
            }),
            r#"{"kind":"sof","pkt":2,"frame":2047,"crc5":1,"ok":true}"#
        );
        assert_eq!(
            json(&UsbEvent::Data {
                pkt,
                offset: 32,
                bytes: vec![1, 2]
            }),
            r#"{"kind":"data","pkt":2,"offset":32,"bytes":[1,2]}"#
        );
        assert_eq!(
            json(&UsbEvent::DataEnd {
                pkt,
                crc16: None,
                ok: false,
                length: 0
            }),
            r#"{"kind":"data-end","pkt":2,"crc16":null,"ok":false,"length":0}"#
        );
        assert_eq!(
            json(&UsbEvent::Split { pkt, raw: 0x123456 }),
            r#"{"kind":"split","pkt":2,"raw":1193046}"#
        );
        assert_eq!(
            json(&UsbEvent::End {
                pkt,
                reason: EndReason::BitStuffError
            }),
            r#"{"kind":"end","pkt":2,"reason":"bit-stuff-error"}"#
        );
        assert_eq!(
            json(&UsbEvent::Reset { ticks: 480 }),
            r#"{"kind":"reset","ticks":480}"#
        );
        assert_eq!(json(&UsbEvent::Suspend), r#"{"kind":"suspend"}"#);
        assert_eq!(json(&UsbEvent::KeepAlive), r#"{"kind":"keep-alive"}"#);
    }
}
