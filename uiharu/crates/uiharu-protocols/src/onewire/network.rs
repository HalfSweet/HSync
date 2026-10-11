// SPDX-License-Identifier: Apache-2.0

//! The 1-Wire network layer: transactions, their ROM level and the bytes after it, made of the
//! link layer's resets and bits.
//!
//! Implemented from Maxim Integrated (Analog Devices) application note 937, "Book of iButton
//! Standards", and the DS2401 and DS2431 datasheets:
//!
//! - a transaction begins at a reset, answered by a presence pulse or not, and ends at the
//!   next reset; bits before the first reset — a transaction already under way at the first
//!   sample or after a gap — belong to no transaction and are skipped until a reset;
//! - eight bits, least significant first, make a byte; the first byte is the ROM command;
//! - Read ROM (`0x33`), Match ROM (`0x55`) and Overdrive Match ROM (`0x69`) are followed by a
//!   64-bit ROM code — the slave's for Read ROM, the master's otherwise — made of the family
//!   code, a 48-bit serial number and the CRC-8 of both, each least significant bit first;
//!   Skip ROM (`0xCC`) and Overdrive Skip ROM (`0x3C`) are followed by no ROM code;
//! - Search ROM (`0xF0`) and Alarm Search (`0xEC`) are followed by 64 triplets: the bit the
//!   devices still in the search send, its complement, and the direction the master writes.
//!   The direction bits are the ROM code found; a bit and a complement both 0 are a conflict
//!   the master resolved by its direction, both 1 mean that no device answered;
//! - every byte after the ROM level is a transport byte, reported raw: the network layer does
//!   not know the device. After a ROM command it does not know, every byte is one;
//! - when the transaction ends, the bits after its last whole transport byte are reported, so
//!   that a single read slot — a device's status bit — is not lost; bits left in the ROM
//!   level make nothing, and the transaction's end covers them;
//! - a bad slot ends the transaction (`bad-slot`): the bits after it can no longer be told
//!   apart, and those up to the next reset are skipped;
//! - a gap or the end of the samples ends the transaction as `aborted`; the gap or the end is
//!   passed on after it.
//!
//! The speed does not matter here: the link layer measured the slots. Every event spans the
//! slots it is made of and settles with the last of them — a byte its eight slots, a ROM code
//! its 64, a search its 192 —; `begin` is the reset; trailing bits and `end` settle where the
//! transaction was known to end — the reset, the bad slot, the gap's start or the end of the
//! samples — and `end` covers the slots read but not reported (an unfinished ROM level, the
//! bad slot).

use std::convert::Infallible;

use uiharu_core::{
    InputRef, InputSpec, Io, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec,
    ProgressSink, Props, Provenance, Requires, Tick, TickRange,
};

use super::bus::{LinkBus, LinkError, LinkNext};
use super::events::{EndReason, LinkEvent, OneWireEvent, TxId};
use super::rom::{ALARM_SEARCH, MATCH_ROM, OVERDRIVE_MATCH_ROM, READ_ROM, SEARCH_ROM, rom_crc_ok};

/// The 1-Wire network-layer node: `onewire.link` in, `onewire.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OneWireNetwork;

impl OneWireNetwork {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The link-layer input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<LinkEvent> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<OneWireEvent> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the link-layer stream it reads: nothing beyond its
    /// schema, whatever the speed.
    pub fn requires() -> Requires {
        Requires::new()
    }

    /// What the decoder's output declares: bytes of eight bits, least significant first.
    pub fn props() -> Props {
        Props::new()
            .with("wordBits", 8)
            .with("bitOrder", "lsb-first")
    }
}

impl Node for OneWireNetwork {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "onewire".into(),
            inputs: vec![InputSpec::new::<LinkEvent>("link", Self::requires())],
            outputs: vec![OutputSpec::new::<OneWireEvent>("events", Self::props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let out = io.output(0);
        let link = LinkBus::new(io.input(0), out.progress_sink());
        Box::pin(run(link, NetOut::new(out)))
    }
}

/// The writer the transaction body uses, which also assembles the transport bytes.
#[derive(Debug)]
struct NetOut {
    out: Outbox<OneWireEvent>,
    sink: ProgressSink,
    next_tx: u64,
    /// The bits of the transport byte under way, the first as bit 0.
    value: u8,
    /// How many there are.
    count: u8,
    /// The slots they cover.
    at: Option<Provenance>,
}

impl NetOut {
    /// Tells the cursor from where this writer holds items it has not reported yet.
    fn sync_hold(&self) {
        self.sink.hold(self.at.map(|at| at.span.start));
    }

    fn new(out: Outbox<OneWireEvent>) -> Self {
        Self {
            sink: out.progress_sink(),
            out,
            next_tx: 1,
            value: 0,
            count: 0,
            at: None,
        }
    }

    async fn emit(&mut self, at: Provenance, event: OneWireEvent) {
        self.out.emit(at, event).await;
    }

    async fn begin(&mut self, presence: bool, at: Provenance) -> TxId {
        let tx = TxId(self.next_tx);
        self.next_tx += 1;
        self.emit(at, OneWireEvent::Begin { tx, presence }).await;
        tx
    }

