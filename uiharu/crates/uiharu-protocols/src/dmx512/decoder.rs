// SPDX-License-Identifier: Apache-2.0

//! The DMX512 decoder: the packets of a DMX512 line, read from the characters and breaks of a
//! UART stream.
//!
//! Implemented from ANSI E1.11, Entertainment Technology — USITT DMX512-A, the packet, its
//! slots and their timing: data at 250 kbit/s over EIA-485, every slot a character of
//! a start bit, eight data bits sent least significant first, no parity and two stop bits; a
//! packet is a break (space for at least 92 µs from a transmitter, 88 µs at a receiver), a mark
//! after break (at least 12 µs, 8 µs at a receiver), the start code slot (0 for dimmer data,
//! other codes for other data) and up to 512 data slots; the time between slots and the time
//! before the next break may be anything up to a second.
//!
//! The rules:
//!
//! - every break the UART stream reports begins a packet, whatever its length, which the
//!   `packet-begin` reports in ticks: a short break shows rather than being judged. A packet
//!   under way when a break begins ends there, `break`;
//! - the first character after the break is the start code, every later one a data slot, and
//!   all are reported as the line carried them, in batches of at most [`SLOT_BATCH`] data
//!   values. A batch goes out when it is full, when a slot of it had a framing error, or when the
//!   packet ends;
//! - a character with a framing error, a stop bit read at space, still reports its value, and an
//!   `error` naming its slot follows the batch that holds it (slot 0 being the start code);
//! - characters before the first break, and after a gap until the next break, belong to no
//!   packet and are dropped: the start code cannot be told from data there;
//! - a gap or the end of the stream ends the packet under way `aborted`, at the gap's start or
//!   the end. A packet is only ended by the next break or by the stream: E1.11 allows the line to
//!   idle for up to a second between slots and before the next break, and ending a packet on a
//!   timeout is out of scope;
//! - the mark after break and the times between slots are not checked: they can be read from
//!   the spans, the mark after break being the ticks from the end of the `packet-begin` to the
//!   start of the `start-code`;
//! - `packet-begin` spans the break and is settled when the line returned to mark, where the
//!   UART reported it; `start-code` and `error` take the provenance of their character; a batch
//!   spans its characters and is settled with its last one, or at the packet's end that sent it
//!   out; `packet-end` is empty at the start of the break that ends it and settled with that
//!   break, or empty at the gap's start or the end of the stream and settled there;
//! - progress is passed on, held back to the first character of a batch not yet sent.
//!
//! Limits: no timeout ends a packet; the break, mark-after-break and slot timing are reported
//! but not checked; start codes are not interpreted, so the data of RDM and other alternate
//! start codes is reported as plain slots; packets longer than 512 data slots are reported as
//! they are.

use uiharu_core::{
    Inbox, InputRef, InputSpec, Io, Item, Node, NodeFuture, NodeId, NodeSpec, Outbox, OutputRef,
    OutputSpec, Props, Provenance, Requires, Tick, TickRange,
};

use super::events::{Dmx512Event, EndReason, PacketId, SLOT_BATCH};
use crate::uart::{BitOrder, Parity, ParityCheck, StopBits, UartConfig, UartEvent};

/// The bit rate of a DMX512 line.
pub const BAUD: u32 = 250_000;

/// The DMX512 decoder node: `uart.events` in, `dmx512.events` out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dmx512;

impl Dmx512 {
    /// A decoder.
    pub fn new() -> Self {
        Self
    }

    /// The UART input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<UartEvent> {
        InputRef::of_node(node, 0)
    }

    /// The packet output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<Dmx512Event> {
        OutputRef::of_node(node, 0)
    }

    /// What the decoder requires of the UART stream it reads: the character format of E1.11,
    /// 250 000 baud, eight data bits least significant first, no parity, two stop bits.
    pub fn requires() -> Requires {
        Requires::new()
            .equals("baud", i64::from(BAUD))
            .equals("dataBits", 8)
            .equals("parity", "none")
            .equals("stopBits", "two")
            .equals("bitOrder", "lsb-first")
    }

    /// The configuration of a UART decoder reading a DMX512 line on channel `line`, mark being
    /// high: the receiver's output of an EIA-485 transceiver, or the transmitter's input.
    pub fn uart_config(line: u8) -> UartConfig {
        UartConfig {
            line,
            baud: BAUD,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: StopBits::Two,
            bit_order: BitOrder::LsbFirst,
            ..UartConfig::default()
        }
    }
}

impl Node for Dmx512 {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "dmx512".into(),
            inputs: vec![InputSpec::new::<UartEvent>("uart", Self::requires())],
            outputs: vec![OutputSpec::new::<Dmx512Event>("events", Props::new())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        let packets = Packets::new(io.output(0));
        Box::pin(run(io.input(0), packets))
    }
}

/// The packet under way.
#[derive(Clone, Copy, Debug)]
struct Open {
    id: PacketId,
    /// Whether the start code has come.
    started: bool,
    /// The data slots so far.
    slots: u32,
}

/// The decoder's state and its typed writer.
#[derive(Debug)]
struct Packets {
    out: Outbox<Dmx512Event>,
    next_id: u64,
    open: Option<Open>,
    /// Data values not reported yet, at most [`SLOT_BATCH`].
    batch: Vec<u8>,
    batch_at: Option<Provenance>,
    batch_offset: u32,
}

impl Packets {
    fn new(out: Outbox<Dmx512Event>) -> Self {
        Self {
            out,
            next_id: 1,
            open: None,
            batch: Vec::with_capacity(SLOT_BATCH),
            batch_at: None,
            batch_offset: 0,
        }
    }

