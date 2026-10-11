// SPDX-License-Identifier: Apache-2.0

//! Decoders as the executor runs them: a node declares its ports, is started with its ports
//! bound to queues, and runs as one future that reads through an [`Inbox`] or a
//! [`SignalCursor`] and writes through an [`Outbox`].
//!
//! Every read and write is an `await` point. When input is missing, output is full or the
//! execution budget is spent, the operation returns `Pending`, the future keeps its state, and
//! the executor polls it again once the condition has changed. Nothing is consumed by an
//! operation that did not complete, so a decoder paused on output never reads its input again
//! or sends an event twice.

use std::any::Any;
use std::cell::{Cell, Ref, RefCell};
use std::fmt;
use std::future::poll_fn;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;

use crate::schema::{InputSpec, OutputSpec, Props};
use crate::signal::{LogicBlock, SignalCursor};
use crate::stream::{Broadcast, Event, Item, QueueControl, ReaderId};
use crate::time::{Tick, TickRange, TimeBase};

/// What a node is waiting for when its future returns `Pending`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wait {
    /// Nothing: the node has not reported a reason.
    #[default]
    Nothing,
    /// An input queue is empty.
    Input,
    /// An output queue is full.
    Output,
    /// The budget of this poll is spent; the node can go on at once.
    Budget,
}

/// The executor's side of a node: its budget, why it last paused and how much it has done.
#[derive(Debug, Default)]
pub struct NodeCx {
    budget: Cell<u32>,
    waiting: Cell<Wait>,
    ops: Cell<u64>,
}

impl NodeCx {
    /// A context with no budget yet.
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Gives the node `budget` operations for its next poll and clears the wait reason.
    pub fn refill(&self, budget: u32) {
        self.budget.set(budget);
        self.waiting.set(Wait::Nothing);
    }

    /// Whether an operation may complete now.
    pub fn has_budget(&self) -> bool {
        self.budget.get() > 0
    }

    /// Records a completed operation.
    pub fn spend(&self) {
        self.budget.set(self.budget.get().saturating_sub(1));
        self.ops.set(self.ops.get() + 1);
    }

    /// Records why the node is about to return `Pending`.
    pub fn wait(&self, on: Wait) {
        self.waiting.set(on);
    }

    /// Why the node last returned `Pending`.
    pub fn waiting(&self) -> Wait {
        self.waiting.get()
    }

    /// Operations completed over the node's lifetime.
    pub fn ops(&self) -> u64 {
        self.ops.get()
    }
}

/// Where an event comes from: the ticks it covers and when it became certain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The ticks the event covers.
    pub span: TickRange,
    /// The tick at which the result became certain.
    pub settled: Tick,
}

impl Provenance {
    /// A result decided at `tick` that covers no ticks.
    pub const fn at(tick: Tick) -> Self {
        Self {
            span: TickRange::at(tick),
            settled: tick,
        }
    }

    /// A result covering `span`, certain at `settled`.
    pub const fn over(span: TickRange, settled: Tick) -> Self {
        Self { span, settled }
    }

    /// A result covering the single tick `tick`.
    pub const fn one(tick: Tick) -> Self {
        Self {
            span: TickRange::one(tick),
            settled: tick,
        }
    }

    /// The provenance of a result built from both: the union of the spans, certain when the
    /// later of the two was.
    pub fn join(self, other: Provenance) -> Provenance {
        Provenance {
            span: self.span.join(other.span),
            settled: self.settled.max(other.settled),
        }
    }
}

/// A queue shared between the graph, the executor and the nodes reading and writing it.
pub(crate) type SharedQueue<T> = Rc<RefCell<Broadcast<T>>>;

/// A node's reader on one of its input queues.
pub struct Inbox<T> {
    queue: SharedQueue<T>,
    reader: ReaderId,
    cx: Rc<NodeCx>,
}

impl<T> fmt::Debug for Inbox<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inbox")
            .field("reader", &self.reader)
            .finish_non_exhaustive()
    }
}

impl<T> Inbox<T> {
    pub(crate) fn new(queue: SharedQueue<T>, cx: Rc<NodeCx>) -> Self {
        let reader = queue.borrow_mut().add_reader();
        Self { queue, reader, cx }
    }

    /// The node context the inbox charges.
    pub fn cx(&self) -> &Rc<NodeCx> {
        &self.cx
    }

