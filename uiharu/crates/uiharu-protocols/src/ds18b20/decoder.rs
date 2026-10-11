// SPDX-License-Identifier: Apache-2.0

//! The DS18B20 digital thermometer over `onewire.events`: its function commands and what they
//! carry, decoded as they stream.
//!
//! Implemented from the DS18B20 datasheet ("DS18B20 Programmable Resolution 1-Wire Digital
//! Thermometer", Maxim Integrated / Analog Devices):
//!
//! - a transaction addresses the device when its ROM command is Skip ROM, Match ROM or Read
//!   ROM with the family code `0x28`, or one of their overdrive variants (Overdrive Skip ROM,
//!   Overdrive Match ROM with `0x28`), which the datasheet does not list: the decoder reports
//!   what the master sent, and a device that ignores them leaves its read slots high. A
//!   transaction addressed to another family, a search (after which the datasheet asks for a
//!   reset) and an unknown ROM command are ignored;
//! - the first byte after the ROM level is the function command: Convert T (`0x44`), Write
//!   Scratchpad (`0x4E`), Read Scratchpad (`0xBE`), Copy Scratchpad (`0x48`), Recall E²
//!   (`0xB8`) or Read Power Supply (`0xB4`). Another code is an unknown command, and the rest
//!   of its transaction is skipped;
//! - Read Scratchpad: the nine bytes that follow are the scratchpad — temperature LSB and MSB,
//!   TH, TL, configuration, three reserved bytes and the CRC-8 of the first eight. It is
//!   reported as soon as its ninth byte is read, with the temperature register as it is (two's
//!   complement in sixteenths of a degree Celsius) and the resolution bits 5 and 6 of the
//!   configuration register set (9 to 12 bits); bytes read after the ninth are skipped. When
//!   the transaction ends before the ninth byte, the bytes read are reported as a partial
//!   scratchpad, with the temperature when both its bytes were read, before the end;
//! - Write Scratchpad: the three bytes that follow are TH, TL and the configuration;
//! - Read Power Supply: the read slot after the command — bit 0 of the next byte, or the first
//!   of the bits the transaction ended with — is 0 when a device is parasite-powered;
//! - Convert T, Copy Scratchpad and Recall E²: the command alone; the read slots a master may
//!   poll afterwards (the device's busy bits) are skipped;
//! - a command ends `complete` at the transaction's normal end, the next reset, once what it
//!   carries was whole; `incomplete` when the reset came first; `aborted` when a bad slot, a
//!   gap or the end of the samples cut the transaction off.
//!
//! A command spans its transaction from the reset through the command byte; a scratchpad, a
//! write or a power supply bit the bytes they were read from; an end what was read but not
//! reported, through the transaction's end. A partial scratchpad settles at the end that
//! decided it.

use uiharu_core::{
    InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec,
    ProgressSink, Props, Provenance, Requires, Tick, TickRange,
};

use super::events::{
    COMMANDS, Ds18b20End, Ds18b20Event, FAMILY_CODE, READ_POWER_SUPPLY, READ_SCRATCHPAD,
    SCRATCHPAD_LEN, WRITE_SCRATCHPAD, resolution,
};
use crate::onewire::rom::{
    MATCH_ROM, OVERDRIVE_MATCH_ROM, OVERDRIVE_SKIP_ROM, READ_ROM, SKIP_ROM, crc8,
};
use crate::onewire::{Next, OneWireBus, OneWireEvent, Read, ReadError, Transaction, TxId};

/// The DS18B20 decoder node: `onewire.events` in, `ds18b20.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ds18b20;

impl Ds18b20 {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The 1-Wire input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<OneWireEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Ds18b20Event> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the 1-Wire stream it reads: nothing beyond its schema.
    pub fn requires() -> Requires {
        Requires::new()
    }
}

