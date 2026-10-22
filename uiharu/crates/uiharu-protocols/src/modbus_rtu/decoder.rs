// SPDX-License-Identifier: Apache-2.0

//! The Modbus RTU decoder: the frames of a Modbus serial line in RTU mode, read from the
//! characters of a UART stream.
//!
//! Implemented from the MODBUS over Serial Line Specification and Implementation Guide V1.02
//! (§2.5.1, RTU transmission mode: the framing by silences of 1.5 and 3.5 character times, their
//! fixed values above 19200 baud, the CRC check; the CRC generation in its appendix) and the
//! MODBUS Application Protocol Specification V1.1b3 (§4.1, a PDU is a function code and its data;
//! §7, exception responses).
//!
//! The rules:
//!
//! - a character is 11 bits on the line, a start bit, 8 data bits, a parity bit or a second stop
//!   bit, and a stop bit, so a character time is 11 bit times at the baud rate the UART stream
//!   declares. Up to 19200 baud the inter-character limit t1.5 is 1.5 character times and the
//!   inter-frame silence t3.5 is 3.5; above 19200 baud they are fixed at 750 µs and 1.75 ms, the
//!   values the specification recommends there;
//! - both are turned into ticks of the source's clock and rounded up, so that comparing a silence
//!   of whole ticks with them is exact;
//! - the silence before a character runs from the end of the character before it, the end of its
//!   stop bits, to its start edge;
//! - a frame is a run of characters; a silence of t3.5 or more ends it, and the character after
//!   that silence begins the next frame;
//! - a character after a silence of t1.5 or more but less than t3.5 still belongs to the frame,
//!   which it breaks, as a receiver declares such a frame incomplete; a break on the line inside
//!   a frame breaks it too, and silences are then measured from the end of the break;
//! - a frame ends once its silence is known to have passed, when the next character arrives after
//!   it or when progress of the UART stream reaches `end of the last character + t3.5`; its `end`
//!   is placed at that computed tick with an empty span whichever told the decoder, so that the
//!   result does not depend on how the samples were cut;
//! - a gap or the end of the samples before that silence was seen aborts the frame, its `end`
//!   placed at the gap's start or at the end. The UART stream announces progress up to a gap or
//!   the end before passing it on, so a frame whose silence had passed by then ends as usual;
//! - the first byte is the address, the second the function code, an exception response when its
//!   top bit is set; the last two are the CRC, and the bytes in between are data. Which two bytes
//!   are the CRC is only known when the frame ends, so the decoder holds the last two back: data
//!   goes out in batches of at most [`DATA_BATCH`] bytes, a batch as soon as it is full and the
//!   two bytes after it have arrived, the rest when the frame ends;
//! - the CRC, sent low byte first, is reported for every frame of four bytes or more that was not
//!   aborted, and compared with the CRC-16 of the bytes before it;
//! - a frame ends `broken` as above, otherwise `short` with fewer than four bytes, otherwise
//!   `bad-crc` or `complete`. An aborted frame reports all its bytes as data and no CRC. A frame
//!   of one byte has no function code, and its `begin` goes out when it ends;
//! - a frame is partial when its first character started less than t3.5 after the samples began,
//!   taken to be tick 0 where a source counts from, or resumed after a gap: earlier bytes of it
//!   may have gone unseen;
//! - characters are taken as the UART read them: their parity and framing errors are not judged
//!   here, the CRC covering the frame, and breaks only count for their timing;
//! - progress is passed on, but never past a byte the decoder still holds unreported.
//!
//! Limits: the PDU is not interpreted, neither function codes nor exception codes; the address
//! is not checked against the ranges the specification assigns; frames are not cut at the 256
//! bytes the specification allows. One decoder reads one UART stream: on a two-wire line requests
//! and responses share it, separate receive and transmit lines take a decoder each.

