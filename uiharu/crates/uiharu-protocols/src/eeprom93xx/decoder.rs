// SPDX-License-Identifier: Apache-2.0

//! The 93Cxx EEPROM decoder node: its configuration, the body that reads one instruction from
//! a Microwire transaction, and the recovery around it.

use serde::{Deserialize, Serialize};
use uiharu_core::{
    InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec,
    ProgressSink, PropValue, Props, Provenance, Requires, Tick, TickRange,
};

use super::events::{Command, DATA_BATCH, Direction, Eeprom93xxEvent, EndReason, OpId};
use crate::microwire::{MicrowireBus, MicrowireEvent, Next, Read, ReadError, Transaction, TxId};

/// The width of the EEPROM's words, which its ORG pin or its part number sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Organisation {
    /// Words of 16 bits.
    #[default]
    X16,
    /// Words of 8 bits.
    X8,
}

impl Organisation {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Organisation::X16 => "x16",
            Organisation::X8 => "x8",
        }
    }

    /// The bits of a word.
    pub const fn data_bits(self) -> u8 {
        match self {
            Organisation::X16 => 16,
            Organisation::X8 => 8,
        }
    }
}

/// How the EEPROM's instructions are laid out. A description may leave fields out: they take
/// the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Eeprom93xxConfig {
    /// The address bits of an instruction, 2 to 16: 6 for a 93C46 of 16-bit words and 7 of
    /// 8-bit words, 8 and 9 for the 93C56 and 93C66, 10 and 11 for the 93C86.
    pub address_bits: u8,
    /// The width of the words.
    pub organisation: Organisation,
}

impl Default for Eeprom93xxConfig {
    fn default() -> Self {
        Self {
            address_bits: 6,
            organisation: Organisation::X16,
        }
    }
}

impl Eeprom93xxConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), Eeprom93xxConfigError> {
        if !(2..=16).contains(&self.address_bits) {
            return Err(Eeprom93xxConfigError::AddressBits(self.address_bits));
        }
        Ok(())
    }

    /// What the decoder's output port declares: the address and word widths.
    pub fn props(&self) -> Props {
        Props::new()
            .with("addressBits", i64::from(self.address_bits))
            .with("dataBits", i64::from(self.organisation.data_bits()))
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Eeprom93xxConfigError {
    /// The address width is out of range.
    #[error("{0} address bits; 2 to 16 are supported")]
    AddressBits(u8),
}

/// The 93Cxx EEPROM decoder node: `microwire.events` in, `eeprom93xx.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Eeprom93xx {
    config: Eeprom93xxConfig,
}

impl Eeprom93xx {
    /// A decoder for `config`.
    pub fn new(config: Eeprom93xxConfig) -> Result<Self, Eeprom93xxConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &Eeprom93xxConfig {
        &self.config
    }

    /// The Microwire input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<MicrowireEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Eeprom93xxEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the Microwire stream it reads: transactions framed by chip
    /// select, with DI. DO is optional: without it reads report no words.
    pub fn requires() -> Requires {
        Requires::new().equals("cs", true).equals("di", true)
    }
}

impl Node for Eeprom93xx {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "eeprom93xx".into(),
            inputs: vec![InputSpec::new::<MicrowireEvent>(
                "microwire",
                Self::requires(),
            )],
            outputs: vec![OutputSpec::new::<Eeprom93xxEvent>(
                "events",
                self.config.props(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let layout = Layout {
            address_bits: self.config.address_bits,
            data_bits: self.config.organisation.data_bits(),
            data_out: io.input_props(0).get("do") == Some(&PropValue::Bool(true)),
        };
        let out = io.output(0);
        let bus = MicrowireBus::new(io.input(0), out.progress_sink());
        Box::pin(run(bus, Out::new(out), layout))
    }
}

/// The widths of an instruction's fields, and whether DO is there to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    address_bits: u8,
    data_bits: u8,
    data_out: bool,
}

/// The operation under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Open {
    op: OpId,
    tx: TxId,
    command: Command,
    /// Whether its instruction event went out.
    told: bool,
}

