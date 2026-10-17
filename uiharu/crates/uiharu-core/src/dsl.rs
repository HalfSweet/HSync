// SPDX-License-Identifier: Apache-2.0

//! The runtime of decoders compiled from the DSL: what the backend emits generated code
//! against (`docs/dsl.md` §3 and §5).
//!
//! A compiled decoder is a state struct with the body the author wrote; everything around it
//! is here. The main loops ([`run_logic`], [`run_stream`], [`run_bus`]) drive the decoder's
//! `run` or `transaction`, call its hooks and pass gaps, progress and the end on; the
//! operation wrappers on the decoder traits turn progress into a forward held back to `hold()`
//! and a gap or the end into a [`Cut`] that unwinds the body; [`FramedBus`] is the cursor over
//! any framed contract, generic over how the contract frames its traffic ([`Framed`]);
//! [`Batch`] is the one buffer a decoder keeps; [`Int`] is the integer arithmetic of the DSL.

use std::cell::Cell;
use std::fmt;
use std::future::Future;
use std::rc::Rc;

use crate::node::{Inbox, Outbox, ProgressSink, Provenance};
use crate::schema::EventSchema;
use crate::signal::{Change, ChannelMask, Levels, Sample, SignalCursor};
use crate::stream::{Event, Item};
use crate::time::{Tick, TickRange};

// Integers

/// The integer arithmetic of the DSL: `+`, `-` and `*` wrap at the width, division and
/// remainder by zero yield zero, and a shift by the width or more yields zero.
pub trait Int: Copy + PartialOrd {
    /// Wrapping addition.
    fn wadd(self, other: Self) -> Self;
    /// Wrapping subtraction.
    fn wsub(self, other: Self) -> Self;
    /// Wrapping multiplication.
    fn wmul(self, other: Self) -> Self;
    /// Division; zero when `other` is zero.
    fn div0(self, other: Self) -> Self;
    /// Remainder; zero when `other` is zero.
    fn rem0(self, other: Self) -> Self;
    /// A left shift; zero when `n` reaches the width.
    fn shl(self, n: u64) -> Self;
    /// A right shift; zero when `n` reaches the width.
    fn shr(self, n: u64) -> Self;
    /// `self - other`, or zero when `other` is larger.
    fn ssub(self, other: Self) -> Self;
}

macro_rules! int {
    ($($t:ty),*) => {$(
        impl Int for $t {
            fn wadd(self, other: Self) -> Self {
                self.wrapping_add(other)
            }
            fn wsub(self, other: Self) -> Self {
                self.wrapping_sub(other)
            }
            fn wmul(self, other: Self) -> Self {
                self.wrapping_mul(other)
            }
            fn div0(self, other: Self) -> Self {
                self.checked_div(other).unwrap_or(0)
            }
            fn rem0(self, other: Self) -> Self {
                self.checked_rem(other).unwrap_or(0)
            }
            fn shl(self, n: u64) -> Self {
                u32::try_from(n).ok().and_then(|n| self.checked_shl(n)).unwrap_or(0)
            }
            fn shr(self, n: u64) -> Self {
                u32::try_from(n).ok().and_then(|n| self.checked_shr(n)).unwrap_or(0)
            }
            fn ssub(self, other: Self) -> Self {
                self.saturating_sub(other)
            }
        }
    )*};
}

int!(u8, u16, u32, u64);

/// `⌊a × b / c⌋` without overflowing in between; zero when `c` is zero, the largest value when
/// the result does not fit.
pub fn mul_div(a: u64, b: u64, c: u64) -> u64 {
    if c == 0 {
        return 0;
    }
    u64::try_from(u128::from(a) * u128::from(b) / u128::from(c)).unwrap_or(u64::MAX)
}

/// `⌈a × b / c⌉` without overflowing in between; zero when `c` is zero.
pub fn mul_div_ceil(a: u64, b: u64, c: u64) -> u64 {
    if c == 0 {
        return 0;
    }
    u64::try_from((u128::from(a) * u128::from(b)).div_ceil(u128::from(c))).unwrap_or(u64::MAX)
}

// Cuts

