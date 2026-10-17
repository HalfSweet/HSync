// SPDX-License-Identifier: Apache-2.0

//! The I²C decoder: bytes clocked on SCL between START and STOP conditions, implemented from
//! NXP UM10204, "I²C-bus specification and user manual", Rev. 7 (2021).
//!
//! The rules, with the manual's sections in brackets:
//!
//! - a START condition is SDA falling while SCL is high, a STOP condition SDA rising while SCL
//!   is high [START and STOP conditions]; an SDA change at the very tick SCL changes is a data
//!   change, never a condition, as the order of two changes within one sample cannot be seen;
//! - a bit is SDA's level at SCL's rising edge [Data validity]; the rising edge a START or
//!   STOP condition then happens on belongs to the condition, not to a byte;
//! - a byte is eight bits, most significant first, and the acknowledge bit on the ninth rising
//!   edge, driven by the receiver: SDA low is ACK, high is NACK [Byte format; Acknowledge
//!   (ACK) and Not Acknowledge (NACK)]; the byte is reported at that edge, so a START or STOP
//!   during the acknowledge clock comes after the byte — eight bits followed directly by a
//!   STOP read as an acknowledged byte, as a STOP raises SCL with SDA low;
//! - the first byte after a START or repeated START is the address, seven bits and R/W̄
//!   (1 = read) [The target address and R/W bit]. `1111 0xx` with R/W̄ = 0 begins a 10-bit
//!   address that the next byte (A7–A0) completes; with R/W̄ = 1 it reads from the 10-bit
//!   address the transaction last addressed for writing, if that address has the same two high
//!   bits and no other address came between [10-bit addressing]. A `1111 0xx` byte whose other
//!   part was not seen — a write address cut off before its second byte, a read with no such
//!   write before it — is reported as the 7-bit address it is. Reserved addresses (general
//!   call, START byte, Hs-mode controller codes, device ID …) are reported like any other
//!   [Reserved addresses];
//! - a transaction runs from a START to a STOP; a repeated START stays in it, as a `restart`
//!   event, and the address after it is another `address` event;
//! - bits clocked since the last whole byte when a START, STOP, gap or the end of the samples
//!   interrupts them make no byte; the `restart` or `end` that interrupted them counts them
//!   (at a gap or the end of the samples, a bit whose clock is still high is not counted);
//! - the bus is busy when the samples begin, or resume after a gap, with SCL or SDA low, and
//!   when SCL changes while no transaction is open (clocks before any START): the transaction
//!   under way begins `partial` at that tick. Its bytes cannot be aligned, so it reports none,
//!   and it ends at the STOP, or where the next START begins the next transaction (reason
//!   `restart`);
//! - a gap or the end of the samples while a transaction is open ends it with reason
//!   `aborted`, at the gap's start or the end of the samples;
//! - progress is passed on, held back to the first bit of the byte under way, or to the first
//!   byte of a 10-bit write address whose second byte is still to come, since the event they
//!   make spans from there;
//! - clock stretching, SCL held low by a target [Clock stretching], needs nothing: only SCL's
//!   rising edges are sampled.
//!
//! The sample rate does not enter: the decoder follows edges, and needs every SCL level and
//! every SDA change between two SCL edges on a sample of its own. There is no spike filter, so
//! a glitch on SCL counts as a clock. Arbitration between controllers, Hs-mode and SMBus
//! extensions (timeouts, packet error checking) are out of scope; their bytes decode as bytes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::{EndReason, I2cEvent, TxId};

/// How the bus is wired.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct I2cConfig {
    /// The clock channel.
    pub scl: u8,
    /// The data channel.
    pub sda: u8,
}

impl Default for I2cConfig {
    fn default() -> Self {
        Self { scl: 0, sda: 1 }
    }
}

impl I2cConfig {
    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> {
        [self.scl, self.sda].into_iter()
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), I2cConfigError> {
        for channel in self.channels() {
            if channel >= MAX_CHANNELS {
                return Err(I2cConfigError::Channel(channel));
            }
        }
        if self.scl == self.sda {
            return Err(I2cConfigError::DuplicateChannel(self.scl));
        }
        Ok(())
    }

