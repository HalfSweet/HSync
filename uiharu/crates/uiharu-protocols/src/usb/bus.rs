// SPDX-License-Identifier: Apache-2.0

//! Reading a USB packet stream packet by packet, for decoders built on USB packets.
//!
//! [`UsbBus::next`] waits for a packet to begin and hands out a [`Packet`], a cursor over its
//! fields: each read consumes one field — the PID, a token's or SOF's fields, a batch of data,
//! the end of a data packet's payload, SPLIT's bits — and can wait across blocks. A packet's
//! normal end is only ever returned by [`Packet::field_or_end`]; everywhere else, running out
//! of fields is an error that tells the decoder how the packet ended, so that it can close what
//! it had opened. Between packets, `next` hands out the bus's own events: resets, suspends,
//! resumes and keep-alives.
//!
//! The cursor offers the contract's primitives and nothing more: the next field, the normal
//! end, draining. What packets mean together — transactions, transfers — is the consuming
//! decoder's own code.

use uiharu_core::{Event, Inbox, Item, Provenance, Tick, TickRange};

use super::events::{EndReason, PacketId, Pid, UsbEvent};

/// Why a read did not return a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The packet ended normally where a field was expected.
    #[error("the packet ended before the field")]
    Incomplete,
    /// The packet ended on a line error: a bit stuff error, a failed PID check or SE1.
    #[error("the packet was damaged ({0:?})")]
    Damaged(EndReason),
    /// The packet was cut off: the samples ended or a gap began while it was under way.
    #[error("the packet was cut off")]
    Truncated,
    /// Samples are missing from here on; the gap itself follows from [`UsbBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`UsbBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
}

/// A field of a packet: what the packet carries between its begin and its end, as the
/// contract's events of the same names carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// The PID.
    Pid {
        /// The type.
        pid: Pid,
        /// Whether the check nibble matches.
        ok: bool,
    },
    /// A token's fields.
    Token {
        /// ADDR.
        address: u8,
        /// ENDP.
        endpoint: u8,
        /// The CRC5 field.
        crc5: u8,
        /// Whether the CRC5 matches.
        ok: bool,
    },
    /// A start-of-frame packet's fields.
    Sof {
        /// The frame number.
        frame: u16,
        /// The CRC5 field.
        crc5: u8,
        /// Whether the CRC5 matches.
        ok: bool,
    },
    /// Payload bytes.
    Data {
        /// The position of the first byte in the payload.
        offset: u32,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// The end of the payload.
    DataEnd {
        /// The CRC16 field, if two whole bytes were there to make it.
        crc16: Option<u16>,
        /// Whether the CRC16 matches.
        ok: bool,
        /// The payload's length.
        length: u16,
    },
    /// The bits after a SPLIT PID.
    Split {
        /// The bits, first in bit 0.
        raw: u32,
    },
}

/// What [`Packet::field_or_end`] finds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next field.
    Field(Field),
    /// The packet ended normally: with an EOP, or after a preamble's PID.
    End,
}

/// What happened on the bus between packets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusEvent {
    /// A reset, and how long its SE0 lasted.
    Reset {
        /// The SE0's length.
        ticks: u64,
    },
    /// The bus has been idle for 3 ms.
    Suspend,
    /// Resume signalling.
    Resume,
    /// A low-speed keep-alive.
    KeepAlive,
}

/// What [`UsbBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a> {
    /// A packet begins.
    Packet(Packet<'a>),
    /// The bus signalled between packets.
    Bus(BusEvent),
    /// Samples, and whatever was on the bus, are missing.
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
}

/// A decoder's reader on a USB packet stream.
#[derive(Debug)]
pub struct UsbBus {
    inbox: Inbox<UsbEvent>,
    current: Option<Current>,
    /// The ticks of the items consumed since [`UsbBus::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

impl UsbBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<UsbEvent>) -> Self {
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

    /// Waits for the next packet, bus event, gap, progress or end. Fields and ends outside a
    /// packet are skipped: their beginning was consumed by an earlier packet that was not
    /// drained, which [`UsbBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(provenance(&event));
                    let bus = match event.payload {
                        UsbEvent::Begin { pkt } => {
                            self.current = Some(Current { pkt, ended: false });
                            return Next::Packet(Packet { bus: self });
                        }
                        UsbEvent::Reset { ticks } => BusEvent::Reset { ticks },
                        UsbEvent::Suspend => BusEvent::Suspend,
                        UsbEvent::Resume => BusEvent::Resume,
                        UsbEvent::KeepAlive => BusEvent::KeepAlive,
                        _ => continue,
                    };
                    return Next::Bus(bus);
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
                Peeked::Own(at, Own::End(reason)) => {
                    self.consume(at);
                    match reason {
                        EndReason::Eop | EndReason::Preamble => Ok(None),
                        EndReason::Aborted => Err(ReadError::Truncated),
                        damage => Err(ReadError::Damaged(damage)),
                    }
                }
                // Progress inside a packet says nothing about its fields.
                Peeked::Progress => {
                    self.inbox.take();
                    continue;
                }
                // Another packet's event, or the bus's: this packet never ended. It stays for
                // `next`.
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
fn provenance(event: &Event<UsbEvent>) -> Provenance {
    Provenance::over(event.span, event.settled)
}

/// What an event of the packet being read is to it.
enum Own {
    Field(Field),
    End(EndReason),
}

/// `payload` as a field or the end of packet `pkt`; `None` when it belongs elsewhere.
fn own(payload: &UsbEvent, pkt: PacketId) -> Option<Own> {
    if payload.pkt() != Some(pkt) {
        return None;
    }
    Some(match payload {
        UsbEvent::Pid { pid, ok, .. } => Own::Field(Field::Pid { pid: *pid, ok: *ok }),
        UsbEvent::Token {
            address,
            endpoint,
            crc5,
            ok,
            ..
        } => Own::Field(Field::Token {
            address: *address,
            endpoint: *endpoint,
            crc5: *crc5,
            ok: *ok,
        }),
        UsbEvent::Sof {
            frame, crc5, ok, ..
        } => Own::Field(Field::Sof {
            frame: *frame,
            crc5: *crc5,
            ok: *ok,
        }),
        UsbEvent::Data { offset, bytes, .. } => Own::Field(Field::Data {
            offset: *offset,
            bytes: bytes.clone(),
        }),
        UsbEvent::DataEnd {
            crc16, ok, length, ..
        } => Own::Field(Field::DataEnd {
            crc16: *crc16,
            ok: *ok,
            length: *length,
        }),
        UsbEvent::Split { raw, .. } => Own::Field(Field::Split { raw: *raw }),
        UsbEvent::End { reason, .. } => Own::End(*reason),
        // A second begin of the same packet is not its field.
        UsbEvent::Begin { .. }
        | UsbEvent::Reset { .. }
        | UsbEvent::Suspend
        | UsbEvent::Resume
        | UsbEvent::KeepAlive => return None,
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
    bus: &'a mut UsbBus,
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

    /// The ticks of what was consumed since this was last called; see [`UsbBus::consumed`].
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
