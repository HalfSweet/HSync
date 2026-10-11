// SPDX-License-Identifier: Apache-2.0

//! The APA102 decoder: the frames a controller clocks into a chain of APA102 or SK9822 LEDs,
//! read from the MOSI words of an SPI stream.
//!
//! Implemented from the APA102 and APA102C datasheets (Shiji Lighting) and the SK9822 datasheet
//! (Shenzhen Normand Electronic): two wires, clock and data, the data taken on the clock's
//! rising edge, most significant bit first — SPI without chip select; a frame is a start frame
//! of 32 zero bits, one 32-bit LED frame per LED — the bits `111`, five bits of global
//! brightness, then blue, green and red bytes — and an end frame of at least half as many bits as
//! there are LEDs, ones on the APA102; the SK9822 takes zeros, after a reset frame of 32 zero bits
//! before the next start frame. Each LED keeps the first LED frame it receives after a start
//! frame and passes the rest on.
//!
//! The rules:
//!
//! - the bytes are the stream's MOSI words, 8 bits wide, most significant bit first; MISO is
//!   not looked at; the parts take data on the rising edge, so SPI mode 0 or 3 is required;
//! - a start frame is four zero bytes: the last four zero bytes before a byte that is not zero,
//!   so that a run of zeros longer than four — an end frame or a reset frame of zeros followed by
//!   the next start frame — is told apart when it ends. That byte begins the frame, as its first
//!   LED frame's first byte;
//! - a byte with its top three bits `111` at the start of an LED frame begins one, which takes
//!   it and the three bytes after it: brightness (its low five bits), blue, green and red;
//! - LED frames follow one another until a byte at the start of an LED frame lacks the `111`
//!   bits: from there on the bytes are the end of the frame, counted in `end_bits` and not
//!   interpreted, up to the next start frame. When the first byte after a start frame lacks the
//!   bits, the frame reports an `error` (`header`) and has no LED frames;
//! - an LED frame whose last bytes are zero can hold the beginning of the next start frame — an
//!   end frame shorter than an LED frame, followed by a start frame. It is reported once it is
//!   known not to: when a byte that is not zero comes before four zero bytes have followed it,
//!   when four have, or when the frame ends otherwise. When the start frame reaches into it, it
//!   is no LED: its bytes before the start frame are end bits;
//! - with chip select (`cs = true` on the stream) each transaction is read on its own, from a
//!   search for a start frame: a frame under way when chip select is released ends `release`,
//!   with the bytes after its last LED frame — an LED frame cut short among them — as end bits.
//!   Without chip select the stream is one: a frame ends when the next frame's start frame is
//!   found, `next-frame`;
//! - a gap, the end of the stream or a transaction cut off ends the frame under way `aborted`,
//!   its bytes after the last LED frame counted the same way;
//! - bytes before the first start frame — of the stream, after a gap, or of a transaction —
//!   belong to no frame, so a frame under way at the first sample yields nothing until a start
//!   frame;
//! - a `frame-begin` spans its start frame and is settled with the byte after it, which decided
//!   it; an `led` spans its four bytes and is settled with its last one, or with the byte or the
//!   end that showed it was no start frame; an `error` takes its byte's provenance; a
//!   `frame-end` spans its end bits — up to the next start frame, through chip select's release,
//!   or to the last of them when aborted; empty at the start frame, the release, the gap's start
//!   or the end when there are none — and is settled with what ended it: the byte after the next
//!   start frame, the release, the gap's start or the end;
//! - progress is passed on, held back to the first byte of whatever is still to be reported: an
//!   LED frame, the end bits, or zero bytes that may become a start frame.
//!
//! Limits: the end frame of the APA102 datasheet, 32 one bits, has the shape of an LED frame —
//! brightness 31, full white — and is reported as one, since only a reader that knows the length
//! of the chain can tell the two apart; shorter end frames are counted as end bits. The bytes
//! are the SPI decoder's words: without chip select they line up with the bytes the controller
//! sent only if the samples begin, and resume after a gap, while the clock is idle between two
//! bytes — otherwise every later word is shifted and nothing after is read right — whereas
//! with chip select every transaction lines them up anew. A frame split over several
//! transactions is not joined: read such a bus without chip select. The brightness is reported
//! as sent, not applied, and when the LEDs latch the data they received is not modelled.