/// What unwinds a decoder's body: a gap or the end of what it reads, and, for a body over a
/// frame cursor, the frame's end where an item was expected, a cut-off frame, and a value
/// the body required that was not there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// Samples, and whatever was on them, are missing.
    Gap(TickRange),
    /// The stream ended.
    End(Tick),
    /// The frame ended normally where an item was expected.
    Incomplete,
    /// The frame was cut off by the lower decoder.
    Truncated,
    /// An item had no value where the body required one.
    Missing,
}

/// The cuts a bus body meets, as the decoder's recovery names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutKind {
    Incomplete,
    Truncated,
    Missing,
    Gap,
    End,
}

/// Why a read of a frame cursor did not return an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The frame ended normally where an item was expected: the frame is whole, what the
    /// decoder was reading is not.
    #[error("the frame ended before the item")]
    Incomplete,
    /// The frame was cut off: the samples ended or a gap began while it was under way.
    #[error("the frame was cut off")]
    Truncated,
    /// Samples are missing from here on; the gap itself follows from [`FramedBus::next`].
    #[error("samples are missing from tick {}", .0.start)]
    Gap(TickRange),
    /// The stream ended; the end itself follows from [`FramedBus::next`].
    #[error("the stream ended at tick {0}")]
    EndOfInput(Tick),
}

impl From<ReadError> for Cut {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::Incomplete => Cut::Incomplete,
            ReadError::Truncated => Cut::Truncated,
            ReadError::Gap(range) => Cut::Gap(range),
            ReadError::EndOfInput(tick) => Cut::End(tick),
        }
    }
}

/// The value of an option a bus body requires; `Cut::Missing` when it is not there.
pub fn required<T>(value: Option<T>) -> Result<T, Cut> {
    value.ok_or(Cut::Missing)
}

/// Why a configuration is rejected: the message the decoder's check returns.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

// What the operations return

/// A tick where some watched channels changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub tick: u64,
    /// The watched channels whose level differs from the tick before.
    pub changed: ChannelMask,
    /// The levels of every channel at `tick`.
    pub levels: Levels,
}

/// An edge before a deadline, or the deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timed {
    Edge {
        tick: u64,
        changed: ChannelMask,
        levels: Levels,
    },
    /// No watched channel changed before the deadline; the cursor stands at `tick`.
    Quiet { tick: u64 },
}

/// An event of a lower stream with where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received<E> {
    pub event: E,
    pub at: Provenance,
}

/// What a batch hands out: its items and the provenance of all of them (`None` when it was
/// empty).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Taken<T> {
    pub items: Vec<T>,
    pub at: Option<Provenance>,
}

// Batches

/// At most `N` items waiting to go out in one event, with the provenance of what it holds.
#[derive(Clone, Debug)]
pub struct Batch<T, const N: usize> {
    items: Vec<T>,
    at: Option<Provenance>,
}

impl<T, const N: usize> Default for Batch<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Batch<T, N> {
    /// An empty batch.
    pub fn new() -> Self {
        Self {
            items: Vec::with_capacity(N),
            at: None,
        }
    }

    /// How many items it holds.
    pub fn len(&self) -> u32 {
        u32::try_from(self.items.len()).unwrap_or(u32::MAX)
    }

    /// How many it holds at most.
    pub fn capacity(&self) -> u32 {
        u32::try_from(N).unwrap_or(u32::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether it holds `N` items: it goes out now.
    pub fn is_full(&self) -> bool {
        self.items.len() >= N
    }

    /// The provenance of the items held, `None` when there are none.
    pub fn at(&self) -> Option<Provenance> {
        self.at
    }

    /// Adds an item, joining its provenance into the batch's. A full batch takes nothing: it
    /// should have gone out (a programming error, caught in debug builds).
    pub fn push(&mut self, item: T, at: Provenance) {
        debug_assert!(!self.is_full(), "a batch goes out when it is full");
        if self.is_full() {
            return;
        }
        self.at = Some(self.at.map_or(at, |have| have.join(at)));
        self.items.push(item);
    }

    /// Takes every item out.
    pub fn take(&mut self) -> Taken<T> {
        Taken {
            items: std::mem::replace(&mut self.items, Vec::with_capacity(N)),
            at: self.at.take(),
        }
    }

    /// Drops every item.
    pub fn clear(&mut self) {
        self.items.clear();
        self.at = None;
    }
}

// Framed contracts

/// How a contract frames its traffic, for the cursor consumers read it through.
pub trait Framed: EventSchema + Clone {
    /// What names a frame on its events.
    type Id: Copy + PartialEq + fmt::Debug;
    /// What the frame's items carry: the item event without its kind and the frame's id.
    type Item: Clone + fmt::Debug;

    /// What the event is to the framing.
    fn frame(&self) -> Frame<Self::Id, Self::Item>;

    /// The id of the one frame of a bus without a framing line, which never begins or ends;
    /// `None` when every frame begins and ends.
    fn unframed() -> Option<Self::Id> {
        None
    }
}

/// What an event of a framed contract is to its framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame<I, T> {
    /// A frame begins; `partial` when its beginning was not seen.
    Begin { id: I, partial: bool },
    /// An item of the frame.
    Item { id: I, item: T },
    /// The frame ends, normally or not.
    End { id: I, normal: bool },
}

/// What [`Transaction::item_or_end`] finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read<T> {
    Item(T),
    /// The frame ended normally.
    End,
}