    /// What the decoder's output port declares: an I²C bus, framed by START and STOP
    /// conditions, on which 10-bit addresses may occur.
    pub fn props(&self) -> Props {
        Props::new()
            .with("bus", "i2c")
            .with("framing", "start-stop")
            .with("tenBitAddressing", true)
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum I2cConfigError {
    /// SCL and SDA are the same channel.
    #[error("channel {0} is used for both SCL and SDA")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The I²C decoder node: logic samples in, `i2c.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct I2c {
    config: I2cConfig,
}

impl I2c {
    /// A decoder for `config`.
    pub fn new(config: I2cConfig) -> Result<Self, I2cConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &I2cConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<I2cEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for I2c {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "i2c".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<I2cEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

/// The 7-bit address `1111 0xx` that begins a 10-bit address.
const TEN_BIT_PREFIX: u8 = 0b111_1000;

/// What the next whole byte of a transaction is.
#[derive(Clone, Copy, Debug)]
enum Expect {
    /// The first byte after a START or repeated START.
    Address,
    /// A7–A0 of a 10-bit write address, after its first byte.
    TenBitLow {
        /// A9 and A8.
        high: u8,
        /// The first byte, through its acknowledge bit.
        first: TickRange,
        /// Whether the first byte was acknowledged.
        ack: bool,
    },
    /// Data bytes.
    Data,
}

/// A transaction whose START was seen.
#[derive(Debug)]
struct Transfer {
    tx: TxId,
    /// Rising edges taken into the current byte, the acknowledge bit's included.
    bits: u8,
    byte: u8,
    /// The rising edge of the current byte's first bit.
    first_bit: Tick,
    next: Expect,
    /// The 10-bit address the transaction last addressed for writing, which a read address
    /// after a repeated START refers back to.
    ten_bit: Option<u16>,
}

impl Transfer {
    fn new(tx: TxId) -> Self {
        Self {
            tx,
            bits: 0,
            byte: 0,
            first_bit: Tick::ZERO,
            next: Expect::Address,
            ten_bit: None,
        }
    }

    /// Takes the bit on SCL's rising edge at `tick`; the address or data byte, once the
    /// acknowledge bit completes it.
    fn bit(&mut self, tick: Tick, sda: bool) -> Option<(Provenance, I2cEvent)> {
        self.bits += 1;
        if self.bits == 1 {
            self.first_bit = tick;
        }
        if self.bits <= 8 {
            self.byte = (self.byte << 1) | u8::from(sda);
            return None;
        }
        let (tx, byte, ack) = (self.tx, self.byte, !sda);
        let span = TickRange::new(self.first_bit, tick.next());
        self.bits = 0;
        self.byte = 0;
        match self.next {
            Expect::Data => Some((
                Provenance::over(span, tick),
                I2cEvent::Data { tx, byte, ack },
            )),
            Expect::TenBitLow {
                high,
                first,
                ack: first_ack,
            } => {
                let address = (u16::from(high) << 8) | u16::from(byte);
                self.ten_bit = Some(address);
                self.next = Expect::Data;
                let event = I2cEvent::Address {
                    tx,
                    address,
                    ten_bit: true,
                    read: false,
                    ack: first_ack && ack,
                };
                Some((Provenance::over(first.join(span), tick), event))
            }
            Expect::Address => {
                let (address, read) = (byte >> 1, byte & 1 != 0);
                self.next = Expect::Data;
                if address & !0b11 == TEN_BIT_PREFIX {
                    let high = address & 0b11;
                    if !read {
                        // The second byte decides what this one is.
                        self.next = Expect::TenBitLow {
                            high,
                            first: span,
                            ack,
                        };
                        return None;
                    }
                    if let Some(ten_bit) = self.ten_bit
                        && ten_bit >> 8 == u16::from(high)
                    {
                        let event = I2cEvent::Address {
                            tx,
                            address: ten_bit,
                            ten_bit: true,
                            read: true,
                            ack,
                        };
                        return Some((Provenance::over(span, tick), event));
                    }
                }
                self.ten_bit = None;
                let event = I2cEvent::Address {
                    tx,
                    address: u16::from(address),
                    ten_bit: false,
                    read,
                    ack,
                };
                Some((Provenance::over(span, tick), event))
            }
        }
    }

    /// The first byte of a 10-bit write address whose second byte never came, reported as the
    /// 7-bit address it is once that is certain, at `tick`.
    fn unfinished_ten_bit(&mut self, tick: Tick) -> Option<(Provenance, I2cEvent)> {
        let Expect::TenBitLow { high, first, ack } = self.next else {
            return None;
        };
        self.next = Expect::Data;
        self.ten_bit = None;
        let event = I2cEvent::Address {
            tx: self.tx,
            address: u16::from(TEN_BIT_PREFIX | high),
            ten_bit: false,
            read: false,
            ack,
        };
        Some((Provenance::over(first, tick), event))
    }

    /// The bits of the current byte a START or STOP condition cuts short: the rising edge the
    /// condition happens on is not one of them.
    fn dropped_by_condition(&self) -> u8 {
        self.bits.saturating_sub(1)
    }

    /// Where the address or data event under way will start: the first byte of a 10-bit write
    /// address whose second byte is still to come, or else the first bit of the byte being
    /// clocked in; `None` between bytes.
    fn in_flight(&self) -> Option<Tick> {
        match self.next {
            Expect::TenBitLow { first, .. } => Some(first.start),
            Expect::Address | Expect::Data => (self.bits > 0).then_some(self.first_bit),
        }
    }
}

/// Where the bus stands.
#[derive(Debug)]
enum State {
    /// No transaction is open.
    Idle,
    /// A transaction found under way; its bytes cannot be aligned.
    Partial(TxId),
    /// A transaction whose START was seen.
    Active(Transfer),
}

/// The events one change of the lines decides: at most two.
#[derive(Debug, Default)]
struct Decided([Option<(Provenance, I2cEvent)>; 2]);

impl Decided {
    fn push(&mut self, at: Provenance, event: I2cEvent) {
        if let Some(slot) = self.0.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some((at, event));
        }
    }

    fn push_some(&mut self, decided: Option<(Provenance, I2cEvent)>) {
        if let Some((at, event)) = decided {
            self.push(at, event);
        }
    }
}

struct Decoder {
    config: I2cConfig,
    watch: ChannelMask,
    next_tx: u64,
    state: State,
    /// SCL's level at the last tick seen.
    scl_high: bool,
}

impl Decoder {
    fn new(config: I2cConfig) -> Self {
        Self {
            config,
            watch: ChannelMask::one(config.scl).with(config.sda),
            next_tx: 1,
            state: State::Idle,
            scl_high: true,
        }
    }

    fn new_tx(&mut self) -> TxId {
        let tx = self.next_tx;
        self.next_tx += 1;
        tx
    }

    /// Progress to `tick`, held back to the start of the event under way, which promises that
    /// nothing starting earlier follows.
    fn hold(&self, tick: Tick) -> Tick {
        let start = match &self.state {
            State::Active(transfer) => transfer.in_flight(),
            State::Idle | State::Partial(_) => None,
        };
        start.map_or(tick, |start| start.min(tick))
    }

    /// Learns the bus at the first sample, or the first after a gap: free with both lines
    /// high, busy otherwise.
    fn resync(&mut self, tick: Tick, levels: Levels) -> Decided {
        let mut decided = Decided::default();
        self.scl_high = levels.high(self.config.scl);
        if self.scl_high && levels.high(self.config.sda) {
            self.state = State::Idle;
        } else {
            let tx = self.new_tx();
            self.state = State::Partial(tx);
            decided.push(Provenance::one(tick), I2cEvent::Begin { tx, partial: true });
        }
        decided
    }

    /// What the lines changing at `tick` decide.
    fn change(&mut self, tick: Tick, changed: ChannelMask, levels: Levels) -> Decided {
        let scl_high = levels.high(self.config.scl);
        self.scl_high = scl_high;
        if changed.has(self.config.scl) {
            // SDA changing at the same tick is a data change, whatever its direction.
            return self.clock(tick, scl_high, levels.high(self.config.sda));
        }
        if !scl_high {
            return Decided::default();
        }
        if levels.high(self.config.sda) {
            self.stop(tick)
        } else {
            self.start(tick)
        }
    }

    /// An SCL edge, rising or not.
    fn clock(&mut self, tick: Tick, rising: bool, sda: bool) -> Decided {
        let mut decided = Decided::default();
        match &mut self.state {
            State::Idle => {
                // A clock with no START seen: a transaction is under way whose beginning was
                // missed.
                let tx = self.new_tx();
                self.state = State::Partial(tx);
                decided.push(Provenance::one(tick), I2cEvent::Begin { tx, partial: true });
            }
            State::Partial(_) => {}
            State::Active(transfer) => {
                if rising {
                    decided.push_some(transfer.bit(tick, sda));
                }
            }
        }
        decided
    }

    /// A START condition: a transaction begins, or a repeated START goes on with one.
    fn start(&mut self, tick: Tick) -> Decided {
        let mut decided = Decided::default();
        let at = Provenance::one(tick);
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Idle => {}
            State::Partial(tx) => decided.push(
                at,
                I2cEvent::End {
                    tx,
                    reason: EndReason::Restart,
                    dropped_bits: 0,
                },
            ),
            State::Active(mut transfer) => {
                decided.push_some(transfer.unfinished_ten_bit(tick));
                decided.push(
                    at,
                    I2cEvent::Restart {
                        tx: transfer.tx,
                        dropped_bits: transfer.dropped_by_condition(),
                    },
                );
                transfer.bits = 0;
                transfer.byte = 0;
                transfer.next = Expect::Address;
                self.state = State::Active(transfer);
                return decided;
            }
        }
        let tx = self.new_tx();
        self.state = State::Active(Transfer::new(tx));
        decided.push(at, I2cEvent::Begin { tx, partial: false });
        decided
    }

    /// A STOP condition: the open transaction ends.
    fn stop(&mut self, tick: Tick) -> Decided {
        let mut decided = Decided::default();
        let at = Provenance::one(tick);
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Idle => {}
            State::Partial(tx) => decided.push(
                at,
                I2cEvent::End {
                    tx,
                    reason: EndReason::Stop,
                    dropped_bits: 0,
                },
            ),
            State::Active(mut transfer) => {
                decided.push_some(transfer.unfinished_ten_bit(tick));
                decided.push(
                    at,
                    I2cEvent::End {
                        tx: transfer.tx,
                        reason: EndReason::Stop,
                        dropped_bits: transfer.dropped_by_condition(),
                    },
                );
            }
        }
        decided
    }

    /// The samples stop at `tick`, for a gap or for good: the open transaction is cut off.
    fn cut(&mut self, tick: Tick) -> Decided {
        let mut decided = Decided::default();
        let at = Provenance::at(tick);
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Idle => {}
            State::Partial(tx) => decided.push(
                at,
                I2cEvent::End {
                    tx,
                    reason: EndReason::Aborted,
                    dropped_bits: 0,
                },
            ),
            State::Active(mut transfer) => {
                decided.push_some(transfer.unfinished_ten_bit(tick));
                // A bit whose clock is still high could yet have turned into a condition.
                let unsettled = u8::from(self.scl_high && transfer.bits > 0);
                decided.push(
                    at,
                    I2cEvent::End {
                        tx: transfer.tx,
                        reason: EndReason::Aborted,
                        dropped_bits: transfer.bits - unsettled,
                    },
                );
            }
        }
        decided
    }
}

