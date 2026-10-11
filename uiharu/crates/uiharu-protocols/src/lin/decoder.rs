// SPDX-License-Identifier: Apache-2.0

//! The LIN decoder: the frames of a LIN bus, read from the characters and breaks of a UART
//! stream.
//!
//! Implemented from the LIN Specification Package Revision 2.2A (LIN Consortium, 2010; LIN is
//! now standardized as ISO 17987), Protocol Specification: §2.3.1 (the frame: break field, sync
//! byte field, protected identifier field, data and checksum, the byte field), §2.3.2 (a
//! receiving node accepts a frame up to the next break field) and §2.3.3.4 (the diagnostic
//! frames, which always carry the classic checksum); and from the LIN Specification Package
//! Revision 1.3, §3.1.2 (the data length coded in the identifier).
//!
//! The rules:
//!
//! - the UART stream carries LIN's byte field, a start bit, eight data bits least significant
//!   bit first and a stop bit, so the input requires 8 data bits, no parity, one stop bit and
//!   the least significant bit first; the bit time is taken from the stream's `baud` and the
//!   source's rate;
//! - a break field is a UART break at least `min_break_bits` nominal bit times long: 11 by
//!   default, the threshold LIN slaves detect a break with (§2.3.1.1), since a master sends at
//!   least 13 but one whose clock runs a little fast sends breaks that measure just under 13
//!   nominal bit times. That length is `⌊min_break_bits × rate / baud⌋` ticks, rounded down, so
//!   that a break of exactly that many bit times is taken whatever the phase of the samples;
//! - every break ends the frame under way, since the master begins each frame with one, and a
//!   break long enough begins the next frame; after a shorter one, characters are skipped up to
//!   the next break field;
//! - the character after the break field must be the sync byte, 0x55 without a framing error;
//!   anything else is `error { sync }`: the header is not recognized and the frame ends at once;
//! - the next character is the protected identifier, and the header goes out, spanning from the
//!   start of the break field to the end of the identifier and settled with the identifier;
//!   parity bits that do not match the frame identifier are `error { parity }`;
//! - every character after it is the response, up to the frame's end: the next break, a gap or
//!   the end of the samples, or, with `length_by_id`, the checksum that follows the number of
//!   data bytes bits 4 and 5 of the identifier code (2, 2, 4 or 8), after which characters are
//!   skipped up to the next break field;
//! - the response's last byte is its checksum, which is only known when the frame ends, so the
//!   decoder holds the last byte back. The data goes out in one batch of at most eight bytes, as
//!   soon as the eighth is known to be data, or when the frame ends; with `length_by_id` a byte
//!   is known to be data as it comes, and the batch goes out with the last of them;
//! - the checksum is compared with the classic checksum, the inverted eight-bit sum with carry
//!   of the data bytes, and with the enhanced one, which adds in the protected identifier.
//!   Matching neither, or, in a diagnostic frame (60 or 61), not matching the classic one, is
//!   `error { checksum }`;
//! - a response of a single byte has no room for a data byte and a checksum: the byte is
//!   reported as data, with `error { too-short }`; so is a response that ends before the length
//!   its identifier codes. A tenth byte, more than eight data bytes and a checksum, is
//!   `error { too-long }`, and the frame ends at once;
//! - a character of the identifier or the response with a framing error is `error { framing }`,
//!   and its value is taken as read;
//! - a frame ends `aborted` when a gap or the end of the samples cuts it, otherwise `error` when
//!   an error was reported for it, `no-response` when no byte followed its header, and
//!   `complete` when its data and checksum came without an error;
//! - the `end` of a frame that a break ends is placed at the start of the break and settled
//!   where the break was recognized, its end; an end decided by a character (the checksum with
//!   `length_by_id`, a wrong sync byte, a tenth byte) is placed at the end of that character; an
//!   aborted frame's at the gap's start or the end of the samples. Data, checksums and errors
//!   that the end decides settle with it;
//! - an aborted frame reports the bytes it holds as data and no checksum; a frame whose header
//!   had not gone out reports nothing. At the start of the samples and after a gap the decoder
//!   waits for a break field;
//! - progress is passed on, held back to the start of a break field whose header has not gone
//!   out, then to the first byte not reported, or, with `length_by_id`, to the start of the
//!   response, which a break before its checksum reports too short as a whole.
//!
//! Limits: the data length of a frame is not on the bus but in the cluster's description, so
//! without `length_by_id` a response ends at the next break field; signals are not interpreted;
//! the frame-time maximum of §2.3.2, `TFrame_Maximum`, is not checked; a wake-up signal long
//! enough to be a break begins a frame that reports nothing unless a byte follows it. One decoder
//! reads one UART stream: the master and the slaves share LIN's single wire.

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Inbox, InputRef, InputSpec, Io, Item, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, PropValue, Props, Provenance, Requires, Tick, TickRange,
};