/// What [`FramedBus::next`] finds.
#[derive(Debug)]
pub enum Next<'a, E: Framed> {
    /// A frame begins; on a bus without a framing line, the unframed frame continues from
    /// its next item.
    Transaction(Transaction<'a, E>),
    Gap(TickRange),
    Progress(Tick),
    End(Tick),
}

#[derive(Debug)]
struct Current<I> {
    id: I,
    partial: bool,
    ended: bool,
}

/// The ticks of what a bus cursor consumed since the provenance was last taken, shared between
/// the cursor and the decoder's handles so that `consumed()` is one thing wherever it is
/// called.
#[derive(Clone, Debug, Default)]
pub struct Consumed(Rc<Cell<ConsumedState>>);

#[derive(Clone, Copy, Debug, Default)]
struct ConsumedState {
    /// The ticks of the items consumed since they were last taken.
    at: Option<Provenance>,
    /// Where the last consumed item was settled.
    last_settled: Tick,
}

impl Consumed {
    fn record(&self, span: TickRange, settled: Tick) {
        let mut state = self.0.get();
        let at = Provenance::over(span, settled);
        state.at = Some(state.at.map_or(at, |have| have.join(at)));
        state.last_settled = settled;
        self.0.set(state);
    }

    /// Where what was consumed and not taken starts.
    fn start(&self) -> Option<Tick> {
        self.0.get().at.map(|at| at.span.start)
    }

    /// The ticks of what was consumed since this was last called; an empty span where the
    /// last item was settled when nothing was.
    pub fn take(&self) -> Provenance {
        let mut state = self.0.get();
        let taken = state
            .at
            .take()
            .unwrap_or(Provenance::at(state.last_settled));
        self.0.set(state);
        taken
    }
}

/// A decoder's reader on a framed stream, transaction by transaction: the generic form of
/// the cursors the hand-written decoders read their buses through. [`FramedBus::next`] waits
/// for a frame to begin and hands out a [`Transaction`], a cursor over its items; each read
/// consumes one item and can wait across blocks, the normal end is only ever returned by
/// [`Transaction::item_or_end`], and running out of items anywhere else is an error naming
/// how the frame ended. Progress met inside a frame is passed on through the output's
/// progress sink, held back to what was consumed since the provenance was last taken and to
/// what the decoder says it still holds.
pub struct FramedBus<E: Framed> {
    inbox: Inbox<E>,
    progress: ProgressSink,
    current: Option<Current<E::Id>>,
    /// The ticks of the items consumed since [`FramedBus::consumed`] last took them.
    consumed: Consumed,
}

impl<E: Framed> fmt::Debug for FramedBus<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramedBus")
            .field("current", &self.current)
            .field("consumed", &self.consumed)
            .finish_non_exhaustive()
    }
}

/// What the next item of the stream means to a reader waiting for a frame.
enum Start<E: Framed> {
    Begin(Event<E>, E::Id, bool),
    Unframed,
    Skip(Event<E>),
    Gap(TickRange),
    Progress(Tick),
    End(Tick),
}

/// What the next item of the stream means to the frame being read.
enum Peeked<E: Framed> {
    Item(Event<E>, E::Item),
    End(Event<E>, bool),
    Foreign,
    Gap(TickRange),
    Progress(Tick),
    EndOfInput(Tick),
}

impl<E: Framed> FramedBus<E> {
    /// A reader on `inbox`, passing progress on through `progress`.
    pub fn new(inbox: Inbox<E>, progress: ProgressSink) -> Self {
        Self::sharing(inbox, progress, Consumed::default())
    }