use uiharu_core::{
    Inbox, InputRef, InputSpec, Io, Item, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, ProgressSink, PropValue, Props, Provenance, Requires, Tick, TickRange,
};

use super::events::{Apa102Event, EndReason, ErrorReason, FrameId};
use crate::spi::{Next, Read, ReadError, SpiBus, SpiEvent, Transaction};

/// The bits of an LED frame's first byte that say it is one.
const HEADER_MASK: u8 = 0b1110_0000;

/// Whether `byte` can begin an LED frame.
const fn is_header(byte: u8) -> bool {
    byte & HEADER_MASK == HEADER_MASK
}

/// The APA102 decoder node: `spi.events` in, `apa102.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Apa102;

impl Apa102 {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The SPI input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<SpiEvent> {
        InputRef::of_node(node, 0)
    }

    /// The frame output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Apa102Event> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the SPI stream it reads: bytes on MOSI, most significant
    /// bit first. Chip select and MISO may be there or not.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("wordBits", 8)
            .equals("bitOrder", "msb-first")
            .equals("mosi", true)
            .one_of("mode", [0, 3])
    }
}

impl Node for Apa102 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "apa102".into(),
            inputs: vec![InputSpec::new::<SpiEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<Apa102Event>("events", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let framed = matches!(io.input_props(0).get("cs"), Some(PropValue::Bool(true)));
        let out = io.output(0);
        let sink = out.progress_sink();
        let frames = Frames::new(out);
        if framed {
            Box::pin(framed_run(SpiBus::new(io.input(0), sink), frames))
        } else {
            Box::pin(unframed_run(io.input(0), frames))
        }
    }
}

/// Up to four bytes of an LED frame and where each came from.
#[derive(Clone, Copy, Debug)]
struct LedBytes {
    count: usize,
    bytes: [u8; 4],
    at: [Provenance; 4],
}

impl LedBytes {
    const EMPTY: LedBytes = LedBytes {
        count: 0,
        bytes: [0; 4],
        at: [Provenance::at(Tick::ZERO); 4],
    };

    fn push(&mut self, byte: u8, at: Provenance) {
        if let (Some(slot), Some(place)) =
            (self.bytes.get_mut(self.count), self.at.get_mut(self.count))
        {
            *slot = byte;
            *place = at;
            self.count += 1;
        }
    }

    fn is_whole(&self) -> bool {
        self.count == 4
    }

    /// The zero bytes the frame ends with.
    fn trailing_zeros(&self) -> u32 {
        let zeros = self.bytes[..self.count]
            .iter()
            .rev()
            .take_while(|byte| **byte == 0)
            .count();
        u32::try_from(zeros).unwrap_or(4)
    }

    /// The ticks of the first `bytes` bytes, if any.
    fn span_of(&self, bytes: usize) -> Option<TickRange> {
        let bytes = bytes.min(self.count);
        (bytes > 0).then(|| TickRange::new(self.at[0].span.start, self.at[bytes - 1].span.end))
    }

    /// Where the first byte begins, if there is one.
    fn start(&self) -> Option<Tick> {
        (self.count > 0).then_some(self.at[0].span.start)
    }
}

/// A whole LED frame that ends in zero bytes, kept until it is known whether the next start
/// frame reaches into it.
#[derive(Clone, Copy, Debug)]
struct Pending {
    led: LedBytes,
    /// Its zero bytes at the end, one to three.
    tail: u32,
}

/// The provenance of the last four bytes read.
#[derive(Clone, Copy, Debug)]
struct Recent {
    at: [Provenance; 4],
    next: usize,
    len: usize,
}

impl Recent {
    const EMPTY: Recent = Recent {
        at: [Provenance::at(Tick::ZERO); 4],
        next: 0,
        len: 0,
    };