    /// The next item, if one is waiting; otherwise `Pending`, with the node set to wait for
    /// input. Does not move past the item and does not charge the budget.
    pub fn poll_peek(&self) -> Poll<Ref<'_, Item<T>>> {
        let reader = self.reader;
        match Ref::filter_map(self.queue.borrow(), |queue| queue.peek(reader)) {
            Ok(item) => Poll::Ready(item),
            Err(_) => {
                self.cx.wait(Wait::Input);
                Poll::Pending
            }
        }
    }

    /// Moves past the next item. The end of the stream stays.
    pub fn advance(&self) {
        self.queue.borrow_mut().advance(self.reader);
    }

    /// Waits for the next item and reads it through `f` without taking it. Waits for budget
    /// first, so that a node looking at many items yields like one taking them.
    pub fn peek_with<'a, R, F>(&'a self, f: F) -> impl Future<Output = R> + 'a
    where
        F: FnOnce(&Item<T>) -> R + 'a,
    {
        let mut f = Some(f);
        poll_fn(move |_| {
            if !self.cx.has_budget() {
                self.cx.wait(Wait::Budget);
                return Poll::Pending;
            }
            let item = match self.poll_peek() {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(item) => item,
            };
            match f.take() {
                Some(f) => Poll::Ready(f(&item)),
                None => Poll::Pending,
            }
        })
    }

    /// Takes the item last peeked at: moves past it and counts the operation.
    pub fn take(&self) {
        self.advance();
        self.cx.spend();
    }

    /// Waits for the next item and takes it; the end of the stream is returned again and
    /// again.
    pub fn recv(&mut self) -> impl Future<Output = Item<T>> + '_
    where
        T: Clone,
    {
        poll_fn(move |_| {
            if !self.cx.has_budget() {
                self.cx.wait(Wait::Budget);
                return Poll::Pending;
            }
            let item = match self.poll_peek() {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(item) => item.clone(),
            };
            self.advance();
            self.cx.spend();
            Poll::Ready(item)
        })
    }
}

impl<T> Drop for Inbox<T> {
    fn drop(&mut self) {
        self.queue.borrow_mut().remove_reader(self.reader);
    }
}

/// A node's writer on one of its output queues.
pub struct Outbox<T> {
    queue: SharedQueue<T>,
    cx: Rc<NodeCx>,
    sink: Rc<SinkInner>,
}

impl<T> fmt::Debug for Outbox<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outbox").finish_non_exhaustive()
    }
}

impl<T: 'static> Outbox<T> {
    pub(crate) fn new(queue: SharedQueue<T>, cx: Rc<NodeCx>) -> Self {
        let sink = Rc::new(SinkInner {
            control: Rc::clone(&queue) as Rc<dyn QueueControl>,
            cx: Rc::clone(&cx),
            forwarded: Cell::new(None),
            held: Cell::new(None),
        });
        Self { queue, cx, sink }
    }
}

impl<T> Outbox<T> {
    /// The node context the outbox charges.
    pub fn cx(&self) -> &Rc<NodeCx> {
        &self.cx
    }

    /// Whether the queue would take an item right now.
    pub fn has_room(&self) -> bool {
        !self.queue.borrow().is_full()
    }

    fn poll_push(&mut self, slot: &mut Option<Item<T>>) -> Poll<()> {
        let Some(item) = slot.take() else {
            return Poll::Ready(());
        };
        if !self.cx.has_budget() {
            *slot = Some(item);
            self.cx.wait(Wait::Budget);
            return Poll::Pending;
        }
        match self.queue.borrow_mut().try_push(item) {
            Ok(()) => {
                self.cx.spend();
                Poll::Ready(())
            }
            Err(item) => {
                *slot = Some(item);
                self.cx.wait(Wait::Output);
                Poll::Pending
            }
        }
    }

