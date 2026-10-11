// SPDX-License-Identifier: Apache-2.0

//! Reading an SWD event stream packet by packet, for decoders built on SWD.
//!
//! [`SwdBus::next`] waits for the next packet, line reset, switch, gap, progress or end and
//! hands out a [`Packet`], a cursor over the fields of one packet — its request, acknowledge
//! and data, in the order the wire carried them. How the packet ended is only ever returned
//! by [`Packet::field_or_end`] and [`Packet::drain`]; [`Packet::field`] reports an end as an
//! error that says how the packet ended, so that the decoder can close what it had opened.
//!
//! The cursor offers the contract's primitives and nothing more: the next field, the end,
//! draining. What the fields mean — which debug or access port register an address selects,
//! what a value written there does — is the consuming decoder's own code.

use uiharu_core::{Event, Inbox, Item, ProgressSink, Provenance, Tick, TickRange};

use super::events::{Ack, EndReason, Interface, SwdEvent, TxId};

/// A packet request, as the host sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Request {
    /// APnDP: the request is for an access port, not the debug port.
    pub apndp: bool,
    /// RnW: the request reads, rather than writes.
    pub rnw: bool,
    /// A\[3:2\] as a byte address: `0x0`, `0x4`, `0x8` or `0xC`.
    pub addr: u8,
    /// Whether the request's parity bit was right.
    pub parity_ok: bool,
}

/// A packet's data phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Data {
    /// The data word.
    pub value: u32,
    /// Whether the data's parity bit was right.
    pub parity_ok: bool,
    /// Whether the target sent the word (a read) rather than the host (a write).
    pub from_target: bool,
}

/// One field of a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// The request.
    Request(Request),
    /// The acknowledge.
    Ack(Ack),
    /// The data phase.
    Data(Data),
}

/// What [`Packet::field_or_end`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next field.
    Field(Field),
    /// The packet ended on the wire: complete, after WAIT or FAULT, or with a protocol error.
    End(EndReason),
}

/// Why a read did not return a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The packet ended on the wire where a field was expected.
    #[error("the packet ended ({0:?}) before the field")]
    Ended(EndReason),
    /// The packet was cut off: the samples ended or were lost while it was under way.
    #[error("the packet was cut off")]
    Aborted,
    /// Samples are missing from here on; the gap itself follows from [`SwdBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`SwdBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
}

/// What [`SwdBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a> {
    /// A packet begins.
    Packet(Packet<'a>),
    /// A line reset.
    LineReset,
    /// A select sequence switched the port to an interface.
    Switch(Interface),
    /// Samples, and whatever was on the wire, are missing.
    Gap(TickRange),
    /// Nothing more comes before this tick.
    Progress(Tick),
    /// The stream ended.
    End(Tick),
}

#[derive(Debug)]
struct Current {
    tx: TxId,
    /// How the packet ended, once it has: `Aborted` too when the stream failed under it.
    end: Option<EndReason>,
}

impl Current {
    /// What reading the packet finds once it has ended.
    fn after_end(&self) -> Option<Result<Read, ReadError>> {
        self.end.map(|reason| match reason {
            EndReason::Aborted => Err(ReadError::Aborted),
            reason => Ok(Read::End(reason)),
        })
    }
}

