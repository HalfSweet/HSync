// SPDX-License-Identifier: Apache-2.0

//! The MIDI decoder: the messages of a MIDI 1.0 link, read from the characters of a UART
//! stream.
//!
//! Implemented from the MIDI 1.0 Detailed Specification, document version 4.2.1 (MIDI
//! Manufacturers Association, 1996): the hardware (31.25 kbaud, a start bit, eight data bits
//! least significant bit first and a stop bit), status and data bytes, running status, the
//! messages and the data bytes each takes (Tables I to VII), system exclusive messages and their
//! end, and, in the Additional Explanations and Application Notes, the receiver's running status
//! buffer and the undefined status bytes.
//!
//! The rules:
//!
//! - a byte with bit 7 set is a status byte, any other a data byte;
//! - a real-time status byte, 0xF8 to 0xFF, is a message of its own wherever it comes, even
//!   between the bytes of another message or inside a system exclusive message: it is reported
//!   at once and changes nothing else, so the message it falls in goes on and the running status
//!   stays. 0xF9 and 0xFD are undefined, and reported as such;
//! - any other status byte ends what is under way: a message still missing data bytes is
//!   `error { incomplete }`, and a system exclusive message ends `interrupted`, unless the byte is
//!   the end of exclusive, 0xF7, which ends it normally;
//! - a channel status byte, 0x80 to 0xEF, begins a message of one data byte (program change,
//!   channel pressure) or two (the others) and becomes the running status. A system common or
//!   exclusive status byte, 0xF0 to 0xF7, clears the running status — the undefined 0xF4 and
//!   0xF5 as well, which are `error { undefined-status }`; an end of exclusive with no system
//!   exclusive message under way is `error { unexpected-end-of-exclusive }`;
//! - a channel or system common message goes out with its last data byte, a tune request
//!   (0xF6), which has none, at once; data bytes are reported as sent;
//! - a data byte when no message is under way begins a message of the running status, marked
//!   `running`. Without a running status it is `error { unexpected-data }`; the running status
//!   is not known at the start of the samples or after a gap, so data bytes before the first
//!   status byte there are reported so as well;
//! - a system exclusive message, 0xF0, carries data bytes until it ends: they go out in batches
//!   of at most 32 bytes, each as soon as it is full and the rest when the message ends, and
//!   `sysex-end` gives their number;
//! - a gap or the end of the samples ends what is under way: a message still missing data
//!   bytes is `error { aborted }`, a system exclusive message ends `aborted`. A gap clears the
//!   running status;
//! - characters are taken as the UART read them: framing errors are not judged here, and
//!   breaks, which carry no byte, are passed over;
//! - a message spans its bytes, from its status byte — or its first data byte, under running
//!   status — to its last data byte, and settles with the last. A system exclusive message's
//!   first batch spans from the start of exclusive, and its `sysex-end` spans the end of
//!   exclusive, or, when another status byte, a gap or the end of the samples ends it, is placed
//!   where that began, settled where the end was decided; it covers the start of exclusive too
//!   when no batch did. A message cut short is reported over its bytes and settled where it was
//!   cut;
//! - progress is passed on, held back to the start of the message under way, or of the system
//!   exclusive bytes not reported yet.
//!
//! Limits: messages are not interpreted — neither note on with velocity 0, nor channel mode
//! messages, MIDI time code, manufacturers' identifiers or universal system exclusive messages;
//! the active sensing timeout is not checked; a system reset is a real-time message like the
//! others and does not clear the running status. One decoder reads one UART stream: one MIDI
//! cable, in one direction.

use uiharu_core::{
    Inbox, InputRef, InputSpec, Io, Item, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Props, Provenance, Requires, Tick, TickRange,
};

use super::events::{
    ChannelStatus, ChannelStatusExt, CommonStatus, CommonStatusExt, EndReason, ErrorReason,
    MessageId, MidiEvent, RealtimeStatus, RealtimeStatusExt, SYSEX_BATCH,
};
use crate::uart::UartEvent;

/// The start of exclusive, which begins a system exclusive message.
const SOX: u8 = 0xf0;
/// The end of exclusive, which ends it.
const EOX: u8 = 0xf7;

/// The MIDI decoder node: `uart.events` in, `midi.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Midi;

impl Midi {
    /// MIDI's bit rate: 31.25 kbaud.
    pub const BAUD: u32 = 31_250;
    /// How far from [`Midi::BAUD`] a stream may be declared: the specification's ±1 %.
    pub const BAUD_TOLERANCE: u32 = Self::BAUD / 100;

    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The UART input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<UartEvent> {
        InputRef::of_node(node, 0)
    }