    /// A reader whose consumed ticks are shared through `consumed`.
    pub fn sharing(inbox: Inbox<E>, progress: ProgressSink, consumed: Consumed) -> Self {
        Self {
            inbox,
            progress,
            current: None,
            consumed,
        }
    }

    fn consume(&mut self, event: &Event<E>) {
        self.inbox.take();
        self.consumed.record(event.span, event.settled);
    }

    async fn forward_progress(&mut self, tick: Tick) {
        let held = [self.consumed.start(), self.progress.held()]
            .into_iter()
            .flatten()
            .fold(tick, Tick::min);
        self.progress.progress(held).await;
    }

    /// The ticks of what was consumed since this was last called; an empty span where the
    /// last item was settled when nothing was.
    pub fn consumed(&mut self) -> Provenance {
        self.consumed.take()
    }

    /// Waits for the next frame, gap, progress or end. Items and ends outside a frame are
    /// skipped: their beginning was consumed by an earlier frame that was not drained, which
    /// [`FramedBus::finish`] prevents. The items of the unframed frame open it at the first
    /// of them, which is left for the frame to read.
    pub async fn next(&mut self) -> Next<'_, E> {
        loop {
            let start = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) => match event.payload.frame() {
                        Frame::Begin { id, partial } => Start::Begin(event.clone(), id, partial),
                        Frame::Item { id, .. } if E::unframed() == Some(id) => Start::Unframed,
                        Frame::Item { .. } | Frame::End { .. } => Start::Skip(event.clone()),
                    },
                    Item::Gap(range) => Start::Gap(*range),
                    Item::Progress(tick) => Start::Progress(*tick),
                    Item::End(tick) => Start::End(*tick),
                })
                .await;
            let (id, partial) = match start {
                Start::Begin(event, id, partial) => {
                    self.consume(&event);
                    (id, partial)
                }
                Start::Unframed => match E::unframed() {
                    Some(id) => (id, false),
                    None => continue,
                },
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
                id,
                partial,
                ended: false,
            });
            return Next::Transaction(Transaction { bus: self });
        }
    }

    /// Consumes what is left of the current frame, if the decoder left it open, and returns
    /// how it ended when it did not end normally. The unframed frame has nothing to drain to
    /// and is simply handed back.
    pub async fn finish(&mut self) -> Result<(), ReadError> {
        let result = match &self.current {
            Some(current) if !current.ended && E::unframed() != Some(current.id) => {
                Transaction { bus: self }.drain().await
            }
            _ => Ok(()),
        };
        self.current = None;
        result
    }

    /// Takes the next item of the current frame, or reports how it ended instead.
    async fn next_item(&mut self) -> Result<Option<E::Item>, ReadError> {
        let Some(current) = self.current.as_ref() else {
            return Err(ReadError::Incomplete);
        };
        if current.ended {
            return Err(ReadError::Incomplete);
        }
        let id = current.id;
        loop {
            let peeked = self
                .inbox
                .peek_with(|item| match item {
                    Item::Event(event) => match event.payload.frame() {
                        Frame::Item { id: at, item } if at == id => {
                            Peeked::Item(event.clone(), item)
                        }
                        Frame::End { id: at, normal } if at == id => {
                            Peeked::End(event.clone(), normal)
                        }
                        _ => Peeked::Foreign,
                    },
                    Item::Gap(range) => Peeked::Gap(*range),
                    Item::Progress(tick) => Peeked::Progress(*tick),
                    Item::End(tick) => Peeked::EndOfInput(*tick),
                })
                .await;
            let result = match peeked {
                Peeked::Item(event, item) => {
                    self.consume(&event);
                    return Ok(Some(item));
                }
                Peeked::End(event, true) => {
                    self.consume(&event);
                    Ok(None)
                }
                Peeked::End(event, false) => {
                    self.consume(&event);
                    Err(ReadError::Truncated)
                }
                // Progress inside a frame says nothing about its items.
                Peeked::Progress(tick) => {
                    self.inbox.take();
                    self.forward_progress(tick).await;
                    continue;
                }
                // Another frame's event: this one never ended. It stays for `next`.
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

/// A cursor over the items of one frame.
#[derive(Debug)]
pub struct Transaction<'a, E: Framed> {
    bus: &'a mut FramedBus<E>,
}

impl<E: Framed> Transaction<'_, E> {
    /// The frame's id.
    pub fn id(&self) -> E::Id {
        self.bus.current.as_ref().map_or_else(
            || E::unframed().expect("a transaction has an id"),
            |current| current.id,
        )
    }

    /// Whether the frame's beginning was not seen.
    pub fn is_partial(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_some_and(|current| current.partial)
    }

    /// Whether the frame has ended.
    pub fn is_ended(&self) -> bool {
        self.bus
            .current
            .as_ref()
            .is_none_or(|current| current.ended)
    }

    /// The ticks of what was consumed since this was last called; see [`FramedBus::consumed`].
    pub fn consumed(&mut self) -> Provenance {
        self.bus.consumed()
    }

    /// Reads the next item; the frame's end, normal or not, is an error here.
    pub async fn item(&mut self) -> Result<E::Item, ReadError> {
        match self.item_or_end().await? {
            Read::Item(item) => Ok(item),
            Read::End => Err(ReadError::Incomplete),
        }
    }

    /// Reads the next item, or finds the frame's normal end.
    pub async fn item_or_end(&mut self) -> Result<Read<E::Item>, ReadError> {
        Ok(self.bus.next_item().await?.map_or(Read::End, Read::Item))
    }

    /// Consumes the rest of the frame up to its normal end.
    pub async fn drain(&mut self) -> Result<(), ReadError> {
        loop {
            if self.bus.next_item().await?.is_none() {
                return Ok(());
            }
        }
    }
}

// The handles

/// What a logic decoder's body reads and writes through.
#[derive(Debug)]
pub struct LogicIo<E> {
    pub signal: SignalCursor,
    pub out: Outbox<E>,
}

/// What a stream decoder's body reads and writes through.
#[derive(Debug)]
pub struct StreamIo<L, E> {
    pub input: Inbox<L>,
    pub out: Outbox<E>,
}

/// What a bus decoder's body writes through; it reads through the transaction cursor it is
/// given.
#[derive(Debug)]
pub struct BusIo<E> {
    pub out: Outbox<E>,
    /// The output's progress sink, told what the decoder holds before every read of the
    /// cursor.
    pub sink: ProgressSink,
    /// What the cursor consumed since the provenance was last taken.
    pub consumed: Consumed,
}

impl<E> BusIo<E> {
    /// The ticks of what was read through the cursor since this was last called (or an event
    /// was emitted without provenance).
    pub fn consumed(&self) -> Provenance {
        self.consumed.take()
    }
}

/// Progress to `tick`, held back to what the decoder holds.
fn held(hold: Option<u64>, tick: Tick) -> Tick {
    hold.map_or(tick, |from| Tick(from).min(tick))
}

/// What to do with a frame whose beginning was not seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partial {
    /// Drain it without running the body.
    Drain,
    /// Run the body on it.
    Read,
}