use super::events::{DATA_BYTES, EndReason, ErrorReason, FrameId, LinEvent};
use super::fields::{SYNC, add_with_carry, data_length_by_id, frame_id, pid_ok};
use crate::uart::UartEvent;

/// The frame identifiers of the diagnostic frames, the master request frame and the slave
/// response frame, which always carry the classic checksum.
const DIAGNOSTIC: [u8; 2] = [0x3c, 0x3d];

/// How the decoder delimits frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LinConfig {
    /// End a frame with its checksum after the number of data bytes bits 4 and 5 of its
    /// identifier code, as LIN 1.x allowed — 2 for identifiers 0 to 31, 4 for 32 to 47, 8 for
    /// 48 to 63 — instead of at the next break field.
    pub length_by_id: bool,
    /// The shortest UART break taken as a break field, in nominal bit times, 9 to 13. The
    /// default, 11, is the threshold LIN slaves detect a break with: a master sends at least 13,
    /// but one whose clock runs a little fast sends breaks that measure just under 13 at the
    /// nominal bit time. 13 takes only breaks as long as a master must send them; 9 takes every
    /// break the UART reports, as receivers with an accurate clock may, from 9.5 bit times.
    pub min_break_bits: u8,
}

impl Default for LinConfig {
    fn default() -> Self {
        Self {
            length_by_id: false,
            min_break_bits: 11,
        }
    }
}

impl LinConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), LinConfigError> {
        if !(9..=13).contains(&self.min_break_bits) {
            return Err(LinConfigError::MinBreakBits(self.min_break_bits));
        }
        Ok(())
    }

    /// What the decoder's output port declares: whether frames end with the length their
    /// identifier codes.
    pub fn props(&self) -> Props {
        Props::new().with("lengthById", self.length_by_id)
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LinConfigError {
    /// The shortest break field is not 9 to 13 bit times.
    #[error("a break field of at least {0} bit times; 9 to 13 are supported")]
    MinBreakBits(u8),
}

/// The LIN decoder node: `uart.events` in, `lin.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lin {
    config: LinConfig,
}

impl Lin {
    /// A decoder for `config`.
    pub fn new(config: LinConfig) -> Result<Self, LinConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &LinConfig {
        &self.config
    }

    /// The UART input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<UartEvent> {
        InputRef::of_node(node, 0)
    }

    /// The frame output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<LinEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the UART stream it reads: LIN's byte field, 8 data bits
    /// least significant bit first, no parity and one stop bit, at a declared baud rate.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("dataBits", 8)
            .equals("parity", "none")
            .equals("stopBits", "one")
            .equals("bitOrder", "lsb-first")
            .at_least("baud", 1)
    }
}

impl Node for Lin {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "lin".into(),
            inputs: vec![InputSpec::new::<UartEvent>("uart", Self::requires())],
            outputs: vec![OutputSpec::new::<LinEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let rate = io.time_base().map_or(0, |base| base.rate_hz);
        let baud = match io.input_props(0).get("baud") {
            Some(PropValue::Int(baud)) => u64::try_from(*baud).unwrap_or(0),
            _ => 0,
        };
        let min_break = break_ticks(self.config.min_break_bits, rate, baud);
        let frames = Frames::new(io.output(0), self.config.length_by_id, min_break);
        Box::pin(run(io.input(0), frames))
    }
}

