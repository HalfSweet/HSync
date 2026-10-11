// SPDX-License-Identifier: Apache-2.0

//! Reading an I²C event stream transaction by transaction, for decoders built on I²C.
//!
//! [`I2cBus::next`] waits for a transaction to begin and hands out a [`Transaction`], a cursor
//! over its events: each read consumes one address, data byte or repeated START, and can wait
//! across blocks. A transaction's normal end is only ever returned by
//! [`Transaction::event_or_end`]; everywhere else, running out of events is an error that
//! tells the decoder how the transaction ended, so that it can close what it had opened.
//!
//! The cursor offers the contract's primitives and nothing more: the next event, the normal
//! end, draining. What a protocol makes of the bytes — which addresses are its own, how wide a
//! register address is, byte order — is that protocol's own code, so that the contract stays as
//! general as the bus.

use uiharu_core::{Inbox, Item, Provenance, Tick, TickRange};

use super::events::{EndReason, I2cEvent, TxId};

/// Why a read did not return an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The transaction ended normally where an event was expected: it is whole, what the
    /// decoder was reading is not.
    #[error("the transaction ended before the event")]
    Incomplete,
    /// The transaction was aborted: the samples ended or a gap began while it was under way.
    #[error("the transaction was cut off")]
    Truncated,
    /// Samples are missing from here on; the gap itself follows from [`I2cBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`I2cBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
}

/// An address as the bus carried it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    /// The 7-bit or 10-bit address.
    pub address: u16,
    /// Whether it is a 10-bit address.
    pub ten_bit: bool,
    /// The R/W̄ bit: whether the controller reads from the target.
    pub read: bool,
    /// Whether it was acknowledged.
    pub ack: bool,
}

/// One event of a transaction, as the cursor hands it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TxEvent {
    /// An address: the first event of a transaction and the first after each repeated START.
    Address(Address),
    /// A data byte.
    Data {
        /// The byte.
        byte: u8,
        /// Whether the receiver acknowledged it.
        ack: bool,
    },
    /// A repeated START; an address follows.
    Restart {
        /// Bits of an unfinished byte it cut short.
        dropped_bits: u8,
    },
}

/// What [`Transaction::event_or_end`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next event.
    Event(TxEvent),
    /// The transaction ended normally: a STOP, or, for a partial transaction, the START of
    /// the next one.
    End,
}

/// What [`I2cBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a> {
    /// A transaction begins.
    Transaction(Transaction<'a>),
    /// Samples, and whatever was on the bus, are missing.
    Gap(TickRange),
    /// Nothing more comes before this tick.
    Progress(Tick),
    /// The stream ended.
    End(Tick),
}

#[derive(Debug)]
struct Current {
    tx: TxId,
    partial: bool,
    /// Whether the transaction's end was reached (or the stream failed under it).
    ended: bool,
}

/// A decoder's reader on an I²C event stream.
#[derive(Debug)]
pub struct I2cBus {
    inbox: Inbox<I2cEvent>,
    current: Option<Current>,
    /// The ticks of the items consumed since [`Transaction::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

/// What the next item of the stream means to the transaction being read.
enum Peeked {
    Event(Provenance, TxEvent),
    End(Provenance, EndReason),
    Foreign,
    Gap(TickRange),
    Progress,
    EndOfInput(Tick),
}

impl I2cBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<I2cEvent>) -> Self {
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