    /// Adds a bit to the transport byte under way, which goes out once it has eight.
    async fn transport_bit(&mut self, tx: TxId, bit: bool, at: Provenance) {
        self.at = Some(self.at.map_or(at, |have| have.join(at)));
        self.sync_hold();
        self.value |= u8::from(bit) << self.count;
        self.count += 1;
        if self.count == 8 {
            let value = self.value;
            let at = self.at.take().unwrap_or(at);
            self.sync_hold();
            self.value = 0;
            self.count = 0;
            self.emit(at, OneWireEvent::Byte { tx, value }).await;
        }
    }

    /// Sends the bits of an unfinished transport byte, if any, settled where the transaction
    /// was known to end.
    async fn flush_bits(&mut self, tx: TxId, decided: Tick) {
        if self.count == 0 {
            return;
        }
        let (value, count) = (self.value, self.count);
        let mut at = self.at.take().unwrap_or(Provenance::at(decided));
        self.sync_hold();
        at.settled = at.settled.max(decided);
        self.value = 0;
        self.count = 0;
        self.emit(at, OneWireEvent::Bits { tx, value, count }).await;
    }
}

/// Reads `count` bits, least significant first.
async fn bits(link: &mut LinkBus, count: u32) -> Result<u64, LinkError> {
    let mut value = 0u64;
    for n in 0..count {
        value |= u64::from(link.bit().await?) << n;
    }
    Ok(value)
}

/// The body of the decoder: one transaction, from the bit after its reset until something
/// ends it, which is the only way it returns.
async fn transaction(
    link: &mut LinkBus,
    out: &mut NetOut,
    tx: TxId,
) -> Result<Infallible, LinkError> {
    let [code, ..] = bits(link, 8).await?.to_le_bytes();
    out.emit(link.consumed(), OneWireEvent::RomCommand { tx, code })
        .await;
    match code {
        READ_ROM | MATCH_ROM | OVERDRIVE_MATCH_ROM => {
            // Pausing between the bits keeps those read so far: they are the state of this
            // future.
            let id = bits(link, 64).await?;
            let [family, ..] = id.to_le_bytes();
            let rom = OneWireEvent::Rom {
                tx,
                id,
                family,
                crc_ok: rom_crc_ok(id),
            };
            out.emit(link.consumed(), rom).await;
        }
        SEARCH_ROM | ALARM_SEARCH => {
            let (mut id, mut conflicts, mut unanswered) = (0u64, 0u8, 0u8);
            for n in 0..64 {
                let bit = link.bit().await?;
                let complement = link.bit().await?;
                let direction = link.bit().await?;
                match (bit, complement) {
                    (false, false) => conflicts += 1,
                    (true, true) => unanswered += 1,
                    _ => {}
                }
                id |= u64::from(direction) << n;
            }
            let search = OneWireEvent::Search {
                tx,
                id,
                crc_ok: rom_crc_ok(id),
                conflicts,
                unanswered,
            };
            out.emit(link.consumed(), search).await;
        }
        _ => {}
    }
    loop {
        let bit = link.bit().await?;
        out.transport_bit(tx, bit, link.consumed()).await;
    }
}

/// Ends the transaction the way `error` says it ended: the bits of an unfinished transport
/// byte first, then the end, covering the slots read but not reported.
async fn close(error: LinkError, link: &mut LinkBus, out: &mut NetOut, tx: TxId) {
    let unreported = link.consumed();
    let placed = |start: Tick, settled: Tick| Provenance {
        span: if unreported.span.is_empty() {
            TickRange::at(start)
        } else {
            unreported.span
        },
        settled,
    };
    let (reason, at) = match error {
        LinkError::Reset(reset) => (EndReason::Reset, placed(reset.span.start, reset.settled)),
        // The bad slot was consumed: the unreported slots end with it.
        LinkError::BadSlot { .. } => (EndReason::BadSlot, unreported),
        LinkError::Gap(range) => (EndReason::Aborted, placed(range.start, range.start)),
        LinkError::EndOfInput(tick) => (EndReason::Aborted, placed(tick, tick)),
    };
    out.flush_bits(tx, at.settled).await;
    out.emit(at, OneWireEvent::End { tx, reason }).await;
}

async fn run(mut link: LinkBus, mut out: NetOut) {
    loop {
        match link.next().await {
            LinkNext::Reset { presence } => {
                let tx = out.begin(presence, link.consumed()).await;
                let Err(error) = transaction(&mut link, &mut out, tx).await;
                close(error, &mut link, &mut out, tx).await;
            }
            LinkNext::Gap(range) => out.out.gap(range).await,
            LinkNext::Progress(tick) => out.out.progress(tick).await,
            LinkNext::End(tick) => {
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
    fn ports_declare_bytes_and_require_nothing_but_the_schema() {
        let spec = OneWireNetwork::new().spec();
        assert_eq!(spec.inputs[0].requires, Requires::new());
        assert_eq!(spec.outputs[0].props.get("wordBits"), Some(&8i64.into()));
        assert_eq!(
            spec.outputs[0].props.get("bitOrder"),
            Some(&"lsb-first".into())
        );
    }
}