// The decoders

/// A compiled decoder of logic samples: the hooks the main loop calls and the operations the
/// body awaits.
pub trait LogicDecoder: Sized {
    type Event: 'static;

    /// From which tick on the decoder holds what it has not reported yet.
    fn hold(&self) -> Option<u64>;

    /// The body: entered at the first sample and again after every gap; returning means
    /// nothing more is decoded.
    fn run(&mut self, io: &mut LogicIo<Self::Event>) -> impl Future<Output = Result<(), Cut>>;

    fn on_gap(
        &mut self,
        io: &mut LogicIo<Self::Event>,
        range: TickRange,
    ) -> impl Future<Output = ()>;

    fn on_end(&mut self, io: &mut LogicIo<Self::Event>, tick: u64) -> impl Future<Output = ()>;

    fn on_progress(&mut self, io: &mut LogicIo<Self::Event>, tick: u64)
    -> impl Future<Output = ()>;

    /// Passes progress on, after `on_progress`, held back to `hold()`.
    fn forward(&mut self, io: &mut LogicIo<Self::Event>, tick: Tick) -> impl Future<Output = ()> {
        async move {
            self.on_progress(io, tick.0).await;
            let held = held(self.hold(), tick);
            io.out.progress(held).await;
        }
    }

    /// The next edge on `mask`.
    fn next_change(
        &mut self,
        io: &mut LogicIo<Self::Event>,
        mask: ChannelMask,
    ) -> impl Future<Output = Result<Edge, Cut>> {
        async move {
            loop {
                match io.signal.next_change(mask).await {
                    Change::At {
                        tick,
                        changed,
                        levels,
                    } => {
                        return Ok(Edge {
                            tick: tick.0,
                            changed,
                            levels,
                        });
                    }
                    Change::Quiet(_) => continue,
                    Change::Progress(tick) => self.forward(io, tick).await,
                    Change::Gap(range) => return Err(Cut::Gap(range)),
                    Change::End(tick) => return Err(Cut::End(tick)),
                }
            }
        }
    }