    /// Waits for the next transaction, gap, progress or end. Events outside a transaction are
    /// skipped: their beginning was consumed by an earlier transaction that was not drained,
    /// which [`I2cBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(Provenance::over(event.span, event.settled));
                    if let I2cEvent::Begin { tx, partial } = event.payload {
                        self.current = Some(Current {
                            tx,
                            partial,
                            ended: false,
                        });
                        return Next::Transaction(Transaction { bus: self });
                    }
                }
                Item::Gap(range) => return Next::Gap(range),
                Item::Progress(tick) => return Next::Progress(tick),
                Item::End(tick) => return Next::End(tick),
            }
        }
    }

    /// Consumes what is left of the current transaction, if the decoder left it open.
    /// Returns how the transaction ended when it did not end normally.
    pub async fn finish(&mut self) -> Result<(), ReadError> {
        let result = match &self.current {
            Some(current) if !current.ended => Transaction { bus: self }.drain().await,
            _ => Ok(()),
        };
        self.current = None;
        result
    }

    /// Takes the next event of the current transaction, or reports how it ended instead.
    async fn next_event(&mut self) -> Result<Option<TxEvent>, ReadError> {
        let Some(current) = self.current.as_ref() else {
            return Err(ReadError::Incomplete);
        };
        if current.ended {
            return Err(ReadError::Incomplete);
        }
        let tx = current.tx;
        loop {
            let peeked = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) if event.payload.tx() == tx => {
                        let at = Provenance::over(event.span, event.settled);
                        match event.payload {
                            I2cEvent::Address {
                                address,
                                ten_bit,
                                read,
                                ack,
                                ..
                            } => Peeked::Event(
                                at,
                                TxEvent::Address(Address {
                                    address,
                                    ten_bit,
                                    read,
                                    ack,
                                }),
                            ),
                            I2cEvent::Data { byte, ack, .. } => {
                                Peeked::Event(at, TxEvent::Data { byte, ack })
                            }
                            I2cEvent::Restart { dropped_bits, .. } => {
                                Peeked::Event(at, TxEvent::Restart { dropped_bits })
                            }
                            I2cEvent::End { reason, .. } => Peeked::End(at, reason),
                            I2cEvent::Begin { .. } => Peeked::Foreign,
                        }
                    }
                    Item::Event(_) => Peeked::Foreign,
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(_) => Peeked::Progress,
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let result = match peeked {
                Peeked::Event(at, event) => {
                    self.consume(at);
                    return Ok(Some(event));
                }
                Peeked::End(at, EndReason::Stop | EndReason::Restart) => {
                    self.consume(at);
                    Ok(None)
                }
                Peeked::End(at, EndReason::Aborted) => {
                    self.consume(at);
                    Err(ReadError::Truncated)
                }
                // Progress inside a transaction says nothing about its events.
                Peeked::Progress => {
                    self.inbox.take();
                    continue;
                }
                // Another transaction's event: this one never ended. It stays for `next`.
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

/// A cursor over the events of one transaction.
#[derive(Debug)]
pub struct Transaction<'a> {
    bus: &'a mut I2cBus,
}

impl Transaction<'_> {
    /// The transaction's id.
    pub fn id(&self) -> TxId {
        self.bus
            .current
            .as_ref()
            .map_or(TxId(0), |current| current.tx)
    }

    /// Whether the transaction's beginning was not seen: it was already under way at the
    /// start of the samples or after a gap, and carries no bytes.
    pub fn is_partial(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_some_and(|current| current.partial)
    }

    /// Whether the transaction has ended.
    pub fn is_ended(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_none_or(|current| current.ended)
    }

    /// The ticks of what was consumed since this was last called; see [`I2cBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next event; the transaction's end, normal or not, is an error here. A
    /// sequence of reads that pauses between them keeps what it has read; nothing is read
    /// twice.
    pub async fn event(&mut self) -> Result<TxEvent, ReadError> {
        match self.event_or_end().await? {
            Read::Event(event) => Ok(event),
            Read::End => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next event, or finds the transaction's normal end.
    pub async fn event_or_end(&mut self) -> Result<Read, ReadError> {
        Ok(self.bus.next_event().await?.map_or(Read::End, Read::Event))
    }

    /// Consumes the rest of the transaction up to its normal end.
    pub async fn drain(&mut self) -> Result<(), ReadError> {
        loop {
            if self.bus.next_event().await?.is_none() {
                return Ok(());
            }
        }
    }
}