/// `bits` nominal bit times at `baud` in ticks of a source of `rate` samples per second,
/// rounded down.
fn break_ticks(bits: u8, rate: u64, baud: u64) -> u64 {
    let ticks = u128::from(bits) * u128::from(rate) / u128::from(baud.max(1));
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// A character as the UART stream carried it.
#[derive(Clone, Copy, Debug)]
struct Byte {
    value: u8,
    framing_ok: bool,
    at: Provenance,
}

/// A frame whose header has gone out.
#[derive(Clone, Copy, Debug)]
struct Response {
    frame: FrameId,
    pid: u8,
    /// The number of data bytes the identifier codes, with `length_by_id`.
    length: Option<u8>,
    /// Whether an error was reported for the frame.
    error: bool,
    /// The ticks of the response's bytes so far.
    received: Option<TickRange>,
    /// The data bytes so far, reported or not.
    data: u8,
    /// The eight-bit sum with carry of the data bytes so far.
    sum: u8,
    /// The last byte, held back: the checksum, should the frame end after it.
    held: Option<Byte>,
}

/// Where the decoder is.
#[derive(Clone, Copy, Debug)]
enum State {
    /// Outside frames: waiting for a break field.
    Idle,
    /// A break field began a frame; the sync byte comes next.
    Sync { brk: Provenance },
    /// The sync byte was right; the protected identifier comes next.
    Identifier { brk: Provenance },
    /// The header has gone out; the response comes until the frame ends.
    Response(Response),
}

/// The decoder's state and its typed writer.
#[derive(Debug)]
struct Frames {
    out: Outbox<LinEvent>,
    length_by_id: bool,
    /// The shortest break field, in ticks.
    min_break: u64,
    next_id: u64,
    state: State,
    /// Data bytes not reported yet, at most [`DATA_BYTES`].
    batch: Vec<u8>,
    batch_at: Option<Provenance>,
    batch_offset: u8,
}

impl Frames {
    fn new(out: Outbox<LinEvent>, length_by_id: bool, min_break: u64) -> Self {
        Self {
            out,
            length_by_id,
            min_break,
            next_id: 1,
            state: State::Idle,
            batch: Vec::with_capacity(DATA_BYTES),
            batch_at: None,
            batch_offset: 0,
        }
    }

    fn next_frame(&mut self) -> FrameId {
        let id = FrameId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Where the oldest part of the frame that may still be reported begins.
    fn unreported_from(&self) -> Option<Tick> {
        match self.state {
            State::Idle => None,
            State::Sync { brk } | State::Identifier { brk } => Some(brk.span.start),
            // A response that ends before its checksum is reported too short as a whole.
            State::Response(response) if response.length.is_some() => {
                response.received.map(|span| span.start)
            }
            State::Response(response) => self
                .batch_at
                .or(response.held.map(|byte| byte.at))
                .map(|at| at.span.start),
        }
    }

    async fn error(&mut self, frame: FrameId, at: Provenance, reason: ErrorReason) {
        self.out.emit(at, LinEvent::Error { frame, reason }).await;
    }

    async fn close(&mut self, frame: FrameId, at: Provenance, reason: EndReason) {
        self.out.emit(at, LinEvent::End { frame, reason }).await;
    }

    /// A break on the line, `ticks` long.
    async fn line_break(&mut self, at: Provenance, ticks: u64) {
        if let State::Response(response) = self.state {
            // The frame ends where the break begins, which is known once the break is.
            let end = Provenance::over(TickRange::at(at.span.start), at.settled);
            self.finish(response, end).await;
        }
        self.state = if ticks >= self.min_break {
            State::Sync { brk: at }
        } else {
            State::Idle
        };
    }

    /// A character.
    async fn character(&mut self, byte: Byte) {
        match self.state {
            State::Idle => {}
            State::Sync { brk } if byte.value == SYNC && byte.framing_ok => {
                self.state = State::Identifier { brk };
            }
            State::Sync { .. } => {
                // The header is not recognized: the frame ends here.
                self.state = State::Idle;
                let frame = self.next_frame();
                self.error(frame, byte.at, ErrorReason::Sync).await;
                let end = Provenance::at(byte.at.span.end);
                self.close(frame, end, EndReason::Error).await;
            }
            State::Identifier { brk } => self.header(brk, byte).await,
            State::Response(response) => self.response(response, byte).await,
        }
    }

    /// The protected identifier after the sync byte: the header goes out.
    async fn header(&mut self, brk: Provenance, byte: Byte) {
        let frame = self.next_frame();
        let pid = byte.value;
        let parity_ok = pid_ok(pid);
        let header = LinEvent::Header {
            frame,
            pid,
            id: frame_id(pid),
            pid_ok: parity_ok,
        };
        let at = Provenance::over(brk.span.join(byte.at.span), byte.at.settled);
        self.out.emit(at, header).await;
        if !byte.framing_ok {
            self.error(frame, byte.at, ErrorReason::Framing).await;
        }
        if !parity_ok {
            self.error(frame, byte.at, ErrorReason::Parity).await;
        }
        self.state = State::Response(Response {
            frame,
            pid,
            length: self.length_by_id.then(|| data_length_by_id(frame_id(pid))),
            error: !byte.framing_ok || !parity_ok,
            received: None,
            data: 0,
            sum: 0,
            held: None,
        });
    }

    /// A byte of the response.
    async fn response(&mut self, mut response: Response, byte: Byte) {
        let frame = response.frame;
        if !byte.framing_ok {
            response.error = true;
            self.error(frame, byte.at, ErrorReason::Framing).await;
        }
        response.received = Some(match response.received {
            Some(span) => span.join(byte.at.span),
            None => byte.at.span,
        });
        match response.length {
            Some(length) if response.data < length => {
                // The identifier says how many data bytes come, and this is one.
                self.push_data(&mut response, byte);
                if response.data == length {
                    self.flush(frame, byte.at.settled).await;
                }
                self.state = State::Response(response);
            }
            Some(_) => {
                // The checksum, and with it the frame's end.
                self.state = State::Idle;
                self.checksum(&mut response, byte, byte.at.settled).await;
                let end = Provenance::at(byte.at.span.end);
                self.close(frame, end, ended(&response)).await;
            }
            None => {
                if let Some(previous) = response.held.take() {
                    if usize::from(response.data) == DATA_BYTES {
                        // A tenth byte: more than eight data bytes and a checksum.
                        self.state = State::Idle;
                        let span = previous.at.span.join(byte.at.span);
                        let at = Provenance::over(span, byte.at.settled);
                        self.error(frame, at, ErrorReason::TooLong).await;
                        let end = Provenance::at(byte.at.span.end);
                        self.close(frame, end, EndReason::Error).await;
                        return;
                    }
                    self.push_data(&mut response, previous);
                    if usize::from(response.data) == DATA_BYTES {
                        self.flush(frame, byte.at.settled).await;
                    }
                }
                response.held = Some(byte);
                self.state = State::Response(response);
            }
        }
    }

    /// Adds a byte known to be data to the batch.
    fn push_data(&mut self, response: &mut Response, byte: Byte) {
        self.batch_at = Some(match self.batch_at {
            Some(at) => at.join(byte.at),
            None => {
                self.batch_offset = response.data;
                byte.at
            }
        });
        self.batch.push(byte.value);
        response.sum = add_with_carry(response.sum, byte.value);
        response.data = response.data.saturating_add(1);
    }

    /// Sends the batch, if any, settled no earlier than `decided`.
    async fn flush(&mut self, frame: FrameId, decided: Tick) {
        let Some(mut at) = self.batch_at.take() else {
            return;
        };
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BYTES));
        at.settled = at.settled.max(decided);
        let offset = self.batch_offset;
        self.out
            .emit(
                at,
                LinEvent::Data {
                    frame,
                    offset,
                    bytes,
                },
            )
            .await;
    }

    /// Reports `byte` as the frame's checksum, settled at `decided`, with an error when it is
    /// not the one the frame calls for.
    async fn checksum(&mut self, response: &mut Response, byte: Byte, decided: Tick) {
        let value = byte.value;
        let classic_ok = value == !response.sum;
        let enhanced_ok = value == !add_with_carry(response.sum, response.pid);
        let at = Provenance::over(byte.at.span, decided);
        let checksum = LinEvent::Checksum {
            frame: response.frame,
            value,
            classic_ok,
            enhanced_ok,
        };
        self.out.emit(at, checksum).await;
        let diagnostic = DIAGNOSTIC.contains(&frame_id(response.pid));
        if !classic_ok && (diagnostic || !enhanced_ok) {
            response.error = true;
            self.error(response.frame, at, ErrorReason::Checksum).await;
        }
    }

    /// Ends the frame at `end`, the start of a break, settled where the break was recognized.
    async fn finish(&mut self, mut response: Response, end: Provenance) {
        let frame = response.frame;
        let decided = end.settled;
        match response.held.take() {
            // Without a length, the last byte is the checksum, unless it is the only one.
            Some(last) if response.data > 0 => {
                self.flush(frame, decided).await;
                self.checksum(&mut response, last, decided).await;
            }
            Some(only) => {
                self.push_data(&mut response, only);
                self.flush(frame, decided).await;
                response.error = true;
                let at = Provenance::over(only.at.span, decided);
                self.error(frame, at, ErrorReason::TooShort).await;
            }
            // No byte held: no response at all, or, with a length, a response that the
            // checksum would have ended.
            None => {
                self.flush(frame, decided).await;
                if let Some(span) = response.received {
                    response.error = true;
                    let at = Provenance::over(span, decided);
                    self.error(frame, at, ErrorReason::TooShort).await;
                }
            }
        }
        self.close(frame, end, ended(&response)).await;
    }

    /// Ends the frame under way at `tick`, where a gap begins or the samples end.
    async fn abort(&mut self, tick: Tick) {
        let state = std::mem::replace(&mut self.state, State::Idle);
        if let State::Response(mut response) = state {
            if let Some(last) = response.held.take() {
                self.push_data(&mut response, last);
            }
            self.flush(response.frame, tick).await;
            self.close(response.frame, Provenance::at(tick), EndReason::Aborted)
                .await;
        }
    }

    /// Progress of the UART stream: nothing comes before `tick`.
    async fn progress(&mut self, tick: Tick) {
        let tick = self.unreported_from().map_or(tick, |from| from.min(tick));
        self.out.progress(tick).await;
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

/// Why a frame that was not cut off ends.
fn ended(response: &Response) -> EndReason {
    if response.error {
        EndReason::Error
    } else if response.received.is_none() {
        EndReason::NoResponse
    } else {
        EndReason::Complete
    }
}

async fn run(mut input: Inbox<UartEvent>, mut frames: Frames) {
    loop {
        match input.recv().await {
            Item::Event(event) => {
                let at = Provenance::over(event.span, event.settled);
                match event.payload {
                    UartEvent::Frame {
                        value, framing_ok, ..
                    } => {
                        // The requirements make every character 8 bits wide.
                        let [value, _] = value.to_le_bytes();
                        frames
                            .character(Byte {
                                value,
                                framing_ok,
                                at,
                            })
                            .await;
                    }
                    UartEvent::Break { ticks } => frames.line_break(at, ticks).await,
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
    fn configurations_are_checked() {
        assert_eq!(LinConfig::default().validate(), Ok(()));
        for min_break_bits in 9..=13 {
            assert!(
                Lin::new(LinConfig {
                    min_break_bits,
                    ..LinConfig::default()
                })
                .is_ok()
            );
        }
        for min_break_bits in [0, 8, 14, 255] {
            assert_eq!(
                Lin::new(LinConfig {
                    min_break_bits,
                    ..LinConfig::default()
                }),
                Err(LinConfigError::MinBreakBits(min_break_bits))
            );
        }
    }

    #[test]
    fn configurations_read_from_json() {
        let config: LinConfig =
            serde_json::from_str(r#"{"length_by_id":true}"#).expect("a configuration");
        // The threshold LIN slaves detect a break with, unless told otherwise.
        assert_eq!(
            config,
            LinConfig {
                length_by_id: true,
                min_break_bits: 11,
            }
        );
        let config: LinConfig =
            serde_json::from_str(r#"{"min_break_bits":13}"#).expect("a configuration");
        assert_eq!(config.min_break_bits, 13);
        assert!(serde_json::from_str::<LinConfig>(r#"{"break":13}"#).is_err());
    }

    #[test]
    fn props_say_how_frames_end() {
        assert_eq!(
            LinConfig::default().props().get("lengthById"),
            Some(&false.into())
        );
        let config = LinConfig {
            length_by_id: true,
            ..LinConfig::default()
        };
        assert_eq!(config.props().get("lengthById"), Some(&true.into()));
    }

    #[test]
    fn requires_lin_byte_fields() {
        let uart = |data_bits: i64, parity: &str, stop_bits: &str, order: &str| {
            Props::new()
                .with("baud", 19_200)
                .with("dataBits", data_bits)
                .with("parity", parity)
                .with("stopBits", stop_bits)
                .with("bitOrder", order)
        };
        let requires = Lin::requires();
        let unmet = |props: &Props| requires.unmet(props).map(|(key, _)| key);
        assert_eq!(unmet(&uart(8, "none", "one", "lsb-first")), None);
        assert_eq!(
            unmet(&uart(9, "none", "one", "lsb-first")),
            Some("dataBits")
        );
        assert_eq!(unmet(&uart(8, "even", "one", "lsb-first")), Some("parity"));
        assert_eq!(
            unmet(&uart(8, "none", "two", "lsb-first")),
            Some("stopBits")
        );
        assert_eq!(
            unmet(&uart(8, "none", "one", "msb-first")),
            Some("bitOrder")
        );
        let no_baud = Props::new()
            .with("dataBits", 8)
            .with("parity", "none")
            .with("stopBits", "one")
            .with("bitOrder", "lsb-first");
        assert_eq!(unmet(&no_baud), Some("baud"));
    }

    #[test]
    fn break_fields_are_measured_in_whole_ticks_rounded_down() {
        // Ten ticks per bit.
        assert_eq!(break_ticks(13, 192_000, 19_200), 130);
        assert_eq!(break_ticks(11, 192_000, 19_200), 110);
        // 52.08 ticks per bit at 1 MHz: 13 bits are 677.08 ticks.
        assert_eq!(break_ticks(13, 1_000_000, 19_200), 677);
        assert_eq!(break_ticks(13, 0, 19_200), 0);
    }
}