    fn push(&mut self, at: Provenance) {
        self.at[self.next] = at;
        self.next = (self.next + 1) % 4;
        self.len = (self.len + 1).min(4);
    }

    /// The `n`-th last byte, 1 being the last, while `n` is one of the bytes held.
    fn back(&self, n: usize) -> Option<Provenance> {
        (1..=self.len)
            .contains(&n)
            .then(|| self.at[(self.next + 4 - n) % 4])
    }

    /// The ticks of the last `n` bytes.
    fn span(&self, n: usize) -> Option<TickRange> {
        let first = self.back(n)?;
        let last = self.back(1)?;
        Some(TickRange::new(first.span.start, last.span.end))
    }
}

/// The frame under way.
#[derive(Debug)]
struct Open {
    id: FrameId,
    /// The LED frames reported.
    leds: u32,
    /// The LED frame being read.
    led: LedBytes,
    pending: Option<Pending>,
    /// Whether the LED frames are over: bytes are end bits now.
    ending: bool,
    /// The bytes after the last LED frame, and their ticks.
    end_bytes: u32,
    end_span: Option<TickRange>,
}

impl Open {
    fn new(id: FrameId) -> Self {
        Self {
            id,
            leds: 0,
            led: LedBytes::EMPTY,
            pending: None,
            ending: false,
            end_bytes: 0,
            end_span: None,
        }
    }

    /// Counts a byte, or bytes, as end bits.
    fn add_end(&mut self, bytes: u32, span: TickRange) {
        self.end_bytes = self.end_bytes.saturating_add(bytes);
        self.end_span = Some(self.end_span.map_or(span, |have| have.join(span)));
    }

    /// Reports `led` as the next LED frame, settled at `settled`.
    async fn report(&mut self, out: &mut Outbox<Apa102Event>, led: LedBytes, settled: Tick) {
        let [header, blue, green, red] = led.bytes;
        let event = Apa102Event::Led {
            frame: self.id,
            index: self.leds,
            brightness: header & !HEADER_MASK,
            blue,
            green,
            red,
        };
        let span = led.span_of(4).unwrap_or(TickRange::at(settled));
        out.emit(Provenance::over(span, settled), event).await;
        self.leds = self.leds.saturating_add(1);
    }

    /// Settles what is held when the frame ends without a next start frame, at `settled`: a
    /// pending LED frame is an LED, and an LED frame cut short is end bits.
    async fn settle(&mut self, out: &mut Outbox<Apa102Event>, settled: Tick) {
        if let Some(pending) = self.pending.take() {
            self.report(out, pending.led, settled).await;
        }
        let cut = std::mem::replace(&mut self.led, LedBytes::EMPTY);
        if let Some(span) = cut.span_of(cut.count) {
            self.add_end(u32::try_from(cut.count).unwrap_or(4), span);
        }
    }

    fn end_bits(bytes: u32) -> u32 {
        bytes.saturating_mul(8)
    }
}

/// The decoder's state and its typed writer.
#[derive(Debug)]
struct Frames {
    out: Outbox<Apa102Event>,
    sink: ProgressSink,
    next_id: u64,
    open: Option<Open>,
    /// The zero bytes just read, in a row.
    zeros: u32,
    recent: Recent,
}

impl Frames {
    /// Tells the cursor from where this writer holds items it has not reported yet.
    fn sync_hold(&self) {
        self.sink.hold(self.held_from());
    }

    fn new(out: Outbox<Apa102Event>) -> Self {
        Self {
            sink: out.progress_sink(),
            out,
            next_id: 1,
            open: None,
            zeros: 0,
            recent: Recent::EMPTY,
        }
    }

    /// Forgets the bytes read so far: what comes next is looked at for a start frame afresh.
    fn forget(&mut self) {
        self.zeros = 0;
        self.recent = Recent::EMPTY;
        self.sync_hold();
    }

