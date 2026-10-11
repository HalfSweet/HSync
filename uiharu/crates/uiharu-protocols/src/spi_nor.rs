// SPDX-License-Identifier: Apache-2.0

//! SPI NOR flash commands over `spi.events`: the `READ` command with a three-byte address
//! (opcode `0x03`), decoded as it streams.
//!
//! The decoder follows the shape a DSL decoder takes: a `transaction` body reads the command
//! through a transaction cursor and writes typed events, and the recovery around it closes
//! whatever the body had opened when the transaction ends early, is cut off or runs into a
//! gap. Data is reported in batches of at most [`DATA_BATCH`] bytes, each as soon as it is
//! full, so a long read shows up while it is still under way.

use serde::{Deserialize, Serialize};
use uiharu_core::{
    EventSchema, InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Props, Provenance, Requires, SchemaId, Tick, TickRange,
};

use crate::spi::{Line, Next, Read, ReadError, SpiBus, SpiEvent, Transaction, TxId};

/// The schema of SPI NOR event streams.
pub const NOR_EVENTS: SchemaId = SchemaId::new("nor.events", 1);

/// The most bytes one data event carries.
pub const DATA_BATCH: usize = 32;

/// The opcode of `READ` with a three-byte address.
pub const READ: u8 = 0x03;

/// An operation on the flash, numbered from 1 by the decoder.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// The commands the decoder understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    /// `READ`, opcode `0x03`, with a three-byte address.
    Read,
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NorEnd {
    /// The command ran to its end: chip select was released after it was whole.
    Complete,
    /// Chip select was released before the command was whole.
    Incomplete,
    /// The samples ended or were lost while the command was under way.
    Aborted,
}

/// What travels on an SPI NOR event stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum NorEvent {
    /// A command was recognised.
    Begin {
        /// The operation that begins.
        op: OpId,
        /// The SPI transaction it rides on.
        tx: TxId,
        /// The command.
        command: Command,
    },
    /// The command's address.
    Address {
        /// The operation.
        op: OpId,
        /// The 24-bit address.
        address: u32,
    },
    /// Bytes read from the flash, in order.
    Data {
        /// The operation.
        op: OpId,
        /// The position of the first byte, counted from the first byte read.
        offset: u32,
        /// At most [`DATA_BATCH`] bytes.
        bytes: Vec<u8>,
    },
    /// The operation ended.
    End {
        /// The operation.
        op: OpId,
        /// How it ended.
        reason: NorEnd,
    },
    /// A transaction started with an opcode the decoder does not know; its words were
    /// skipped.
    UnknownCommand {
        /// The SPI transaction.
        tx: TxId,
        /// Its first word on MOSI.
        opcode: u8,
    },
}

impl EventSchema for NorEvent {
    const SCHEMA: SchemaId = NOR_EVENTS;
}

/// The SPI NOR decoder node: `spi.events` in, `nor.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpiNorRead24;

impl SpiNorRead24 {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The SPI input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<SpiEvent> {
        InputRef::of_node(node, 0)
    }

    /// The operations output of the decoder added as `node`.
    pub fn operations(node: NodeId) -> OutputRef<NorEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the SPI stream it reads.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("wordBits", 8)
            .equals("bitOrder", "msb-first")
            .equals("cs", true)
            .equals("mosi", true)
            .equals("miso", true)
    }
}

impl Node for SpiNorRead24 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "spi-nor-read24".into(),
            inputs: vec![InputSpec::new::<SpiEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<NorEvent>(
                "operations",
                Props::new().with("commands", "read"),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let bus = SpiBus::new(io.input(0));
        Box::pin(run(bus, NorOut::new(io.output(0))))
    }
}

/// The typed writer the transaction body uses, which also remembers what is open.
#[derive(Debug)]
struct NorOut {
    out: Outbox<NorEvent>,
    next_op: u64,
    open: Option<OpId>,
    batch: Vec<u8>,
    batch_offset: u32,
    batch_at: Option<Provenance>,
    read_count: u32,
}