use uiharu_core::{
    Inbox, InputRef, InputSpec, Io, Item, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, PropValue, Props, Provenance, Requires, Tick, TickRange,
};

use super::crc::{INITIAL, crc16_update};
use super::events::{DATA_BATCH, EndReason, FrameId, ModbusEvent};
use crate::uart::UartEvent;

/// The silences of RTU framing, in ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Silences {
    /// t1.5: a silence at least this long between two characters of a frame breaks it.
    pub inter_character: u64,
    /// t3.5: a silence at least this long ends a frame.
    pub inter_frame: u64,
}

impl Silences {
    /// The baud rate above which the silences are fixed instead of counted in characters.
    pub const FIXED_ABOVE_BAUD: u64 = 19_200;

    /// The silences of a line at `baud` on a source of `rate_hz` samples per second, rounded up
    /// to whole ticks.
    pub fn new(rate_hz: u64, baud: u64) -> Self {
        let rate = u128::from(rate_hz);
        let (inter_character, inter_frame) = if baud > Self::FIXED_ABOVE_BAUD {
            // 750 µs and 1.75 ms.
            (
                (rate * 750).div_ceil(1_000_000),
                (rate * 1_750).div_ceil(1_000_000),
            )
        } else {
            // 1.5 and 3.5 characters of 11 bits: 16.5 and 38.5 bit times.
            let baud = u128::from(baud.max(1));
            (
                (rate * 33).div_ceil(2 * baud),
                (rate * 77).div_ceil(2 * baud),
            )
        };
        Self {
            inter_character: u64::try_from(inter_character).unwrap_or(u64::MAX),
            inter_frame: u64::try_from(inter_frame).unwrap_or(u64::MAX),
        }
    }
}

/// The Modbus RTU decoder node: `uart.events` in, `modbus.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModbusRtu;

impl ModbusRtu {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The UART input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<UartEvent> {
        InputRef::of_node(node, 0)
    }

    /// The frame output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<ModbusEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the UART stream it reads: characters of 8 data bits, least
    /// significant bit first, as RTU mode sends them, at a declared baud rate.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("dataBits", 8)
            .equals("bitOrder", "lsb-first")
            .at_least("baud", 1)
    }
}

impl Node for ModbusRtu {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "modbus-rtu".into(),
            inputs: vec![InputSpec::new::<UartEvent>("uart", Self::requires())],
            outputs: vec![OutputSpec::new::<ModbusEvent>(
                "events",
                Props::new().with("mode", "rtu"),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz);
        let baud = match io.input_props(0).get("baud") {
            Some(PropValue::Int(baud)) => u64::try_from(*baud).unwrap_or(0),
            _ => 0,
        };
        let frames = Frames::new(io.output(0), Silences::new(rate, baud));
        Box::pin(run(io.input(0), frames))
    }
}

/// A byte as the UART stream carried it.
#[derive(Clone, Copy, Debug)]
struct Byte {
    value: u8,
    at: Provenance,
}

/// The last two bytes of the frame so far: its CRC, unless more bytes follow.
#[derive(Clone, Copy, Debug, Default)]
struct Held([Option<Byte>; 2]);

impl Held {
    /// Holds `byte`; once two are held, the older one comes back out.
    fn push(&mut self, byte: Byte) -> Option<Byte> {
        match self.0 {
            [None, _] => {
                self.0[0] = Some(byte);
                None
            }
            [Some(_), None] => {
                self.0[1] = Some(byte);
                None
            }
            [Some(older), Some(newer)] => {
                self.0 = [Some(newer), Some(byte)];
                Some(older)
            }
        }
    }

    /// The two bytes held, the older first, once there are two.
    fn pair(&self) -> Option<(Byte, Byte)> {
        match self.0 {
            [Some(older), Some(newer)] => Some((older, newer)),
            _ => None,
        }
    }

    /// The bytes held, the older first.
    fn bytes(&self) -> impl Iterator<Item = Byte> + use<> {
        self.0.into_iter().flatten()
    }
}