    /// The next edge on `mask` before `until`, or the deadline.
    fn next_change_before(
        &mut self,
        io: &mut LogicIo<Self::Event>,
        mask: ChannelMask,
        until: u64,
    ) -> impl Future<Output = Result<Timed, Cut>> {
        async move {
            loop {
                match io.signal.next_change_before(mask, Tick(until)).await {
                    Change::At {
                        tick,
                        changed,
                        levels,
                    } => {
                        return Ok(Timed::Edge {
                            tick: tick.0,
                            changed,
                            levels,
                        });
                    }
                    Change::Quiet(tick) => return Ok(Timed::Quiet { tick: tick.0 }),
                    Change::Progress(tick) => self.forward(io, tick).await,
                    Change::Gap(range) => return Err(Cut::Gap(range)),
                    Change::End(tick) => return Err(Cut::End(tick)),
                }
            }
        }
    }

    /// The levels at the cursor.
    fn sample(
        &mut self,
        io: &mut LogicIo<Self::Event>,
    ) -> impl Future<Output = Result<Levels, Cut>> {
        async move {
            loop {
                match io.signal.sample().await {
                    Sample::At { levels, .. } => return Ok(levels),
                    Sample::Progress(tick) => self.forward(io, tick).await,
                    Sample::Gap(range) => return Err(Cut::Gap(range)),
                    Sample::End(tick) => return Err(Cut::End(tick)),
                }
            }
        }
    }
}

/// Runs a logic decoder to the end of its samples.
pub async fn run_logic<D: LogicDecoder>(
    mut decoder: D,
    signal: SignalCursor,
    out: Outbox<D::Event>,
) {
    let mut io = LogicIo { signal, out };
    loop {
        match decoder.run(&mut io).await {
            Ok(()) => return pass_through_logic(&mut io).await,
            Err(Cut::Gap(range)) => {
                decoder.on_gap(&mut io, range).await;
                let held = held(decoder.hold(), range.start);
                io.out.progress(held).await;
                io.out.gap(range).await;
            }
            Err(Cut::End(tick)) => {
                decoder.on_end(&mut io, tick.0).await;
                let held = held(decoder.hold(), tick);
                io.out.progress(held).await;
                io.out.end(tick).await;
                return;
            }
            // A logic body has no frame cursor, so no other cut reaches here.
            Err(Cut::Incomplete | Cut::Truncated | Cut::Missing) => continue,
        }
    }
}

/// Lets the samples pass without reading them, once the body returned.
async fn pass_through_logic<E>(io: &mut LogicIo<E>) {
    loop {
        match io.signal.next_change(ChannelMask::EMPTY).await {
            Change::At { .. } | Change::Quiet(_) => {}
            Change::Progress(tick) => io.out.progress(tick).await,
            Change::Gap(range) => {
                io.out.progress(range.start).await;
                io.out.gap(range).await;
            }
            Change::End(tick) => {
                io.out.progress(tick).await;
                io.out.end(tick).await;
                return;
            }
        }
    }
}

/// A compiled decoder of an unframed lower stream.
pub trait StreamDecoder: Sized {
    type Lower: Clone + 'static;
    type Event: 'static;

    fn hold(&self) -> Option<u64>;

    fn run(
        &mut self,
        io: &mut StreamIo<Self::Lower, Self::Event>,
    ) -> impl Future<Output = Result<(), Cut>>;

    fn on_gap(
        &mut self,
        io: &mut StreamIo<Self::Lower, Self::Event>,
        range: TickRange,
    ) -> impl Future<Output = ()>;

    fn on_end(
        &mut self,
        io: &mut StreamIo<Self::Lower, Self::Event>,
        tick: u64,
    ) -> impl Future<Output = ()>;