    /// A byte of the stream.
    async fn byte(&mut self, value: u8, at: Provenance) {
        if value == 0 {
            self.zero(at).await;
        } else {
            self.nonzero(value, at).await;
        }
        self.recent.push(at);
        self.sync_hold();
    }

    async fn zero(&mut self, at: Provenance) {
        let Self {
            out, open, zeros, ..
        } = self;
        *zeros = zeros.saturating_add(1);
        let Some(frame) = open.as_mut() else {
            return;
        };
        if frame.ending {
            frame.add_end(1, at.span);
        } else if frame.led.count > 0 {
            frame.led.push(0, at);
            if frame.led.is_whole() {
                let led = std::mem::replace(&mut frame.led, LedBytes::EMPTY);
                frame.pending = Some(Pending {
                    tail: led.trailing_zeros(),
                    led,
                });
            }
            return;
        } else {
            // A byte without the `111` bits where an LED frame would begin ends the LED frames.
            frame.ending = true;
            frame.add_end(1, at.span);
        }
        // Four zero bytes after a pending LED frame: no start frame reaches into it.
        if let Some(pending) = frame.pending
            && zeros.saturating_sub(pending.tail) >= 4
        {
            frame.pending = None;
            frame.report(out, pending.led, at.settled).await;
        }
    }

    async fn nonzero(&mut self, value: u8, at: Provenance) {
        let run = std::mem::take(&mut self.zeros);
        if run >= 4
            && let Some(start) = self.recent.span(4)
        {
            self.next_frame(value, at, run, start).await;
            return;
        }
        let Self { out, open, .. } = self;
        let Some(frame) = open.as_mut() else {
            // No start frame yet: the byte belongs to no frame.
            return;
        };
        if let Some(pending) = frame.pending.take() {
            frame.report(out, pending.led, at.settled).await;
        }
        if frame.ending {
            frame.add_end(1, at.span);
        } else if frame.led.count > 0 {
            frame.led.push(value, at);
            if frame.led.is_whole() {
                let led = std::mem::replace(&mut frame.led, LedBytes::EMPTY);
                frame.report(out, led, at.settled).await;
            }
        } else if is_header(value) {
            frame.led.push(value, at);
        } else {
            frame.ending = true;
            frame.add_end(1, at.span);
        }
    }

    /// The `run` zero bytes before `value`, the last four of which span `start`, were a start
    /// frame: the frame under way ends and the next begins, `value` its first byte.
    async fn next_frame(&mut self, value: u8, at: Provenance, run: u32, start: TickRange) {
        if let Some(frame) = self.open.take() {
            let (end_bytes, from) = match frame.pending {
                // The start frame reaches into the pending LED frame, which is no LED: its
                // bytes before the start frame, as many as zero bytes came after it, are end
                // bits.
                Some(pending) => (run.saturating_sub(pending.tail), pending.led.start()),
                // The start frame is the last four of the end bytes.
                None => {
                    let bytes = frame.end_bytes.saturating_sub(4);
                    let from = frame.end_span.map(|span| span.start).filter(|_| bytes > 0);
                    (bytes, from)
                }
            };
            let end = Apa102Event::FrameEnd {
                frame: frame.id,
                leds: frame.leds,
                end_bits: Open::end_bits(end_bytes),
                reason: EndReason::NextFrame,
            };
            // From the first end byte up to the start frame.
            let span = from.map_or(TickRange::at(start.start), |from| {
                TickRange::new(from, start.start)
            });
            self.out.emit(Provenance::over(span, at.settled), end).await;
        }
        let id = FrameId(self.next_id);
        self.next_id += 1;
        self.out
            .emit(
                Provenance::over(start, at.settled),
                Apa102Event::FrameBegin { frame: id },
            )
            .await;
        let mut frame = Open::new(id);
        if is_header(value) {
            frame.led.push(value, at);
        } else {
            let error = Apa102Event::Error {
                frame: id,
                reason: ErrorReason::Header,
            };
            self.out.emit(at, error).await;
            frame.ending = true;
            frame.add_end(1, at.span);
        }
        self.open = Some(frame);
    }