/// The frame under way.
#[derive(Debug)]
struct Open {
    id: FrameId,
    partial: bool,
    /// The first byte, until the second arrives and the frame's `begin` goes out.
    address: Option<Byte>,
    /// The bytes so far.
    count: u32,
    /// Where the last character, or break, ended: silences are measured from here.
    last_end: Tick,
    broken: bool,
    /// The CRC of the bytes known not to be the frame's CRC.
    crc: u16,
    held: Held,
    /// The offset of the next data byte.
    data: u32,
}

/// The decoder's state and its typed writer.
#[derive(Debug)]
struct Frames {
    out: Outbox<ModbusEvent>,
    silences: Silences,
    next_id: u64,
    open: Option<Open>,
    /// Where the samples began or resumed after a gap, until a frame begins after it.
    resumed: Option<Tick>,
    /// Data bytes not reported yet, at most [`DATA_BATCH`].
    batch: Vec<u8>,
    batch_at: Option<Provenance>,
    batch_offset: u32,
}

impl Frames {
    fn new(out: Outbox<ModbusEvent>, silences: Silences) -> Self {
        Self {
            out,
            silences,
            next_id: 1,
            open: None,
            resumed: Some(Tick::ZERO),
            batch: Vec::with_capacity(DATA_BATCH),
            batch_at: None,
            batch_offset: 0,
        }
    }

    /// Where the open frame's silence passes.
    fn silence_end(&self) -> Option<Tick> {
        let open = self.open.as_ref()?;
        Some(open.last_end.offset(self.silences.inter_frame))
    }

    /// Where the oldest byte not reported yet begins.
    fn unreported_from(&self) -> Option<Tick> {
        let open = self.open.as_ref()?;
        let address = open.address.map(|byte| byte.at.span.start);
        let batch = self.batch_at.map(|at| at.span.start);
        let held = open.held.bytes().next().map(|byte| byte.at.span.start);
        address.or(batch).or(held)
    }

    /// A character from the UART stream.
    async fn character(&mut self, byte: Byte) {
        if let Some(end) = self.silence_end() {
            if byte.at.span.start < end {
                self.add(byte).await;
                return;
            }
            self.close(end).await;
        }
        self.open_frame(byte);
    }

    /// Begins a frame with its first byte; nothing goes out before the second.
    fn open_frame(&mut self, byte: Byte) {
        let partial = self
            .resumed
            .take()
            .is_some_and(|resumed| resumed.until(byte.at.span.start) < self.silences.inter_frame);
        let id = self.next_id;
        self.next_id += 1;
        self.open = Some(Open {
            id,
            partial,
            address: Some(byte),
            count: 1,
            last_end: byte.at.span.end,
            broken: false,
            crc: crc16_update(INITIAL, byte.value),
            held: Held::default(),
            data: 0,
        });
    }

    /// Adds a byte that came before the open frame's silence passed.
    async fn add(&mut self, byte: Byte) {
        let inter_character = self.silences.inter_character;
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.last_end.until(byte.at.span.start) >= inter_character {
            open.broken = true;
        }
        open.last_end = open.last_end.max(byte.at.span.end);
        open.count = open.count.saturating_add(1);
        if let Some(address) = open.address.take() {
            // The function code: the frame's beginning is known.
            open.crc = crc16_update(open.crc, byte.value);
            let begin = ModbusEvent::Begin {
                frame: open.id,
                address: address.value,
                function: Some(byte.value),
                exception: byte.value & 0x80 != 0,
                partial: open.partial,
            };
            self.out.emit(address.at.join(byte.at), begin).await;
            return;
        }
        // The byte this one pushes out of the last two is data: it is not the CRC.
        if let Some(data) = open.held.push(byte) {
            open.crc = crc16_update(open.crc, data.value);
            let (frame, offset) = (open.id, open.data);
            open.data = open.data.saturating_add(1);
            self.data(frame, offset, data, byte.at.settled).await;
        }
    }

