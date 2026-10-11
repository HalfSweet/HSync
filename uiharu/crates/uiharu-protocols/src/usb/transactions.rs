// SPDX-License-Identifier: Apache-2.0

//! USB transactions over `usb.packets`: the packets of one exchange between the host and an
//! endpoint, after the Universal Serial Bus Specification Revision 2.0 (usb.org), chapter 8:
//! §8.4.6 for handshakes, §8.5 for the transactions of each transfer type.
//!
//! The rules:
//!
//! - a token starts a transaction, named after it: SETUP and OUT expect the host's data packet,
//!   then the device's handshake; IN expects the device's data packet, or NAK or STALL instead,
//!   then the host's handshake; PING expects the device's handshake; SOF is a transaction of
//!   its own, whole when its packet is;
//! - the transaction begins once the token's address and endpoint are read, and ends with its
//!   last expected packet: ACK, or NYET after data, `complete`; NAK `nak`; STALL `stall`;
//! - a token, SOF, PRE or SPLIT before then, or a reset, suspend, resume or keep-alive, ends it
//!   `no-response`, placed where that packet's PID or that bus event settled; nothing ends a
//!   transaction on time alone, so the last one before a quiet bus stays open until the next
//!   packet; isochronous transactions, which have no handshake, end `no-response` as well:
//!   the endpoint's type is in its descriptors, which are out of scope;
//! - a packet the transaction does not expect next — a handshake where data belongs, a second
//!   data packet, a handshake its stage does not allow — and an expected packet that is
//!   damaged — a CRC that does not match, a failed PID check, a bit stuff error, SE1, missing
//!   fields — end it `error`; a token whose CRC5 does not match begins a transaction that ends
//!   `error` at once, since the device ignores it;
//! - data passes through in the batches of the packet stream, with its data PID; a data packet
//!   with no payload is one `data` event with no bytes, so that its PID is not lost; a SETUP
//!   transaction's data packet, when it carries 8 bytes and its CRC16 matches, is also
//!   reported as `setup`: bmRequestType, bRequest, wValue, wIndex and wLength as numbers, the
//!   16-bit ones little-endian;
//! - packets no transaction expects — data or handshakes with none open, ERR and the reserved
//!   PID — start none; nor do PRE and SPLIT, which announce traffic this decoder cannot follow
//!   (low-speed packets on a full-speed bus, high-speed split transactions);
//! - a gap or the end of the stream ends the open transaction `aborted`.

use serde::{Deserialize, Serialize};
use uiharu_core::{
    EventSchema, InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Props, Provenance, Requires, SchemaId, Tick, TickRange,
};

use super::bus::{Field, Next, Packet, Read, ReadError, UsbBus};
use super::events::{EndReason, Pid, UsbEvent};

/// The schema of USB transaction streams.
pub const USB_TRANSACTIONS: SchemaId = SchemaId::new("usb.transactions", 1);

/// The bytes of a setup packet.
const SETUP_BYTES: usize = 8;

/// A transaction, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct XactId(pub u64);

/// The token a transaction is named after.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Token {
    /// SETUP: a request to a control endpoint.
    Setup,
    /// IN: data from the device.
    In,
    /// OUT: data to the device.
    Out,
    /// PING: may the host send.
    Ping,
    /// SOF: the start of a frame.
    Sof,
}

/// How a transaction ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum XactEnd {
    /// The last expected packet arrived: ACK (or NYET after data), or SOF's own end.
    Complete,
    /// The device answered NAK.
    Nak,
    /// The device answered STALL.
    Stall,
    /// Something else began before the expected answer.
    NoResponse,
    /// A packet came that the transaction did not expect, or one it expected was damaged.
    Error,
    /// The samples ended or were lost while the transaction was under way.
    Aborted,
}

/// What travels on a USB transaction stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum TransactionEvent {
    /// A token began a transaction.
    Begin {
        /// The transaction that begins.
        xact: XactId,
        /// Its token.
        token: Token,
        /// The token's ADDR; `None` for SOF, which has none.
        address: Option<u8>,
        /// The token's ENDP; `None` for SOF.
        endpoint: Option<u8>,
        /// SOF's frame number; `None` for the other tokens.
        frame: Option<u16>,
    },
    /// The setup packet a SETUP transaction carries.
    Setup {
        /// The transaction.
        xact: XactId,
        /// bmRequestType.
        bm_request_type: u8,
        /// bRequest.
        b_request: u8,
        /// wValue.
        w_value: u16,
        /// wIndex.
        w_index: u16,
        /// wLength.
        w_length: u16,
    },
    /// Bytes of the transaction's data packet, in order.
    Data {
        /// The transaction.
        xact: XactId,
        /// The data packet's PID.
        pid: Pid,
        /// The position of the first byte in the packet's payload.
        offset: u32,
        /// At most [`DATA_BATCH`](super::DATA_BATCH) bytes.
        bytes: Vec<u8>,
    },
    /// A handshake packet within the transaction.
    Handshake {
        /// The transaction.
        xact: XactId,
        /// ACK, NAK, STALL or NYET.
        pid: Pid,
    },
    /// The transaction ended.
    End {
        /// The transaction.
        xact: XactId,
        /// How it ended.
        reason: XactEnd,
    },
}