    /// Chip select was released, at `at`: the frame under way ends there.
    async fn release(&mut self, at: Provenance) {
        self.forget();
        let Self { out, open, .. } = self;
        let Some(mut frame) = open.take() else {
            return;
        };
        frame.settle(out, at.settled).await;
        let span = frame.end_span.map_or(at.span, |span| span.join(at.span));
        let end = Apa102Event::FrameEnd {
            frame: frame.id,
            leds: frame.leds,
            end_bits: Open::end_bits(frame.end_bytes),
            reason: EndReason::Release,
        };
        out.emit(Provenance::over(span, at.settled), end).await;
        self.sync_hold();
    }

    /// The bytes stopped at `tick`, cut off: the frame under way ends there.
    async fn abort(&mut self, tick: Tick) {
        self.forget();
        let Self { out, open, .. } = self;
        let Some(mut frame) = open.take() else {
            return;
        };
        frame.settle(out, tick).await;
        let span = frame.end_span.unwrap_or(TickRange::at(tick));
        let end = Apa102Event::FrameEnd {
            frame: frame.id,
            leds: frame.leds,
            end_bits: Open::end_bits(frame.end_bytes),
            reason: EndReason::Aborted,
        };
        out.emit(Provenance::over(span, tick), end).await;
        self.sync_hold();
    }

    /// Progress of the stream: nothing comes before `tick`.
    /// The first tick of whatever is still to be reported: the zero bytes that may become a
    /// start frame (the last four at most), the LED frame being read and the one held back.
    fn held_from(&self) -> Option<Tick> {
        let zeros = usize::try_from(self.zeros).unwrap_or(4).min(4);
        let mut held = self.recent.back(zeros).map(|first| first.span.start);
        if let Some(frame) = &self.open {
            let starts = [
                frame.end_span.map(|span| span.start),
                frame.pending.and_then(|pending| pending.led.start()),
                frame.led.start(),
            ];
            held = starts.into_iter().flatten().chain(held).min();
        }
        held
    }

    async fn progress(&mut self, tick: Tick) {
        let held = self.held_from().map_or(tick, |held| held.min(tick));
        self.out.progress(held).await;
    }

    async fn gap(&mut self, range: TickRange) {
        self.abort(range.start).await;
        self.out.gap(range).await;
    }

    async fn end(&mut self, tick: Tick) {
        self.abort(tick).await;
        self.out.end(tick).await;
    }
}

/// The body for one transaction of a bus with chip select: its words, read on their own.
async fn transaction(mut tx: Transaction<'_>, frames: &mut Frames) -> Result<(), ReadError> {
    frames.forget();
    // The begin's ticks are no byte's.
    tx.consumed();
    loop {
        match tx.word_or_end().await? {
            Read::Word(word) => {
                let value = word.mosi()?;
                frames.byte(value, tx.consumed()).await;
            }
            Read::End => {
                frames.release(tx.consumed()).await;
                return Ok(());
            }
        }
    }
}

/// Ends the frame the body left open, as the transaction's end calls for.
async fn recover(error: ReadError, bus: &mut SpiBus, frames: &mut Frames) {
    let tick = match error {
        ReadError::Gap(range) => range.start,
        ReadError::EndOfInput(tick) => tick,
        // Cut off: the stream's aborted end was consumed, and says where.
        ReadError::Truncated | ReadError::Incomplete | ReadError::MissingLine(_) => {
            bus.consumed().settled
        }
    };
    frames.abort(tick).await;
}

/// Reads a bus with chip select, one transaction at a time.
async fn framed_run(mut bus: SpiBus, mut frames: Frames) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                if let Err(error) = transaction(tx, &mut frames).await {
                    recover(error, &mut bus, &mut frames).await;
                }
                if let Err(error) = bus.finish().await {
                    recover(error, &mut bus, &mut frames).await;
                }
                bus.consumed();
            }
            Next::Gap(range) => frames.gap(range).await,
            Next::Progress(tick) => frames.progress(tick).await,
            Next::End(tick) => {
                frames.end(tick).await;
                return;
            }
        }
    }
}