impl Node for Ds18b20 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "ds18b20".into(),
            inputs: vec![InputSpec::new::<OneWireEvent>("bus", Self::requires())],
            outputs: vec![OutputSpec::new::<Ds18b20Event>(
                "events",
                Props::new().with("family", i64::from(FAMILY_CODE)),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let out = io.output(0);
        let bus = OneWireBus::new(io.input(0), out.progress_sink());
        Box::pin(run(bus, Out::new(out)))
    }
}

/// The typed writer the transaction body uses, which also remembers what is open.
#[derive(Debug)]
struct Out {
    out: Outbox<Ds18b20Event>,
    sink: ProgressSink,
    /// The transaction whose command is under way.
    open: Option<TxId>,
    /// The scratchpad bytes the Read Scratchpad under way has read.
    scratchpad: [u8; SCRATCHPAD_LEN],
    read: usize,
    /// The slots they cover.
    scratchpad_at: Option<Provenance>,
}

impl Out {
    /// Tells the cursor from where this writer holds items it has not reported yet.
    fn sync_hold(&self) {
        self.sink.hold(self.scratchpad_at.map(|at| at.span.start));
    }

    fn new(out: Outbox<Ds18b20Event>) -> Self {
        Self {
            sink: out.progress_sink(),
            out,
            open: None,
            scratchpad: [0; SCRATCHPAD_LEN],
            read: 0,
            scratchpad_at: None,
        }
    }

    async fn emit(&mut self, at: Provenance, event: Ds18b20Event) {
        self.out.emit(at, event).await;
    }

    async fn command(&mut self, tx: TxId, code: u8, at: Provenance) {
        self.open = Some(tx);
        self.read = 0;
        self.scratchpad_at = None;
        self.emit(at, Ds18b20Event::Command { tx, code }).await;
    }

    /// Adds a byte to the scratchpad under way, which goes out with its ninth.
    async fn scratchpad_byte(&mut self, tx: TxId, byte: u8, at: Provenance) {
        if let Some(slot) = self.scratchpad.get_mut(self.read) {
            *slot = byte;
            self.read += 1;
        }
        self.scratchpad_at = Some(self.scratchpad_at.map_or(at, |have| have.join(at)));
        self.sync_hold();
        if self.read < SCRATCHPAD_LEN {
            return;
        }
        let bytes = self.scratchpad;
        let at = self.scratchpad_at.take().unwrap_or(at);
        self.sync_hold();
        self.read = 0;
        let [lsb, msb, th, tl, config, ..] = bytes;
        let event = Ds18b20Event::Scratchpad {
            tx,
            bytes,
            crc_ok: crc8(&bytes[..SCRATCHPAD_LEN - 1]) == bytes[SCRATCHPAD_LEN - 1],
            temperature_raw: i16::from_le_bytes([lsb, msb]),
            th: i8::from_le_bytes([th]),
            tl: i8::from_le_bytes([tl]),
            config,
            resolution: resolution(config),
        };
        self.emit(at, event).await;
    }

    /// Sends the bytes of an unfinished scratchpad, if any, settled where the transaction was
    /// known to end.
    async fn flush_scratchpad(&mut self, tx: TxId, decided: Tick) {
        if self.read == 0 {
            return;
        }
        let bytes = self.scratchpad[..self.read].to_vec();
        let temperature_raw =
            (self.read >= 2).then(|| i16::from_le_bytes([self.scratchpad[0], self.scratchpad[1]]));
        let mut at = self.scratchpad_at.take().unwrap_or(Provenance::at(decided));
        self.sync_hold();
        at.settled = at.settled.max(decided);
        self.read = 0;
        let event = Ds18b20Event::PartialScratchpad {
            tx,
            bytes,
            temperature_raw,
        };
        self.emit(at, event).await;
    }

    async fn end(&mut self, reason: Ds18b20End, at: Provenance) {
        if let Some(tx) = self.open.take() {
            self.flush_scratchpad(tx, at.settled).await;
            self.emit(at, Ds18b20Event::End { tx, reason }).await;
        }
    }
}