/// A decoder's reader on an SWD event stream.
#[derive(Debug)]
pub struct SwdBus {
    inbox: Inbox<SwdEvent>,
    progress: ProgressSink,
    current: Option<Current>,
    /// The ticks of the items consumed since [`Packet::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

impl SwdBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<SwdEvent>, progress: ProgressSink) -> Self {
        Self {
            inbox,
            progress,
            current: None,
            consumed: None,
            last_settled: Tick::ZERO,
        }
    }

    fn record(&mut self, event: &Event<SwdEvent>) {
        let at = Provenance::over(event.span, event.settled);
        self.consumed = Some(self.consumed.map_or(at, |have| have.join(at)));
        self.last_settled = event.settled;
    }

    /// The ticks of what was consumed since this was last called; an empty span where the
    /// last item was settled when nothing was.
    /// Passes progress met inside a frame on, held back to what the decoder has read through
    /// this cursor and not reported yet: what was consumed since it last took the provenance,
    /// and what it says it still holds ([`ProgressSink::hold`]).
    async fn forward_progress(&mut self, tick: Tick) {
        let held = [self.consumed.map(|at| at.span.start), self.progress.held()]
            .into_iter()
            .flatten()
            .fold(tick, Tick::min);
        self.progress.progress(held).await;
    }

    pub fn consumed(&mut self) -> Provenance {
        self.consumed
            .take()
            .unwrap_or(Provenance::at(self.last_settled))
    }

    /// Waits for the next packet, line reset, switch, gap, progress or end. Fields and ends
    /// outside a packet are skipped: their beginning was consumed by an earlier packet that
    /// was not drained, which [`SwdBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(&event);
                    match event.payload {
                        SwdEvent::Begin { tx } => {
                            self.current = Some(Current { tx, end: None });
                            return Next::Packet(Packet { bus: self, tx });
                        }
                        SwdEvent::LineReset => return Next::LineReset,
                        SwdEvent::Switch { to } => return Next::Switch(to),
                        SwdEvent::Request { .. }
                        | SwdEvent::Ack { .. }
                        | SwdEvent::Data { .. }
                        | SwdEvent::End { .. } => {}
                    }
                }
                Item::Gap(range) => return Next::Gap(range),
                Item::Progress(tick) => return Next::Progress(tick),
                Item::End(tick) => return Next::End(tick),
            }
        }
    }

    /// Consumes what is left of the current packet, if the decoder left it open. Returns how
    /// it ended when it was cut off.
    pub async fn finish(&mut self) -> Result<(), ReadError> {
        let result = match &self.current {
            Some(current) if current.end.is_none() => self.drain().await.map(|_| ()),
            _ => Ok(()),
        };
        self.current = None;
        result
    }

    async fn drain(&mut self) -> Result<EndReason, ReadError> {
        loop {
            if let Read::End(reason) = self.next_field().await? {
                return Ok(reason);
            }
        }
    }

    /// Takes the next field of the current packet, or finds how it ended.
    async fn next_field(&mut self) -> Result<Read, ReadError> {
        let Some(current) = self.current.as_ref() else {
            return Err(ReadError::Aborted);
        };
        if let Some(after_end) = current.after_end() {
            return after_end;
        }
        let tx = current.tx;
        loop {
            let peeked = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) => match event.payload {
                        SwdEvent::Request {
                            tx: of,
                            apndp,
                            rnw,
                            addr,
                            parity_ok,
                        } if of == tx => Peeked::Field(
                            event.clone(),
                            Field::Request(Request {
                                apndp,
                                rnw,
                                addr,
                                parity_ok,
                            }),
                        ),
                        SwdEvent::Ack { tx: of, value } if of == tx => {
                            Peeked::Field(event.clone(), Field::Ack(value))
                        }
                        SwdEvent::Data {
                            tx: of,
                            value,
                            parity_ok,
                            from_target,
                        } if of == tx => Peeked::Field(
                            event.clone(),
                            Field::Data(Data {
                                value,
                                parity_ok,
                                from_target,
                            }),
                        ),
                        SwdEvent::End { tx: of, reason } if of == tx => {
                            Peeked::End(event.clone(), reason)
                        }
                        _ => Peeked::Foreign,
                    },
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(tick) => Peeked::Progress(*tick),
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let (end, result) = match peeked {
                Peeked::Field(event, field) => {
                    self.consume(&event);
                    return Ok(Read::Field(field));
                }
                // Progress inside a packet says nothing about its fields.
                Peeked::Progress(tick) => {
                    self.inbox.take();
                    self.forward_progress(tick).await;
                    continue;
                }
                Peeked::End(event, EndReason::Aborted) => {
                    self.consume(&event);
                    (EndReason::Aborted, Err(ReadError::Aborted))
                }
                Peeked::End(event, reason) => {
                    self.consume(&event);
                    (reason, Ok(Read::End(reason)))
                }
                // Another packet's event, or a line reset: this one never ended. It stays for
                // `next`.
                Peeked::Foreign => (EndReason::Aborted, Err(ReadError::Aborted)),
                // The gap and the end stay in the stream too, for `next` to report.
                Peeked::Gap(range) => (EndReason::Aborted, Err(ReadError::Gap(range))),
                Peeked::EndOfInput(tick) => (EndReason::Aborted, Err(ReadError::EndOfInput(tick))),
            };
            if let Some(current) = self.current.as_mut() {
                current.end = Some(end);
            }
            return result;
        }
    }

    /// Moves past `event`, charging the budget and recording its ticks.
    fn consume(&mut self, event: &Event<SwdEvent>) {
        self.inbox.take();
        self.record(event);
    }
}

/// What the next item of the stream means to the packet being read.
enum Peeked {
    Field(Event<SwdEvent>, Field),
    End(Event<SwdEvent>, EndReason),
    Foreign,
    Gap(TickRange),
    Progress(Tick),
    EndOfInput(Tick),
}

/// A cursor over the fields of one packet.
#[derive(Debug)]
pub struct Packet<'a> {
    bus: &'a mut SwdBus,
    tx: TxId,
}

impl Packet<'_> {
    /// The packet's id.
    pub fn id(&self) -> TxId {
        self.tx
    }

    /// Whether the packet has ended.
    pub fn is_ended(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_none_or(|current| current.end.is_some())
    }

    /// The ticks of what was consumed since this was last called; see [`SwdBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next field; the packet's end is an error here. A sequence of reads that
    /// pauses between them keeps what it has read; nothing is read twice.
    pub async fn field(&mut self) -> Result<Field, ReadError> {
        match self.field_or_end().await? {
            Read::Field(field) => Ok(field),
            Read::End(reason) => Err(ReadError::Ended(reason)),
        }
    }

    /// Reads the next field, or finds how the packet ended on the wire.
    pub async fn field_or_end(&mut self) -> Result<Read, ReadError> {
        self.bus.next_field().await
    }

    /// Consumes the rest of the packet up to its end, and says how it ended.
    pub async fn drain(&mut self) -> Result<EndReason, ReadError> {
        self.bus.drain().await
    }
}
