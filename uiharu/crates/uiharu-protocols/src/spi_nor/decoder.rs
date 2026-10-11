// SPDX-License-Identifier: Apache-2.0

//! The SPI NOR decoder node: its configuration, the body that reads one command from a
//! transaction, and the recovery around it.

use std::convert::Infallible;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec,
    Props, Provenance, Requires, Tick, TickRange,
};

use super::commands::Phase;
use super::events::{Command, DATA_BATCH, Direction, IdScheme, NorEnd, NorEvent, OpId};
use crate::spi::{Next, Read, ReadError, SpiBus, SpiEvent, Transaction, TxId};

/// How many address bytes the commands whose width follows the flash's address mode take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AddressBytes {
    /// Three: flashes of up to 16 MiB, and larger ones in their power-on mode.
    #[default]
    Three,
    /// Four: a flash kept in four-byte address mode.
    Four,
    /// Three until Enter 4-Byte Address Mode completes, four until Exit 4-Byte Address Mode or
    /// a software reset completes, and three again after a gap.
    Auto,
}

impl AddressBytes {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            AddressBytes::Three => "three",
            AddressBytes::Four => "four",
            AddressBytes::Auto => "auto",
        }
    }
}

/// How the decoder reads the flash's commands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpiNorConfig {
    /// The address width of the commands that follow the flash's address mode.
    pub address_bytes: AddressBytes,
}

impl SpiNorConfig {
    /// Checks the configuration: every one is valid, which the error type says.
    pub fn validate(&self) -> Result<(), Infallible> {
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("addressBytes", self.address_bytes.name())
    }
}

/// The SPI NOR decoder node: `spi.events` in, `nor.events` out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpiNor {
    config: SpiNorConfig,
}

impl SpiNor {
    /// A decoder for `config`.
    pub fn new(config: SpiNorConfig) -> Self {
        Self { config }
    }

    /// The configuration.
    pub fn config(&self) -> &SpiNorConfig {
        &self.config
    }

    /// The SPI input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<SpiEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<NorEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the SPI stream it reads: bytes, most significant bit
    /// first, framed by chip select, with both data lines.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("wordBits", 8)
            .equals("bitOrder", "msb-first")
            .equals("cs", true)
            .equals("mosi", true)
            .equals("miso", true)
    }
}

impl Node for SpiNor {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "spi-nor".into(),
            inputs: vec![InputSpec::new::<SpiEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<NorEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let bus = SpiBus::new(io.input(0));
        let flash = Flash::new(self.config.address_bytes);
        Box::pin(run(bus, NorOut::new(io.output(0)), flash))
    }
}

/// What the decoder knows of the flash from one transaction to the next: its address mode.
#[derive(Debug)]
struct Flash {
    setting: AddressBytes,
    /// Whether the commands that follow the mode take four address bytes now.
    four_byte: bool,
    /// Whether the last command was a complete Reset Enable, which arms Reset.
    reset_enabled: bool,
}

impl Flash {
    fn new(setting: AddressBytes) -> Self {
        Self {
            setting,
            four_byte: setting == AddressBytes::Four,
            reset_enabled: false,
        }
    }

    /// The width of an address that follows the mode.
    fn address_bytes(&self) -> u8 {
        if self.four_byte { 4 } else { 3 }
    }

    /// Follows a command that completed; `reset_enabled` says whether a complete Reset Enable
    /// came directly before it.
    fn completed(&mut self, command: Command, reset_enabled: bool) {
        let auto = self.setting == AddressBytes::Auto;
        match command {
            Command::Enter4bMode if auto => self.four_byte = true,
            Command::Exit4bMode if auto => self.four_byte = false,
            // A reset returns the flash to its power-on mode, which auto takes for three bytes.
            Command::Reset if auto && reset_enabled => self.four_byte = false,
            Command::ResetEnable => self.reset_enabled = true,
            _ => {}
        }
    }

    /// Forgets what the commands lost in a gap may have changed.
    fn forget(&mut self) {
        self.four_byte = self.setting == AddressBytes::Four;
        self.reset_enabled = false;
    }
}

