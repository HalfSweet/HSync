// SPDX-License-Identifier: Apache-2.0

//! The SD card decoder node: its configuration, the body that reads the operations of one
//! transaction, and the recovery around it.

use serde::{Deserialize, Serialize};
use uiharu_core::{
    InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec,
    ProgressSink, Props, Provenance, Requires, Tick, TickRange,
};

use super::commands::{
    DEFAULT_BLOCK_LENGTH, Length, MAX_BLOCK_LENGTH, R1_ERRORS, R1_REFUSED, Transfer,
    is_application, shape, tail,
};
use super::crc::{Crc7, Crc16};
use super::events::{DATA_BATCH, OpId, ResponseFormat, SdEnd, SdEvent, Token};
use crate::spi::{Next, Read, ReadError, SpiBus, SpiEvent, Transaction, TxId};

/// What a line carries when nothing is sent on it.
const FILL: u8 = 0xff;
/// The token before a data block, and before the block of a single-block write.
const START_BLOCK: u8 = 0xfe;
/// The token before each block of a multiple-block write.
const START_MULTI_BLOCK: u8 = 0xfc;
/// The token that ends a multiple-block write.
const STOP_TRAN: u8 = 0xfd;
/// What MISO carries while the card is busy.
const BUSY: u8 = 0x00;

/// Whether a byte from the host begins a command frame: a start bit of 0, then a
/// transmission bit of 1.
const fn starts_command(byte: u8) -> bool {
    byte & 0xc0 == 0x40
}

/// Whether a byte from the card is a data error token: `0000xxxx` with a flag set.
const fn is_error_token(byte: u8) -> bool {
    byte != 0 && byte & 0xf0 == 0
}

/// Whether a byte from the card is a data response token: `xxx0sss1`.
const fn is_data_response(byte: u8) -> bool {
    byte & 0x11 == 0x01
}

/// How the decoder reads a card's traffic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SdCardSpiConfig {
    /// The block length of reads and writes until a CMD16 changes it: 512 bytes for every
    /// card after power-up.
    pub block_length: u16,
}

impl Default for SdCardSpiConfig {
    fn default() -> Self {
        Self {
            block_length: DEFAULT_BLOCK_LENGTH,
        }
    }
}

impl SdCardSpiConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), SdCardSpiConfigError> {
        if (1..=MAX_BLOCK_LENGTH).contains(&self.block_length) {
            Ok(())
        } else {
            Err(SdCardSpiConfigError::BlockLength(self.block_length))
        }
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("blockLength", i64::from(self.block_length))
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SdCardSpiConfigError {
    /// The block length is out of range.
    #[error("blocks of {0} bytes; 1 to {MAX_BLOCK_LENGTH} bytes are supported")]
    BlockLength(u16),
}

/// The SD card decoder node: `spi.events` in, `sdcard.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdCardSpi {
    config: SdCardSpiConfig,
}

impl SdCardSpi {
    /// A decoder for `config`.
    pub fn new(config: SdCardSpiConfig) -> Result<Self, SdCardSpiConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &SdCardSpiConfig {
        &self.config
    }

    /// The SPI input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<SpiEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<SdEvent> {
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

impl Node for SdCardSpi {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "sdcard-spi".into(),
            inputs: vec![InputSpec::new::<SpiEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<SdEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let out = io.output(0);
        let bus = SpiBus::new(io.input(0), out.progress_sink());
        let card = Card::new(self.config.block_length);
        Box::pin(run(bus, SdOut::new(out), card))
    }
}

/// What the decoder knows of the card from one command to the next.
#[derive(Debug)]
struct Card {
    /// The length of the blocks of reads and writes.
    block_length: u16,
    /// Whether a CMD55 the card did not refuse makes the next command an application one.
    app_cmd: bool,
}

impl Card {
    fn new(block_length: u16) -> Self {
        Self {
            block_length,
            app_cmd: false,
        }
    }

    fn length(&self, length: Length) -> u16 {
        match length {
            Length::Block => self.block_length,
            Length::Fixed(length) => length,
        }
    }