/// The typed writer the transaction body uses, which also remembers what is open.
#[derive(Debug)]
struct Out {
    out: Outbox<Eeprom93xxEvent>,
    sink: ProgressSink,
    next_op: u64,
    open: Option<Open>,
    /// At most [`DATA_BATCH`] words waiting to go out.
    batch: Vec<u16>,
    batch_direction: Direction,
    batch_offset: u32,
    batch_at: Option<Provenance>,
    /// The words of the open operation so far.
    count: u32,
    /// The programming operation whose busy and ready state DO may show, and the state last
    /// reported for it.
    polling: Option<(OpId, Option<bool>)>,
}

impl Out {
    /// Tells the cursor from where this writer holds items it has not reported yet.
    fn sync_hold(&self) {
        self.sink.hold(self.batch_at.map(|at| at.span.start));
    }

    fn new(out: Outbox<Eeprom93xxEvent>) -> Self {
        Self {
            sink: out.progress_sink(),
            out,
            next_op: 1,
            open: None,
            batch: Vec::with_capacity(DATA_BATCH),
            batch_direction: Direction::Out,
            batch_offset: 0,
            batch_at: None,
            count: 0,
            polling: None,
        }
    }

    /// Opens an operation for `command`, on transaction `tx`; its instruction goes out once
    /// whole.
    fn open(&mut self, command: Command, tx: TxId) -> OpId {
        let op = OpId(self.next_op);
        self.next_op += 1;
        self.open = Some(Open {
            op,
            tx,
            command,
            told: false,
        });
        self.count = 0;
        op
    }

    /// Reports the open operation's instruction with `address`.
    async fn instruction(&mut self, address: Option<u16>, at: Provenance) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.told = true;
        let event = Eeprom93xxEvent::Instruction {
            op: open.op,
            tx: open.tx,
            command: open.command,
            address,
        };
        self.out.emit(at, event).await;
    }

    /// Adds a word to the batch, which goes out once it holds [`DATA_BATCH`] words.
    async fn push(&mut self, direction: Direction, value: u16, at: Provenance) {
        if self.batch.is_empty() {
            self.batch_direction = direction;
            self.batch_offset = self.count;
            self.batch_at = Some(at);
        } else if let Some(have) = self.batch_at {
            self.batch_at = Some(have.join(at));
        }
        self.batch.push(value);
        self.sync_hold();
        self.count = self.count.saturating_add(1);
        if self.batch.len() >= DATA_BATCH {
            self.flush(None).await;
        }
    }

    /// Sends the batch, if any: settled by its last word when it is full, by the end that
    /// decided it otherwise (`decided`), so that the stream's events settle in order.
    async fn flush(&mut self, decided: Option<Tick>) {
        let Some(open) = self.open else {
            return;
        };
        if self.batch.is_empty() {
            return;
        }
        let values = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BATCH));
        let mut at = self
            .batch_at
            .take()
            .unwrap_or(Provenance::at(decided.unwrap_or(Tick::ZERO)));
        self.sync_hold();
        if let Some(decided) = decided {
            at.settled = at.settled.max(decided);
        }
        let event = Eeprom93xxEvent::Data {
            op: open.op,
            direction: self.batch_direction,
            offset: self.batch_offset,
            values,
        };
        self.out.emit(at, event).await;
    }

    /// Ends the open operation with `reason`, its end covering `at`. An operation whose
    /// instruction was cut short reports it first, without its address, and the end then
    /// marks the tick alone.
    async fn end(&mut self, reason: EndReason, at: Provenance) {
        let Some(open) = self.open else {
            return;
        };
        let mut at = at;
        if !open.told {
            self.instruction(None, at).await;
            at = Provenance::at(at.settled);
        }
        self.flush(Some(at.settled)).await;
        self.open = None;
        let event = Eeprom93xxEvent::End {
            op: open.op,
            reason,
        };
        self.out.emit(at, event).await;
    }

    /// Notes DO's level, as a transaction before its start bit or at its end shows it: while a
    /// programming cycle may run, a level other than the one last reported is the device's
    /// busy (low) or ready (high) state. Once ready, the cycle is over.
    async fn observe(&mut self, level: Option<bool>, tx: &mut Transaction<'_>) {
        let (Some((op, last)), Some(ready)) = (self.polling, level) else {
            return;
        };
        if last == Some(ready) {
            return;
        }
        self.polling = (!ready).then_some((op, Some(ready)));
        let event = Eeprom93xxEvent::Status {
            op,
            tx: tx.id(),
            ready,
        };
        self.out.emit(tx.consumed(), event).await;
    }
}

