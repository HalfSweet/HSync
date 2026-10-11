// SPDX-License-Identifier: Apache-2.0

//! Reading a USB PD event stream packet by packet, for decoders built on USB PD messages.
//!
//! [`UsbPdBus::next`] waits for a packet to begin and hands out a [`Packet`], a cursor over its
//! fields: each read consumes one field — the ordered set or the reset signalling, the header,
//! a data object or the power or request data object read from it, the extended header, a
//! batch of data, the CRC — and can wait across blocks. A packet's normal end — its EOP, or the
//! end of Hard Reset or Cable Reset signalling — is only ever returned by
//! [`Packet::field_or_end`]; everywhere else, running out of fields is an error that tells the
//! decoder how the packet ended, so that it can close what it had opened.
//!
//! The cursor offers the contract's primitives and nothing more: the next field, the normal
//! end, draining. What messages mean together — the exchanges that negotiate a contract, a
//! role swap — is the consuming decoder's own code.

use uiharu_core::{Event, Inbox, Item, Provenance, Tick, TickRange};

use super::events::{
    EndReason, ErrorReason, ExtendedMessageHeader, MessageHeader, OrderedSet, PacketId,
    PowerObject, RequestObject, UsbPdEvent,
};

/// Why a read did not return a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The packet ended normally where a field was expected.
    #[error("the packet ended before the field")]
    Incomplete,
    /// The packet ended in an error on the line: the reason its `error` event gave, `None`
    /// when the stream had none.
    #[error("the packet was damaged ({0:?})")]
    Damaged(Option<ErrorReason>),
    /// The packet was cut off: the samples ended or a gap began while it was under way.
    #[error("the packet was cut off")]
    Truncated,
    /// Samples are missing from here on; the gap itself follows from [`UsbPdBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`UsbPdBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
}

/// A field of a packet: what the packet carries between its begin and its end, as the
/// contract's events of the same names carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// The ordered set of a message.
    Sop {
        /// Which set it is.
        set: OrderedSet,
        /// The position of a corrupted K-code, if one was.
        corrupt: Option<u8>,
    },
    /// Hard Reset signalling: the packet's only field.
    HardReset {
        /// The position of a corrupted K-code, if one was.
        corrupt: Option<u8>,
    },
    /// Cable Reset signalling: the packet's only field.
    CableReset {
        /// The position of a corrupted K-code, if one was.
        corrupt: Option<u8>,
    },
    /// The message header.
    Header(MessageHeader),
    /// The extended message header.
    ExtendedHeader(ExtendedMessageHeader),
    /// A data object.
    Object {
        /// Its position, from 1.
        index: u8,
        /// Its 32 bits.
        raw: u32,
    },
    /// The data object before, read as a power data object.
    Pdo {
        /// Its position, from 1.
        index: u8,
        /// Its type and fields.
        object: PowerObject,
    },
    /// The data object before, read as a request data object.
    Rdo(RequestObject),
    /// Data bytes of an extended message.
    Data {
        /// The position of the first byte after the extended header.
        offset: u16,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// The CRC.
    Crc {
        /// The 32 bits as they are sent.
        value: u32,
        /// Whether it matches.
        ok: bool,
    },
}

/// What [`Packet::field_or_end`] finds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next field.
    Field(Field),
    /// The packet ended normally: with its EOP, or after the ordered set of reset signalling.
    End,
}

/// What [`UsbPdBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a> {
    /// A packet begins.
    Packet(Packet<'a>),
    /// Samples, and whatever was on the line, are missing.
    Gap(TickRange),
    /// Nothing more comes before this tick.
    Progress(Tick),
    /// The stream ended.
    End(Tick),
}

#[derive(Debug)]
struct Current {
    pkt: PacketId,
    /// Whether the packet's end was reached (or the stream failed under it).
    ended: bool,
    /// What the packet's `error` event said went wrong.
    error: Option<ErrorReason>,
}

