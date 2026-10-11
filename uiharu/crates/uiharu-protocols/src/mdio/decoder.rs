// SPDX-License-Identifier: Apache-2.0

//! The MDIO decoder node: its configuration, the frame read bit by bit, and the run loop.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, SignalCursor,
    Tick, TickRange,
};

use super::events::{Clause, ErrorReason, MdioEvent, Op};

/// The ones of a preamble. As many in a row put the decoder in sync, and the count a frame
/// reports stops there.
pub const PREAMBLE_BITS: u8 = 32;

/// The bits of a frame after its preamble: ST, OP, the two addresses, the turnaround and the
/// data.
pub const FRAME_BITS: u8 = 32;

/// How the management interface is wired. A description may leave fields out: they take the
/// defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MdioConfig {
    /// The management data clock channel.
    pub mdc: u8,
    /// The management data input/output channel.
    pub mdio: u8,
}

impl Default for MdioConfig {
    fn default() -> Self {
        Self { mdc: 0, mdio: 1 }
    }
}

impl MdioConfig {
    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> {
        [self.mdc, self.mdio].into_iter()
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), MdioConfigError> {
        if let Some(channel) = self.channels().find(|channel| *channel >= MAX_CHANNELS) {
            return Err(MdioConfigError::Channel(channel));
        }
        if self.mdc == self.mdio {
            return Err(MdioConfigError::DuplicateChannel(self.mdc));
        }
        Ok(())
    }

