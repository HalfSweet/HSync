// SPDX-License-Identifier: Apache-2.0

//! Reading the 1-Wire streams, for decoders built on them.
//!
//! [`LinkBus`] reads the link layer: [`LinkBus::next`] waits for a reset, and [`LinkBus::bit`]
//! reads the bits after it until something ends them — the next reset, a bad slot, a gap or
//! the end of the samples — which it reports as an error naming the cause.
//!
//! [`OneWireBus`] reads the network layer transaction by transaction: [`OneWireBus::next`]
//! waits for a transaction to begin and hands out a [`Transaction`], a cursor over its items —
//! the ROM command, the ROM code or search result, the transport bytes and the bits the
//! transaction ended with. Each read consumes one item and can wait across blocks. The normal
//! end of a transaction, the next reset, is only ever returned by the reads that may find it
//! ([`Transaction::item_or_end`], [`Transaction::byte_or_end`]); everywhere else running out
//! of items is an error that tells the decoder how the transaction ended.
//!
//! The cursors offer the contracts' primitives and nothing more: what a device makes of the
//! bytes — which command, which fields, byte order — is that device's own code.

use uiharu_core::{Event, Inbox, Item, ProgressSink, Provenance, Tick, TickRange};

use super::OneWireEventExt;
use super::events::{EndReason, LinkEvent, OneWireEvent, TxId};

/// What ended a run of bits read through [`LinkBus::bit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// A reset, which begins the next transaction; it stays in the stream for
    /// [`LinkBus::next`].
    #[error("a reset ended the bits at tick {}", .0.span.start)]
    Reset(Provenance),
    /// A bad slot, consumed: the bits after it can no longer be told apart.
    #[error("a bad slot of {low_ticks} ticks ended the bits")]
    BadSlot {
        /// How long the line was low, in ticks.
        low_ticks: u64,
    },
    /// Samples are missing from here on; the gap itself follows from [`LinkBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`LinkBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
}

/// What [`LinkBus::next`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkNext {
    /// A reset.
    Reset {
        /// Whether a slave answered it.
        presence: bool,
    },
    /// Samples, and whatever was on the line, are missing.
    Gap(TickRange),
    /// Nothing more comes before this tick.
    Progress(Tick),
    /// The stream ended.
    End(Tick),
}

/// A decoder's reader on a 1-Wire link-layer stream.
#[derive(Debug)]
pub struct LinkBus {
    inbox: Inbox<LinkEvent>,
    progress: ProgressSink,
    /// The ticks of the items consumed since [`LinkBus::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

/// What the next item of a link stream means to a run of bits.
enum LinkPeeked {
    Bit(Provenance, bool),
    BadSlot(Provenance, u64),
    Reset(Provenance),
    Gap(TickRange),
    Progress(Tick),
    End(Tick),
}

fn provenance<T>(event: &Event<T>) -> Provenance {
    Provenance::over(event.span, event.settled)
}

impl LinkBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<LinkEvent>, progress: ProgressSink) -> Self {
        Self {
            inbox,
            progress,
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

    /// Waits for the next reset, gap, progress or end. Bits and bad slots before a reset
    /// belong to no transaction and are skipped.
    pub async fn next(&mut self) -> LinkNext {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    if let LinkEvent::Reset { presence } = event.payload {
                        self.record(provenance(&event));
                        return LinkNext::Reset { presence };
                    }
                }
                Item::Gap(range) => return LinkNext::Gap(range),
                Item::Progress(tick) => return LinkNext::Progress(tick),
                Item::End(tick) => return LinkNext::End(tick),
            }
        }
    }

    /// Reads the next bit. A reset, a gap or the end of the stream ends the bits and stays for
    /// [`LinkBus::next`]; a bad slot ends them and is consumed. Pausing between reads keeps
    /// what was read; nothing is read twice.
    pub async fn bit(&mut self) -> Result<bool, LinkError> {
        loop {
            let peeked = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) => match event.payload {
                        LinkEvent::Bit { value } => LinkPeeked::Bit(provenance(event), value),
                        LinkEvent::BadSlot { low_ticks } => {
                            LinkPeeked::BadSlot(provenance(event), low_ticks)
                        }
                        LinkEvent::Reset { .. } => LinkPeeked::Reset(provenance(event)),
                    },
                    Item::Gap(range) => LinkPeeked::Gap(*range),
                    Item::Progress(tick) => LinkPeeked::Progress(*tick),
                    Item::End(tick) => LinkPeeked::End(*tick),
                })
                .await;
            match peeked {
                LinkPeeked::Bit(at, value) => {
                    self.inbox.take();
                    self.record(at);
                    return Ok(value);
                }
                LinkPeeked::BadSlot(at, low_ticks) => {
                    self.inbox.take();
                    self.record(at);
                    return Err(LinkError::BadSlot { low_ticks });
                }
                // Progress between bits says nothing about them.
                LinkPeeked::Progress(tick) => {
                    self.inbox.take();
                    self.forward_progress(tick).await;
                }
                LinkPeeked::Reset(at) => return Err(LinkError::Reset(at)),
                LinkPeeked::Gap(range) => return Err(LinkError::Gap(range)),
                LinkPeeked::End(tick) => return Err(LinkError::EndOfInput(tick)),
            }
        }
    }
}