    fn push(&mut self, item: Item<T>) -> impl Future<Output = ()> + '_ {
        let mut slot = Some(item);
        poll_fn(move |_| self.poll_push(&mut slot))
    }

    /// Sends an event, waiting for room in the queue.
    pub fn emit(&mut self, at: Provenance, payload: T) -> impl Future<Output = ()> + '_ {
        self.push(Item::Event(Event {
            seq: 0,
            span: at.span,
            settled: at.settled,
            payload,
        }))
    }

    /// Tells the readers that `range` is missing.
    pub fn gap(&mut self, range: TickRange) -> impl Future<Output = ()> + '_ {
        self.push(Item::Gap(range))
    }

    /// Tells the readers that nothing more comes before `tick`.
    pub fn progress(&mut self, tick: Tick) -> impl Future<Output = ()> + '_ {
        self.push(Item::Progress(tick))
    }

    /// Ends the stream at `tick`.
    pub fn end(&mut self, tick: Tick) -> impl Future<Output = ()> + '_ {
        self.push(Item::End(tick))
    }

    /// Where a cursor over a contract passes progress on while the decoder reads a frame
    /// through it, and where the decoder says what it still holds: this output, without its
    /// event type. Every call returns a handle to the same state.
    pub fn progress_sink(&self) -> ProgressSink {
        ProgressSink(Rc::clone(&self.sink))
    }
}

struct SinkInner {
    control: Rc<dyn QueueControl>,
    cx: Rc<NodeCx>,
    /// The latest tick passed on through the sink.
    forwarded: Cell<Option<Tick>>,
    /// From where the decoder holds items it has read and not reported yet.
    held: Cell<Option<Tick>>,
}

/// An output seen only as a place to pass progress on, for cursors over a contract. A cursor
/// passes the progress it meets inside a frame on through it, held back to what the decoder
/// has read through the cursor and not reported yet: what it consumed since it last took the
/// provenance (`consumed()`), and what it says it still holds ([`ProgressSink::hold`]) when
/// it took provenance early for an event still to come. Made by [`Outbox::progress_sink`];
/// clones share their state.
#[derive(Clone)]
pub struct ProgressSink(Rc<SinkInner>);

impl fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgressSink")
            .field("forwarded", &self.0.forwarded.get())
            .field("held", &self.0.held.get())
            .finish()
    }
}

impl ProgressSink {
    /// Passes `Progress(tick)` on, waiting for budget and for room like [`Outbox::progress`];
    /// a tick at or before one already passed on is not sent again.
    pub fn progress(&self, tick: Tick) -> impl Future<Output = ()> + '_ {
        let inner = &*self.0;
        let mut pending = !inner.forwarded.get().is_some_and(|last| last >= tick);
        poll_fn(move |_| {
            if !pending {
                return Poll::Ready(());
            }
            if !inner.cx.has_budget() {
                inner.cx.wait(Wait::Budget);
                return Poll::Pending;
            }
            if inner.control.try_progress(tick) {
                inner.cx.spend();
                inner.forwarded.set(Some(tick));
                pending = false;
                Poll::Ready(())
            } else {
                inner.cx.wait(Wait::Output);
                Poll::Pending
            }
        })
    }

    /// The latest tick passed on through this sink.
    pub fn forwarded(&self) -> Option<Tick> {
        self.0.forwarded.get()
    }

    /// Tells from which tick on the decoder still holds items it read through a cursor and
    /// has not reported — provenance it took early for an event still to come, such as a batch
    /// being filled — so that progress met inside a frame is held back to there; `None` once
    /// nothing is held. A decoder that takes provenance only when it emits never needs this.
    pub fn hold(&self, from: Option<Tick>) {
        self.0.held.set(from);
    }

    /// From where the decoder says it still holds items; see [`ProgressSink::hold`].
    pub fn held(&self) -> Option<Tick> {
        self.0.held.get()
    }
}

#[derive(Clone, Debug)]
pub struct NodeSpec {
    /// The node's name, for diagnostics and statistics.
    pub name: String,
    /// Its input ports.
    pub inputs: Vec<InputSpec>,
    /// Its output ports.
    pub outputs: Vec<OutputSpec>,
}

/// The future that runs a node.
pub type NodeFuture = Pin<Box<dyn Future<Output = ()>>>;

/// A decoder, or any other step of a decode graph.
pub trait Node: 'static {
    /// The node's ports.
    fn spec(&self) -> NodeSpec;

    /// Binds the ports and returns the future that runs the node until its input ends.
    fn start(self: Box<Self>, io: Io) -> NodeFuture;
}

impl Node for Box<dyn Node> {
    fn spec(&self) -> NodeSpec {
        (**self).spec()
    }

    fn start(self: Box<Self>, io: Io) -> NodeFuture {
        (*self).start(io)
    }
}

