// SPDX-License-Identifier: Apache-2.0

//! Event streams: the envelope every event carries, the control items that travel with events,
//! and the bounded broadcast queue that hands one producer's items to every reader.
//!
//! A queue holds each item once. Readers keep their own position and the queue drops an item
//! once every reader has passed it, so a slow reader holds the queue, and through it the
//! producer, back: that is the backpressure decoders feel in [`Outbox::emit`].
//!
//! [`Outbox::emit`]: crate::node::Outbox::emit

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::time::{Tick, TickRange};

/// An event with its envelope: where on the signal it comes from, when it became certain and
/// its position in the stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event<T> {
    /// The event's number in its stream, from 0, counting events only.
    pub seq: u64,
    /// The ticks the event covers on the signal; empty for a result decided at one instant.
    pub span: TickRange,
    /// The tick at which the result became certain. No later event of the stream is settled
    /// earlier.
    pub settled: Tick,
    /// The event itself.
    pub payload: T,
}

/// What travels along a stream: events, and the control items that frame them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item<T> {
    /// An event.
    Event(Event<T>),
    /// Samples, and whatever happened in them, are missing for these ticks.
    Gap(TickRange),
    /// Nothing more will be reported before this tick; time has moved on without events.
    Progress(Tick),
    /// The stream has ended at this tick; nothing follows.
    End(Tick),
}

impl<T> Item<T> {
    /// The event, if this is one.
    pub fn event(&self) -> Option<&Event<T>> {
        match self {
            Item::Event(event) => Some(event),
            _ => None,
        }
    }

    /// The event, if this is one, giving the item up.
    pub fn into_event(self) -> Option<Event<T>> {
        match self {
            Item::Event(event) => Some(event),
            _ => None,
        }
    }

    /// Whether this is the end of the stream.
    pub fn is_end(&self) -> bool {
        matches!(self, Item::End(_))
    }
}

/// A reader's identity on a [`Broadcast`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReaderId(usize);

/// A bounded queue from one producer to any number of readers.
#[derive(Debug)]
pub struct Broadcast<T> {
    items: VecDeque<Item<T>>,
    /// The absolute index of `items[0]`.
    head: u64,
    /// The next absolute index each reader will read; `None` for a detached reader.
    readers: Vec<Option<u64>>,
    capacity: usize,
    next_seq: u64,
    high_water: usize,
    ended: bool,
}

impl<T> Broadcast<T> {
    /// A queue holding at most `capacity` items; at least one.
    pub fn new(capacity: usize) -> Self {
        Self {
            items: VecDeque::new(),
            head: 0,
            readers: Vec::new(),
            capacity: capacity.max(1),
            next_seq: 0,
            high_water: 0,
            ended: false,
        }
    }

    /// Adds a reader that starts at the oldest item still held.
    pub fn add_reader(&mut self) -> ReaderId {
        self.readers.push(Some(self.head));
        ReaderId(self.readers.len() - 1)
    }

    /// Removes a reader; the items it alone held back are dropped.
    pub fn remove_reader(&mut self, reader: ReaderId) {
        if let Some(slot) = self.readers.get_mut(reader.0) {
            *slot = None;
        }
        self.trim();
    }

    /// Whether another item would not fit.
    pub fn is_full(&self) -> bool {
        self.items.len() >= self.capacity
    }

    /// Whether the end of the stream has been pushed.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// The items held right now.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether no item is held.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The most items held at any time so far.
    pub fn high_water(&self) -> usize {
        self.high_water
    }

    /// The most items the queue holds.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The next event's sequence number.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Appends `item`, numbering an event, or hands it back when the queue is full. Two
    /// progress items in a row fold into one, so progress never fills a queue. Nothing can
    /// follow the end of the stream.
    pub fn try_push(&mut self, item: Item<T>) -> Result<(), Item<T>> {
        debug_assert!(
            !self.ended,
            "an item was pushed after the end of the stream"
        );
        if let Item::Progress(tick) = item
            && let Some(Item::Progress(last)) = self.items.back_mut()
        {
            *last = (*last).max(tick);
            return Ok(());
        }
        if self.is_full() {
            return Err(item);
        }
        let item = match item {
            Item::Event(mut event) => {
                event.seq = self.next_seq;
                self.next_seq += 1;
                Item::Event(event)
            }
            Item::End(tick) => {
                self.ended = true;
                Item::End(tick)
            }
            other => other,
        };
        self.items.push_back(item);
        self.high_water = self.high_water.max(self.items.len());
        Ok(())
    }

    /// The items waiting for `reader`.
    pub fn pending(&self, reader: ReaderId) -> usize {
        let Some(Some(next)) = self.readers.get(reader.0).copied() else {
            return 0;
        };
        usize::try_from(self.head + self.items.len() as u64 - next).unwrap_or(0)
    }

    /// The next item for `reader`, if one is waiting.
    pub fn peek(&self, reader: ReaderId) -> Option<&Item<T>> {
        let next = self.readers.get(reader.0).copied().flatten()?;
        self.items.get(usize::try_from(next - self.head).ok()?)
    }

    /// Moves `reader` past its next item. The end of the stream stays in place: a reader never
    /// moves past it.
    pub fn advance(&mut self, reader: ReaderId) {
        let Some(Some(next)) = self.readers.get(reader.0).copied() else {
            return;
        };
        let Some(item) = self
            .items
            .get(usize::try_from(next - self.head).unwrap_or(usize::MAX))
        else {
            return;
        };
        if item.is_end() {
            return;
        }
        if let Some(slot) = self.readers.get_mut(reader.0) {
            *slot = Some(next + 1);
        }
        self.trim();
    }