/// Why a read of a transaction did not return what it asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The transaction ended normally, at a reset, where an item was expected.
    #[error("the transaction ended before the item")]
    Incomplete,
    /// A bad slot ended the transaction.
    #[error("a bad slot ended the transaction")]
    BadSlot,
    /// The transaction was cut off: the samples ended or a gap began while it was under way.
    #[error("the transaction was cut off")]
    Truncated,
    /// Samples are missing from here on; the gap itself follows from [`OneWireBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`OneWireBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
    /// The next item is not of the kind the read asked for; it stays in the stream.
    #[error("the next item is not of the kind asked for")]
    Unexpected,
}

/// A ROM code as Read ROM read it or Match ROM sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rom {
    /// The ROM code, its first bit on the line as bit 0.
    pub id: u64,
    /// The family code, the first byte.
    pub family: u8,
    /// Whether the last byte is the CRC-8 of the first seven.
    pub crc_ok: bool,
}

/// What a Search ROM or Alarm Search found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Search {
    /// The direction bits the master wrote, the first as bit 0.
    pub id: u64,
    /// Whether the last byte of `id` is the CRC-8 of the first seven.
    pub crc_ok: bool,
    /// Triplets where devices disagreed.
    pub conflicts: u8,
    /// Triplets where no device answered.
    pub unanswered: u8,
}

/// One item of a transaction, as the network layer reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TxItem {
    /// The ROM command.
    RomCommand(u8),
    /// The ROM code of Read ROM or Match ROM.
    Rom(Rom),
    /// The result of a search.
    Search(Search),
    /// A transport byte.
    Byte(u8),
    /// The bits the transaction ended with, fewer than eight, the first as bit 0.
    Bits {
        /// The bits.
        value: u8,
        /// How many.
        count: u8,
    },
}

impl TxItem {
    fn of(event: &OneWireEvent) -> Option<TxItem> {
        Some(match *event {
            OneWireEvent::RomCommand { code, .. } => TxItem::RomCommand(code),
            OneWireEvent::Rom {
                id, family, crc_ok, ..
            } => TxItem::Rom(Rom { id, family, crc_ok }),
            OneWireEvent::Search {
                id,
                crc_ok,
                conflicts,
                unanswered,
                ..
            } => TxItem::Search(Search {
                id,
                crc_ok,
                conflicts,
                unanswered,
            }),
            OneWireEvent::Byte { value, .. } => TxItem::Byte(value),
            OneWireEvent::Bits { value, count, .. } => TxItem::Bits { value, count },
            OneWireEvent::Begin { .. } | OneWireEvent::End { .. } => return None,
        })
    }
}

/// What [`Transaction::byte_or_end`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The next transport byte.
    Byte(u8),
    /// The bits the transaction ended with, the first as bit 0; its end follows.
    Bits {
        /// The bits.
        value: u8,
        /// How many, 1 to 7.
        count: u8,
    },
    /// The transaction ended normally, at a reset.
    End,
}

/// What [`OneWireBus::next`] finds.
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
    presence: bool,
    /// Whether the transaction's end was reached (or the stream failed under it).
    ended: bool,
}

