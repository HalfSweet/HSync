// SPDX-License-Identifier: Apache-2.0

//! 24xx-series I²C EEPROM commands over `i2c.events`, decoded as they stream; implemented from
//! the Microchip 24AA256/24LC256/24FC256 and AT24C32 datasheets (device addressing, byte write,
//! page write, current address read, random read, sequential read, acknowledge polling).
//!
//! The decoder follows the shape a DSL decoder takes: a `transaction` body reads each I²C
//! transaction through the cursor, segment by segment — an address and what follows it up to
//! the next repeated START or the STOP — and writes typed events; the recovery around it
//! closes whatever the body had open when the transaction ends early, is cut off or runs into
//! a gap.
//!
//! The rules:
//!
//! - the control byte is the device's 7-bit address `1010 xxx` (`0x50`–`0x57`) and R/W̄. The
//!   decoder follows the configured `device_address`, or any of the eight; a segment addressed
//!   to another device, or with a 10-bit address, is skipped, and so is a partial transaction,
//!   whose bytes cannot be aligned;
//! - a write (R/W̄ = 0) carries the word address, `address_bytes` bytes, most significant
//!   first, then data: no data byte makes a set-address, one a byte write, more a page write.
//!   The command is told as soon as it is certain — a page write at its second data byte, a
//!   byte write or a set-address where the write ends — and its word address and held byte
//!   follow its `begin`, settled there;
//! - a STOP ends a write complete. A repeated START ends a set-address complete as well — it
//!   may be the first half of a random read — but a byte or page write incomplete: the device
//!   programs nothing that a STOP did not end;
//! - a read (R/W̄ = 1) is the second half of a random read when a set-address to the same
//!   device and a repeated START came right before it, and a current-address read otherwise; a
//!   sequential read is a read of more than one byte. A read ends at the STOP or repeated
//!   START after it, complete if it read a byte and incomplete if not;
//! - a control byte that is not acknowledged is an acknowledge poll that found the device busy
//!   with a write cycle (or absent), `ack-poll { acked: false }`, and the device takes no part
//!   in the rest of the segment. A write control byte that is acknowledged and followed at
//!   once by a STOP or repeated START is the poll that found it ready,
//!   `ack-poll { acked: true }`; one followed by a word address begins a command, and is
//!   reported as that command;
//! - a write whose word address a STOP or repeated START cuts short fits no command:
//!   `unknown`;
//! - a gap or the end of the samples while a command is under way ends it `aborted`; a write
//!   cut off before its command could be told is reported as the command `write`, with what
//!   was seen of it, and a set-address that a repeated START had already ended stays a
//!   complete set-address;
//! - data goes out in batches of at most [`DATA_BATCH`] bytes, each as soon as it is full, so
//!   a long read or page write shows up while it is still under way.
//!
//! Word addresses are reported as sent: the wrap-around within a page, the roll-over of the
//! address counter and the block-select bits some small parts take from the control byte are
//! not modelled (the control byte's address is in each `begin`). Acknowledge bits after the
//! control byte are not interpreted.

use serde::{Deserialize, Serialize};
use uiharu_core::{
    InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec,
    Props, Provenance, Requires, Tick, TickRange,
};

use super::events::{Command, DATA_BATCH, EepromEnd, EepromEvent, OpId};
use crate::i2c::{Address, I2cBus, I2cEvent, Next, Read, ReadError, Transaction, TxEvent, TxId};

/// The bits of a 7-bit address that select a 24xx EEPROM, `1010 xxx`.
const DEVICE_MASK: u8 = 0b111_1000;

/// The address group of 24xx EEPROMs.
const DEVICE_GROUP: u8 = 0b101_0000;

/// Which device the decoder follows and how wide its word addresses are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Eeprom24xxConfig {
    /// Bytes in a word address: 1 for parts up to 16 Kbit, 2 for 32 Kbit and above.
    pub address_bytes: u8,
    /// The device's 7-bit address; `None` follows any of `0x50`–`0x57`.
    pub device_address: Option<u8>,
}