/// What the current batch holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Batch {
    Data(Direction),
    Id(IdScheme),
}

/// The typed writer the transaction body uses, which also remembers what is open.
#[derive(Debug)]
struct NorOut {
    out: Outbox<NorEvent>,
    next_op: u64,
    open: Option<OpId>,
    /// At most [`DATA_BATCH`] bytes waiting to go out.
    batch: Vec<u8>,
    batch_of: Batch,
    batch_offset: u32,
    batch_at: Option<Provenance>,
    /// The data bytes of the open operation so far.
    count: u32,
}

impl NorOut {
    fn new(out: Outbox<NorEvent>) -> Self {
        Self {
            out,
            next_op: 1,
            open: None,
            batch: Vec::with_capacity(DATA_BATCH),
            batch_of: Batch::Data(Direction::Out),
            batch_offset: 0,
            batch_at: None,
            count: 0,
        }
    }

    async fn begin(&mut self, command: Command, tx: TxId, at: Provenance) -> OpId {
        let op = OpId(self.next_op);
        self.next_op += 1;
        self.open = Some(op);
        self.count = 0;
        self.out.emit(at, NorEvent::Begin { op, tx, command }).await;
        op
    }

    async fn address(&mut self, op: OpId, address: u32, bytes: u8, at: Provenance) {
        self.out
            .emit(at, NorEvent::Address { op, address, bytes })
            .await;
    }

    async fn dummy(&mut self, op: OpId, bytes: u8, at: Provenance) {
        self.out.emit(at, NorEvent::Dummy { op, bytes }).await;
    }

    async fn status(&mut self, op: OpId, register: u8, value: u8, written: bool, at: Provenance) {
        let event = NorEvent::Status {
            op,
            register,
            value,
            written,
        };
        self.out.emit(at, event).await;
    }

    /// Adds a byte to the current batch, which goes out once it holds [`DATA_BATCH`] bytes.
    async fn push(&mut self, op: OpId, of: Batch, byte: u8, at: Provenance) {
        if !self.batch.is_empty() && self.batch_of != of {
            self.flush(op, None).await;
        }
        if self.batch.is_empty() {
            self.batch_of = of;
            self.batch_offset = self.count;
            self.batch_at = Some(at);
        } else if let Some(have) = self.batch_at {
            self.batch_at = Some(have.join(at));
        }
        self.batch.push(byte);
        if matches!(of, Batch::Data(_)) {
            self.count = self.count.saturating_add(1);
        }
        if self.batch.len() >= DATA_BATCH {
            self.flush(op, None).await;
        }
    }

    /// Sends the batch, if any. A batch is settled when it is decided: by its last byte when it
    /// is full or whole (`decided` is `None`), by the end that decided it otherwise, so that a
    /// stream's events settle in order.
    async fn flush(&mut self, op: OpId, decided: Option<Tick>) {
        if self.batch.is_empty() {
            return;
        }
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BATCH));
        let mut at = match (self.batch_at.take(), decided) {
            (Some(at), _) => at,
            (None, decided) => Provenance::at(decided.unwrap_or(Tick::ZERO)),
        };
        if let Some(decided) = decided {
            at.settled = at.settled.max(decided);
        }
        let event = match self.batch_of {
            Batch::Data(direction) => NorEvent::Data {
                op,
                direction,
                offset: self.batch_offset,
                bytes,
            },
            Batch::Id(scheme) => NorEvent::Id { op, scheme, bytes },
        };
        self.out.emit(at, event).await;
    }

    async fn end(&mut self, op: OpId, reason: NorEnd, at: Provenance) {
        self.flush(op, Some(at.settled)).await;
        self.open = None;
        self.out.emit(at, NorEvent::End { op, reason }).await;
    }

    async fn unknown_command(&mut self, tx: TxId, opcode: u8, at: Provenance) {
        self.out
            .emit(at, NorEvent::UnknownCommand { tx, opcode })
            .await;
    }

    /// Ends the open operation, if any, with `reason`.
    async fn close_open(&mut self, reason: NorEnd, at: Provenance) {
        if let Some(op) = self.open {
            self.end(op, reason, at).await;
        }
    }
}