/// A decoder's reader on a 1-Wire network-layer stream.
#[derive(Debug)]
pub struct OneWireBus {
    inbox: Inbox<OneWireEvent>,
    progress: ProgressSink,
    current: Option<Current>,
    /// The ticks of the items consumed since [`OneWireBus::consumed`] last took them.
    consumed: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

/// What the next item of the stream means to the transaction being read.
enum Peeked {
    Item(Provenance, TxItem),
    End(Provenance, EndReason),
    Foreign,
    Gap(TickRange),
    Progress(Tick),
    EndOfInput(Tick),
}

impl OneWireBus {
    /// A reader on `inbox`.
    pub fn new(inbox: Inbox<OneWireEvent>, progress: ProgressSink) -> Self {
        Self {
            inbox,
            progress,
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

    /// Waits for the next transaction, gap, progress or end. Items outside a transaction are
    /// skipped: their beginning was consumed by an earlier transaction that was not drained,
    /// which [`OneWireBus::finish`] prevents.
    pub async fn next(&mut self) -> Next<'_> {
        loop {
            match self.inbox.recv().await {
                Item::Event(event) => {
                    self.record(provenance(&event));
                    if let OneWireEvent::Begin { tx, presence } = event.payload {
                        self.current = Some(Current {
                            tx,
                            presence,
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

    /// Takes the next item of the current transaction if `accept` takes it, or finds the
    /// transaction's normal end (`None`), or reports how it ended instead.
    async fn next_item(
        &mut self,
        accept: fn(&TxItem) -> bool,
    ) -> Result<Option<TxItem>, ReadError> {
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
                        match (TxItem::of(&event.payload), event.payload) {
                            (Some(found), _) => Peeked::Item(provenance(event), found),
                            (None, OneWireEvent::End { reason, .. }) => {
                                Peeked::End(provenance(event), reason)
                            }
                            (None, _) => Peeked::Foreign,
                        }
                    }
                    Item::Event(_) => Peeked::Foreign,
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(tick) => Peeked::Progress(*tick),
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let result = match peeked {
                Peeked::Item(at, found) => {
                    if !accept(&found) {
                        return Err(ReadError::Unexpected);
                    }
                    self.inbox.take();
                    self.record(at);
                    return Ok(Some(found));
                }
                Peeked::End(at, reason) => {
                    self.inbox.take();
                    self.record(at);
                    match reason {
                        EndReason::Reset => Ok(None),
                        EndReason::BadSlot => Err(ReadError::BadSlot),
                        EndReason::Aborted => Err(ReadError::Truncated),
                    }
                }
                // Progress inside a transaction says nothing about its items.
                Peeked::Progress(tick) => {
                    self.inbox.take();
                    self.forward_progress(tick).await;
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
}

/// A cursor over the items of one transaction.
#[derive(Debug)]
pub struct Transaction<'a> {
    bus: &'a mut OneWireBus,
}

impl Transaction<'_> {
    /// The transaction's id.
    pub fn id(&self) -> TxId {
        self.bus.current.as_ref().map_or(0, |current| current.tx)
    }

    /// Whether a slave answered the reset that began the transaction.
    pub fn presence(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_some_and(|current| current.presence)
    }

    /// Whether the transaction has ended.
    pub fn is_ended(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_none_or(|current| current.ended)
    }

    /// The ticks of what was consumed since this was last called; see
    /// [`OneWireBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next item, or finds the transaction's normal end (`None`).
    pub async fn item_or_end(&mut self) -> Result<Option<TxItem>, ReadError> {
        self.bus.next_item(|_| true).await
    }

    /// Reads the ROM command, the transaction's first item.
    pub async fn rom_command(&mut self) -> Result<u8, ReadError> {
        match self
            .bus
            .next_item(|item| matches!(item, TxItem::RomCommand(_)))
            .await?
        {
            Some(TxItem::RomCommand(code)) => Ok(code),
            Some(_) => Err(ReadError::Unexpected),
            None => Err(ReadError::Incomplete),
        }
    }

    /// Reads the ROM code that follows Read ROM, Match ROM and Overdrive Match ROM.
    pub async fn rom(&mut self) -> Result<Rom, ReadError> {
        match self
            .bus
            .next_item(|item| matches!(item, TxItem::Rom(_)))
            .await?
        {
            Some(TxItem::Rom(rom)) => Ok(rom),
            Some(_) => Err(ReadError::Unexpected),
            None => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next transport byte; the transaction's end, normal or not, is an error here,
    /// and so is a partial byte, which only comes right before the end.
    pub async fn byte(&mut self) -> Result<u8, ReadError> {
        match self.byte_or_end().await? {
            Read::Byte(value) => Ok(value),
            Read::Bits { .. } => match self.bus.next_item(|_| false).await? {
                Some(_) => Err(ReadError::Unexpected),
                None => Err(ReadError::Incomplete),
            },
            Read::End => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next transport byte, the bits the transaction ended with, or its normal end.
    pub async fn byte_or_end(&mut self) -> Result<Read, ReadError> {
        match self
            .bus
            .next_item(|item| matches!(item, TxItem::Byte(_) | TxItem::Bits { .. }))
            .await?
        {
            Some(TxItem::Byte(value)) => Ok(Read::Byte(value)),
            Some(TxItem::Bits { value, count }) => Ok(Read::Bits { value, count }),
            Some(_) => Err(ReadError::Unexpected),
            None => Ok(Read::End),
        }
    }

    /// Consumes the rest of the transaction up to its normal end.
    pub async fn drain(&mut self) -> Result<(), ReadError> {
        while self.bus.next_item(|_| true).await?.is_some() {}
        Ok(())
    }
}