impl Default for Eeprom24xxConfig {
    fn default() -> Self {
        Self {
            address_bytes: 2,
            device_address: None,
        }
    }
}

impl Eeprom24xxConfig {
    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), Eeprom24xxConfigError> {
        if !(1..=2).contains(&self.address_bytes) {
            return Err(Eeprom24xxConfigError::AddressBytes(self.address_bytes));
        }
        if let Some(address) = self.device_address
            && address > 0x7f
        {
            return Err(Eeprom24xxConfigError::DeviceAddress(address));
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new().with("addressBytes", i64::from(self.address_bytes))
    }

    /// Whether the 7-bit `address` is a device the decoder follows.
    pub fn follows(&self, address: u8) -> bool {
        match self.device_address {
            Some(device) => address == device,
            None => address & DEVICE_MASK == DEVICE_GROUP,
        }
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Eeprom24xxConfigError {
    /// Word addresses are one or two bytes.
    #[error("{0} bytes per word address; 24xx EEPROMs take 1 or 2")]
    AddressBytes(u8),
    /// The device address does not fit in seven bits.
    #[error("{0:#04x} is not a 7-bit address")]
    DeviceAddress(u8),
}

/// The 24xx EEPROM decoder node: `i2c.events` in, `eeprom24xx.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eeprom24xx {
    config: Eeprom24xxConfig,
}

impl Eeprom24xx {
    /// A decoder for `config`.
    pub fn new(config: Eeprom24xxConfig) -> Result<Self, Eeprom24xxConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &Eeprom24xxConfig {
        &self.config
    }

    /// The I²C input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<I2cEvent> {
        InputRef::of_node(node, 0)
    }

    /// The operations output of the decoder added as `node`.
    pub fn operations(node: NodeId) -> OutputRef<EepromEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the I²C stream it reads: transactions framed by START and
    /// STOP conditions.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("bus", "i2c")
            .equals("framing", "start-stop")
    }
}