/// A decoder's reader on a USB PD event stream.
#[derive(Debug)]
pub struct UsbPdBus {
    inbox: Inbox<UsbPdEvent>,
    current: Option<Current>,
    /// The ticks of the items consumed since [`UsbPdBus::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

impl UsbPdBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<UsbPdEvent>) -> Self {
        Self {
            inbox,
            current: None,
            consumed: None,
            last_settled: Tick::ZERO,
        }
    }

    fn record(&mut self, at: Provenance) {
        self.consumed = Some(self.consumed.map_or(at, |have| have.join(at)));
        self.last_settled = at.settled;
    }

    /// The ticks of what was consumed since this was last called; an empty span where the
    /// last item was settled when nothing was.
    pub fn consumed(&mut self) -> Provenance {
        self.consumed
            .take()
            .unwrap_or(Provenance::at(self.last_settled))
    }

    /// Waits for the next packet, gap, progress or end. Fields and ends outside a packet are
    /// skipped: their beginning was consumed by an earlier packet that was not drained, which
    /// [`UsbPdBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(provenance(&event));
                    if let UsbPdEvent::Begin { pkt } = event.payload {
                        self.current = Some(Current {
                            pkt,
                            ended: false,
                            error: None,
                        });
                        return Next::Packet(Packet { bus: self });
                    }
                }
                Item::Gap(range) => return Next::Gap(range),
                Item::Progress(tick) => return Next::Progress(tick),
                Item::End(tick) => return Next::End(tick),
            }
        }
    }

    /// Consumes what is left of the current packet, if the decoder left it open. Returns how
    /// the packet ended when it did not end normally.
    pub async fn finish(&mut self) -> Result<(), ReadError> {
        let result = match &self.current {
            Some(current) if !current.ended => Packet { bus: self }.drain().await,
            _ => Ok(()),
        };
        self.current = None;
        result
    }

    /// Takes the next field of the current packet, or reports how it ended instead.
    async fn next_field(&mut self) -> Result<Option<Field>, ReadError> {
        let Some(current) = self.current.as_ref() else {
            return Err(ReadError::Incomplete);
        };
        if current.ended {
            return Err(ReadError::Incomplete);
        }
        let pkt = current.pkt;
        loop {
            let peeked = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) => match own(&event.payload, pkt) {
                        Some(own) => Peeked::Own(provenance(event), own),
                        None => Peeked::Foreign,
                    },
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(_) => Peeked::Progress,
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let result = match peeked {
                Peeked::Own(at, Own::Field(field)) => {
                    self.consume(at);
                    return Ok(Some(field));
                }
                // The error comes before the end it causes: noted, and the end read next.
                Peeked::Own(at, Own::Error(reason)) => {
                    self.consume(at);
                    if let Some(current) = self.current.as_mut() {
                        current.error = Some(reason);
                    }
                    continue;
                }
                Peeked::Own(at, Own::End(reason)) => {
                    self.consume(at);
                    match reason {
                        EndReason::Eop | EndReason::HardReset | EndReason::CableReset => Ok(None),
                        EndReason::Aborted => Err(ReadError::Truncated),
                        EndReason::Error => Err(ReadError::Damaged(
                            self.current.as_ref().and_then(|current| current.error),
                        )),
                    }
                }
                // Progress inside a packet says nothing about its fields.
                Peeked::Progress => {
                    self.inbox.take();
                    continue;
                }
                // Another packet's event: this packet never ended. It stays for `next`.
                Peeked::Foreign => Err(ReadError::Truncated),
                // The gap and the end stay in the stream too, for `next` to report.
                Peeked::Gap(range) => Err(ReadError::Gap(range)),
                Peeked::EndOfInput(tick) => Err(ReadError::EndOfInput(tick)),
            };
            if let Some(current) = self.current.as_mut() {
                current.ended = true;
            }
            return result;
        }
    }

    /// Moves past the item peeked at, charging the budget and recording its ticks.
    fn consume(&mut self, at: Provenance) {
        self.inbox.take();
        self.record(at);
    }
}

/// Where `event` comes from.
fn provenance(event: &Event<UsbPdEvent>) -> Provenance {
    Provenance::over(event.span, event.settled)
}

/// What an event of the packet being read is to it.
enum Own {
    Field(Field),
    Error(ErrorReason),
    End(EndReason),
}

/// `payload` as a field, the error or the end of packet `pkt`; `None` when it belongs
/// elsewhere.
fn own(payload: &UsbPdEvent, pkt: PacketId) -> Option<Own> {
    if payload.pkt() != pkt {
        return None;
    }
    Some(match payload {
        UsbPdEvent::Sop { set, corrupt, .. } => Own::Field(Field::Sop {
            set: *set,
            corrupt: *corrupt,
        }),
        UsbPdEvent::HardReset { corrupt, .. } => Own::Field(Field::HardReset { corrupt: *corrupt }),
        UsbPdEvent::CableReset { corrupt, .. } => {
            Own::Field(Field::CableReset { corrupt: *corrupt })
        }
        UsbPdEvent::Header { header, .. } => Own::Field(Field::Header(*header)),
        UsbPdEvent::ExtendedHeader { header, .. } => Own::Field(Field::ExtendedHeader(*header)),
        UsbPdEvent::Object { index, raw, .. } => Own::Field(Field::Object {
            index: *index,
            raw: *raw,
        }),
        UsbPdEvent::Pdo { index, object, .. } => Own::Field(Field::Pdo {
            index: *index,
            object: *object,
        }),
        UsbPdEvent::Rdo { request, .. } => Own::Field(Field::Rdo(*request)),
        UsbPdEvent::Data { offset, bytes, .. } => Own::Field(Field::Data {
            offset: *offset,
            bytes: bytes.clone(),
        }),
        UsbPdEvent::Crc { value, ok, .. } => Own::Field(Field::Crc {
            value: *value,
            ok: *ok,
        }),
        UsbPdEvent::Error { reason, .. } => Own::Error(*reason),
        UsbPdEvent::End { reason, .. } => Own::End(*reason),
        // A second begin of the same packet is not its field.
        UsbPdEvent::Begin { .. } => return None,
    })
}

/// What the next item of the stream means to the packet being read.
enum Peeked {
    Own(Provenance, Own),
    Foreign,
    Gap(TickRange),
    Progress,
    EndOfInput(Tick),
}

/// A cursor over the fields of one packet.
#[derive(Debug)]
pub struct Packet<'a> {
    bus: &'a mut UsbPdBus,
}

impl Packet<'_> {
    /// The packet's id.
    pub fn id(&self) -> PacketId {
        self.bus
            .current
            .as_ref()
            .map_or(PacketId(0), |current| current.pkt)
    }

    /// Whether the packet has ended.
    pub fn is_ended(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_none_or(|current| current.ended)
    }

    /// The ticks of what was consumed since this was last called; see
    /// [`UsbPdBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next field; the packet's end, normal or not, is an error here.
    pub async fn field(&mut self) -> Result<Field, ReadError> {
        match self.field_or_end().await? {
            Read::Field(field) => Ok(field),
            Read::End => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next field, or finds the packet's normal end.
    pub async fn field_or_end(&mut self) -> Result<Read, ReadError> {
        Ok(self.bus.next_field().await?.map_or(Read::End, Read::Field))
    }

    /// Consumes the rest of the packet up to its normal end.
    pub async fn drain(&mut self) -> Result<(), ReadError> {
        loop {
            if self.bus.next_field().await?.is_none() {
                return Ok(());
            }
        }
    }
}
