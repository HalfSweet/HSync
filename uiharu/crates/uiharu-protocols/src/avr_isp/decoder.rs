// SPDX-License-Identifier: Apache-2.0

//! The AVR serial programming decoder node: the grouping of SPI words into instructions, and
//! the run loop.
//!
//! The decoder reads the SPI stream item by item rather than through
//! [`SpiBus`](crate::spi::SpiBus), because it also reads a bus without chip select, whose words
//! (transaction 0, never begun or ended) `SpiBus` skips; its unit is the four-word instruction,
//! of which chip select, when there is one, frames a whole programming session.

use uiharu_core::{
    Inbox, InputRef, InputSpec, Io, Item, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Props, Provenance, Requires, Tick,
};

use super::events::{Command, ErrorReason, INSTRUCTION_BYTES, IspEvent};
use super::instructions::{CommandExt, CommandFields};
use crate::spi::{EndReason, SpiEvent, TxId};

/// Programming Enable's first two bytes. Where the decoder does not know where instructions
/// start, it takes the grouping up at them once the device echoes the second.
pub const SYNC_BYTES: [u8; 2] = [0xac, 0x53];

/// What a device in sync echoes during Programming Enable's third byte: its second byte.
const ECHO: u8 = SYNC_BYTES[1];

/// The AVR serial programming decoder node: `spi.events` in, `avrisp.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvrIsp;

impl AvrIsp {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The SPI input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<SpiEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<IspEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the SPI stream it reads: bytes, most significant bit
    /// first, with both data lines. Chip select — the target's RESET — may be there or not.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("wordBits", 8)
            .equals("bitOrder", "msb-first")
            .equals("mosi", true)
            .equals("miso", true)
    }
}

impl Node for AvrIsp {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "avr-isp".into(),
            inputs: vec![InputSpec::new::<SpiEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<IspEvent>("events", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.input(0), io.output(0)))
    }
}

/// The words of the instruction under way, and what the decoder knows of where instructions
/// start.
#[derive(Debug)]
struct Instructions {
    /// Whether the decoder knows where instructions start. When it does not, the words it
    /// holds are the start of a Programming Enable whose echo is still to come.
    aligned: bool,
    /// The transaction the words held ride on.
    tx: TxId,
    /// The words held, on MOSI and on MISO: at most [`INSTRUCTION_BYTES`].
    mosi: [u8; INSTRUCTION_BYTES],
    miso: [u8; INSTRUCTION_BYTES],
    len: usize,
    /// Their ticks.
    at: Option<Provenance>,
    next_op: u64,
}