impl Node for Eeprom24xx {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "eeprom24xx".into(),
            inputs: vec![InputSpec::new::<I2cEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<EepromEvent>(
                "operations",
                self.config.props(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let bus = I2cBus::new(io.input(0));
        Box::pin(run(bus, EepromOut::new(io.output(0), self.config)))
    }
}

/// The operation under way, once its command is told.
#[derive(Debug)]
struct Open {
    op: OpId,
    /// Data bytes so far.
    length: u32,
}

/// A write whose command is not told yet: what follows its word address tells.
#[derive(Debug)]
struct PendingWrite {
    tx: TxId,
    device: u8,
    /// The START or repeated START before it and its control byte.
    control: Provenance,
    /// The word address, once whole, and the ticks of its bytes.
    word: Option<(u16, Provenance)>,
    /// The first data byte, held until the second one or the write's end tells the command.
    first: Option<(u8, Provenance)>,
    /// The repeated START that ended it after its word address: the address after it tells a
    /// random read from a set-address.
    restart: Option<Provenance>,
}

/// The typed writer the body uses, which also remembers what is open.
#[derive(Debug)]
struct EepromOut {
    out: Outbox<EepromEvent>,
    config: Eeprom24xxConfig,
    next_op: u64,
    open: Option<Open>,
    write: Option<PendingWrite>,
    batch: Vec<u8>,
    batch_offset: u32,
    batch_at: Option<Provenance>,
}

impl EepromOut {
    fn new(out: Outbox<EepromEvent>, config: Eeprom24xxConfig) -> Self {
        Self {
            out,
            config,
            next_op: 1,
            open: None,
            write: None,
            batch: Vec::with_capacity(DATA_BATCH),
            batch_offset: 0,
            batch_at: None,
        }
    }

    /// The device `address` selects, if the decoder follows it.
    fn device(&self, address: &Address) -> Option<u8> {
        let device = u8::try_from(address.address)
            .ok()
            .filter(|_| !address.ten_bit)?;
        self.config.follows(device).then_some(device)
    }

    async fn begin(&mut self, command: Command, tx: TxId, device: u8, at: Provenance) -> OpId {
        let op = OpId(self.next_op);
        self.next_op += 1;
        self.open = Some(Open { op, length: 0 });
        let begin = EepromEvent::Begin {
            op,
            tx,
            device,
            command,
        };
        self.out.emit(at, begin).await;
        op
    }

    async fn address(&mut self, op: OpId, word_address: u16, at: Provenance) {
        self.out
            .emit(at, EepromEvent::Address { op, word_address })
            .await;
    }

    /// Adds a byte to the open operation's batch, which goes out once it holds
    /// [`DATA_BATCH`] bytes.
    async fn data(&mut self, byte: u8, at: Provenance) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if self.batch.is_empty() {
            self.batch_offset = open.length;
            self.batch_at = Some(at);
        } else if let Some(have) = self.batch_at {
            self.batch_at = Some(have.join(at));
        }
        self.batch.push(byte);
        open.length += 1;
        let op = open.op;
        if self.batch.len() >= DATA_BATCH {
            self.flush(op, at.settled).await;
        }
    }

    /// Sends the batch, if any. A batch is settled when it is decided: by its last byte when
    /// it is full, by the operation's end otherwise, so that the stream's events settle in
    /// order.
    async fn flush(&mut self, op: OpId, decided: Tick) {
        if self.batch.is_empty() {
            return;
        }
        let bytes = std::mem::replace(&mut self.batch, Vec::with_capacity(DATA_BATCH));
        let mut at = self.batch_at.take().unwrap_or(Provenance::at(decided));
        at.settled = at.settled.max(decided);
        let data = EepromEvent::Data {
            op,
            offset: self.batch_offset,
            bytes,
        };
        self.out.emit(at, data).await;
    }

    /// Ends the open operation, if any.
    async fn end(&mut self, reason: EepromEnd, at: Provenance) {
        let Some(open) = self.open.take() else {
            return;
        };
        self.flush(open.op, at.settled).await;
        let end = EepromEvent::End {
            op: open.op,
            reason,
            length: open.length,
        };
        self.out.emit(at, end).await;
    }

    /// Ends the open read: complete if it read a byte, incomplete if not.
    async fn end_read(&mut self, at: Provenance) {
        let read = self.open.as_ref().is_some_and(|open| open.length > 0);
        let reason = if read {
            EepromEnd::Complete
        } else {
            EepromEnd::Incomplete
        };
        self.end(reason, at).await;
    }

    async fn ack_poll(&mut self, tx: TxId, device: u8, acked: bool, at: Provenance) {
        self.out
            .emit(at, EepromEvent::AckPoll { tx, device, acked })
            .await;
    }

    /// A write to `device` begins; its control byte was acknowledged.
    fn open_write(&mut self, tx: TxId, device: u8, control: Provenance) {
        self.write = Some(PendingWrite {
            tx,
            device,
            control,
            word: None,
            first: None,
            restart: None,
        });
    }

    /// The write ended right after its control byte: the acknowledge poll that found the
    /// device ready.
    async fn polled(&mut self, at: Provenance) {
        if let Some(write) = self.write.take() {
            self.ack_poll(write.tx, write.device, true, write.control.join(at))
                .await;
        }
    }

    /// The write ended before its word address was whole: it fits no command.
    async fn short_address(&mut self, at: Provenance) {
        if let Some(write) = self.write.take() {
            let unknown = EepromEvent::Unknown {
                tx: write.tx,
                device: write.device,
            };
            self.out.emit(write.control.join(at), unknown).await;
        }
    }

    /// The write's word address is whole.
    fn word_address(&mut self, word: u16, at: Provenance) {
        if let Some(write) = self.write.as_mut() {
            write.word = Some((word, at));
        }
    }

    /// Reports the pending write as `command`, told at `decided`: its begin, its word address
    /// and the data byte it held.
    async fn tell(&mut self, command: Command, decided: Tick) {
        let Some(write) = self.write.take() else {
            return;
        };
        let at = Provenance::over(write.control.span, decided);
        let op = self.begin(command, write.tx, write.device, at).await;
        if let Some((word, at)) = write.word {
            self.address(op, word, Provenance::over(at.span, decided))
                .await;
        }
        if let Some((byte, at)) = write.first {
            self.data(byte, Provenance::over(at.span, decided)).await;
        }
    }

    /// A data byte of a write: the first is held until the second tells a page write.
    async fn write_data(&mut self, byte: u8, at: Provenance) {
        if let Some(write) = self.write.as_mut() {
            if write.first.is_none() {
                write.first = Some((byte, at));
                return;
            }
            self.tell(Command::PageWrite, at.settled).await;
        }
        self.data(byte, at).await;
    }

    /// The write ended after its word address: at a STOP (`stop`), or at a repeated START.
    async fn end_write(&mut self, stop: bool, at: Provenance) {
        let held = self.write.as_ref().map(|write| write.first.is_some());
        match held {
            Some(false) if !stop => {
                // A set-address, unless the read after the repeated START makes it the first
                // half of a random read.
                if let Some(write) = self.write.as_mut() {
                    write.restart = Some(at);
                }
                return;
            }
            Some(false) => self.tell(Command::SetAddress, at.settled).await,
            Some(true) => self.tell(Command::ByteWrite, at.settled).await,
            None => {}
        }
        // The device programs data only when a STOP ends the write.
        let reason = if stop {
            EepromEnd::Complete
        } else {
            EepromEnd::Incomplete
        };
        self.end(reason, at).await;
    }

    /// Whether a set-address to `device`, ended by a repeated START, waits for the address
    /// after it.
    fn awaits_read(&self, device: u8) -> bool {
        self.write
            .as_ref()
            .is_some_and(|write| write.restart.is_some() && write.device == device)
    }

    /// The read after a set-address and a repeated START: a random read, told at its control
    /// byte, read with `at`.
    async fn random_read(&mut self, at: Provenance) {
        let Some(write) = self.write.take() else {
            return;
        };
        let begin = Provenance::over(write.control.span.join(at.span), at.settled);
        let op = self
            .begin(Command::RandomRead, write.tx, write.device, begin)
            .await;
        if let Some((word, word_at)) = write.word {
            self.address(op, word, Provenance::over(word_at.span, at.settled))
                .await;
        }
    }

    /// Reports a set-address that a repeated START ended as just that, once what came after
    /// showed that it began no random read; `decided` is where that became certain.
    async fn settle(&mut self, decided: Tick) {
        let Some(restart) = self.write.as_ref().and_then(|write| write.restart) else {
            return;
        };
        self.tell(Command::SetAddress, decided).await;
        self.end(EepromEnd::Complete, Provenance::over(restart.span, decided))
            .await;
    }

    /// Closes what the transaction left open, as `reason` says, when it ended early, was cut
    /// off or ran into a gap.
    async fn close(&mut self, reason: EepromEnd, at: Provenance) {
        let pending = self
            .write
            .as_ref()
            .map(|write| (write.restart.is_some(), write.word.is_some()));
        match pending {
            // The set-address had ended at its repeated START; what was lost came after it.
            Some((true, _)) => self.settle(at.settled).await,
            // A STOP before the word address was whole.
            Some((false, false)) if reason == EepromEnd::Incomplete => {
                self.short_address(at).await;
            }
            // Cut off before its command could be told.
            Some(_) => self.tell(Command::Write, at.settled).await,
            None => {}
        }
        self.end(reason, at).await;
    }
}

/// How a segment of a transaction — an address and what follows it — ended.
#[derive(Clone, Copy, Debug)]
enum Boundary {
    /// The transaction's normal end.
    End,
    /// A repeated START: an address comes next.
    Restart,
    /// An address with no repeated START before it, and its ticks. The I²C decoder never sends
    /// one, but the contract's readers do not rely on that.
    Address(Address, Provenance),
}

/// What the next read of a segment found.
enum Step {
    /// A data byte.
    Data(u8, Provenance),
    /// What ended the segment, and the ticks of the end that are the segment's own.
    Ended(Boundary, Provenance),
}

/// Reads the next event of a segment.
async fn step(tx: &mut Transaction<'_>) -> Result<Step, ReadError> {
    Ok(match tx.event_or_end().await? {
        Read::Event(TxEvent::Data { byte, .. }) => Step::Data(byte, tx.consumed()),
        Read::End => Step::Ended(Boundary::End, tx.consumed()),
        Read::Event(TxEvent::Restart { .. }) => Step::Ended(Boundary::Restart, tx.consumed()),
        Read::Event(TxEvent::Address(address)) => {
            // The address belongs to the next segment; this one ends where it begins.
            let at = tx.consumed();
            Step::Ended(
                Boundary::Address(address, at),
                Provenance::at(at.span.start),
            )
        }
    })
}

/// Skips the rest of a segment.
async fn skip(tx: &mut Transaction<'_>) -> Result<Boundary, ReadError> {
    loop {
        if let Step::Ended(boundary, _) = step(tx).await? {
            return Ok(boundary);
        }
    }
}

/// The data bytes of a read, up to the end of its segment.
async fn read(tx: &mut Transaction<'_>, out: &mut EepromOut) -> Result<Boundary, ReadError> {
    loop {
        match step(tx).await? {
            Step::Data(byte, at) => out.data(byte, at).await,
            Step::Ended(boundary, at) => {
                out.end_read(at).await;
                return Ok(boundary);
            }
        }
    }
}

/// A write whose control byte was acknowledged: its word address and data, up to the end of
/// its segment.
async fn write(tx: &mut Transaction<'_>, out: &mut EepromOut) -> Result<Boundary, ReadError> {
    let (first, mut word_at) = match step(tx).await? {
        Step::Data(byte, at) => (byte, at),
        Step::Ended(boundary, at) => {
            out.polled(at).await;
            return Ok(boundary);
        }
    };
    // The rest of the word address, most significant byte first. The normal end here is an
    // error the recovery reports: the word address is not whole.
    let mut word = u16::from(first);
    for _ in 1..out.config.address_bytes {
        match tx.event().await? {
            TxEvent::Data { byte, .. } => {
                word = (word << 8) | u16::from(byte);
                word_at = word_at.join(tx.consumed());
            }
            TxEvent::Restart { .. } => {
                out.short_address(tx.consumed()).await;
                return Ok(Boundary::Restart);
            }
            TxEvent::Address(address) => {
                let at = tx.consumed();
                out.short_address(Provenance::at(at.span.start)).await;
                return Ok(Boundary::Address(address, at));
            }
        }
    }
    out.word_address(word, word_at);
    loop {
        match step(tx).await? {
            Step::Data(byte, at) => out.write_data(byte, at).await,
            Step::Ended(boundary, at) => {
                out.end_write(matches!(boundary, Boundary::End), at).await;
                return Ok(boundary);
            }
        }
    }
}

/// One segment: `address`, which was read with `at`, and what follows it.
async fn segment(
    tx: &mut Transaction<'_>,
    out: &mut EepromOut,
    address: Address,
    at: Provenance,
) -> Result<Boundary, ReadError> {
    let device = out.device(&address);
    if let Some(device) = device
        && address.read
        && address.ack
        && out.awaits_read(device)
    {
        out.random_read(at).await;
        return read(tx, out).await;
    }
    // A set-address waiting for this address was only that.
    out.settle(at.settled).await;
    let Some(device) = device else {
        return skip(tx).await;
    };
    if !address.ack {
        out.ack_poll(tx.id(), device, false, at).await;
        return skip(tx).await;
    }
    if address.read {
        out.begin(Command::CurrentAddressRead, tx.id(), device, at)
            .await;
        return read(tx, out).await;
    }
    out.open_write(tx.id(), device, at);
    write(tx, out).await
}

/// The body of the decoder: one I²C transaction, segment by segment.
async fn transaction(mut tx: Transaction<'_>, out: &mut EepromOut) -> Result<(), ReadError> {
    // The START: an address comes next.
    let mut boundary = Boundary::Restart;
    loop {
        let (address, at) = match boundary {
            Boundary::Address(address, at) => (address, at),
            Boundary::End => {
                let decided = tx.consumed().settled;
                out.settle(decided).await;
                return Ok(());
            }
            Boundary::Restart => match tx.event_or_end().await? {
                Read::Event(TxEvent::Address(address)) => (address, tx.consumed()),
                Read::End => {
                    boundary = Boundary::End;
                    continue;
                }
                Read::Event(TxEvent::Restart { .. }) => {
                    let decided = tx.consumed().settled;
                    out.settle(decided).await;
                    continue;
                }
                // A byte with no address before it is no one's to read.
                Read::Event(TxEvent::Data { .. }) => {
                    tx.consumed();
                    boundary = skip(&mut tx).await?;
                    continue;
                }
            },
        };
        boundary = segment(&mut tx, out, address, at).await?;
    }
}

/// Closes what the body left open, as the transaction's end calls for. The end covers what
/// the body read without reporting it, and the transaction's end when the stream carried one;
/// a gap or the end of the stream only places it.
async fn recover(error: ReadError, bus: &mut I2cBus, out: &mut EepromOut) {
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
        ReadError::Incomplete => (EepromEnd::Incomplete, unreported),
        ReadError::Truncated => (EepromEnd::Aborted, unreported),
        ReadError::Gap(range) => (EepromEnd::Aborted, placed(range.start)),
        ReadError::EndOfInput(tick) => (EepromEnd::Aborted, placed(tick)),
    };
    out.close(reason, at).await;
}