/// Reads a bus without chip select: one unframed stream of words, transaction 0, which never
/// begins, so it is read from the inbox itself.
async fn unframed_run(mut input: Inbox<SpiEvent>, mut frames: Frames) {
    loop {
        match input.recv().await {
            Item::Event(event) => {
                // The requirements put a value on MOSI; a bus without chip select has no begin
                // or end.
                if let SpiEvent::Word {
                    mosi: Some(value), ..
                } = event.payload
                {
                    let at = Provenance::over(event.span, event.settled);
                    frames.byte(value, at).await;
                }
            }
            Item::Progress(tick) => frames.progress(tick).await,
            Item::Gap(range) => frames.gap(range).await,
            Item::End(tick) => {
                frames.end(tick).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apa102::APA102_EVENTS;
    use crate::spi::{BitOrder, SpiConfig};

    #[test]
    fn requires_bytes_on_mosi_most_significant_bit_first() {
        let requires = Apa102::requires();
        let spi = |config: SpiConfig| requires.unmet(&config.props()).map(|(key, _)| key);
        let bus = SpiConfig {
            cs: None,
            miso: None,
            ..SpiConfig::default()
        };
        assert_eq!(spi(bus.clone()), None);
        assert_eq!(spi(SpiConfig::default()), None);
        assert_eq!(
            spi(SpiConfig {
                word_bits: 4,
                ..bus.clone()
            }),
            Some("wordBits")
        );
        assert_eq!(
            spi(SpiConfig {
                bit_order: BitOrder::LsbFirst,
                ..bus.clone()
            }),
            Some("bitOrder")
        );
        assert_eq!(
            spi(SpiConfig {
                mosi: None,
                miso: Some(3),
                ..bus
            }),
            Some("mosi")
        );
        let spec = Apa102::new().spec();
        assert_eq!(spec.inputs[0].name, "bus");
        assert_eq!(spec.outputs[0].name, "events");
        assert_eq!(spec.outputs[0].schema, APA102_EVENTS);
    }

    #[test]
    fn headers_have_their_top_three_bits_set() {
        assert!(is_header(0xe0));
        assert!(is_header(0xff));
        assert!(!is_header(0xdf));
        assert!(!is_header(0x00));
        assert!(!is_header(0x7f));
    }

    #[test]
    fn led_bytes_keep_their_ticks() {
        let mut led = LedBytes::EMPTY;
        assert_eq!(led.start(), None);
        for (i, byte) in [0xe1u8, 0x10, 0x00, 0x00].into_iter().enumerate() {
            let start = Tick(100 + 32 * i as u64);
            led.push(
                byte,
                Provenance::over(TickRange::new(start, start.offset(29)), start),
            );
        }
        assert!(led.is_whole());
        assert_eq!(led.trailing_zeros(), 2);
        assert_eq!(led.start(), Some(Tick(100)));
        assert_eq!(led.span_of(2), Some(TickRange::new(Tick(100), Tick(161))));
        assert_eq!(led.span_of(4), Some(TickRange::new(Tick(100), Tick(225))));
        assert_eq!(led.span_of(0), None);
        // A fifth byte does not fit.
        led.push(0xff, Provenance::at(Tick(500)));
        assert_eq!(led.count, 4);
    }

    #[test]
    fn the_last_four_bytes_are_remembered() {
        let mut recent = Recent::EMPTY;
        assert_eq!(recent.back(1), None);
        for i in 0..6u64 {
            recent.push(Provenance::over(
                TickRange::new(Tick(10 * i), Tick(10 * i + 8)),
                Tick(10 * i + 7),
            ));
        }
        assert_eq!(recent.back(1).map(|at| at.settled), Some(Tick(57)));
        assert_eq!(recent.back(4).map(|at| at.settled), Some(Tick(27)));
        assert_eq!(recent.back(5), None);
        assert_eq!(recent.span(4), Some(TickRange::new(Tick(20), Tick(58))));
    }
}