/// The queues bound to a node's ports, handed to [`Node::start`], with what the node may need
/// to know about them: the time base its inputs are on and the properties of the outputs they
/// read.
pub struct Io {
    cx: Rc<NodeCx>,
    inputs: Vec<Option<Rc<dyn Any>>>,
    outputs: Vec<Option<Rc<dyn Any>>>,
    time_base: Option<TimeBase>,
    input_props: Vec<Props>,
}

impl fmt::Debug for Io {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Io")
            .field("inputs", &self.inputs.len())
            .field("outputs", &self.outputs.len())
            .finish_non_exhaustive()
    }
}

impl Io {
    pub(crate) fn new(
        cx: Rc<NodeCx>,
        inputs: Vec<Rc<dyn Any>>,
        outputs: Vec<Rc<dyn Any>>,
        time_base: Option<TimeBase>,
        input_props: Vec<Props>,
    ) -> Self {
        Self {
            cx,
            inputs: inputs.into_iter().map(Some).collect(),
            outputs: outputs.into_iter().map(Some).collect(),
            time_base,
            input_props,
        }
    }

    /// The clock of the source the node's inputs come from; `None` for a node without inputs.
    /// Decoders turn bit times and timeouts into ticks with it.
    pub fn time_base(&self) -> Option<TimeBase> {
        self.time_base
    }

    /// The properties the output connected to input port `index` declares: what the bus
    /// promised, such as its word width.
    ///
    /// # Panics
    ///
    /// When the node declared no such port.
    pub fn input_props(&self, index: usize) -> &Props {
        self.input_props
            .get(index)
            .unwrap_or_else(|| panic!("the node declared no input port {index}"))
    }

    /// The node context.
    pub fn cx(&self) -> &Rc<NodeCx> {
        &self.cx
    }

