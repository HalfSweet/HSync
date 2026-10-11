// SPDX-License-Identifier: Apache-2.0

//! Reading a Microwire event stream transaction by transaction, for decoders built on
//! Microwire.
//!
//! [`MicrowireBus::next`] waits for a transaction to begin and hands out a [`Transaction`], a
//! cursor over its bits: each read consumes one bit, with both lines' levels, and can wait
//! across blocks. A transaction's normal end is only ever returned by
//! [`Transaction::bit_or_end`]; everywhere else, running out of bits is an error that tells the
//! decoder how the transaction ended, so that it can close what it had opened.
//!
//! The cursor offers the contract's primitives and nothing more: the next bit, the normal end
//! with DO's level there, draining. What a device makes of the bits — start bits, opcodes,
//! address and word widths — is that device's own code.

use uiharu_core::{Event, Inbox, Item, Provenance, Tick, TickRange};

use super::events::{EndReason, MicrowireEvent, TxId};

/// Why a read did not return a bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// Chip select was released where a bit was expected: the transaction is whole, what the
    /// decoder was reading is not.
    #[error("the transaction ended before the bit")]
    Incomplete,
    /// The transaction was aborted: the samples ended or a gap began while it was under way.
    #[error("the transaction was cut off")]
    Truncated,
    /// Samples are missing from here on; the gap itself follows from [`MicrowireBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`MicrowireBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
    /// The bit has no DO level: the line is not connected.
    #[error("DO is not connected")]
    NoDataOut,
}

/// One clock pulse as both lines carried it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bit {
    /// DI at the rising edge.
    pub di: bool,
    /// DO at the falling edge; `None` when DO is not connected.
    pub r#do: Option<bool>,
}

impl Bit {
    /// DO's level; an error when the line is not connected.
    pub fn data_out(&self) -> Result<bool, ReadError> {
        self.r#do.ok_or(ReadError::NoDataOut)
    }
}

/// What [`Transaction::bit_or_end`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next bit.
    Bit(Bit),
    /// Chip select was released: the transaction ended normally.
    End {
        /// DO's level on the transaction's last sample; `None` when DO is not connected.
        r#do: Option<bool>,
    },
}

/// What [`MicrowireBus::next`] finds.
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

/// A decoder's reader on a Microwire event stream.
#[derive(Debug)]
pub struct MicrowireBus {
    inbox: Inbox<MicrowireEvent>,
    current: Option<Current>,
    /// The ticks of the items consumed since [`Transaction::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

/// What the next item of the stream means to the transaction being read.
enum Peeked {
    Bit(Event<MicrowireEvent>, Bit),
    End(Event<MicrowireEvent>, EndReason, Option<bool>),
    Foreign,
    Gap(TickRange),
    Progress,
    EndOfInput(Tick),
}

impl MicrowireBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<MicrowireEvent>) -> Self {
        Self {
            inbox,
            current: None,
            consumed: None,
            last_settled: Tick::ZERO,
        }
    }

    fn record(&mut self, event: &Event<MicrowireEvent>) {
        let at = Provenance::over(event.span, event.settled);
        self.consumed = Some(self.consumed.map_or(at, |have| have.join(at)));
        self.last_settled = event.settled;
    }

    /// The ticks of what was consumed since this was last called; an empty span where the
    /// last item was settled when nothing was.
    pub fn consumed(&mut self) -> Provenance {
        self.consumed
            .take()
            .unwrap_or(Provenance::at(self.last_settled))
    }

    /// Waits for the next transaction, gap, progress or end. Bits and ends outside a
    /// transaction are skipped: their beginning was consumed by an earlier transaction that
    /// was not drained, which [`MicrowireBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(&event);
                    if let MicrowireEvent::Begin { tx, partial } = event.payload {
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

    /// Takes the next bit of the current transaction, or its normal end, or reports how it
    /// ended instead.
    async fn next_bit(&mut self) -> Result<Read, ReadError> {
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
                    Item::Event(event) => match event.payload {
                        MicrowireEvent::Bit {
                            tx: bit_tx,
                            di,
                            r#do,
                        } if bit_tx == tx => Peeked::Bit(event.clone(), Bit { di, r#do }),
                        MicrowireEvent::End {
                            tx: end_tx,
                            reason,
                            r#do,
                            ..
                        } if end_tx == tx => Peeked::End(event.clone(), reason, r#do),
                        _ => Peeked::Foreign,
                    },
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(_) => Peeked::Progress,
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let result = match peeked {
                Peeked::Bit(event, bit) => {
                    self.consume(&event);
                    return Ok(Read::Bit(bit));
                }
                Peeked::End(event, EndReason::Release, r#do) => {
                    self.consume(&event);
                    Ok(Read::End { r#do })
                }
                Peeked::End(event, EndReason::Aborted, _) => {
                    self.consume(&event);
                    Err(ReadError::Truncated)
                }
                // Progress inside a transaction says nothing about its bits.
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

    /// Moves past `event`, charging the budget and recording its ticks.
    fn consume(&mut self, event: &Event<MicrowireEvent>) {
        self.inbox.take();
        self.record(event);
    }
}

/// A cursor over the bits of one transaction.
#[derive(Debug)]
pub struct Transaction<'a> {
    bus: &'a mut MicrowireBus,
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
    /// start of the samples or after a gap.
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

    /// The ticks of what was consumed since this was last called; see
    /// [`MicrowireBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next bit; the transaction's end, normal or not, is an error here. A sequence
    /// of reads that pauses between them keeps what it has read; nothing is read twice.
    pub async fn bit(&mut self) -> Result<Bit, ReadError> {
        match self.bit_or_end().await? {
            Read::Bit(bit) => Ok(bit),
            Read::End { .. } => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next bit, or finds the transaction's normal end.
    pub async fn bit_or_end(&mut self) -> Result<Read, ReadError> {
        self.bus.next_bit().await
    }

    /// Consumes the rest of the transaction up to its normal end.
    pub async fn drain(&mut self) -> Result<(), ReadError> {
        loop {
            if let Read::End { .. } = self.bus.next_bit().await? {
                return Ok(());
            }
        }
    }
}