    /// Forgets the CMD55 a gap separates from the next command. The block length stays: the
    /// card keeps it, and a CMD16 hidden in a gap is rarer than the transfers that make up most
    /// of the traffic.
    fn forget(&mut self) {
        self.app_cmd = false;
    }
}

/// An R1b response waiting for the card to turn ready.
#[derive(Debug)]
struct Busy {
    r1: u8,
    count: u16,
    at: Provenance,
}

/// The typed writer the body uses, which also remembers what is open. It outlives a body that
/// returns early, so that the recovery can close what the body opened.
#[derive(Debug)]
struct SdOut {
    out: Outbox<SdEvent>,
    sink: ProgressSink,
    next_op: u64,
    open: Option<OpId>,
    /// The words read toward an event not sent yet; an end that comes first covers them.
    held: Option<Provenance>,
    busy: Option<Busy>,
    /// At most [`DATA_BATCH`] bytes waiting to go out.
    batch: Vec<u8>,
    batch_offset: u32,
    batch_at: Option<Provenance>,
    /// The data bytes of the open operation so far.
    count: u32,
}

impl SdOut {
    /// Tells the cursor from where this writer holds items it has not reported yet.
    fn sync_hold(&self) {
        let starts = [
            self.held,
            self.batch_at,
            self.busy.as_ref().map(|busy| busy.at),
        ];
        self.sink
            .hold(starts.into_iter().flatten().map(|at| at.span.start).min());
    }

    fn new(out: Outbox<SdEvent>) -> Self {
        Self {
            sink: out.progress_sink(),
            out,
            next_op: 1,
            open: None,
            held: None,
            busy: None,
            batch: Vec::with_capacity(DATA_BATCH),
            batch_offset: 0,
            batch_at: None,
            count: 0,
        }
    }

    fn hold(&mut self, at: Provenance) {
        self.held = Some(self.held.map_or(at, |held| held.join(at)));
        self.sync_hold();
    }

    /// What is held, joined with `at`.
    fn take_held(&mut self, at: Provenance) -> Provenance {
        let at = self.held.take().map_or(at, |held| held.join(at));
        self.sync_hold();
        at
    }

    /// What is held; `last`, the last word held, when nothing is.
    fn take_held_or(&mut self, last: Provenance) -> Provenance {
        let at = self.held.take().unwrap_or(last);
        self.sync_hold();
        at
    }

    async fn command(&mut self, tx: TxId, frame: Frame, acmd: bool, at: Provenance) -> OpId {
        let op = OpId(self.next_op);
        self.next_op += 1;
        self.open = Some(op);
        self.count = 0;
        let event = SdEvent::Command {
            op,
            tx,
            index: frame.index,
            acmd,
            argument: frame.argument,
            crc7: frame.crc7,
            crc_ok: frame.crc_ok,
        };
        self.out.emit(at, event).await;
        op
    }

    async fn response(&mut self, op: OpId, format: ResponseFormat, bytes: &[u8], at: Provenance) {
        let event = SdEvent::Response {
            op,
            format,
            bytes: bytes.to_vec(),
            busy_bytes: 0,
        };
        self.out.emit(at, event).await;
    }

    /// Sends the R1b response once the busy period is over, as decided at `decided`.
    async fn busy_over(&mut self, op: OpId, decided: Tick) {
        if let Some(busy) = self.busy.take() {
            self.sync_hold();
            let at = Provenance::over(busy.at.span, busy.at.settled.max(decided));
            let event = SdEvent::Response {
                op,
                format: ResponseFormat::R1b,
                bytes: vec![busy.r1],
                busy_bytes: busy.count,
            };
            self.out.emit(at, event).await;
        }
    }

    async fn token(&mut self, op: OpId, token: Token, value: u8, at: Provenance) {
        self.out.emit(at, SdEvent::Token { op, token, value }).await;
    }