impl EventSchema for TransactionEvent {
    const SCHEMA: SchemaId = USB_TRANSACTIONS;
}

/// The USB transaction decoder node: `usb.packets` in, `usb.transactions` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsbTransactions;

impl UsbTransactions {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The packet input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<UsbEvent> {
        InputRef::of_node(node, 0)
    }

    /// The transaction output of the decoder added as `node`.
    pub fn transactions(node: NodeId) -> OutputRef<TransactionEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the packet stream it reads: nothing beyond its schema, as
    /// transactions are the same at both speeds.
    pub fn requires() -> Requires {
        Requires::new()
    }
}

impl Node for UsbTransactions {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "usb-transactions".into(),
            inputs: vec![InputSpec::new::<UsbEvent>("packets", Self::requires())],
            outputs: vec![OutputSpec::new::<TransactionEvent>(
                "transactions",
                Props::new(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let bus = UsbBus::new(io.input(0));
        Box::pin(run(bus, XactOut::new(io.output(0))))
    }
}

/// What the open transaction waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// OUT or SETUP: the host's data packet.
    HostData,
    /// IN: the device's data packet, or NAK or STALL.
    DeviceData,
    /// OUT or SETUP after the data: the device's handshake.
    DeviceHandshake,
    /// IN after the data: the host's handshake.
    HostHandshake,
    /// PING: the device's handshake.
    PingHandshake,
    /// SOF: the end of its own packet.
    Frame,
}

/// The first bytes of a SETUP transaction's data packet.
#[derive(Debug)]
struct SetupBytes {
    bytes: [u8; SETUP_BYTES],
    /// The bytes seen, of which the first [`SETUP_BYTES`] are kept.
    len: u32,
    at: Option<Provenance>,
}

impl SetupBytes {
    fn new() -> Self {
        Self {
            bytes: [0; SETUP_BYTES],
            len: 0,
            at: None,
        }
    }

    fn add(&mut self, offset: u32, bytes: &[u8], at: Provenance) {
        for (position, byte) in (offset..).zip(bytes) {
            if let Some(slot) = usize::try_from(position)
                .ok()
                .and_then(|position| self.bytes.get_mut(position))
            {
                *slot = *byte;
            }
        }
        let end = offset.saturating_add(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
        self.len = self.len.max(end);
        self.at = Some(self.at.map_or(at, |have| have.join(at)));
    }
}

/// The transaction under way.
#[derive(Debug)]
struct Open {
    xact: XactId,
    stage: Stage,
    /// A SETUP transaction's setup packet, as it comes.
    setup: Option<SetupBytes>,
}

/// The typed writer the packet body uses, which also remembers the open transaction.
#[derive(Debug)]
struct XactOut {
    out: Outbox<TransactionEvent>,
    next_xact: u64,
    open: Option<Open>,
}

impl XactOut {
    fn new(out: Outbox<TransactionEvent>) -> Self {
        Self {
            out,
            next_xact: 1,
            open: None,
        }
    }

    fn stage(&self) -> Option<Stage> {
        self.open.as_ref().map(|open| open.stage)
    }

    async fn begin(
        &mut self,
        token: Token,
        address: Option<u8>,
        endpoint: Option<u8>,
        frame: Option<u16>,
        at: Provenance,
    ) {
        let xact = XactId(self.next_xact);
        self.next_xact += 1;
        let stage = match token {
            Token::Setup | Token::Out => Stage::HostData,
            Token::In => Stage::DeviceData,
            Token::Ping => Stage::PingHandshake,
            Token::Sof => Stage::Frame,
        };
        self.open = Some(Open {
            xact,
            stage,
            setup: (token == Token::Setup).then(SetupBytes::new),
        });
        let begin = TransactionEvent::Begin {
            xact,
            token,
            address,
            endpoint,
            frame,
        };
        self.out.emit(at, begin).await;
    }

    async fn data(&mut self, pid: Pid, offset: u32, bytes: Vec<u8>, at: Provenance) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let xact = open.xact;
        if let Some(setup) = open.setup.as_mut() {
            setup.add(offset, &bytes, at);
        }
        let data = TransactionEvent::Data {
            xact,
            pid,
            offset,
            bytes,
        };
        self.out.emit(at, data).await;
    }