    /// Drops the items every reader has passed.
    fn trim(&mut self) {
        let slowest = self.readers.iter().flatten().copied().min();
        let keep_from = match slowest {
            Some(index) => index,
            // Without readers nothing holds the items back.
            None => self.head + self.items.len() as u64,
        };
        while self.head < keep_from {
            if self.items.front().is_some_and(Item::is_end) {
                break;
            }
            self.items.pop_front();
            self.head += 1;
        }
    }
}

/// What the executor does with a queue without knowing its payload type.
pub trait QueueControl {
    /// The items held right now.
    fn len(&self) -> usize;
    /// Whether no item is held.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The most items the queue holds.
    fn capacity(&self) -> usize;
    /// The most items held at any time so far.
    fn high_water(&self) -> usize;
    /// Whether the end of the stream has been pushed.
    fn is_ended(&self) -> bool;
    /// Adds a reader that starts at the oldest item still held.
    fn add_reader(&self) -> ReaderId;
}

impl<T: 'static> QueueControl for RefCell<Broadcast<T>> {
    fn len(&self) -> usize {
        self.borrow().len()
    }

    fn add_reader(&self) -> ReaderId {
        self.borrow_mut().add_reader()
    }

    fn capacity(&self) -> usize {
        self.borrow().capacity()
    }

    fn high_water(&self) -> usize {
        self.borrow().high_water()
    }

    fn is_ended(&self) -> bool {
        self.borrow().is_ended()
    }
}

/// A queue as the graph holds it: untyped for binding, and for control and statistics.
#[derive(Clone)]
pub(crate) struct QueueHandles {
    pub(crate) any: Rc<dyn Any>,
    pub(crate) control: Rc<dyn QueueControl>,
}

/// Makes the queue of an output port carrying `T`.
pub(crate) fn make_queue<T: 'static>(capacity: usize) -> QueueHandles {
    let queue = Rc::new(RefCell::new(Broadcast::<T>::new(capacity)));
    QueueHandles {
        any: Rc::clone(&queue) as Rc<dyn Any>,
        control: queue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(payload: u32) -> Item<u32> {
        Item::Event(Event {
            seq: 0,
            span: TickRange::at(Tick(0)),
            settled: Tick(0),
            payload,
        })
    }

    #[test]
    fn readers_share_items_and_the_slowest_holds_them() {
        let mut queue = Broadcast::new(2);
        let fast = queue.add_reader();
        let slow = queue.add_reader();
        assert!(queue.try_push(event(1)).is_ok());
        assert!(queue.try_push(event(2)).is_ok());
        assert!(queue.try_push(event(3)).is_err(), "the queue is full");

        assert_eq!(
            queue.peek(fast).and_then(Item::event).map(|e| e.payload),
            Some(1)
        );
        queue.advance(fast);
        queue.advance(fast);
        assert!(queue.peek(fast).is_none());
        assert_eq!(queue.len(), 2, "the slow reader still holds both");
        assert!(queue.is_full());

        queue.advance(slow);
        assert_eq!(queue.len(), 1);
        assert!(queue.try_push(event(3)).is_ok());
        assert_eq!(
            queue
                .peek(fast)
                .and_then(Item::event)
                .map(|e| (e.seq, e.payload)),
            Some((2, 3))
        );
        assert_eq!(queue.high_water(), 2);
    }

    #[test]
    fn events_are_numbered_in_order() {
        let mut queue = Broadcast::new(8);
        let reader = queue.add_reader();
        for payload in 0..3 {
            queue.try_push(event(payload)).unwrap();
        }
        queue
            .try_push(Item::Gap(TickRange::new(Tick(1), Tick(2))))
            .unwrap();
        queue.try_push(event(3)).unwrap();
        let mut seqs = Vec::new();
        while let Some(item) = queue.peek(reader) {
            if let Some(event) = item.event() {
                seqs.push(event.seq);
            }
            queue.advance(reader);
        }
        assert_eq!(seqs, vec![0, 1, 2, 3]);
    }

    #[test]
    fn progress_folds_and_the_end_stays() {
        let mut queue: Broadcast<u32> = Broadcast::new(1);
        let reader = queue.add_reader();
        queue.try_push(Item::Progress(Tick(5))).unwrap();
        queue.try_push(Item::Progress(Tick(9))).unwrap();
        queue.try_push(Item::Progress(Tick(7))).unwrap();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.peek(reader), Some(&Item::Progress(Tick(9))));
        queue.advance(reader);
        queue.try_push(Item::End(Tick(10))).unwrap();
        assert!(queue.is_ended());
        queue.advance(reader);
        queue.advance(reader);
        assert_eq!(queue.peek(reader), Some(&Item::End(Tick(10))));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn removing_a_reader_releases_its_items() {
        let mut queue = Broadcast::new(4);
        let gone = queue.add_reader();
        let stays = queue.add_reader();
        queue.try_push(event(1)).unwrap();
        queue.advance(stays);
        assert_eq!(queue.len(), 1);
        queue.remove_reader(gone);
        assert!(queue.is_empty());
        assert!(queue.peek(gone).is_none());
    }
}