    /// The message output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<MidiEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the UART stream it reads: MIDI's characters, at 31.25 kbaud,
    /// of 8 data bits least significant bit first, no parity and one stop bit.
    pub fn requires() -> Requires {
        Requires::new()
            .within(
                "baud",
                i64::from(Self::BAUD - Self::BAUD_TOLERANCE),
                i64::from(Self::BAUD + Self::BAUD_TOLERANCE),
            )
            .equals("dataBits", 8)
            .equals("parity", "none")
            .equals("stopBits", "one")
            .equals("bitOrder", "lsb-first")
    }
}

impl Node for Midi {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "midi".into(),
            inputs: vec![InputSpec::new::<UartEvent>("uart", Self::requires())],
            outputs: vec![OutputSpec::new::<MidiEvent>("events", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.input(0), Messages::new(io.output(0))))
    }
}

/// A character as the UART stream carried it.
#[derive(Clone, Copy, Debug)]
struct Byte {
    value: u8,
    at: Provenance,
}

/// What a message is, and so how many data bytes it waits for.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Channel(ChannelStatus),
    Common(CommonStatus),
}

/// A channel or system common message waiting for its data bytes.
#[derive(Clone, Copy, Debug)]
struct Pending {
    /// Its status byte, or the running status it repeats.
    status: u8,
    kind: Kind,
    running: bool,
    /// The data bytes so far, of the two at most a message takes.
    data: [u8; 2],
    count: u8,
    /// Its bytes so far.
    at: Provenance,
}

impl Pending {
    /// A message whose first byte, its status byte or its first data byte, is at `at`.
    fn new(status: u8, kind: Kind, running: bool, at: Provenance) -> Self {
        Self {
            status,
            kind,
            running,
            data: [0; 2],
            count: 0,
            at,
        }
    }

    fn push(&mut self, byte: Byte) {
        if let Some(slot) = self.data.get_mut(usize::from(self.count)) {
            *slot = byte.value;
        }
        self.count = self.count.saturating_add(1);
        self.at = self.at.join(byte.at);
    }

    fn complete(&self) -> bool {
        let wanted = match self.kind {
            Kind::Channel(status) => status.data_bytes(),
            Kind::Common(status) => status.data_bytes(),
        };
        self.count >= wanted
    }
}

/// A system exclusive message under way.
#[derive(Clone, Copy, Debug)]
struct Exclusive {
    message: MessageId,
    /// The start of exclusive, until a batch covers it.
    start: Option<Provenance>,
    /// The data bytes so far.
    count: u32,
}

/// What is under way.
#[derive(Clone, Copy, Debug)]
enum Under {
    Nothing,
    Message(Pending),
    Exclusive(Exclusive),
}

/// The decoder's state and its typed writer.
#[derive(Debug)]
struct Messages {
    out: Outbox<MidiEvent>,
    next_id: u64,
    /// The running status: the last channel status byte and its message, until a system common
    /// or exclusive status byte or a gap clears it.
    running: Option<(u8, ChannelStatus)>,
    under: Under,
    /// System exclusive data not reported yet, at most [`SYSEX_BATCH`] bytes.
    batch: Vec<u8>,
    batch_at: Option<Provenance>,
    batch_offset: u32,
}

impl Messages {
    fn new(out: Outbox<MidiEvent>) -> Self {
        Self {
            out,
            next_id: 1,
            running: None,
            under: Under::Nothing,
            batch: Vec::with_capacity(SYSEX_BATCH),
            batch_at: None,
            batch_offset: 0,
        }
    }

    fn next_message(&mut self) -> MessageId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Where the oldest byte that is not reported yet begins.
    fn unreported_from(&self) -> Option<Tick> {
        match self.under {
            Under::Nothing => None,
            Under::Message(pending) => Some(pending.at.span.start),
            Under::Exclusive(exclusive) => {
                exclusive.start.or(self.batch_at).map(|at| at.span.start)
            }
        }
    }

    async fn error(&mut self, at: Provenance, reason: ErrorReason, byte: u8) {
        self.out.emit(at, MidiEvent::Error { reason, byte }).await;
    }

    /// A character from the UART stream.
    async fn byte(&mut self, byte: Byte) {
        let value = byte.value;
        if value < 0x80 {
            self.data(byte).await;
        } else if let Some(status) = RealtimeStatus::from_status(value) {
            // Real-time messages come anywhere and change nothing else.
            let event = MidiEvent::Realtime {
                status,
                byte: value,
            };
            self.out.emit(byte.at, event).await;
        } else if !self.interrupt(byte).await {
            self.status(byte).await;
        }
    }