    /// The data packet ended whole: the setup packet, if this is one, and the next stage.
    async fn data_end(&mut self, length: u16, at: Provenance) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let xact = open.xact;
        open.stage = match open.stage {
            Stage::HostData => Stage::DeviceHandshake,
            Stage::DeviceData => Stage::HostHandshake,
            other => other,
        };
        let Some(setup) = open.setup.take() else {
            return;
        };
        if usize::from(length) != SETUP_BYTES || usize::try_from(setup.len) != Ok(SETUP_BYTES) {
            return;
        }
        let [b0, b1, b2, b3, b4, b5, b6, b7] = setup.bytes;
        let event = TransactionEvent::Setup {
            xact,
            bm_request_type: b0,
            b_request: b1,
            w_value: u16::from_le_bytes([b2, b3]),
            w_index: u16::from_le_bytes([b4, b5]),
            w_length: u16::from_le_bytes([b6, b7]),
        };
        let at = setup.at.map_or(at, |have| have.join(at));
        self.out.emit(at, event).await;
    }

    async fn handshake(&mut self, pid: Pid, at: Provenance) {
        if let Some(open) = &self.open {
            let xact = open.xact;
            self.out
                .emit(at, TransactionEvent::Handshake { xact, pid })
                .await;
        }
    }

    /// Ends the open transaction, if any, with `reason`.
    async fn close_open(&mut self, reason: XactEnd, at: Provenance) {
        if let Some(open) = self.open.take() {
            let end = TransactionEvent::End {
                xact: open.xact,
                reason,
            };
            self.out.emit(at, end).await;
        }
    }
}

/// The body of the decoder: one packet, read against the open transaction.
async fn packet(mut pkt: Packet<'_>, out: &mut XactOut) -> Result<(), ReadError> {
    let (pid, ok) = match pkt.field_or_end().await? {
        Read::Field(Field::Pid { pid, ok }) => (pid, ok),
        // A SYNC with no PID after it, or fields before one.
        Read::End | Read::Field(_) => return Err(ReadError::Incomplete),
    };
    if !ok {
        return Err(ReadError::Damaged(EndReason::PidError));
    }
    if pid.is_token() || matches!(pid, Pid::Sof | Pid::Pre | Pid::Split) {
        // The host has moved on: whatever waited for an answer got none.
        let header = pkt.consumed();
        out.close_open(XactEnd::NoResponse, Provenance::at(header.settled))
            .await;
        return token(pid, header, pkt, out).await;
    }
    if pid.is_data() {
        if !matches!(out.stage(), Some(Stage::HostData | Stage::DeviceData)) {
            out.close_open(XactEnd::Error, pkt.consumed()).await;
            return pkt.drain().await;
        }
        // The data events cover their bytes alone.
        pkt.consumed();
        let mut forwarded = false;
        loop {
            match pkt.field().await? {
                Field::Data { offset, bytes } => {
                    let at = pkt.consumed();
                    out.data(pid, offset, bytes, at).await;
                    forwarded = true;
                }
                Field::DataEnd { ok, length, .. } => {
                    let at = pkt.consumed();
                    if !ok {
                        out.close_open(XactEnd::Error, at).await;
                    } else {
                        if !forwarded {
                            // A packet with no payload still says which data PID it was.
                            out.data(pid, 0, Vec::new(), at).await;
                        }
                        out.data_end(length, at).await;
                    }
                    return pkt.drain().await;
                }
                _ => return Err(ReadError::Incomplete),
            }
        }
    }
    if pid.is_handshake() {
        let at = pkt.consumed();
        let Some(stage) = out.stage() else {
            return pkt.drain().await;
        };
        out.handshake(pid, at).await;
        pkt.drain().await?;
        let reason = match (stage, pid) {
            (Stage::DeviceHandshake, Pid::Ack | Pid::Nyet)
            | (Stage::HostHandshake | Stage::PingHandshake, Pid::Ack) => XactEnd::Complete,
            (Stage::DeviceData | Stage::DeviceHandshake | Stage::PingHandshake, Pid::Nak) => {
                XactEnd::Nak
            }
            (Stage::DeviceData | Stage::DeviceHandshake | Stage::PingHandshake, Pid::Stall) => {
                XactEnd::Stall
            }
            _ => XactEnd::Error,
        };
        out.close_open(reason, pkt.consumed()).await;
        return Ok(());
    }
    // ERR and the reserved PID: nothing a transaction expects.
    out.close_open(XactEnd::Error, pkt.consumed()).await;
    pkt.drain().await
}