    /// What the decoder's output port declares: nothing, since every frame names its own
    /// clause.
    pub fn props(&self) -> Props {
        Props::new()
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MdioConfigError {
    /// MDC and MDIO are on the same channel.
    #[error("channel {0} is used for both MDC and MDIO")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The MDIO decoder node: logic samples in, `mdio.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mdio {
    config: MdioConfig,
}

impl Mdio {
    /// A decoder for `config`.
    pub fn new(config: MdioConfig) -> Result<Self, MdioConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &MdioConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<MdioEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Mdio {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "mdio".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<MdioEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

/// The clause and the operation of a frame that starts with ST `st` and OP `op`, two bits
/// each; `None` for a clause 22 start with an OP clause 22 does not define.
const fn decode_start(st: u32, op: u32) -> Option<(Clause, Op)> {
    match (st, op) {
        (0b01, 0b10) => Some((Clause::C22, Op::Read)),
        (0b01, 0b01) => Some((Clause::C22, Op::Write)),
        (0b00, 0b00) => Some((Clause::C45, Op::Address)),
        (0b00, 0b01) => Some((Clause::C45, Op::Write)),
        (0b00, 0b11) => Some((Clause::C45, Op::Read)),
        (0b00, 0b10) => Some((Clause::C45, Op::PostReadIncrement)),
        _ => None,
    }
}

/// Whether the turnaround bits `turnaround`, the first in bit 1, keep the rule of `op`: the
/// station sends `10` when it goes on driving; the device drives the second bit `0` when it
/// takes over, the first being nobody's.
const fn turnaround_ok(op: Op, turnaround: u32) -> bool {
    if op.device_drives() {
        turnaround & 0b01 == 0
    } else {
        turnaround == 0b10
    }
}

/// The ticks of the ones clocked in a row since the last zero, the latest
/// [`PREAMBLE_BITS`] of them: where the preamble of the next frame begins.
#[derive(Debug)]
struct Ones {
    ticks: [Tick; PREAMBLE_BITS as usize],
    /// How many are held.
    len: u8,
    /// Where the next one goes.
    next: u8,
}

impl Ones {
    fn new() -> Self {
        Self {
            ticks: [Tick::ZERO; PREAMBLE_BITS as usize],
            len: 0,
            next: 0,
        }
    }

    fn push(&mut self, tick: Tick) {
        self.ticks[usize::from(self.next)] = tick;
        self.next = (self.next + 1) % PREAMBLE_BITS;
        self.len = (self.len + 1).min(PREAMBLE_BITS);
    }

    /// The tick of the earliest one held.
    fn first(&self) -> Option<Tick> {
        if self.len == 0 {
            return None;
        }
        let index = (self.next + PREAMBLE_BITS - self.len) % PREAMBLE_BITS;
        Some(self.ticks[usize::from(index)])
    }

    /// Whether a preamble's worth is held: that many ones in a row.
    fn is_full(&self) -> bool {
        self.len == PREAMBLE_BITS
    }

    fn clear(&mut self) {
        self.len = 0;
        self.next = 0;
    }
}

/// The frame under way, from ST's first bit on.
#[derive(Debug)]
struct Frame {
    /// Where its span starts: the first one of its preamble, or ST's first bit.
    start: Tick,
    /// ST's first bit.
    st: Tick,
    /// The first turnaround bit, once read.
    ta: Tick,
    /// The last bit read.
    last: Tick,
    preamble_bits: u8,
    /// The bits read, the latest in bit 0.
    bits: u32,
    /// How many.
    count: u8,
    /// The clause and the operation, once ST and OP are read.
    kind: Option<(Clause, Op)>,
}

/// The frame state machine, one bit at a time.
#[derive(Debug)]
struct Decoder {
    /// Whether the next zero is known to start a frame: after a whole frame, or after a
    /// preamble's worth of ones in a row.
    synced: bool,
    ones: Ones,
    frame: Option<Frame>,
}

impl Decoder {
    fn new() -> Self {
        Self {
            synced: false,
            ones: Ones::new(),
            frame: None,
        }
    }

    /// Takes the bit MDIO carried at the rising edge of MDC at `tick`; what it decided, if
    /// anything.
    fn bit(&mut self, tick: Tick, high: bool) -> Option<(Provenance, MdioEvent)> {
        let Some(frame) = self.frame.as_mut() else {
            if high {
                self.ones.push(tick);
                if self.ones.is_full() {
                    self.synced = true;
                }
            } else {
                // The first bit of ST, once in sync; before that, a zero only breaks the ones.
                if self.synced {
                    self.frame = Some(Frame {
                        start: self.ones.first().unwrap_or(tick),
                        st: tick,
                        ta: tick,
                        last: tick,
                        preamble_bits: self.ones.len,
                        bits: 0,
                        count: 1,
                        kind: None,
                    });
                }
                self.ones.clear();
            }
            return None;
        };
        frame.bits = (frame.bits << 1) | u32::from(high);
        frame.count += 1;
        frame.last = tick;
        match frame.count {
            // ST and OP.
            4 => match decode_start(frame.bits >> 2 & 0b11, frame.bits & 0b11) {
                Some(kind) => {
                    frame.kind = Some(kind);
                    None
                }
                None => {
                    let at = Provenance::over(TickRange::new(frame.st, tick.next()), tick);
                    self.frame = None;
                    self.synced = false;
                    let reason = ErrorReason::Start;
                    Some((at, MdioEvent::Error { reason }))
                }
            },
            15 => {
                frame.ta = tick;
                None
            }
            16 => {
                let (_, op) = frame.kind?;
                if turnaround_ok(op, frame.bits & 0b11) {
                    return None;
                }
                let at = Provenance::over(TickRange::new(frame.ta, tick.next()), tick);
                let reason = ErrorReason::Turnaround;
                Some((at, MdioEvent::Error { reason }))
            }
            FRAME_BITS => {
                let frame = self.frame.take()?;
                let (clause, op) = frame.kind?;
                let bits = frame.bits;
                let event = MdioEvent::Frame {
                    clause,
                    op,
                    phy: (bits >> 23 & 0x1f) as u8,
                    reg: (bits >> 18 & 0x1f) as u8,
                    turnaround: (bits >> 16 & 0b11) as u8,
                    data: (bits & 0xffff) as u16,
                    preamble_bits: frame.preamble_bits,
                };
                let at = Provenance::over(TickRange::new(frame.start, tick.next()), tick);
                Some((at, event))
            }
            _ => None,
        }
    }

    /// The samples stop at `tick`, for a gap or for good: the frame under way is cut there,
    /// and what was counted before no longer holds.
    fn cut(&mut self, tick: Tick) -> Option<(Provenance, MdioEvent)> {
        self.synced = false;
        self.ones.clear();
        let frame = self.frame.take()?;
        let span = TickRange::new(frame.start, frame.last.next());
        let reason = ErrorReason::Aborted;
        Some((Provenance::over(span, tick), MdioEvent::Error { reason }))
    }

    /// Progress to `tick`, held back to where the next event may start: the frame under way,
    /// or the ones that may be its preamble.
    fn hold(&self, tick: Tick) -> Tick {
        let start = match &self.frame {
            Some(frame) => Some(frame.start),
            None => self.ones.first(),
        };
        start.map_or(tick, |start| start.min(tick))
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<MdioEvent>, config: MdioConfig) {
    let mut decoder = Decoder::new();
    let watch = ChannelMask::one(config.mdc);
    loop {
        match signal.next_change(watch).await {
            Change::At {
                tick,
                changed,
                levels,
            } => {
                if changed.has(config.mdc)
                    && levels.high(config.mdc)
                    && let Some((at, event)) = decoder.bit(tick, levels.high(config.mdio))
                {
                    out.emit(at, event).await;
                }
            }
            Change::Gap(range) => {
                if let Some((at, event)) = decoder.cut(range.start) {
                    out.emit(at, event).await;
                }
                out.gap(range).await;
            }
            // Without a deadline the wait is never quiet.
            Change::Quiet(_) => {}
            Change::Progress(tick) => out.progress(decoder.hold(tick)).await,
            Change::End(tick) => {
                if let Some((at, event)) = decoder.cut(tick) {
                    out.emit(at, event).await;
                }
                out.end(tick).await;
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
        assert_eq!(MdioConfig::default().validate(), Ok(()));
        let bad = |config: MdioConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(MdioConfig { mdc: 3, mdio: 3 }),
            MdioConfigError::DuplicateChannel(3)
        );
        assert_eq!(
            bad(MdioConfig { mdc: 0, mdio: 64 }),
            MdioConfigError::Channel(64)
        );
        assert!(Mdio::new(MdioConfig { mdc: 7, mdio: 7 }).is_err());
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: MdioConfig = serde_json::from_str(r#"{"mdio": 5}"#).expect("parses");
        assert_eq!(config, MdioConfig { mdc: 0, mdio: 5 });
        let whole: MdioConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, MdioConfig::default());
        assert!(serde_json::from_str::<MdioConfig>(r#"{"clock": 0}"#).is_err());
    }

    #[test]
    fn the_input_requires_the_channels_read() {
        let mdio = Mdio::new(MdioConfig { mdc: 4, mdio: 9 }).expect("valid");
        let spec = mdio.spec();
        assert_eq!(spec.inputs[0].name, "logic");
        assert_eq!(spec.outputs[0].name, "events");
        let requires = &spec.inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 10)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 9)).is_some());
        assert_eq!(MdioConfig::default().props(), Props::new());
    }

    #[test]
    fn starts_name_their_clause_and_operation() {
        assert_eq!(decode_start(0b01, 0b10), Some((Clause::C22, Op::Read)));
        assert_eq!(decode_start(0b01, 0b01), Some((Clause::C22, Op::Write)));
        assert_eq!(decode_start(0b01, 0b00), None);
        assert_eq!(decode_start(0b01, 0b11), None);
        assert_eq!(decode_start(0b00, 0b00), Some((Clause::C45, Op::Address)));
        assert_eq!(decode_start(0b00, 0b01), Some((Clause::C45, Op::Write)));
        assert_eq!(decode_start(0b00, 0b11), Some((Clause::C45, Op::Read)));
        assert_eq!(
            decode_start(0b00, 0b10),
            Some((Clause::C45, Op::PostReadIncrement))
        );
    }

    #[test]
    fn turnarounds_follow_who_drives() {
        assert!(turnaround_ok(Op::Write, 0b10));
        assert!(turnaround_ok(Op::Address, 0b10));
        for wrong in [0b00, 0b01, 0b11] {
            assert!(!turnaround_ok(Op::Write, wrong));
            assert!(!turnaround_ok(Op::Address, wrong));
        }
        // The first bit of a read's turnaround is driven by nobody.
        for ok in [0b00, 0b10] {
            assert!(turnaround_ok(Op::Read, ok));
            assert!(turnaround_ok(Op::PostReadIncrement, ok));
        }
        for wrong in [0b01, 0b11] {
            assert!(!turnaround_ok(Op::Read, wrong));
        }
    }

    #[test]
    fn ones_keep_the_latest_preamble() {
        let mut ones = Ones::new();
        assert_eq!(ones.first(), None);
        for tick in 0..10 {
            ones.push(Tick(100 + tick));
        }
        assert_eq!(ones.first(), Some(Tick(100)));
        assert!(!ones.is_full());
        for tick in 10..70 {
            ones.push(Tick(100 + tick));
        }
        assert!(ones.is_full());
        assert_eq!(ones.len, PREAMBLE_BITS);
        // The latest 32 of 70: from the 39th on.
        assert_eq!(ones.first(), Some(Tick(138)));
        ones.clear();
        assert_eq!(ones.first(), None);
        ones.push(Tick(500));
        assert_eq!(ones.first(), Some(Tick(500)));
    }

    #[test]
    fn a_frame_needs_sync_first() {
        let mut decoder = Decoder::new();
        // A zero before any preamble starts nothing.
        assert_eq!(decoder.bit(Tick(0), false), None);
        assert!(decoder.frame.is_none());
        for tick in 1..=32 {
            assert_eq!(decoder.bit(Tick(tick), true), None);
        }
        assert!(decoder.synced);
        assert_eq!(decoder.hold(Tick(40)), Tick(1));
        decoder.bit(Tick(33), false);
        assert_eq!(
            decoder.frame.as_ref().map(|frame| frame.preamble_bits),
            Some(32)
        );
        assert_eq!(decoder.hold(Tick(40)), Tick(1));
        // A cut ends it, and sync is lost.
        let (at, event) = decoder.cut(Tick(50)).expect("an abort");
        assert_eq!(
            event,
            MdioEvent::Error {
                reason: ErrorReason::Aborted
            }
        );
        assert_eq!(
            at,
            Provenance::over(TickRange::new(Tick(1), Tick(34)), Tick(50))
        );
        assert!(!decoder.synced);
        assert_eq!(decoder.hold(Tick(60)), Tick(60));
    }
}
