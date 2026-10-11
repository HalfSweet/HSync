// SPDX-License-Identifier: Apache-2.0

//! Reading an SPI event stream transaction by transaction, for decoders built on SPI.
//!
//! [`SpiBus::next`] waits for a transaction to begin and hands out a [`Transaction`], a cursor
//! over its words: each read consumes one word, with both lines' values, and can wait across
//! blocks. A transaction's normal end is only ever returned by [`Transaction::word_or_end`];
//! everywhere else, running out of words is an error that tells the decoder how the
//! transaction ended, so that it can close what it had opened.
//!
//! The cursor offers the contract's primitives and nothing more: the next word, the normal
//! end, draining. What a protocol makes of the words — which line it reads, byte order, field
//! widths — is that protocol's own code, so that the contract stays as general as the bus.
//!
//! A bus without chip select carries one unframed transaction, `0`, which has no `begin` or
//! `end` event: the cursor opens it at its first word, and it ends only at a gap or the end of
//! the stream. Draining it would consume the rest of the samples, so [`SpiBus::finish`] does
//! not drain it; a decoder that loses its place on such a bus calls [`SpiBus::next`] again,
//! which hands transaction `0` back at the next word.

use uiharu_core::{Event, Inbox, Item, Provenance, Tick, TickRange};

use super::events::{EndReason, SpiEvent, TxId};

/// A data line of the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Line {
    /// Master out, slave in.
    Mosi,
    /// Master in, slave out.
    Miso,
}

/// Why a read did not return a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// Chip select was released where a word was expected: the transaction is whole, the
    /// command on it is not.
    #[error("the transaction ended before the word")]
    Incomplete,
    /// The transaction was aborted: the samples ended or a gap began while it was under way.
    #[error("the transaction was cut off")]
    Truncated,
    /// Samples are missing from here on; the gap itself follows from [`SpiBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`SpiBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
    /// The word has no value on this line: it is not connected.
    #[error("{0:?} is not connected")]
    MissingLine(Line),
}

/// One word as both lines carried it; a line that is not connected reads `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Word {
    /// The word on MOSI.
    pub mosi: Option<u8>,
    /// The word on MISO.
    pub miso: Option<u8>,
}

impl Word {
    /// The word on MOSI; an error when the line is not connected.
    pub fn mosi(&self) -> Result<u8, ReadError> {
        self.mosi.ok_or(ReadError::MissingLine(Line::Mosi))
    }

    /// The word on MISO; an error when the line is not connected.
    pub fn miso(&self) -> Result<u8, ReadError> {
        self.miso.ok_or(ReadError::MissingLine(Line::Miso))
    }
}

/// What [`Transaction::word_or_end`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next word.
    Word(Word),
    /// Chip select was released: the transaction ended normally.
    End,
}

/// What [`SpiBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a> {
    /// A transaction begins; on a bus without chip select, transaction `0` continues from
    /// its next word.
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