    /// Adds a data byte to the batch, which goes out once full; `decided` is the tick at which
    /// the byte became known to be data.
    async fn data(&mut self, frame: FrameId, offset: u32, byte: Byte, decided: Tick) {
        match self.batch_at {
            Some(at) if !self.batch.is_empty() => self.batch_at = Some(at.join(byte.at)),
            _ => {
                self.batch_offset = offset;
                self.batch_at = Some(byte.at);
            }
        }
        self.batch.push(byte.value);
        if self.batch.len() >= DATA_BATCH {
            self.flush(frame, decided).await;
        }
    }

    /// Sends the batch, if any, settled no earlier than `decided`.
    async fn flush(&mut self, frame: FrameId, decided: Tick) {
        if self.batch.is_empty() {
            return;
        }
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BATCH));
        let mut at = self.batch_at.take().unwrap_or(Provenance::at(decided));
        at.settled = at.settled.max(decided);
        let offset = self.batch_offset;
        self.out
            .emit(
                at,
                ModbusEvent::Data {
                    frame,
                    offset,
                    bytes,
                },
            )
            .await;
    }

    /// The `begin` of a frame of a single byte, which has no function code.
    async fn late_begin(&mut self, open: &Open, settled: Tick) {
        if let Some(address) = open.address {
            let begin = ModbusEvent::Begin {
                frame: open.id,
                address: address.value,
                function: None,
                exception: false,
                partial: open.partial,
            };
            self.out
                .emit(Provenance::over(address.at.span, settled), begin)
                .await;
        }
    }

    /// Reports the bytes `open` still holds as data, decided at `decided`, and the batch.
    async fn release_held(&mut self, open: &Open, decided: Tick) {
        let mut offset = open.data;
        for byte in open.held.bytes() {
            self.data(open.id, offset, byte, decided).await;
            offset = offset.saturating_add(1);
        }
        self.flush(open.id, decided).await;
    }

    /// Ends the open frame at `end`, where its silence passed.
    async fn close(&mut self, end: Tick) {
        let Some(open) = self.open.take() else {
            return;
        };
        self.late_begin(&open, end).await;
        let crc = if open.count >= 4 {
            open.held.pair()
        } else {
            None
        };
        let reason = match crc {
            Some((low, high)) => {
                self.flush(open.id, end).await;
                let value = u16::from_le_bytes([low.value, high.value]);
                let ok = value == open.crc;
                let at = Provenance::over(low.at.span.join(high.at.span), end);
                let crc = ModbusEvent::Crc {
                    frame: open.id,
                    value,
                    ok,
                };
                self.out.emit(at, crc).await;
                if ok {
                    EndReason::Complete
                } else {
                    EndReason::BadCrc
                }
            }
            None => {
                self.release_held(&open, end).await;
                EndReason::Short
            }
        };
        let reason = if open.broken {
            EndReason::Broken
        } else {
            reason
        };
        let end_event = ModbusEvent::End {
            frame: open.id,
            reason,
        };
        self.out.emit(Provenance::at(end), end_event).await;
    }

    /// Ends the open frame at `at` before its silence was seen.
    async fn abort(&mut self, at: Tick) {
        let Some(open) = self.open.take() else {
            return;
        };
        self.late_begin(&open, at).await;
        self.release_held(&open, at).await;
        let end = ModbusEvent::End {
            frame: open.id,
            reason: EndReason::Aborted,
        };
        self.out.emit(Provenance::at(at), end).await;
    }

    /// A break on the line.
    async fn line_break(&mut self, at: Provenance) {
        let Some(end) = self.silence_end() else {
            return;
        };
        if at.span.start >= end {
            self.close(end).await;
        } else if let Some(open) = self.open.as_mut() {
            open.broken = true;
            open.last_end = open.last_end.max(at.span.end);
        }
    }

    /// Progress of the UART stream: nothing comes before `tick`.
    async fn progress(&mut self, tick: Tick) {
        if let Some(end) = self.silence_end()
            && tick >= end
        {
            self.close(end).await;
        }
        let tick = self.unreported_from().map_or(tick, |from| from.min(tick));
        self.out.progress(tick).await;
    }

    async fn gap(&mut self, range: TickRange) {
        self.abort(range.start).await;
        self.resumed = Some(range.end);
        self.out.gap(range).await;
    }

    async fn end(&mut self, tick: Tick) {
        self.abort(tick).await;
        self.out.end(tick).await;
    }
}