    /// A break: it ends the packet under way and begins the next.
    async fn line_break(&mut self, ticks: u64, at: Provenance) {
        let end = Provenance {
            span: TickRange::at(at.span.start),
            settled: at.settled,
        };
        self.close(EndReason::Break, end).await;
        let packet = PacketId(self.next_id);
        self.next_id += 1;
        self.open = Some(Open {
            id: packet,
            started: false,
            slots: 0,
        });
        let begin = Dmx512Event::PacketBegin {
            packet,
            break_ticks: ticks,
        };
        self.out.emit(at, begin).await;
    }

    /// A character: the start code or a data slot of the packet under way, if there is one.
    async fn character(&mut self, value: u8, framing_ok: bool, at: Provenance) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let packet = open.id;
        if !open.started {
            open.started = true;
            let start = Dmx512Event::StartCode {
                packet,
                code: value,
            };
            self.out.emit(at, start).await;
            if !framing_ok {
                self.out
                    .emit(at, Dmx512Event::Error { packet, slot: 0 })
                    .await;
            }
            return;
        }
        let offset = open.slots;
        open.slots = open.slots.saturating_add(1);
        if self.batch.is_empty() {
            self.batch_offset = offset;
            self.batch_at = Some(at);
        } else if let Some(have) = self.batch_at {
            self.batch_at = Some(have.join(at));
        }
        self.batch.push(value);
        if !framing_ok {
            self.flush(packet, at.settled).await;
            let slot = offset.saturating_add(1);
            self.out.emit(at, Dmx512Event::Error { packet, slot }).await;
        } else if self.batch.len() >= SLOT_BATCH {
            self.flush(packet, at.settled).await;
        }
    }

    /// Sends the batch, if any, settled no earlier than `decided`.
    async fn flush(&mut self, packet: PacketId, decided: Tick) {
        if self.batch.is_empty() {
            return;
        }
        let values = std::mem::replace(&mut self.batch, Vec::with_capacity(SLOT_BATCH));
        let mut at = self.batch_at.take().unwrap_or(Provenance::at(decided));
        at.settled = at.settled.max(decided);
        let slots = Dmx512Event::Slots {
            packet,
            offset: self.batch_offset,
            values,
        };
        self.out.emit(at, slots).await;
    }

    /// Ends the packet under way, if any, at `at`.
    async fn close(&mut self, reason: EndReason, at: Provenance) {
        let Some(open) = self.open.take() else {
            return;
        };
        self.flush(open.id, at.settled).await;
        let end = Dmx512Event::PacketEnd {
            packet: open.id,
            slots: open.slots,
            reason,
        };
        self.out.emit(at, end).await;
    }

    /// Progress of the UART stream: nothing comes before `tick`.
    async fn progress(&mut self, tick: Tick) {
        let held = match self.batch_at {
            Some(at) if !self.batch.is_empty() => at.span.start.min(tick),
            _ => tick,
        };
        self.out.progress(held).await;
    }

    async fn gap(&mut self, range: TickRange) {
        self.close(EndReason::Aborted, Provenance::at(range.start))
            .await;
        self.out.gap(range).await;
    }

    async fn end(&mut self, tick: Tick) {
        self.close(EndReason::Aborted, Provenance::at(tick)).await;
        self.out.end(tick).await;
    }
}

async fn run(mut input: Inbox<UartEvent>, mut packets: Packets) {
    loop {
        match input.recv().await {
            Item::Event(event) => {
                let at = Provenance::over(event.span, event.settled);
                match event.payload {
                    UartEvent::Break { ticks } => packets.line_break(ticks, at).await,
                    UartEvent::Frame {
                        value,
                        parity,
                        framing_ok,
                    } => {
                        // The requirements make every character 8 bits wide, without parity; a
                        // parity error would count as a framing error does.
                        let [value, _] = value.to_le_bytes();
                        let ok = framing_ok && parity != Some(ParityCheck::Error);
                        packets.character(value, ok, at).await;
                    }
                }
            }
            Item::Progress(tick) => packets.progress(tick).await,
            Item::Gap(range) => packets.gap(range).await,
            Item::End(tick) => {
                packets.end(tick).await;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmx512::DMX512_EVENTS;

    #[test]
    fn requires_the_character_format_of_e1_11() {
        let uart = |config: &UartConfig| config.props();
        let requires = Dmx512::requires();
        let dmx = Dmx512::uart_config(3);
        assert_eq!(dmx.line, 3);
        assert!(dmx.validate().is_ok());
        assert!(requires.unmet(&uart(&dmx)).is_none());
        let refused = |config: UartConfig| requires.unmet(&uart(&config)).map(|(key, _)| key);
        assert_eq!(
            refused(UartConfig {
                baud: 115_200,
                ..dmx
            }),
            Some("baud")
        );
        assert_eq!(
            refused(UartConfig {
                data_bits: 7,
                ..dmx
            }),
            Some("dataBits")
        );
        assert_eq!(
            refused(UartConfig {
                parity: Parity::Even,
                ..dmx
            }),
            Some("parity")
        );
        assert_eq!(
            refused(UartConfig {
                stop_bits: StopBits::One,
                ..dmx
            }),
            Some("stopBits")
        );
        assert_eq!(
            refused(UartConfig {
                bit_order: BitOrder::MsbFirst,
                ..dmx
            }),
            Some("bitOrder")
        );
        // The line may be read inverted, or at another sample point.
        assert_eq!(
            refused(UartConfig {
                inverted: true,
                sample_point: 40,
                ..dmx
            }),
            None
        );
        let spec = Dmx512::new().spec();
        assert_eq!(spec.inputs[0].name, "uart");
        assert_eq!(spec.outputs[0].name, "events");
        assert_eq!(spec.outputs[0].schema, DMX512_EVENTS);
    }
}