    fn take<T: 'static>(
        slots: &mut [Option<Rc<dyn Any>>],
        index: usize,
        what: &str,
    ) -> SharedQueue<T> {
        let slot = slots
            .get_mut(index)
            .unwrap_or_else(|| panic!("the node declared no {what} port {index}"))
            .take()
            .unwrap_or_else(|| panic!("{what} port {index} was bound twice"));
        slot.downcast::<RefCell<Broadcast<T>>>()
            .unwrap_or_else(|_| panic!("{what} port {index} carries another event type"))
    }

    /// The reader on input port `index`.
    ///
    /// # Panics
    ///
    /// When the node declared no such port, bound it already, or declared it with another
    /// event type than `T`: all mistakes in the node itself, not in the graph.
    pub fn input<T: 'static>(&mut self, index: usize) -> Inbox<T> {
        Inbox::new(
            Self::take(&mut self.inputs, index, "input"),
            Rc::clone(&self.cx),
        )
    }

    /// Input port `index` as a cursor over logic samples.
    ///
    /// # Panics
    ///
    /// As [`Io::input`] does.
    pub fn signal(&mut self, index: usize) -> SignalCursor {
        SignalCursor::new(self.input::<Arc<LogicBlock>>(index))
    }

    /// The writer on output port `index`.
    ///
    /// # Panics
    ///
    /// As [`Io::input`] does.
    pub fn output<T: 'static>(&mut self, index: usize) -> Outbox<T> {
        Outbox::new(
            Self::take(&mut self.outputs, index, "output"),
            Rc::clone(&self.cx),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::task::{Context, Waker};

    use super::*;

    fn shared<T>(capacity: usize) -> SharedQueue<T> {
        Rc::new(RefCell::new(Broadcast::new(capacity)))
    }

    fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn reads_wait_for_input_and_writes_for_room() {
        let queue = shared::<u8>(1);
        let cx = NodeCx::new();
        cx.refill(10);
        let mut inbox = Inbox::new(Rc::clone(&queue), Rc::clone(&cx));
        let mut outbox = Outbox::new(Rc::clone(&queue), Rc::clone(&cx));

        let mut read = Box::pin(inbox.recv());
        assert_eq!(poll(read.as_mut()), Poll::Pending);
        assert_eq!(cx.waiting(), Wait::Input);
        drop(read);

        let mut write = Box::pin(outbox.emit(Provenance::at(Tick(3)), 7));
        assert_eq!(poll(write.as_mut()), Poll::Ready(()));
        drop(write);
        let mut write = Box::pin(outbox.emit(Provenance::at(Tick(4)), 8));
        assert_eq!(poll(write.as_mut()), Poll::Pending);
        assert_eq!(cx.waiting(), Wait::Output);
        drop(write);

        let mut read = Box::pin(inbox.recv());
        match poll(read.as_mut()) {
            Poll::Ready(Item::Event(event)) => {
                assert_eq!((event.seq, event.payload, event.settled), (0, 7, Tick(3)));
            }
            other => panic!("expected the event, got {other:?}"),
        }
        drop(read);
        assert_eq!(cx.ops(), 2);
    }

    #[test]
    fn a_spent_budget_pauses_without_consuming() {
        let queue = shared::<u8>(4);
        let cx = NodeCx::new();
        cx.refill(1);
        let mut inbox = Inbox::new(Rc::clone(&queue), Rc::clone(&cx));
        let mut outbox = Outbox::new(Rc::clone(&queue), Rc::clone(&cx));
        let mut write = Box::pin(outbox.emit(Provenance::at(Tick(0)), 1));
        assert_eq!(poll(write.as_mut()), Poll::Ready(()));
        drop(write);
        let mut write = Box::pin(outbox.emit(Provenance::at(Tick(0)), 2));
        assert_eq!(poll(write.as_mut()), Poll::Pending);
        assert_eq!(cx.waiting(), Wait::Budget);
        assert_eq!(queue.borrow().len(), 1);
        drop(write);
        cx.refill(1);
        let mut read = Box::pin(inbox.recv());
        assert!(matches!(poll(read.as_mut()), Poll::Ready(Item::Event(_))));
        drop(read);
        assert!(matches!(
            poll(Box::pin(inbox.recv()).as_mut()),
            Poll::Pending
        ));
        assert_eq!(cx.waiting(), Wait::Budget);
    }

    #[test]
    fn a_sink_passes_progress_on_once_and_waits_for_room() {
        let queue = shared::<u8>(1);
        let cx = NodeCx::new();
        cx.refill(10);
        let mut inbox = Inbox::new(Rc::clone(&queue), Rc::clone(&cx));
        let mut outbox = Outbox::new(Rc::clone(&queue), Rc::clone(&cx));
        let sink = outbox.progress_sink();
        assert_eq!(
            poll(Box::pin(sink.progress(Tick(5))).as_mut()),
            Poll::Ready(())
        );
        assert_eq!(sink.forwarded(), Some(Tick(5)));
        // A tick at or before the one passed on is not sent again; a later one folds into it.
        assert_eq!(
            poll(Box::pin(sink.progress(Tick(3))).as_mut()),
            Poll::Ready(())
        );
        assert_eq!(
            poll(Box::pin(sink.progress(Tick(7))).as_mut()),
            Poll::Ready(())
        );
        assert_eq!(queue.borrow().len(), 1);
        assert!(matches!(
            poll(Box::pin(inbox.recv()).as_mut()),
            Poll::Ready(Item::Progress(Tick(7)))
        ));
        // Behind an event in a full queue, progress waits for room like any item.
        assert_eq!(
            poll(Box::pin(outbox.emit(Provenance::at(Tick(8)), 1)).as_mut()),
            Poll::Ready(())
        );
        assert_eq!(
            poll(Box::pin(sink.progress(Tick(9))).as_mut()),
            Poll::Pending
        );
        assert_eq!(cx.waiting(), Wait::Output);
        assert!(matches!(
            poll(Box::pin(inbox.recv()).as_mut()),
            Poll::Ready(Item::Event(_))
        ));
        assert_eq!(
            poll(Box::pin(sink.progress(Tick(9))).as_mut()),
            Poll::Ready(())
        );
        // Every sink of an output shares what the decoder says it holds.
        let other = outbox.progress_sink();
        other.hold(Some(Tick(4)));
        assert_eq!(sink.held(), Some(Tick(4)));
        sink.hold(None);
        assert_eq!(other.held(), None);
    }

    #[test]
    fn dropping_an_inbox_detaches_its_reader() {
        let queue = shared::<u8>(1);
        let cx = NodeCx::new();
        cx.refill(10);
        let inbox = Inbox::new(Rc::clone(&queue), Rc::clone(&cx));
        let mut outbox = Outbox::new(Rc::clone(&queue), Rc::clone(&cx));
        assert_eq!(
            poll(Box::pin(outbox.emit(Provenance::at(Tick(0)), 1)).as_mut()),
            Poll::Ready(())
        );
        assert!(!outbox.has_room());
        drop(inbox);
        assert!(outbox.has_room());
    }
}