async fn run(mut input: Inbox<UartEvent>, mut frames: Frames) {
    loop {
        match input.recv().await {
            Item::Event(event) => {
                let at = Provenance::over(event.span, event.settled);
                match event.payload {
                    UartEvent::Frame { value, .. } => {
                        // The requirements make every character 8 bits wide.
                        let [value, _] = value.to_le_bytes();
                        frames.character(Byte { value, at }).await;
                    }
                    UartEvent::Break { .. } => frames.line_break(at).await,
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

    #[test]
    fn silences_count_characters_up_to_19200_baud_and_are_fixed_above() {
        // Ten ticks per bit: 1.5 and 3.5 characters of 11 bits are 165 and 385 ticks.
        let silences = Silences::new(96_000, 9_600);
        assert_eq!((silences.inter_character, silences.inter_frame), (165, 385));
        assert_eq!(
            Silences::new(192_000, 19_200),
            Silences {
                inter_character: 165,
                inter_frame: 385
            }
        );
        // Above 19200 baud: 750 µs and 1.75 ms, whatever the baud rate.
        for baud in [19_201, 115_200, 1_000_000] {
            assert_eq!(
                Silences::new(1_000_000, baud),
                Silences {
                    inter_character: 750,
                    inter_frame: 1_750
                }
            );
        }
        // Rounded up: at 1 MHz and 9600 baud 3.5 characters are 4010.4 ticks.
        assert_eq!(Silences::new(1_000_000, 9_600).inter_frame, 4_011);
        assert_eq!(Silences::new(1_000_000, 9_600).inter_character, 1_719);
        assert_eq!(Silences::new(33_333, 115_200).inter_frame, 59);
    }

    #[test]
    fn the_last_two_bytes_are_held_back() {
        let byte = |value| Byte {
            value,
            at: Provenance::at(Tick(u64::from(value))),
        };
        let mut held = Held::default();
        assert!(held.push(byte(1)).is_none());
        assert!(held.pair().is_none());
        assert!(held.push(byte(2)).is_none());
        assert_eq!(held.push(byte(3)).map(|byte| byte.value), Some(1));
        assert_eq!(
            held.pair().map(|(older, newer)| (older.value, newer.value)),
            Some((2, 3))
        );
        assert_eq!(
            held.bytes().map(|byte| byte.value).collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn requires_eight_bit_characters_least_significant_bit_first() {
        let uart = |data_bits: i64, order: &str| {
            Props::new()
                .with("baud", 9_600)
                .with("dataBits", data_bits)
                .with("bitOrder", order)
        };
        let requires = ModbusRtu::requires();
        assert!(requires.unmet(&uart(8, "lsb-first")).is_none());
        assert_eq!(
            requires.unmet(&uart(7, "lsb-first")).map(|(key, _)| key),
            Some("dataBits")
        );
        assert_eq!(
            requires.unmet(&uart(8, "msb-first")).map(|(key, _)| key),
            Some("bitOrder")
        );
        assert_eq!(
            requires
                .unmet(
                    &Props::new()
                        .with("dataBits", 8)
                        .with("bitOrder", "lsb-first")
                )
                .map(|(key, _)| key),
            Some("baud")
        );
    }
}