    fn on_progress(
        &mut self,
        io: &mut StreamIo<Self::Lower, Self::Event>,
        tick: u64,
    ) -> impl Future<Output = ()>;

    /// The next event of the lower stream.
    fn next(
        &mut self,
        io: &mut StreamIo<Self::Lower, Self::Event>,
    ) -> impl Future<Output = Result<Received<Self::Lower>, Cut>> {
        async move {
            loop {
                match io.input.recv().await {
                    Item::Event(event) => {
                        return Ok(Received {
                            event: event.payload,
                            at: Provenance::over(event.span, event.settled),
                        });
                    }
                    Item::Progress(tick) => {
                        self.on_progress(io, tick.0).await;
                        let held = held(self.hold(), tick);
                        io.out.progress(held).await;
                    }
                    Item::Gap(range) => return Err(Cut::Gap(range)),
                    Item::End(tick) => return Err(Cut::End(tick)),
                }
            }
        }
    }
}

/// Runs a stream decoder to the end of its input.
pub async fn run_stream<D: StreamDecoder>(
    mut decoder: D,
    input: Inbox<D::Lower>,
    out: Outbox<D::Event>,
) {
    let mut io = StreamIo { input, out };
    loop {
        match decoder.run(&mut io).await {
            Ok(()) => loop {
                match io.input.recv().await {
                    Item::Event(_) => {}
                    Item::Progress(tick) => io.out.progress(tick).await,
                    Item::Gap(range) => {
                        io.out.progress(range.start).await;
                        io.out.gap(range).await;
                    }
                    Item::End(tick) => {
                        io.out.progress(tick).await;
                        io.out.end(tick).await;
                        return;
                    }
                }
            },
            Err(Cut::Gap(range)) => {
                decoder.on_gap(&mut io, range).await;
                let held = held(decoder.hold(), range.start);
                io.out.progress(held).await;
                io.out.gap(range).await;
            }
            Err(Cut::End(tick)) => {
                decoder.on_end(&mut io, tick.0).await;
                let held = held(decoder.hold(), tick);
                io.out.progress(held).await;
                io.out.end(tick).await;
                return;
            }
            Err(Cut::Incomplete | Cut::Truncated | Cut::Missing) => continue,
        }
    }
}

/// A compiled decoder of a framed lower stream: a body per frame, and the recovery that closes
/// a cut-off body.
pub trait BusDecoder: Sized {
    type Lower: Framed;
    type Event: 'static;

    /// What to do with a frame whose beginning was not seen.
    const PARTIAL: Partial;

    fn hold(&self) -> Option<u64>;

    /// The body of one frame.
    fn transaction(
        &mut self,
        io: &mut BusIo<Self::Event>,
        tx: &mut Transaction<'_, Self::Lower>,
    ) -> impl Future<Output = Result<(), Cut>>;

    /// Closes what a cut-off body left open, placed over what was read but not reported.
    fn recover(
        &mut self,
        io: &mut BusIo<Self::Event>,
        cut: CutKind,
        at: Provenance,
    ) -> impl Future<Output = ()>;

    fn on_gap(&mut self, io: &mut BusIo<Self::Event>, range: TickRange)
    -> impl Future<Output = ()>;

    fn on_end(&mut self, io: &mut BusIo<Self::Event>, tick: u64) -> impl Future<Output = ()>;

    fn on_progress(&mut self, io: &mut BusIo<Self::Event>, tick: u64) -> impl Future<Output = ()>;
}

/// Runs a bus decoder to the end of its input.
pub async fn run_bus<D: BusDecoder>(mut decoder: D, input: Inbox<D::Lower>, out: Outbox<D::Event>) {
    let sink = out.progress_sink();
    let consumed = Consumed::default();
    let mut bus = FramedBus::sharing(input, sink.clone(), consumed.clone());
    let mut io = BusIo {
        out,
        sink,
        consumed,
    };
    loop {
        match bus.next().await {
            Next::Transaction(mut tx) => {
                io.sink.hold(decoder.hold().map(Tick));
                let result = if tx.is_partial() && D::PARTIAL == Partial::Drain {
                    tx.drain().await.map_err(Cut::from)
                } else {
                    decoder.transaction(&mut io, &mut tx).await
                };
                if let Err(cut) = result {
                    recover(&mut decoder, &mut io, &mut bus, cut).await;
                }
                if let Err(error) = bus.finish().await {
                    recover(&mut decoder, &mut io, &mut bus, Cut::from(error)).await;
                }
                bus.consumed();
            }
            Next::Gap(range) => {
                decoder.on_gap(&mut io, range).await;
                io.out.gap(range).await;
            }
            Next::Progress(tick) => {
                decoder.on_progress(&mut io, tick.0).await;
                let held = held(decoder.hold(), tick);
                io.out.progress(held).await;
            }
            Next::End(tick) => {
                decoder.on_end(&mut io, tick.0).await;
                io.out.end(tick).await;
                return;
            }
        }
    }
}