    /// Ends what is under way at a status byte other than real-time; whether the byte, an end
    /// of exclusive, was what ended a system exclusive message.
    async fn interrupt(&mut self, byte: Byte) -> bool {
        match std::mem::replace(&mut self.under, Under::Nothing) {
            Under::Nothing => false,
            Under::Message(pending) => {
                let at = Provenance::over(pending.at.span, byte.at.settled);
                self.error(at, ErrorReason::Incomplete, pending.status)
                    .await;
                false
            }
            Under::Exclusive(exclusive) if byte.value == EOX => {
                self.end_exclusive(exclusive, byte.at, EndReason::EndOfExclusive)
                    .await;
                true
            }
            Under::Exclusive(exclusive) => {
                let at = Provenance::over(TickRange::at(byte.at.span.start), byte.at.settled);
                self.end_exclusive(exclusive, at, EndReason::Interrupted)
                    .await;
                false
            }
        }
    }

    /// A status byte other than real-time, once what was under way has ended.
    async fn status(&mut self, byte: Byte) {
        let value = byte.value;
        if let Some(status) = ChannelStatus::from_status(value) {
            self.running = Some((value, status));
            self.under = Under::Message(Pending::new(value, Kind::Channel(status), false, byte.at));
            return;
        }
        // System common and exclusive status bytes clear the running status.
        self.running = None;
        if let Some(status) = CommonStatus::from_status(value) {
            let pending = Pending::new(value, Kind::Common(status), false, byte.at);
            self.complete_or_wait(pending).await;
        } else if value == SOX {
            let message = self.next_message();
            self.under = Under::Exclusive(Exclusive {
                message,
                start: Some(byte.at),
                count: 0,
            });
        } else if value == EOX {
            self.error(byte.at, ErrorReason::UnexpectedEndOfExclusive, value)
                .await;
        } else {
            // 0xF4 and 0xF5.
            self.error(byte.at, ErrorReason::UndefinedStatus, value)
                .await;
        }
    }

    /// A data byte.
    async fn data(&mut self, byte: Byte) {
        match self.under {
            Under::Exclusive(mut exclusive) => {
                self.push_exclusive(&mut exclusive, byte);
                self.under = Under::Exclusive(exclusive);
                if self.batch.len() >= SYSEX_BATCH {
                    self.flush(exclusive.message, byte.at.settled).await;
                }
            }
            Under::Message(mut pending) => {
                pending.push(byte);
                self.complete_or_wait(pending).await;
            }
            Under::Nothing => match self.running {
                Some((status, kind)) => {
                    let mut pending = Pending::new(status, Kind::Channel(kind), true, byte.at);
                    pending.push(byte);
                    self.complete_or_wait(pending).await;
                }
                None => {
                    self.error(byte.at, ErrorReason::UnexpectedData, byte.value)
                        .await;
                }
            },
        }
    }

    /// Sends `pending` if it has all its data bytes, or waits for the rest.
    async fn complete_or_wait(&mut self, pending: Pending) {
        if !pending.complete() {
            self.under = Under::Message(pending);
            return;
        }
        self.under = Under::Nothing;
        let message = self.next_message();
        let [data1, data2] = pending.data;
        let event = match pending.kind {
            Kind::Channel(status) => MidiEvent::ChannelMessage {
                message,
                channel: pending.status & 0x0f,
                status,
                data1,
                data2: (status.data_bytes() == 2).then_some(data2),
                running: pending.running,
            },
            Kind::Common(status) => MidiEvent::SystemCommon {
                message,
                status,
                data1: (status.data_bytes() >= 1).then_some(data1),
                data2: (status.data_bytes() == 2).then_some(data2),
            },
        };
        self.out.emit(pending.at, event).await;
    }

    /// Adds a data byte to the system exclusive message's batch.
    fn push_exclusive(&mut self, exclusive: &mut Exclusive, byte: Byte) {
        let at = match self.batch_at {
            Some(at) => at.join(byte.at),
            None => {
                self.batch_offset = exclusive.count;
                // The first batch covers the start of exclusive.
                exclusive
                    .start
                    .take()
                    .map_or(byte.at, |start| start.join(byte.at))
            }
        };
        self.batch_at = Some(at);
        self.batch.push(byte.value);
        exclusive.count = exclusive.count.saturating_add(1);
    }

