// SPDX-License-Identifier: Apache-2.0

//! The nRF24L01+ decoder node: the body that reads the command one transaction carries, and
//! the recovery around it.

use uiharu_core::{
    InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec,
    ProgressSink, Props, Provenance, Requires, Tick, TickRange,
};

use super::commands::{ACTIVATE, ACTIVATE_KEY, Decoded, Payload, decode};
use super::events::{Command, DATA_BATCH, Direction, EndReason, Nrf24Event, OpId, Register};
use crate::spi::{Cut, Next, Read, SpiBus, SpiEvent, Transaction, TxId, required};

/// The nRF24L01+ decoder node: `spi.events` in, `nrf24l01.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nrf24l01;

impl Nrf24l01 {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The SPI input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<SpiEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Nrf24Event> {
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

impl Node for Nrf24l01 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "nrf24l01".into(),
            inputs: vec![InputSpec::new::<SpiEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<Nrf24Event>("events", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let out = io.output(0);
        let bus = SpiBus::new(io.input(0), out.progress_sink());
        Box::pin(run(bus, Out::new(out)))
    }
}

/// The typed writer the body uses, which also remembers what is open.
#[derive(Debug)]
struct Out {
    out: Outbox<Nrf24Event>,
    sink: ProgressSink,
    next_op: u64,
    open: Option<OpId>,
    /// At most [`DATA_BATCH`] bytes waiting to go out, and which way they moved.
    batch: Vec<u8>,
    batch_direction: Direction,
    batch_offset: u32,
    batch_at: Option<Provenance>,
    /// The data bytes of the open operation so far.
    count: u32,
}

impl Out {
    /// Tells the cursor from where this writer holds items it has not reported yet.
    fn sync_hold(&self) {
        self.sink.hold(self.batch_at.map(|at| at.span.start));
    }

    fn new(out: Outbox<Nrf24Event>) -> Self {
        Self {
            sink: out.progress_sink(),
            out,
            next_op: 1,
            open: None,
            batch: Vec::with_capacity(DATA_BATCH),
            batch_direction: Direction::In,
            batch_offset: 0,
            batch_at: None,
            count: 0,
        }
    }

    async fn command(&mut self, tx: TxId, decoded: Decoded, status: u8, at: Provenance) -> OpId {
        let op = OpId(self.next_op);
        self.next_op += 1;
        self.open = Some(op);
        self.count = 0;
        let event = Nrf24Event::Command {
            op,
            tx,
            command: decoded.command,
            register: decoded.register,
            register_name: decoded.register.and_then(Register::from_address),
            pipe: decoded.pipe,
            status,
        };
        self.out.emit(at, event).await;
        op
    }

    async fn unknown(&mut self, tx: TxId, command: u8, status: u8, at: Provenance) {
        let event = Nrf24Event::Unknown {
            tx,
            command,
            status,
        };
        self.out.emit(at, event).await;
    }

    /// Adds a byte to the batch, which goes out once it holds [`DATA_BATCH`] bytes.
    async fn push(&mut self, op: OpId, direction: Direction, byte: u8, at: Provenance) {
        if self.batch.is_empty() {
            self.batch_direction = direction;
            self.batch_offset = self.count;
            self.batch_at = Some(at);
        } else if let Some(have) = self.batch_at {
            self.batch_at = Some(have.join(at));
        }
        self.batch.push(byte);
        self.sync_hold();
        self.count = self.count.saturating_add(1);
        if self.batch.len() >= DATA_BATCH {
            self.flush(op, None).await;
        }
    }

    /// Sends the batch, if any. A batch is settled when it is decided: by its last byte when
    /// the command takes no more (`decided` is `None`), by the end that cut it short
    /// otherwise.
    async fn flush(&mut self, op: OpId, decided: Option<Tick>) {
        if self.batch.is_empty() {
            return;
        }
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BATCH));
        let mut at = self
            .batch_at
            .take()
            .unwrap_or(Provenance::at(decided.unwrap_or(Tick::ZERO)));
        self.sync_hold();
        if let Some(decided) = decided {
            at.settled = at.settled.max(decided);
        }
        let event = Nrf24Event::Data {
            op,
            direction: self.batch_direction,
            offset: self.batch_offset,
            bytes,
        };
        self.out.emit(at, event).await;
    }

    async fn end(&mut self, op: OpId, reason: EndReason, at: Provenance) {
        self.flush(op, Some(at.settled)).await;
        self.open = None;
        self.out.emit(at, Nrf24Event::End { op, reason }).await;
    }

    /// Ends the open operation, if any, with `reason`.
    async fn close_open(&mut self, reason: EndReason, at: Provenance) {
        if let Some(op) = self.open {
            self.end(op, reason, at).await;
        }
    }
}

/// The body of the decoder: the command one transaction carries. The first word is the
/// command byte on MOSI and STATUS on MISO; the words after it are the command's data, in
/// the direction the command says.
async fn transaction(mut tx: Transaction<'_>, out: &mut Out) -> Result<(), Cut> {
    let first = tx.item().await?;
    let (byte, status) = (required(first.mosi)?, required(first.miso)?);
    // Chip select's assertion through the command byte.
    let head = tx.consumed();
    if byte == ACTIVATE {
        return activate(tx, out, status, head).await;
    }
    let Some(decoded) = decode(byte) else {
        out.unknown(tx.id(), byte, status, head).await;
        return tx.drain().await.map_err(Cut::from);
    };
    let op = out.command(tx.id(), decoded, status, head).await;
    let payload = decoded.command.payload(decoded.register);
    data(&mut tx, out, op, payload, 0).await
}