impl Instructions {
    fn new() -> Self {
        Self {
            // Only a transaction seen to begin, or Programming Enable, tells.
            aligned: false,
            tx: 0,
            mosi: [0; INSTRUCTION_BYTES],
            miso: [0; INSTRUCTION_BYTES],
            len: 0,
            at: None,
            next_op: 1,
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.at = None;
    }

    fn take(&mut self, tx: TxId, mosi: u8, miso: u8, at: Provenance) {
        if self.len == 0 {
            self.tx = tx;
        }
        self.mosi[self.len] = mosi;
        self.miso[self.len] = miso;
        self.len += 1;
        self.at = Some(self.at.map_or(at, |held| held.join(at)));
    }

    /// Chip select was asserted: a programming session begins, grouped from its first word
    /// unless it was under way already. The SPI stream ends a transaction before it begins
    /// the next, so nothing is held here.
    fn begin(&mut self, partial: bool) {
        self.clear();
        self.aligned = !partial;
    }

    /// A word of transaction `tx`, read at `at`: the instruction it completes, if any.
    fn word(
        &mut self,
        tx: TxId,
        mosi: u8,
        miso: u8,
        at: Provenance,
    ) -> Option<(Provenance, IspEvent)> {
        if !self.aligned {
            // Looking for `AC 53` answered by the echo: held words are its start so far.
            let fits = match self.len {
                0 => mosi == SYNC_BYTES[0],
                1 => mosi == SYNC_BYTES[1],
                _ => miso == ECHO,
            };
            if fits {
                self.aligned = self.len == 2;
                self.take(tx, mosi, miso, at);
            } else {
                // The words held begin nothing; this one may begin the next try.
                self.clear();
                if mosi == SYNC_BYTES[0] {
                    self.take(tx, mosi, miso, at);
                }
            }
            return None;
        }
        self.take(tx, mosi, miso, at);
        if self.len < INSTRUCTION_BYTES {
            return None;
        }
        let at = self.at.take()?;
        self.len = 0;
        Some((at, self.decode()))
    }

    /// The four words held, as an event.
    fn decode(&mut self) -> IspEvent {
        let bytes = self.mosi;
        let [first, second, third, fourth] = bytes;
        let Some(command) = Command::from_bytes(first, second) else {
            return IspEvent::Unknown { tx: self.tx, bytes };
        };
        let fields = command.fields();
        let op = self.next_op;
        self.next_op += 1;
        IspEvent::Instruction {
            op,
            tx: self.tx,
            command,
            address: fields
                .address
                .then_some(u16::from_be_bytes([second, third])),
            data_in: fields.data_in.then_some(fourth),
            data_out: fields.data_out.then_some(self.miso[3]),
            in_sync: fields.echo.then_some(self.miso[2] == ECHO),
            bytes,
        }
    }

    /// The words held were cut off as `reason` says, by what `at` covers: an error if they
    /// began an instruction. Words that only may have begun Programming Enable, while the
    /// grouping is not known, are dropped.
    fn cut(&mut self, reason: ErrorReason, at: Provenance) -> Option<(Provenance, IspEvent)> {
        let len = std::mem::take(&mut self.len);
        let held = self.at.take()?;
        if !self.aligned || len == 0 {
            return None;
        }
        let event = IspEvent::Error {
            tx: self.tx,
            reason,
            bytes: self.mosi[..len].to_vec(),
        };
        Some((held.join(at), event))
    }

    /// Samples are missing: where the next instruction starts is no longer known. Chip select,
    /// if the bus has one, tells again when the next transaction begins.
    fn lose(&mut self) {
        self.aligned = false;
    }

    /// Progress to `tick`, held back to the first word held.
    fn hold(&self, tick: Tick) -> Tick {
        self.at.map_or(tick, |at| at.span.start.min(tick))
    }
}

async fn run(mut input: Inbox<SpiEvent>, mut out: Outbox<IspEvent>) {
    let mut instructions = Instructions::new();
    loop {
        match input.recv().await {
            Item::Event(event) => {
                let at = Provenance::over(event.span, event.settled);
                let decided = match event.payload {
                    // The assertion of chip select belongs to no instruction.
                    SpiEvent::Begin { partial, .. } => {
                        instructions.begin(partial);
                        None
                    }
                    SpiEvent::Word {
                        tx,
                        mosi: Some(mosi),
                        miso: Some(miso),
                    } => instructions.word(tx, mosi, miso, at),
                    // The requirements make both lines present.
                    SpiEvent::Word { .. } => None,
                    SpiEvent::End { reason, .. } => {
                        let reason = match reason {
                            EndReason::CsRelease => ErrorReason::Incomplete,
                            EndReason::Aborted => ErrorReason::Aborted,
                        };
                        instructions.cut(reason, at)
                    }
                };
                if let Some((at, event)) = decided {
                    out.emit(at, event).await;
                }
            }
            Item::Gap(range) => {
                let at = Provenance::at(range.start);
                if let Some((at, event)) = instructions.cut(ErrorReason::Aborted, at) {
                    out.emit(at, event).await;
                }
                instructions.lose();
                out.gap(range).await;
            }
            Item::Progress(tick) => out.progress(instructions.hold(tick)).await,
            Item::End(tick) => {
                let at = Provenance::at(tick);
                if let Some((at, event)) = instructions.cut(ErrorReason::Aborted, at) {
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
    use uiharu_core::TickRange;

    fn at(tick: u64) -> Provenance {
        Provenance::over(TickRange::new(Tick(tick), Tick(tick + 10)), Tick(tick + 9))
    }

    #[test]
    fn the_input_requires_bytes_on_both_lines_but_not_chip_select() {
        let spec = AvrIsp::new().spec();
        assert_eq!(spec.name, "avr-isp");
        assert_eq!(spec.inputs[0].name, "bus");
        assert_eq!(spec.outputs[0].name, "events");
        let requires = AvrIsp::requires();
        let bus = crate::spi::SpiConfig {
            cs: None,
            ..crate::spi::SpiConfig::default()
        };
        assert!(requires.unmet(&bus.props()).is_none());
        let narrow = crate::spi::SpiConfig {
            word_bits: 7,
            ..crate::spi::SpiConfig::default()
        };
        assert!(requires.unmet(&narrow.props()).is_some());
    }

    #[test]
    fn four_words_make_an_instruction() {
        let mut instructions = Instructions::new();
        instructions.begin(false);
        let tx = 1;
        assert_eq!(instructions.word(tx, 0x30, 0xff, at(0)), None);
        assert_eq!(instructions.hold(Tick(5)), Tick(0));
        assert_eq!(instructions.word(tx, 0x00, 0x30, at(10)), None);
        assert_eq!(instructions.word(tx, 0x01, 0x00, at(20)), None);
        let (provenance, event) = instructions
            .word(tx, 0x00, 0x95, at(30))
            .expect("an instruction");
        assert_eq!(
            provenance,
            Provenance::over(TickRange::new(Tick(0), Tick(40)), Tick(39))
        );
        assert_eq!(
            event,
            IspEvent::Instruction {
                op: 1,
                tx,
                command: Command::ReadSignatureByte,
                address: Some(1),
                data_in: None,
                data_out: Some(0x95),
                in_sync: None,
                bytes: [0x30, 0x00, 0x01, 0x00],
            }
        );
        assert_eq!(instructions.hold(Tick(50)), Tick(50));
    }

    #[test]
    fn the_grouping_is_taken_up_at_programming_enable_answered_in_sync() {
        let mut instructions = Instructions::new();
        instructions.begin(true);
        let tx = 2;
        // A read, AC alone, AC 53 without the echo, then AC AC 53 and the echo.
        let words = [
            (0x30, 0x00),
            (0x00, 0x30),
            (0xac, 0x00),
            (0x00, 0xac),
            (0xac, 0x00),
            (0x53, 0xac),
            (0x00, 0x29),
            (0xac, 0x00),
            (0xac, 0xac),
        ];
        for (n, (mosi, miso)) in words.into_iter().enumerate() {
            assert_eq!(instructions.word(tx, mosi, miso, at(10 * n as u64)), None);
        }
        // The word that may begin Programming Enable holds progress back.
        assert_eq!(instructions.hold(Tick(1000)), Tick(80));
        assert_eq!(instructions.word(tx, 0x53, 0xac, at(90)), None);
        assert!(!instructions.aligned);
        assert_eq!(instructions.word(tx, 0x00, 0x53, at(100)), None);
        assert!(instructions.aligned);
        let (provenance, event) = instructions
            .word(tx, 0x00, 0x00, at(110))
            .expect("an instruction");
        assert_eq!(provenance.span, TickRange::new(Tick(80), Tick(120)));
        assert!(matches!(
            event,
            IspEvent::Instruction {
                command: Command::ProgrammingEnable,
                in_sync: Some(true),
                ..
            }
        ));
    }

    #[test]
    fn a_cut_instruction_is_an_error_and_a_lone_ac_is_not() {
        let mut instructions = Instructions::new();
        instructions.begin(false);
        let tx = 3;
        instructions.word(tx, 0xac, 0xff, at(0));
        instructions.word(tx, 0x80, 0xac, at(10));
        let (provenance, event) = instructions
            .cut(ErrorReason::Incomplete, Provenance::one(Tick(25)))
            .expect("an error");
        assert_eq!(
            provenance,
            Provenance::over(TickRange::new(Tick(0), Tick(26)), Tick(25))
        );
        assert_eq!(
            event,
            IspEvent::Error {
                tx,
                reason: ErrorReason::Incomplete,
                bytes: vec![0xac, 0x80],
            }
        );
        assert_eq!(instructions.cut(ErrorReason::Aborted, at(30)), None);
        instructions.lose();
        instructions.word(tx, 0xac, 0xff, at(40));
        assert_eq!(instructions.cut(ErrorReason::Aborted, at(50)), None);
        assert_eq!(instructions.hold(Tick(60)), Tick(60));
    }
}