    /// Sends the batch, if any, settled no earlier than `decided`.
    async fn flush(&mut self, message: MessageId, decided: Tick) {
        let Some(mut at) = self.batch_at.take() else {
            return;
        };
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(SYSEX_BATCH));
        at.settled = at.settled.max(decided);
        let offset = self.batch_offset;
        self.out
            .emit(
                at,
                MidiEvent::Sysex {
                    message,
                    offset,
                    bytes,
                },
            )
            .await;
    }

    /// Ends the system exclusive message at `at`: the rest of its data goes out, then its end.
    async fn end_exclusive(&mut self, mut exclusive: Exclusive, at: Provenance, reason: EndReason) {
        self.flush(exclusive.message, at.settled).await;
        let at = match exclusive.start.take() {
            // No batch covered the start of exclusive: the end does.
            Some(start) => Provenance::over(start.span.join(at.span), at.settled),
            None => at,
        };
        let end = MidiEvent::SysexEnd {
            message: exclusive.message,
            bytes: exclusive.count,
            reason,
        };
        self.out.emit(at, end).await;
    }

    /// Ends what is under way at `tick`, where a gap begins or the samples end.
    async fn abort(&mut self, tick: Tick) {
        match std::mem::replace(&mut self.under, Under::Nothing) {
            Under::Nothing => {}
            Under::Message(pending) => {
                let at = Provenance::over(pending.at.span, tick);
                self.error(at, ErrorReason::Aborted, pending.status).await;
            }
            Under::Exclusive(exclusive) => {
                self.end_exclusive(exclusive, Provenance::at(tick), EndReason::Aborted)
                    .await;
            }
        }
    }

    /// Progress of the UART stream: nothing comes before `tick`.
    async fn progress(&mut self, tick: Tick) {
        let tick = self.unreported_from().map_or(tick, |from| from.min(tick));
        self.out.progress(tick).await;
    }

    async fn gap(&mut self, range: TickRange) {
        self.abort(range.start).await;
        // Whatever status the lost samples carried is not known.
        self.running = None;
        self.out.gap(range).await;
    }

    async fn end(&mut self, tick: Tick) {
        self.abort(tick).await;
        self.out.end(tick).await;
    }
}

async fn run(mut input: Inbox<UartEvent>, mut messages: Messages) {
    loop {
        match input.recv().await {
            Item::Event(event) => {
                // A break carries no byte.
                if let UartEvent::Frame { value, .. } = event.payload {
                    // The requirements make every character 8 bits wide.
                    let [value, _] = value.to_le_bytes();
                    let at = Provenance::over(event.span, event.settled);
                    messages.byte(Byte { value, at }).await;
                }
            }
            Item::Progress(tick) => messages.progress(tick).await,
            Item::Gap(range) => messages.gap(range).await,
            Item::End(tick) => {
                messages.end(tick).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_midi_characters() {
        let uart = |baud: i64, data_bits: i64, parity: &str, stop_bits: &str, order: &str| {
            Props::new()
                .with("baud", baud)
                .with("dataBits", data_bits)
                .with("parity", parity)
                .with("stopBits", stop_bits)
                .with("bitOrder", order)
        };
        let requires = Midi::requires();
        let unmet = |props: &Props| requires.unmet(props).map(|(key, _)| key);
        assert_eq!(unmet(&uart(31_250, 8, "none", "one", "lsb-first")), None);
        assert_eq!(unmet(&uart(31_562, 8, "none", "one", "lsb-first")), None);
        assert_eq!(unmet(&uart(30_938, 8, "none", "one", "lsb-first")), None);
        assert_eq!(
            unmet(&uart(31_563, 8, "none", "one", "lsb-first")),
            Some("baud")
        );
        assert_eq!(
            unmet(&uart(30_937, 8, "none", "one", "lsb-first")),
            Some("baud")
        );
        assert_eq!(
            unmet(&uart(31_250, 7, "none", "one", "lsb-first")),
            Some("dataBits")
        );
        assert_eq!(
            unmet(&uart(31_250, 8, "odd", "one", "lsb-first")),
            Some("parity")
        );
        assert_eq!(
            unmet(&uart(31_250, 8, "none", "two", "lsb-first")),
            Some("stopBits")
        );
        assert_eq!(
            unmet(&uart(31_250, 8, "none", "one", "msb-first")),
            Some("bitOrder")
        );
    }

    #[test]
    fn messages_wait_for_their_data_bytes() {
        let byte = |value: u8, tick: u64| Byte {
            value,
            at: Provenance::over(TickRange::new(Tick(tick), Tick(tick + 10)), Tick(tick + 9)),
        };
        let mut pending = Pending::new(
            0x93,
            Kind::Channel(ChannelStatus::NoteOn),
            false,
            byte(0x93, 0).at,
        );
        assert!(!pending.complete());
        pending.push(byte(0x3c, 10));
        assert!(!pending.complete());
        pending.push(byte(0x40, 20));
        assert!(pending.complete());
        assert_eq!(pending.data, [0x3c, 0x40]);
        assert_eq!(pending.at.span, TickRange::new(Tick(0), Tick(30)));
        assert_eq!(pending.at.settled, Tick(29));
        let tune = Pending::new(
            0xf6,
            Kind::Common(CommonStatus::TuneRequest),
            false,
            byte(0xf6, 0).at,
        );
        assert!(tune.complete());
    }
}