/// A packet the host sends to start something: a token begins a transaction; PRE and SPLIT
/// begin none this decoder can follow.
async fn token(
    pid: Pid,
    header: Provenance,
    mut pkt: Packet<'_>,
    out: &mut XactOut,
) -> Result<(), ReadError> {
    let token = match pid {
        Pid::Setup => Token::Setup,
        Pid::In => Token::In,
        Pid::Out => Token::Out,
        Pid::Ping => Token::Ping,
        Pid::Sof => Token::Sof,
        _ => return pkt.drain().await,
    };
    match pkt.field().await? {
        Field::Sof { frame, ok, .. } if token == Token::Sof => {
            let at = header.join(pkt.consumed());
            out.begin(token, None, None, Some(frame), at).await;
            pkt.drain().await?;
            let reason = if ok {
                XactEnd::Complete
            } else {
                XactEnd::Error
            };
            out.close_open(reason, pkt.consumed()).await;
        }
        Field::Token {
            address,
            endpoint,
            ok,
            ..
        } if token != Token::Sof => {
            let at = header.join(pkt.consumed());
            out.begin(token, Some(address), Some(endpoint), None, at)
                .await;
            if !ok {
                // The device ignores a damaged token.
                out.close_open(XactEnd::Error, at).await;
            }
            pkt.drain().await?;
        }
        _ => return Err(ReadError::Incomplete),
    }
    Ok(())
}

/// Closes the open transaction as the packet's end calls for. The end covers what was read
/// without being reported, and the packet's end when the stream carried one; a gap or the end
/// of the stream only places it.
async fn recover(error: ReadError, bus: &mut UsbBus, out: &mut XactOut) {
    let unreported = bus.consumed();
    let placed = |tick: Tick| Provenance {
        span: if unreported.span.is_empty() {
            TickRange::at(tick)
        } else {
            unreported.span
        },
        settled: tick,
    };
    let (reason, at) = match error {
        ReadError::Incomplete | ReadError::Damaged(_) => (XactEnd::Error, unreported),
        ReadError::Truncated => (XactEnd::Aborted, unreported),
        ReadError::Gap(range) => (XactEnd::Aborted, placed(range.start)),
        ReadError::EndOfInput(tick) => (XactEnd::Aborted, placed(tick)),
    };
    out.close_open(reason, at).await;
}

async fn run(mut bus: UsbBus, mut out: XactOut) {
    loop {
        match bus.next().await {
            Next::Packet(pkt) => {
                if let Err(error) = packet(pkt, &mut out).await {
                    recover(error, &mut bus, &mut out).await;
                }
                if let Err(error) = bus.finish().await {
                    recover(error, &mut bus, &mut out).await;
                }
                bus.consumed();
            }
            Next::Bus(_) => {
                let at = bus.consumed();
                out.close_open(XactEnd::NoResponse, Provenance::at(at.settled))
                    .await;
            }
            Next::Gap(range) => {
                out.close_open(XactEnd::Aborted, Provenance::at(range.start))
                    .await;
                out.out.gap(range).await;
            }
            Next::Progress(tick) => out.out.progress(tick).await,
            Next::End(tick) => {
                out.close_open(XactEnd::Aborted, Provenance::at(tick)).await;
                out.out.end(tick).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_the_contract_spells_it() {
        let json = |event: &TransactionEvent| serde_json::to_string(event).expect("serializes");
        assert_eq!(
            json(&TransactionEvent::Begin {
                xact: XactId(1),
                token: Token::Setup,
                address: Some(0),
                endpoint: Some(0),
                frame: None,
            }),
            r#"{"kind":"begin","xact":1,"token":"setup","address":0,"endpoint":0,"frame":null}"#
        );
        assert_eq!(
            json(&TransactionEvent::Setup {
                xact: XactId(1),
                bm_request_type: 0x80,
                b_request: 6,
                w_value: 0x0100,
                w_index: 0,
                w_length: 64,
            }),
            r#"{"kind":"setup","xact":1,"bmRequestType":128,"bRequest":6,"wValue":256,"wIndex":0,"wLength":64}"#
        );
        assert_eq!(
            json(&TransactionEvent::Data {
                xact: XactId(2),
                pid: Pid::Data1,
                offset: 0,
                bytes: vec![0x12],
            }),
            r#"{"kind":"data","xact":2,"pid":"data1","offset":0,"bytes":[18]}"#
        );
        assert_eq!(
            json(&TransactionEvent::Handshake {
                xact: XactId(2),
                pid: Pid::Nak,
            }),
            r#"{"kind":"handshake","xact":2,"pid":"nak"}"#
        );
        assert_eq!(
            json(&TransactionEvent::End {
                xact: XactId(2),
                reason: XactEnd::NoResponse,
            }),
            r#"{"kind":"end","xact":2,"reason":"no-response"}"#
        );
    }

    #[test]
    fn setup_bytes_keep_the_first_eight() {
        let mut setup = SetupBytes::new();
        let at = Provenance::one(Tick(3));
        setup.add(0, &[1, 2, 3], at);
        setup.add(3, &[4, 5, 6, 7, 8, 9], at);
        assert_eq!(setup.bytes, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(setup.len, 9);
    }
}