/// A decoder's reader on an SPI event stream.
#[derive(Debug)]
pub struct SpiBus {
    inbox: Inbox<SpiEvent>,
    current: Option<Current>,
    /// The ticks of the items consumed since [`Transaction::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

impl SpiBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<SpiEvent>) -> Self {
        Self {
            inbox,
            current: None,
            consumed: None,
            last_settled: Tick::ZERO,
        }
    }

    fn record(&mut self, event: &Event<SpiEvent>) {
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

    /// Waits for the next transaction, gap, progress or end. Words and ends outside a
    /// transaction are skipped: their beginning was consumed by an earlier transaction that
    /// was not drained, which [`SpiBus::finish`] prevents. The words of a bus without chip
    /// select belong to transaction `0`, which has no beginning: it opens at its first word,
    /// which is left for the transaction to read.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            let start = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) => match event.payload {
                        SpiEvent::Begin { tx, partial } => Start::Begin(event.clone(), tx, partial),
                        SpiEvent::Word { tx: UNFRAMED, .. } => Start::Unframed,
                        SpiEvent::Word { .. } | SpiEvent::End { .. } => Start::Skip(event.clone()),
                    },
                    Item::Gap(range) => Start::Gap(*range),
                    Item::Progress(tick) => Start::Progress(*tick),
                    Item::End(tick) => Start::End(*tick),
                })
                .await;
            let (tx, partial) = match start {
                Start::Begin(event, tx, partial) => {
                    self.consume(&event);
                    (tx, partial)
                }
                Start::Unframed => (UNFRAMED, false),
                Start::Skip(event) => {
                    self.consume(&event);
                    continue;
                }
                Start::Gap(range) => {
                    self.inbox.take();
                    return Next::Gap(range);
                }
                Start::Progress(tick) => {
                    self.inbox.take();
                    return Next::Progress(tick);
                }
                Start::End(tick) => {
                    self.inbox.take();
                    return Next::End(tick);
                }
            };
            self.current = Some(Current {
                tx,
                partial,
                ended: false,
            });
            return Next::Transaction(Transaction { bus: self });
        }
    }

    /// Consumes what is left of the current transaction, if the decoder left it open.
    /// Returns how the transaction ended when it did not end normally. Transaction `0` of a
    /// bus without chip select has nothing to drain to and is simply handed back.
    pub async fn finish(&mut self) -> Result<(), ReadError> {
        let result = match &self.current {
            Some(current) if !current.ended && current.tx != UNFRAMED => {
                Transaction { bus: self }.drain().await
            }
            _ => Ok(()),
        };
        self.current = None;
        result
    }

    /// Takes the next word of the current transaction, or reports how it ended instead.
    async fn next_word(&mut self) -> Result<Option<Word>, ReadError> {
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
                        SpiEvent::Word {
                            tx: word_tx,
                            mosi,
                            miso,
                        } if word_tx == tx => Peeked::Word(event.clone(), mosi, miso),
                        SpiEvent::End { tx: end_tx, reason } if end_tx == tx => {
                            Peeked::End(event.clone(), reason)
                        }
                        _ => Peeked::Foreign,
                    },
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(_) => Peeked::Progress,
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let result = match peeked {
                Peeked::Word(event, mosi, miso) => {
                    self.consume(&event);
                    return Ok(Some(Word { mosi, miso }));
                }
                Peeked::End(event, EndReason::CsRelease) => {
                    self.consume(&event);
                    Ok(None)
                }
                Peeked::End(event, EndReason::Aborted) => {
                    self.consume(&event);
                    Err(ReadError::Truncated)
                }
                // Progress inside a transaction says nothing about its words.
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
    fn consume(&mut self, event: &Event<SpiEvent>) {
        self.inbox.take();
        self.record(event);
    }
}

/// The transaction of a bus without chip select: every word's, never begun or ended.
const UNFRAMED: TxId = TxId(0);

/// What the next item of the stream means to a reader waiting for a transaction.
enum Start {
    Begin(Event<SpiEvent>, TxId, bool),
    Unframed,
    Skip(Event<SpiEvent>),
    Gap(TickRange),
    Progress(Tick),
    End(Tick),
}

/// What the next item of the stream means to the transaction being read.
enum Peeked {
    Word(Event<SpiEvent>, Option<u8>, Option<u8>),
    End(Event<SpiEvent>, EndReason),
    Foreign,
    Gap(TickRange),
    Progress,
    EndOfInput(Tick),
}

/// A cursor over the words of one transaction.
#[derive(Debug)]
pub struct Transaction<'a> {
    bus: &'a mut SpiBus,
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

    /// The ticks of what was consumed since this was last called; see [`SpiBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next word; the transaction's end, normal or not, is an error here. A
    /// sequence of reads that pauses between them keeps what it has read; nothing is read
    /// twice.
    pub async fn word(&mut self) -> Result<Word, ReadError> {
        match self.word_or_end().await? {
            Read::Word(word) => Ok(word),
            Read::End => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next word, or finds the transaction's normal end.
    pub async fn word_or_end(&mut self) -> Result<Read, ReadError> {
        Ok(self.bus.next_word().await?.map_or(Read::End, Read::Word))
    }

    /// Consumes the rest of the transaction up to its normal end.
    pub async fn drain(&mut self) -> Result<(), ReadError> {
        loop {
            if self.bus.next_word().await?.is_none() {
                return Ok(());
            }
        }
    }
}