/// ACTIVATE's byte: the command when its key follows, an unknown byte otherwise, decided by
/// the next word or by the transaction's end.
async fn activate(
    mut tx: Transaction<'_>,
    out: &mut Out,
    status: u8,
    head: Provenance,
) -> Result<(), Cut> {
    // The command byte's ticks are held until the next word says what the command was.
    out.sink.hold(Some(head.span.start));
    let next = tx.item_or_end().await?;
    let at = tx.consumed();
    out.sink.hold(None);
    let decided = Provenance::over(head.span, at.settled);
    match next {
        Read::Item(word) if required(word.mosi)? == ACTIVATE_KEY => {
            let decoded = Decoded {
                command: Command::Activate,
                register: None,
                pipe: None,
            };
            let op = out.command(tx.id(), decoded, status, decided).await;
            out.push(op, Direction::In, ACTIVATE_KEY, at).await;
            let payload = Command::Activate.payload(None);
            out.flush(op, None).await;
            data(&mut tx, out, op, payload, 1).await
        }
        Read::Item(_) => {
            out.unknown(tx.id(), ACTIVATE, status, decided).await;
            tx.drain().await.map_err(Cut::from)
        }
        Read::End => {
            out.unknown(tx.id(), ACTIVATE, status, decided).await;
            Ok(())
        }
    }
}

/// The command's data after the `read` bytes taken already, up to the transaction's end:
/// `complete` once the command has its bytes, `incomplete` before, `too-long` at the first
/// byte past the most it takes. Data goes out once the command takes no more, or with its
/// end.
async fn data(
    tx: &mut Transaction<'_>,
    out: &mut Out,
    op: OpId,
    payload: Payload,
    read: u8,
) -> Result<(), Cut> {
    let mut count = read;
    loop {
        let word = match tx.item_or_end().await? {
            Read::End => {
                let reason = if count < payload.min {
                    EndReason::Incomplete
                } else {
                    EndReason::Complete
                };
                out.end(op, reason, tx.consumed()).await;
                return Ok(());
            }
            Read::Item(word) => word,
        };
        let direction = match payload.direction {
            Some(direction) if count < payload.max => direction,
            _ => {
                // The command takes no more: the rest of the transaction is skipped.
                out.end(op, EndReason::TooLong, tx.consumed()).await;
                return tx.drain().await.map_err(Cut::from);
            }
        };
        let byte = match direction {
            Direction::In => required(word.mosi)?,
            Direction::Out => required(word.miso)?,
        };
        out.push(op, direction, byte, tx.consumed()).await;
        count += 1;
        if count == payload.max {
            out.flush(op, None).await;
        }
    }
}

/// Closes what the body left open, as the transaction's end calls for. The end covers the
/// words the body read without reporting them, and the transaction's end when the stream
/// carried one; a gap or the end of the stream only places it.
async fn recover(error: Cut, bus: &mut SpiBus, out: &mut Out) {
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
        Cut::Incomplete => (EndReason::Incomplete, unreported),
        Cut::Truncated | Cut::Missing => (EndReason::Aborted, unreported),
        Cut::Gap(range) => (EndReason::Aborted, placed(range.start)),
        Cut::End(tick) => (EndReason::Aborted, placed(tick)),
    };
    out.close_open(reason, at).await;
}

async fn run(mut bus: SpiBus, mut out: Out) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                // A transaction whose beginning was not seen cannot be read: its first word
                // is not the command, and its words may not be aligned on bytes.
                let result = if tx.is_partial() {
                    let mut tx = tx;
                    tx.drain().await.map_err(Cut::from)
                } else {
                    transaction(tx, &mut out).await
                };
                if let Err(error) = result {
                    recover(error, &mut bus, &mut out).await;
                }
                if let Err(error) = bus.finish().await {
                    recover(Cut::from(error), &mut bus, &mut out).await;
                }
                bus.consumed();
            }
            Next::Gap(range) => {
                out.close_open(EndReason::Aborted, Provenance::at(range.start))
                    .await;
                out.out.gap(range).await;
            }
            Next::Progress(tick) => out.out.progress(tick).await,
            Next::End(tick) => {
                out.close_open(EndReason::Aborted, Provenance::at(tick))
                    .await;
                out.out.end(tick).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spi::SpiConfig;

    #[test]
    fn the_bus_must_be_framed_bytes_on_both_lines() {
        let spec = Nrf24l01::new().spec();
        assert_eq!(spec.name, "nrf24l01");
        assert_eq!(spec.inputs[0].name, "bus");
        assert_eq!(spec.outputs[0].name, "events");
        let requires = Nrf24l01::requires();
        assert!(requires.unmet(&SpiConfig::default().props()).is_none());
        for bus in [
            SpiConfig {
                cs: None,
                ..SpiConfig::default()
            },
            SpiConfig {
                miso: None,
                ..SpiConfig::default()
            },
            SpiConfig {
                word_bits: 4,
                ..SpiConfig::default()
            },
        ] {
            assert!(requires.unmet(&bus.props()).is_some(), "{bus:?}");
        }
    }
}