/// The body of the decoder: the command one transaction carries.
async fn transaction(
    mut tx: Transaction<'_>,
    out: &mut NorOut,
    flash: &mut Flash,
) -> Result<(), ReadError> {
    let opcode = tx.word().await?.mosi()?;
    // Any command after Reset Enable disarms it.
    let reset_enabled = std::mem::take(&mut flash.reset_enabled);
    let Some(command) = Command::from_opcode(opcode) else {
        out.unknown_command(tx.id(), opcode, tx.consumed()).await;
        tx.drain().await?;
        return Ok(());
    };
    let op = out.begin(command, tx.id(), tx.consumed()).await;
    let layout = command.layout();
    let width = layout.address.bytes(flash.address_bytes());
    if width > 0 {
        // Most significant byte first. Pausing between the reads keeps the bytes read so far:
        // they are the state of this future.
        let mut address = 0u32;
        for _ in 0..width {
            address = (address << 8) | u32::from(tx.word().await?.mosi()?);
        }
        out.address(op, address, width, tx.consumed()).await;
    }
    if layout.dummy > 0 {
        for _ in 0..layout.dummy {
            tx.word().await?;
        }
        out.dummy(op, layout.dummy, tx.consumed()).await;
    }
    if !phase(&mut tx, out, op, layout.phase).await? {
        // The command is whole and takes nothing more; what follows is skipped.
        tx.drain().await?;
    }
    out.end(op, NorEnd::Complete, tx.consumed()).await;
    flash.completed(command, reset_enabled);
    Ok(())
}

/// Reads what the command transfers after its address and dummy bytes. Returns whether it
/// found the transaction's normal end. What the host must send is read with `word`, so that
/// chip select released before it ends the operation incomplete; what may stop after any byte
/// — the flash's answers, a program's further bytes — is read with `word_or_end`.
async fn phase(
    tx: &mut Transaction<'_>,
    out: &mut NorOut,
    op: OpId,
    phase: Phase,
) -> Result<bool, ReadError> {
    match phase {
        Phase::None => Ok(false),
        Phase::DataOut => loop {
            match tx.word_or_end().await? {
                Read::End => return Ok(true),
                Read::Word(word) => {
                    let data = Batch::Data(Direction::Out);
                    out.push(op, data, word.miso()?, tx.consumed()).await;
                }
            }
        },
        Phase::DataIn => {
            let data = Batch::Data(Direction::In);
            let first = tx.word().await?.mosi()?;
            out.push(op, data, first, tx.consumed()).await;
            loop {
                match tx.word_or_end().await? {
                    Read::End => return Ok(true),
                    Read::Word(word) => out.push(op, data, word.mosi()?, tx.consumed()).await,
                }
            }
        }
        Phase::ReadStatus(register) => loop {
            match tx.word_or_end().await? {
                Read::End => return Ok(true),
                Read::Word(word) => {
                    let value = word.miso()?;
                    out.status(op, register, value, false, tx.consumed()).await;
                }
            }
        },
        Phase::WriteStatus { first, last } => {
            let value = tx.word().await?.mosi()?;
            out.status(op, first, value, true, tx.consumed()).await;
            for register in first + 1..=last {
                match tx.word_or_end().await? {
                    Read::End => return Ok(true),
                    Read::Word(word) => {
                        let value = word.mosi()?;
                        out.status(op, register, value, true, tx.consumed()).await;
                    }
                }
            }
            Ok(false)
        }
        Phase::Id { scheme, len } => {
            // `None` reads as many bytes as the host clocks, in batches.
            let mut left = len;
            loop {
                if left == Some(0) {
                    out.flush(op, None).await;
                    return Ok(false);
                }
                match tx.word_or_end().await? {
                    Read::End => return Ok(true),
                    Read::Word(word) => {
                        out.push(op, Batch::Id(scheme), word.miso()?, tx.consumed())
                            .await;
                        left = left.map(|left| left - 1);
                    }
                }
            }
        }
        Phase::Signature => {
            // Release from power-down alone, or three dummy bytes and the signature.
            if tx.word_or_end().await? == Read::End {
                return Ok(true);
            }
            for _ in 1..3 {
                tx.word().await?;
            }
            out.dummy(op, 3, tx.consumed()).await;
            match tx.word_or_end().await? {
                Read::End => Ok(true),
                Read::Word(word) => {
                    let id = Batch::Id(IdScheme::ElectronicSignature);
                    out.push(op, id, word.miso()?, tx.consumed()).await;
                    out.flush(op, None).await;
                    Ok(false)
                }
            }
        }
    }
}