/// The body of the decoder: the instruction one transaction carries.
async fn transaction(
    mut tx: Transaction<'_>,
    out: &mut Out,
    layout: Layout,
) -> Result<(), ReadError> {
    // Clocks with DI low before the start bit are no instruction; while a programming cycle
    // may run, DO shows the device's state.
    loop {
        match tx.bit_or_end().await? {
            Read::Bit(bit) if bit.di => break,
            Read::Bit(bit) => out.observe(bit.r#do, &mut tx).await,
            Read::End { r#do } => {
                out.observe(r#do, &mut tx).await;
                return Ok(());
            }
        }
    }
    // A start bit clears the device's status from DO.
    out.polling = None;
    let opcode = (tx.bit().await?.di, tx.bit().await?.di);
    let (command, mut address, mut read) = match opcode {
        (true, false) => (Command::Read, 0u16, 0u8),
        (false, true) => (Command::Write, 0, 0),
        (true, true) => (Command::Erase, 0, 0),
        (false, false) => {
            // The address's two top bits tell the instruction.
            let top = (tx.bit().await?.di, tx.bit().await?.di);
            let command = match top {
                (true, true) => Command::Ewen,
                (false, false) => Command::Ewds,
                (true, false) => Command::Eral,
                (false, true) => Command::Wral,
            };
            (command, (u16::from(top.0) << 1) | u16::from(top.1), 2)
        }
    };
    let op = out.open(command, tx.id());
    // Most significant bit first. Pausing between the reads keeps the bits read so far: they
    // are the state of this future.
    while read < layout.address_bits {
        address = (address << 1) | u16::from(tx.bit().await?.di);
        read += 1;
    }
    let address = command.addressed().then_some(address);
    out.instruction(address, tx.consumed()).await;
    match command {
        Command::Read => {
            // A dummy 0 on DO came with the address's last bit; each word follows on DO, and
            // the next one after it while chip select stays asserted.
            let mut words = 0u32;
            loop {
                let first = match tx.bit_or_end().await? {
                    Read::Bit(bit) => bit,
                    Read::End { .. } => break,
                };
                let mut value = u16::from(first.r#do == Some(true));
                for _ in 1..layout.data_bits {
                    value = (value << 1) | u16::from(tx.bit().await?.r#do == Some(true));
                }
                words += 1;
                if layout.data_out {
                    out.push(Direction::Out, value, tx.consumed()).await;
                }
            }
            let reason = if words > 0 {
                EndReason::Complete
            } else {
                EndReason::Incomplete
            };
            out.end(reason, tx.consumed()).await;
        }
        Command::Write | Command::Wral => {
            let mut value = 0u16;
            for _ in 0..layout.data_bits {
                value = (value << 1) | u16::from(tx.bit().await?.di);
            }
            out.push(Direction::In, value, tx.consumed()).await;
            out.flush(None).await;
            // The device takes nothing more: the bits up to the release are skipped.
            tx.drain().await?;
            out.end(EndReason::Complete, tx.consumed()).await;
        }
        Command::Erase | Command::Eral | Command::Ewen | Command::Ewds => {
            tx.drain().await?;
            out.end(EndReason::Complete, tx.consumed()).await;
        }
    }
    if command.programs() {
        out.polling = Some((op, None));
    }
    Ok(())
}

/// Closes what the body left open, as the transaction's end calls for. The end covers the
/// bits the body read without reporting them, and the transaction's end when the stream
/// carried one; a gap or the end of the stream only places it.
async fn recover(error: ReadError, bus: &mut MicrowireBus, out: &mut Out) {
    let unreported = bus.consumed();
    let placed = |tick: Tick| Provenance {
        span: if unreported.span.is_empty() {
            TickRange::at(tick)
        } else {
            unreported.span
        },
        settled: tick,
    };
    let (reason, at) = match error {
        ReadError::Incomplete => (EndReason::Incomplete, unreported),
        ReadError::Truncated | ReadError::NoDataOut => (EndReason::Aborted, unreported),
        ReadError::Gap(range) => (EndReason::Aborted, placed(range.start)),
        ReadError::EndOfInput(tick) => (EndReason::Aborted, placed(tick)),
    };
    out.end(reason, at).await;
}

async fn run(mut bus: MicrowireBus, mut out: Out, layout: Layout) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                // A transaction whose beginning was not seen cannot be read: its first bits
                // are not the instruction's.
                let result = if tx.is_partial() {
                    let mut tx = tx;
                    tx.drain().await
                } else {
                    transaction(tx, &mut out, layout).await
                };
                if let Err(error) = result {
                    recover(error, &mut bus, &mut out).await;
                }
                if let Err(error) = bus.finish().await {
                    recover(error, &mut bus, &mut out).await;
                }
                bus.consumed();
            }
            Next::Gap(range) => {
                out.end(EndReason::Aborted, Provenance::at(range.start))
                    .await;
                // A programming cycle may have ended unseen in the gap.
                out.polling = None;
                out.out.gap(range).await;
            }
            Next::Progress(tick) => out.out.progress(tick).await,
            Next::End(tick) => {
                out.end(EndReason::Aborted, Provenance::at(tick)).await;
                out.out.end(tick).await;
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
        assert_eq!(Eeprom93xxConfig::default().validate(), Ok(()));
        for address_bits in [0, 1, 17] {
            assert_eq!(
                Eeprom93xxConfig {
                    address_bits,
                    ..Eeprom93xxConfig::default()
                }
                .validate(),
                Err(Eeprom93xxConfigError::AddressBits(address_bits))
            );
        }
        assert!(
            Eeprom93xx::new(Eeprom93xxConfig {
                address_bits: 11,
                organisation: Organisation::X8,
            })
            .is_ok()
        );
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: Eeprom93xxConfig =
            serde_json::from_str(r#"{"organisation": "x8", "address_bits": 7}"#).expect("parses");
        assert_eq!(
            config,
            Eeprom93xxConfig {
                address_bits: 7,
                organisation: Organisation::X8
            }
        );
        let whole: Eeprom93xxConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, Eeprom93xxConfig::default());
        assert!(serde_json::from_str::<Eeprom93xxConfig>(r#"{"organization": "x8"}"#).is_err());
    }

    #[test]
    fn ports_declare_the_widths_and_require_framed_bits_with_di() {
        let props = Eeprom93xxConfig {
            address_bits: 9,
            organisation: Organisation::X8,
        }
        .props();
        assert_eq!(props.get("addressBits"), Some(&9i64.into()));
        assert_eq!(props.get("dataBits"), Some(&8i64.into()));
        let bus = |cs: bool, di: bool, data_out: bool| {
            Props::new()
                .with("cs", cs)
                .with("di", di)
                .with("do", data_out)
        };
        let requires = Eeprom93xx::requires();
        assert!(requires.unmet(&bus(true, true, true)).is_none());
        assert!(requires.unmet(&bus(true, true, false)).is_none());
        assert_eq!(
            requires.unmet(&bus(false, true, true)).map(|(key, _)| key),
            Some("cs")
        );
        assert_eq!(
            requires.unmet(&bus(true, false, true)).map(|(key, _)| key),
            Some("di")
        );
    }
}