/// Whether the transaction's ROM level addresses a DS18B20.
async fn addressed(tx: &mut Transaction<'_>) -> Result<bool, ReadError> {
    Ok(match tx.rom_command().await? {
        SKIP_ROM | OVERDRIVE_SKIP_ROM => true,
        READ_ROM | MATCH_ROM | OVERDRIVE_MATCH_ROM => tx.rom().await?.family == FAMILY_CODE,
        _ => false,
    })
}

/// The body of the decoder: one transaction.
async fn transaction(mut tx: Transaction<'_>, out: &mut Out) -> Result<(), ReadError> {
    if !addressed(&mut tx).await? {
        return Ok(());
    }
    let id = tx.id();
    let code = tx.byte().await?;
    if !COMMANDS.contains(&code) {
        let event = Ds18b20Event::UnknownCommand { tx: id, code };
        out.emit(tx.consumed(), event).await;
        return Ok(());
    }
    out.command(id, code, tx.consumed()).await;
    match code {
        READ_SCRATCHPAD => {
            // Pausing between the bytes keeps those read so far: the writer holds them, so
            // that an early end can still report them.
            for _ in 0..SCRATCHPAD_LEN {
                let byte = tx.byte().await?;
                out.scratchpad_byte(id, byte, tx.consumed()).await;
            }
        }
        WRITE_SCRATCHPAD => {
            let th = i8::from_le_bytes([tx.byte().await?]);
            let tl = i8::from_le_bytes([tx.byte().await?]);
            let config = tx.byte().await?;
            let event = Ds18b20Event::WriteScratchpad {
                tx: id,
                th,
                tl,
                config,
            };
            out.emit(tx.consumed(), event).await;
        }
        READ_POWER_SUPPLY => {
            let slots = match tx.byte_or_end().await? {
                Read::Byte(value) | Read::Bits { value, .. } => value,
                Read::End => return Err(ReadError::Incomplete),
            };
            let event = Ds18b20Event::PowerSupply {
                tx: id,
                parasite: slots & 1 == 0,
            };
            out.emit(tx.consumed(), event).await;
        }
        _ => {}
    }
    loop {
        if tx.byte_or_end().await? == Read::End {
            out.end(Ds18b20End::Complete, tx.consumed()).await;
            return Ok(());
        }
    }
}

/// Closes what the body left open, as the transaction's end calls for. The end covers what
/// the body read without reporting, and the transaction's end when the stream carried one; a
/// gap or the end of the stream only places it.
async fn recover(error: ReadError, bus: &mut OneWireBus, out: &mut Out) {
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
        ReadError::Incomplete => (Ds18b20End::Incomplete, unreported),
        ReadError::BadSlot | ReadError::Truncated | ReadError::Unexpected => {
            (Ds18b20End::Aborted, unreported)
        }
        ReadError::Gap(range) => (Ds18b20End::Aborted, placed(range.start)),
        ReadError::EndOfInput(tick) => (Ds18b20End::Aborted, placed(tick)),
    };
    out.end(reason, at).await;
}

async fn run(mut bus: OneWireBus, mut out: Out) {
    loop {
        match bus.next().await {
            Next::Transaction(tx) => {
                if let Err(error) = transaction(tx, &mut out).await {
                    recover(error, &mut bus, &mut out).await;
                }
                if let Err(error) = bus.finish().await {
                    recover(error, &mut bus, &mut out).await;
                }
                bus.consumed();
            }
            Next::Gap(range) => {
                out.end(Ds18b20End::Aborted, Provenance::at(range.start))
                    .await;
                out.out.gap(range).await;
            }
            Next::Progress(tick) => out.out.progress(tick).await,
            Next::End(tick) => {
                out.end(Ds18b20End::Aborted, Provenance::at(tick)).await;
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
    fn ports_declare_the_family_and_require_nothing_but_the_schema() {
        let spec = Ds18b20::new().spec();
        assert_eq!(spec.inputs[0].requires, Requires::new());
        assert_eq!(spec.outputs[0].props.get("family"), Some(&40i64.into()));
    }
}