/// Closes what the body left open, as the transaction's end calls for. The end covers the
/// words the body read without reporting them, and the transaction's end when the stream
/// carried one; a gap or the end of the stream only places it.
async fn recover(error: ReadError, bus: &mut SpiBus, out: &mut NorOut) {
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
        ReadError::Incomplete => (NorEnd::Incomplete, unreported),
        ReadError::Truncated | ReadError::MissingLine(_) => (NorEnd::Aborted, unreported),
        ReadError::Gap(range) => (NorEnd::Aborted, placed(range.start)),
        ReadError::EndOfInput(tick) => (NorEnd::Aborted, placed(tick)),
    };
    out.close_open(reason, at).await;
}

async fn run(mut bus: SpiBus, mut out: NorOut, mut flash: Flash) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                // A transaction whose beginning was not seen cannot be read: its first word
                // is not the opcode.
                let result = if tx.is_partial() {
                    let mut tx = tx;
                    tx.drain().await
                } else {
                    transaction(tx, &mut out, &mut flash).await
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
                out.close_open(NorEnd::Aborted, Provenance::at(range.start))
                    .await;
                flash.forget();
                out.out.gap(range).await;
            }
            Next::Progress(tick) => out.out.progress(tick).await,
            Next::End(tick) => {
                out.close_open(NorEnd::Aborted, Provenance::at(tick)).await;
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
    fn props_say_how_addresses_are_read() {
        let props = SpiNorConfig {
            address_bytes: AddressBytes::Auto,
        }
        .props();
        assert_eq!(props.get("addressBytes"), Some(&"auto".into()));
        assert_eq!(
            SpiNorConfig::default().props().get("addressBytes"),
            Some(&"three".into())
        );
        assert_eq!(SpiNorConfig::default().validate(), Ok(()));
    }

    #[test]
    fn the_address_mode_follows_commands_only_in_auto() {
        let mut flash = Flash::new(AddressBytes::Auto);
        assert_eq!(flash.address_bytes(), 3);
        flash.completed(Command::Enter4bMode, false);
        assert_eq!(flash.address_bytes(), 4);
        // Reset without Reset Enable directly before it does nothing.
        flash.completed(Command::Reset, false);
        assert_eq!(flash.address_bytes(), 4);
        flash.completed(Command::ResetEnable, false);
        assert!(flash.reset_enabled);
        flash.completed(Command::Reset, true);
        assert_eq!(flash.address_bytes(), 3);
        flash.completed(Command::Enter4bMode, false);
        flash.forget();
        assert_eq!(flash.address_bytes(), 3);

        let mut fixed = Flash::new(AddressBytes::Four);
        fixed.completed(Command::Exit4bMode, false);
        fixed.forget();
        assert_eq!(fixed.address_bytes(), 4);
        let mut fixed = Flash::new(AddressBytes::Three);
        fixed.completed(Command::Enter4bMode, false);
        assert_eq!(fixed.address_bytes(), 3);
    }

    #[test]
    fn configurations_serialize_by_name() {
        let config = SpiNorConfig {
            address_bytes: AddressBytes::Four,
        };
        assert_eq!(
            serde_json::to_string(&config).expect("serializes"),
            r#"{"address_bytes":"four"}"#
        );
    }
}