async fn run(mut bus: I2cBus, mut out: EepromOut) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                // A transaction whose beginning was not seen carries no bytes to read.
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
                out.close(EepromEnd::Aborted, Provenance::at(range.start))
                    .await;
                out.out.gap(range).await;
            }
            Next::Progress(tick) => out.out.progress(tick).await,
            Next::End(tick) => {
                out.close(EepromEnd::Aborted, Provenance::at(tick)).await;
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
        assert_eq!(Eeprom24xxConfig::default().validate(), Ok(()));
        let bad = |config: Eeprom24xxConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(Eeprom24xxConfig {
                address_bytes: 3,
                ..Eeprom24xxConfig::default()
            }),
            Eeprom24xxConfigError::AddressBytes(3)
        );
        assert_eq!(
            bad(Eeprom24xxConfig {
                address_bytes: 0,
                ..Eeprom24xxConfig::default()
            }),
            Eeprom24xxConfigError::AddressBytes(0)
        );
        // The 8-bit form of the address, with R/W̄ in bit 0, is a common mistake.
        assert_eq!(
            bad(Eeprom24xxConfig {
                device_address: Some(0xa0),
                ..Eeprom24xxConfig::default()
            }),
            Eeprom24xxConfigError::DeviceAddress(0xa0)
        );
        assert!(Eeprom24xx::new(Eeprom24xxConfig::default()).is_ok());
    }

    #[test]
    fn devices_are_followed_by_address() {
        let any = Eeprom24xxConfig::default();
        assert!((0x50..=0x57).all(|address| any.follows(address)));
        assert!(!any.follows(0x4f) && !any.follows(0x58) && !any.follows(0x68));
        let one = Eeprom24xxConfig {
            device_address: Some(0x53),
            ..Eeprom24xxConfig::default()
        };
        assert!(one.follows(0x53));
        assert!(!one.follows(0x50));
    }

    #[test]
    fn props_and_requirements() {
        let props = Eeprom24xxConfig {
            address_bytes: 1,
            device_address: None,
        }
        .props();
        assert_eq!(props.get("addressBytes"), Some(&1i64.into()));
        let i2c = crate::i2c::I2cConfig::default().props();
        assert!(Eeprom24xx::requires().unmet(&i2c).is_none());
        assert_eq!(
            Eeprom24xx::requires()
                .unmet(&Props::new().with("bus", "i2c"))
                .map(|(key, _)| key),
            Some("framing")
        );
    }
}