async fn emit(out: &mut Outbox<I2cEvent>, decided: Decided) {
    for (at, event) in decided.0.into_iter().flatten() {
        out.emit(at, event).await;
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<I2cEvent>, config: I2cConfig) {
    let mut decoder = Decoder::new(config);
    // Each round of this loop starts where levels are unknown: at the first sample and after
    // every gap.
    'resync: loop {
        let decided = loop {
            match signal.sample().await {
                Sample::At { tick, levels } => break decoder.resync(tick, levels),
                Sample::Gap(range) => out.gap(range).await,
                Sample::Progress(tick) => out.progress(decoder.hold(tick)).await,
                Sample::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        };
        emit(&mut out, decided).await;
        loop {
            match signal.next_change(decoder.watch).await {
                Change::At {
                    tick,
                    changed,
                    levels,
                } => {
                    let decided = decoder.change(tick, changed, levels);
                    emit(&mut out, decided).await;
                }
                Change::Gap(range) => {
                    let decided = decoder.cut(range.start);
                    emit(&mut out, decided).await;
                    out.gap(range).await;
                    continue 'resync;
                }
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => out.progress(decoder.hold(tick)).await,
                Change::End(tick) => {
                    let decided = decoder.cut(tick);
                    emit(&mut out, decided).await;
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurations_are_checked() {
        assert_eq!(I2cConfig::default().validate(), Ok(()));
        assert_eq!(
            I2cConfig { scl: 3, sda: 3 }.validate(),
            Err(I2cConfigError::DuplicateChannel(3))
        );
        assert_eq!(
            I2cConfig { scl: 0, sda: 64 }.validate(),
            Err(I2cConfigError::Channel(64))
        );
        assert!(I2c::new(I2cConfig { scl: 64, sda: 0 }).is_err());
    }

    #[test]
    fn props_describe_the_bus() {
        let props = I2cConfig::default().props();
        assert_eq!(props.get("bus"), Some(&"i2c".into()));
        assert_eq!(props.get("framing"), Some(&"start-stop".into()));
        assert_eq!(props.get("tenBitAddressing"), Some(&true.into()));
        let spec = I2c::new(I2cConfig { scl: 7, sda: 2 })
            .expect("valid")
            .spec();
        assert!(
            spec.inputs[0]
                .requires
                .unmet(&Props::new().with("channels", 8))
                .is_none()
        );
        assert!(
            spec.inputs[0]
                .requires
                .unmet(&Props::new().with("channels", 7))
                .is_some()
        );
    }

    fn address(transfer: &mut Transfer, tick: u64, byte: u8, ack: bool) -> Option<I2cEvent> {
        let mut found = None;
        for i in 0..8 {
            found = transfer.bit(Tick(tick + i), byte >> (7 - i) & 1 != 0);
        }
        assert!(found.is_none(), "eight bits make no byte without the ninth");
        transfer.bit(Tick(tick + 8), !ack).map(|(_, event)| event)
    }

    #[test]
    fn ten_bit_reads_refer_back_to_the_last_write_address() {
        let mut transfer = Transfer::new(1);
        assert_eq!(address(&mut transfer, 0, 0b1111_0100, true), None);
        assert_eq!(
            address(&mut transfer, 10, 0xa5, false),
            Some(I2cEvent::Address {
                tx: 1,
                address: 0x2a5,
                ten_bit: true,
                read: false,
                ack: false,
            })
        );
        transfer.next = Expect::Address;
        assert_eq!(
            address(&mut transfer, 20, 0b1111_0101, true),
            Some(I2cEvent::Address {
                tx: 1,
                address: 0x2a5,
                ten_bit: true,
                read: true,
                ack: true,
            })
        );
        // Other high bits refer to nothing: the byte is the 7-bit address it is, and the
        // 10-bit address is no longer the one addressed.
        transfer.next = Expect::Address;
        assert_eq!(
            address(&mut transfer, 30, 0b1111_0011, true),
            Some(I2cEvent::Address {
                tx: 1,
                address: 0x79,
                ten_bit: false,
                read: true,
                ack: true,
            })
        );
        transfer.next = Expect::Address;
        assert_eq!(
            address(&mut transfer, 40, 0b1111_0101, true),
            Some(I2cEvent::Address {
                tx: 1,
                address: 0x7a,
                ten_bit: false,
                read: true,
                ack: true,
            })
        );
    }
}