/// Closes what a cut-off body left open: the cut's reason, placed over the items read but
/// not reported, or at the cut's tick when there are none.
async fn recover<D: BusDecoder>(
    decoder: &mut D,
    io: &mut BusIo<D::Event>,
    bus: &mut FramedBus<D::Lower>,
    cut: Cut,
) {
    let unreported = bus.consumed();
    let placed = |tick: Tick| Provenance {
        span: if unreported.span.is_empty() {
            TickRange::at(tick)
        } else {
            unreported.span
        },
        settled: tick,
    };
    let (kind, at) = match cut {
        Cut::Incomplete => (CutKind::Incomplete, unreported),
        Cut::Truncated => (CutKind::Truncated, unreported),
        Cut::Missing => (CutKind::Missing, unreported),
        Cut::Gap(range) => (CutKind::Gap, placed(range.start)),
        Cut::End(tick) => (CutKind::End, placed(tick)),
    };
    decoder.recover(io, kind, at).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_wrap_and_divide_by_zero_to_zero() {
        assert_eq!(250u8.wadd(10), 4);
        assert_eq!(3u16.wsub(5), u16::MAX - 1);
        assert_eq!(0x8000_0000u32.wmul(2), 0);
        assert_eq!(7u64.div0(0), 0);
        assert_eq!(7u64.rem0(0), 0);
        assert_eq!(7u64.div0(2), 3);
        assert_eq!(0x80u8.shl(1), 0);
        assert_eq!(1u8.shl(8), 0);
        assert_eq!(1u16.shl(8), 0x100);
        assert_eq!(0x100u16.shr(8), 1);
        assert_eq!(1u64.shr(64), 0);
        assert_eq!(3u8.ssub(5), 0);
        assert_eq!(mul_div(u64::MAX, 2, 4), u64::MAX / 2);
        assert_eq!(mul_div(1_000_000, 100, 200 * 115_200), 4);
        assert_eq!(mul_div_ceil(1_000_000, 77, 2 * 9_600), 4011);
        assert_eq!(mul_div(1, 1, 0), 0);
    }

    #[test]
    fn batches_join_provenance_and_hand_everything_out() {
        let mut batch: Batch<u8, 2> = Batch::new();
        assert!(batch.is_empty());
        assert_eq!(batch.capacity(), 2);
        batch.push(
            1,
            Provenance::over(TickRange::new(Tick(10), Tick(20)), Tick(19)),
        );
        batch.push(
            2,
            Provenance::over(TickRange::new(Tick(20), Tick(30)), Tick(29)),
        );
        assert!(batch.is_full());
        assert_eq!(
            batch.at().map(|at| at.span),
            Some(TickRange::new(Tick(10), Tick(30)))
        );
        let taken = batch.take();
        assert_eq!(taken.items, vec![1, 2]);
        assert_eq!(taken.at.map(|at| at.settled), Some(Tick(29)));
        assert!(batch.is_empty());
        assert_eq!(batch.take().at, None);
    }

    #[test]
    fn cuts_come_from_read_errors() {
        assert_eq!(Cut::from(ReadError::Incomplete), Cut::Incomplete);
        assert_eq!(Cut::from(ReadError::Truncated), Cut::Truncated);
        assert_eq!(
            Cut::from(ReadError::Gap(TickRange::new(Tick(1), Tick(2)))),
            Cut::Gap(TickRange::new(Tick(1), Tick(2)))
        );
        assert_eq!(Cut::from(ReadError::EndOfInput(Tick(5))), Cut::End(Tick(5)));
        assert_eq!(required(Some(3u8)), Ok(3));
        assert_eq!(required::<u8>(None), Err(Cut::Missing));
    }
}