    /// Adds a byte to the current batch, which goes out once it holds [`DATA_BATCH`] bytes.
    async fn data(&mut self, op: OpId, byte: u8, at: Provenance) {
        if self.batch.is_empty() {
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

    /// Sends the batch, if any. A batch is settled when it is decided: by its last byte when it
    /// is full or ends its block (`decided` is `None`), by the end that cut its block short
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
        let event = SdEvent::Data {
            op,
            offset: self.batch_offset,
            bytes,
        };
        self.out.emit(at, event).await;
    }

    async fn block_end(&mut self, op: OpId, crc16: u16, crc_ok: bool, length: u16, at: Provenance) {
        let event = SdEvent::BlockEnd {
            op,
            crc16,
            crc_ok,
            length,
        };
        self.out.emit(at, event).await;
    }

    /// Ends `op` at `at`, after what it still holds back.
    async fn end(&mut self, op: OpId, reason: SdEnd, at: Provenance) {
        self.busy_over(op, at.settled).await;
        self.flush(op, Some(at.settled)).await;
        self.open = None;
        self.held = None;
        self.sync_hold();
        self.out.emit(at, SdEvent::End { op, reason }).await;
    }

    /// Ends `op` as decided at `decided`, covering the words held for an event that never
    /// came, if any.
    async fn end_at(&mut self, op: OpId, reason: SdEnd, decided: Tick) {
        let span = self
            .held
            .take()
            .map_or(TickRange::at(decided), |held| held.span);
        self.end(op, reason, Provenance::over(span, decided)).await;
    }

    async fn unknown_command(&mut self, tx: TxId, byte: u8, at: Provenance) {
        self.out
            .emit(at, SdEvent::UnknownCommand { tx, byte })
            .await;
    }

    /// Ends the open operation, if any, with `reason`.
    async fn close_open(&mut self, reason: SdEnd, at: Provenance) {
        match self.open {
            Some(op) => self.end(op, reason, at).await,
            None => {
                self.held = None;
                self.sync_hold();
            }
        }
    }
}

/// A command frame as the host sent it.
#[derive(Clone, Copy, Debug)]
struct Frame {
    index: u8,
    argument: u32,
    crc7: u8,
    crc_ok: bool,
}

/// What ended an operation, for the transaction it is part of.
enum After {
    /// Nothing: the transaction goes on.
    Done,
    /// The host began the next command frame with this byte, read at that provenance.
    Command(u8, Provenance),
    /// Chip select was released.
    Released,
}

/// The body of the decoder: the operations of one transaction.
async fn transaction(
    mut tx: Transaction<'_>,
    out: &mut SdOut,
    card: &mut Card,
) -> Result<(), ReadError> {
    // The assertion of chip select belongs to no event.
    tx.consumed();
    let mut first = true;
    let mut next = None;
    loop {
        let (byte, at) = match next.take() {
            Some(started) => started,
            None => match tx.word_or_end().await? {
                Read::End => return Ok(()),
                Read::Word(word) => (word.mosi()?, tx.consumed()),
            },
        };
        if starts_command(byte) {
            first = false;
            match operation(&mut tx, out, card, byte, at).await? {
                After::Done => {}
                After::Command(byte, at) => next = Some((byte, at)),
                After::Released => return Ok(()),
            }
        } else if first && byte != FILL {
            out.unknown_command(tx.id(), byte, at).await;
            tx.drain().await?;
            return Ok(());
        }
        // Fill, or a stray byte between operations: skipped.
    }
}

/// One operation, from the first byte of its command frame on.
async fn operation(
    tx: &mut Transaction<'_>,
    out: &mut SdOut,
    card: &mut Card,
    first: u8,
    first_at: Provenance,
) -> Result<After, ReadError> {
    // The frame: the argument, most significant byte first, then the CRC7 and the end bit.
    out.hold(first_at);
    let mut crc = Crc7::new();
    crc.update(first);
    let mut argument = 0u32;
    for _ in 0..4 {
        let byte = tx.word().await?.mosi()?;
        crc.update(byte);
        argument = (argument << 8) | u32::from(byte);
    }
    let last = tx.word().await?.mosi()?;
    let at = out.take_held(tx.consumed());
    let frame = Frame {
        index: first & 0x3f,
        argument,
        crc7: last >> 1,
        crc_ok: crc.value() == last >> 1,
    };
    let index = frame.index;
    let app = std::mem::take(&mut card.app_cmd) && is_application(index);
    let op = out.command(tx.id(), frame, app, at).await;
    // The command after CMD55 is an application command, unless the card refuses CMD55.
    card.app_cmd = index == 55 && !app;
    let shape = shape(index, app, argument);

    // R1: the first byte from the card with bit 7 clear. Before it the card sends fill; after
    // CMD12 the first byte is a stuff byte, which may still carry bits of the stopped block.
    let mut stuff = index == 12;
    let (r1, r1_at) = loop {
        let word = tx.word().await?;
        let at = tx.consumed();
        let mosi = word.mosi()?;
        if starts_command(mosi) {
            // The host gave up on the response.
            out.end_at(op, SdEnd::Incomplete, at.settled).await;
            return Ok(After::Command(mosi, at));
        }
        let miso = word.miso()?;
        if !std::mem::take(&mut stuff) && miso & 0x80 == 0 {
            break (miso, at);
        }
    };
    let refused = r1 & R1_REFUSED != 0;
    if index == 55 && refused {
        card.app_cmd = false;
    }

    if shape.format == ResponseFormat::R1b {
        out.busy = Some(Busy {
            r1,
            count: 0,
            at: r1_at,
        });
        out.sync_hold();
        return busy(tx, out, op).await;
    }
    out.hold(r1_at);
    let mut bytes = [r1, 0, 0, 0, 0];
    let len = if refused { 1 } else { 1 + tail(shape.format) };
    for byte in &mut bytes[1..len] {
        let word = tx.word().await?;
        let at = tx.consumed();
        let mosi = word.mosi()?;
        if starts_command(mosi) {
            out.end_at(op, SdEnd::Incomplete, at.settled).await;
            return Ok(After::Command(mosi, at));
        }
        *byte = word.miso()?;
        out.hold(at);
    }
    let format = if refused {
        ResponseFormat::R1
    } else {
        shape.format
    };
    let at = out.take_held_or(r1_at);
    out.response(op, format, &bytes[..len], at).await;

    // What the card did is now known.
    if !app && r1 & R1_ERRORS == 0 {
        match index {
            0 => card.block_length = DEFAULT_BLOCK_LENGTH,
            16 => {
                if let Ok(length) = u16::try_from(argument)
                    && (1..=MAX_BLOCK_LENGTH).contains(&length)
                {
                    card.block_length = length;
                }
            }
            _ => {}
        }
    }
    // A command the card refused, or failed, moves no data.
    let transfer = if r1 & R1_ERRORS == 0 {
        shape.transfer
    } else {
        Transfer::None
    };
    match transfer {
        Transfer::None => {
            out.end_at(op, SdEnd::Complete, at.settled).await;
            Ok(After::Done)
        }
        Transfer::Read(length) => read(tx, out, op, card.length(length), false).await,
        Transfer::ReadMultiple => read(tx, out, op, card.block_length, true).await,
        Transfer::Write(length) => write(tx, out, op, card.length(length), false).await,
        Transfer::WriteMultiple => write(tx, out, op, card.block_length, true).await,
    }
}

/// The busy bytes after an R1b response, until the card turns ready. The host may release
/// chip select or send its next command meanwhile; the response is whole all the same.
async fn busy(tx: &mut Transaction<'_>, out: &mut SdOut, op: OpId) -> Result<After, ReadError> {
    loop {
        let word = match tx.word_or_end().await? {
            Read::End => {
                let at = tx.consumed();
                out.end_at(op, SdEnd::Complete, at.settled).await;
                return Ok(After::Released);
            }
            Read::Word(word) => word,
        };
        let at = tx.consumed();
        let mosi = word.mosi()?;
        if starts_command(mosi) {
            out.end_at(op, SdEnd::Complete, at.settled).await;
            return Ok(After::Command(mosi, at));
        }
        if word.miso()? != BUSY {
            out.end_at(op, SdEnd::Complete, at.settled).await;
            return Ok(After::Done);
        }
        if let Some(busy) = out.busy.as_mut() {
            busy.count = busy.count.saturating_add(1);
            busy.at = busy.at.join(at);
        }
    }
}

/// The blocks of a read: one, or as many as come before the host stops them with its next
/// command. While it waits for a block the card sends fill.
async fn read(
    tx: &mut Transaction<'_>,
    out: &mut SdOut,
    op: OpId,
    length: u16,
    multiple: bool,
) -> Result<After, ReadError> {
    // The host stopping a multiple read is its normal end; a single block it gave up on is
    // not whole.
    let stopped = if multiple {
        SdEnd::Complete
    } else {
        SdEnd::Incomplete
    };
    loop {
        loop {
            let word = tx.word().await?;
            let at = tx.consumed();
            let mosi = word.mosi()?;
            if starts_command(mosi) {
                out.end_at(op, stopped, at.settled).await;
                return Ok(After::Command(mosi, at));
            }
            let miso = word.miso()?;
            if miso == START_BLOCK {
                out.token(op, Token::StartBlock, miso, at).await;
                break;
            }
            if is_error_token(miso) {
                out.token(op, Token::Error, miso, at).await;
                if !multiple {
                    out.end_at(op, SdEnd::Complete, at.settled).await;
                    return Ok(After::Done);
                }
            }
        }
        let mut crc = Crc16::new();
        for _ in 0..length {
            let word = tx.word().await?;
            let at = tx.consumed();
            let mosi = word.mosi()?;
            if starts_command(mosi) {
                out.end_at(op, stopped, at.settled).await;
                return Ok(After::Command(mosi, at));
            }
            let byte = word.miso()?;
            crc.update(byte);
            out.data(op, byte, at).await;
        }
        // The block's last batch is whole with its last byte.
        out.flush(op, None).await;
        let mut sent = 0u16;
        let mut last = Provenance::at(Tick::ZERO);
        for _ in 0..2 {
            let word = tx.word().await?;
            let at = tx.consumed();
            let mosi = word.mosi()?;
            if starts_command(mosi) {
                out.end_at(op, stopped, at.settled).await;
                return Ok(After::Command(mosi, at));
            }
            sent = (sent << 8) | u16::from(word.miso()?);
            out.hold(at);
            last = at;
        }
        let at = out.take_held_or(last);
        out.block_end(op, sent, sent == crc.value(), length, at)
            .await;
        if !multiple {
            out.end_at(op, SdEnd::Complete, at.settled).await;
            return Ok(After::Done);
        }
    }
}

/// The blocks of a write: one, or as many as the host sends before its stop token. Each is
/// answered by a data response token; the busy period after it is not part of the operation.
async fn write(
    tx: &mut Transaction<'_>,
    out: &mut SdOut,
    op: OpId,
    length: u16,
    multiple: bool,
) -> Result<After, ReadError> {
    // A multiple write the host stops with a command (CMD12 after an error) has ended; a
    // single block that never came is not whole.
    let stopped = if multiple {
        SdEnd::Complete
    } else {
        SdEnd::Incomplete
    };
    loop {
        // The host's token; MOSI carries fill before it, whatever MISO says.
        loop {
            let word = tx.word().await?;
            let at = tx.consumed();
            match word.mosi()? {
                START_BLOCK => {
                    out.token(op, Token::StartBlock, START_BLOCK, at).await;
                    break;
                }
                START_MULTI_BLOCK => {
                    let value = START_MULTI_BLOCK;
                    out.token(op, Token::StartMultiBlock, value, at).await;
                    break;
                }
                STOP_TRAN if multiple => {
                    out.token(op, Token::StopTran, STOP_TRAN, at).await;
                    out.end_at(op, SdEnd::Complete, at.settled).await;
                    return Ok(After::Done);
                }
                byte if starts_command(byte) => {
                    out.end_at(op, stopped, at.settled).await;
                    return Ok(After::Command(byte, at));
                }
                _ => {}
            }
        }
        // The block from the host: every byte is data here, whatever it looks like.
        let mut crc = Crc16::new();
        for _ in 0..length {
            let byte = tx.word().await?.mosi()?;
            crc.update(byte);
            out.data(op, byte, tx.consumed()).await;
        }
        out.flush(op, None).await;
        let mut sent = 0u16;
        for _ in 0..2 {
            sent = (sent << 8) | u16::from(tx.word().await?.mosi()?);
        }
        let at = tx.consumed();
        out.block_end(op, sent, sent == crc.value(), length, at)
            .await;
        // The card's data response.
        loop {
            let word = tx.word().await?;
            let at = tx.consumed();
            let mosi = word.mosi()?;
            if starts_command(mosi) {
                out.end_at(op, SdEnd::Incomplete, at.settled).await;
                return Ok(After::Command(mosi, at));
            }
            let miso = word.miso()?;
            if is_data_response(miso) {
                out.token(op, Token::DataResponse, miso, at).await;
                if !multiple {
                    out.end_at(op, SdEnd::Complete, at.settled).await;
                    return Ok(After::Done);
                }
                break;
            }
        }
    }
}

/// Closes what the body left open, as the transaction's end calls for. The end covers the
/// words read toward an event that never came, and the transaction's end when the stream
/// carried one; a gap or the end of the stream only places it.
async fn recover(error: ReadError, bus: &mut SpiBus, out: &mut SdOut) {
    let consumed = bus.consumed();
    let unreported = out.take_held(consumed);
    let placed = |tick: Tick| Provenance {
        span: if unreported.span.is_empty() {
            TickRange::at(tick)
        } else {
            unreported.span
        },
        settled: tick,
    };
    let (reason, at) = match error {
        ReadError::Incomplete => (SdEnd::Incomplete, unreported),
        ReadError::Truncated | ReadError::MissingLine(_) => (SdEnd::Aborted, unreported),
        ReadError::Gap(range) => (SdEnd::Aborted, placed(range.start)),
        ReadError::EndOfInput(tick) => (SdEnd::Aborted, placed(tick)),
    };
    out.close_open(reason, at).await;
}

async fn run(mut bus: SpiBus, mut out: SdOut, mut card: Card) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                // A transaction whose beginning was not seen cannot be read: its words may
                // not be aligned on bytes.
                let result = if tx.is_partial() {
                    let mut tx = tx;
                    tx.drain().await
                } else {
                    transaction(tx, &mut out, &mut card).await
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
                out.close_open(SdEnd::Aborted, Provenance::at(range.start))
                    .await;
                card.forget();
                out.out.gap(range).await;
            }
            Next::Progress(tick) => out.out.progress(tick).await,
            Next::End(tick) => {
                out.close_open(SdEnd::Aborted, Provenance::at(tick)).await;
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
        assert_eq!(SdCardSpiConfig::default().validate(), Ok(()));
        let config = |block_length| SdCardSpiConfig { block_length };
        assert_eq!(config(16).validate(), Ok(()));
        assert_eq!(config(2048).validate(), Ok(()));
        assert_eq!(
            config(0).validate(),
            Err(SdCardSpiConfigError::BlockLength(0))
        );
        assert_eq!(
            config(4096).validate(),
            Err(SdCardSpiConfigError::BlockLength(4096))
        );
        assert!(SdCardSpi::new(config(0)).is_err());
    }

    #[test]
    fn props_declare_the_block_length() {
        let props = SdCardSpiConfig { block_length: 64 }.props();
        assert_eq!(props.get("blockLength"), Some(&64i64.into()));
    }

    #[test]
    fn tokens_are_told_apart() {
        assert!(starts_command(0x40) && starts_command(0x7f));
        assert!(!starts_command(0xff) && !starts_command(0x00) && !starts_command(0xfe));
        assert!(is_error_token(0x08) && !is_error_token(0x00) && !is_error_token(0xfe));
        assert!(is_data_response(0xe5) && is_data_response(0x05) && is_data_response(0x0b));
        assert!(!is_data_response(0xff) && !is_data_response(0x00));
    }
}