impl NorOut {
    fn new(out: Outbox<NorEvent>) -> Self {
        Self {
            out,
            next_op: 1,
            open: None,
            batch: Vec::with_capacity(DATA_BATCH),
            batch_offset: 0,
            batch_at: None,
            read_count: 0,
        }
    }

    async fn begin(&mut self, command: Command, tx: TxId, at: Provenance) -> OpId {
        let op = OpId(self.next_op);
        self.next_op += 1;
        self.open = Some(op);
        self.read_count = 0;
        self.out.emit(at, NorEvent::Begin { op, tx, command }).await;
        op
    }

    async fn address(&mut self, op: OpId, address: u32, at: Provenance) {
        self.out.emit(at, NorEvent::Address { op, address }).await;
    }

    /// Adds a byte to the current batch, which goes out once it holds [`DATA_BATCH`] bytes.
    async fn data(&mut self, op: OpId, byte: u8, at: Provenance) {
        if self.batch.is_empty() {
            self.batch_offset = self.read_count;
            self.batch_at = Some(at);
        } else if let Some(have) = self.batch_at {
            self.batch_at = Some(have.join(at));
        }
        self.batch.push(byte);
        self.read_count += 1;
        if self.batch.len() >= DATA_BATCH {
            self.flush(op).await;
        }
    }

    async fn flush(&mut self, op: OpId) {
        if self.batch.is_empty() {
            return;
        }
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BATCH));
        let at = self.batch_at.take().unwrap_or(Provenance::at(Tick::ZERO));
        self.out
            .emit(
                at,
                NorEvent::Data {
                    op,
                    offset: self.batch_offset,
                    bytes,
                },
            )
            .await;
    }

    async fn end(&mut self, op: OpId, reason: NorEnd, at: Provenance) {
        self.flush(op).await;
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

/// The body of the decoder: one `READ` transaction.
async fn transaction(mut tx: Transaction<'_>, out: &mut NorOut) -> Result<(), ReadError> {
    let opcode = tx.read_u8(Line::Mosi).await?;
    if opcode != READ {
        out.unknown_command(tx.id(), opcode, tx.consumed()).await;
        tx.drain().await?;
        return Ok(());
    }
    let op = out.begin(Command::Read, tx.id(), tx.consumed()).await;
    let address = tx.read_u24_be(Line::Mosi).await?;
    out.address(op, address, tx.consumed()).await;
    loop {
        match tx.read_u8_or_end(Line::Miso).await? {
            Read::End => {
                out.end(op, NorEnd::Complete, tx.consumed()).await;
                return Ok(());
            }
            Read::Value(byte) => out.data(op, byte, tx.consumed()).await,
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

async fn run(mut bus: SpiBus, mut out: NorOut) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                // A transaction whose beginning was not seen cannot be read: its first word
                // is not the opcode.
                let result = if tx.is_partial() {
                    let mut tx = tx;
                    tx.drain().await
                } else {
                    transaction(tx, &mut out).await
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
    fn serializes_as_the_contract_spells_it() {
        let event = NorEvent::Begin {
            op: OpId(1),
            tx: TxId(4),
            command: Command::Read,
        };
        assert_eq!(
            serde_json::to_string(&event).expect("serializes"),
            r#"{"kind":"begin","op":1,"tx":4,"command":"read"}"#
        );
        let event = NorEvent::Data {
            op: OpId(1),
            offset: 32,
            bytes: vec![1, 2],
        };
        assert_eq!(
            serde_json::to_string(&event).expect("serializes"),
            r#"{"kind":"data","op":1,"offset":32,"bytes":[1,2]}"#
        );
        let event = NorEvent::End {
            op: OpId(1),
            reason: NorEnd::Incomplete,
        };
        assert_eq!(
            serde_json::to_string(&event).expect("serializes"),
            r#"{"kind":"end","op":1,"reason":"incomplete"}"#
        );
        let event = NorEvent::UnknownCommand {
            tx: TxId(4),
            opcode: 0x9f,
        };
        assert_eq!(
            serde_json::to_string(&event).expect("serializes"),
            r#"{"kind":"unknown-command","tx":4,"opcode":159}"#
        );
    }
}
