// SPDX-License-Identifier: Apache-2.0

//! The Microwire decoder: the bits clocked on SK while chip select is asserted, each chip
//! select period a transaction.
//!
//! Implemented from the serial interface the 93C46, 93C56, 93C66 and 93C86 EEPROM datasheets
//! describe (Microchip 93AA46/93LC46/93C46, Atmel AT93C46, ST M93C46): chip select is active
//! high; the device reads DI on the rising edge of SK, and drives DO, which changes a short
//! delay (TPD) after the rising edge and holds until the next one.
//!
//! The rules:
//!
//! - a bit is a clock pulse while chip select is asserted: DI is read on SK's rising edge, DO
//!   on the falling edge after it, where the device's output has settled — DO is read late
//!   rather than at the next rising edge, where a fast device may already have moved on; a
//!   bus without DO reports each bit at its rising edge;
//! - a bit spans its rising edge through the falling edge DO was read on, and is settled
//!   there; without DO it is the rising edge alone;
//! - chip select released before the falling edge of a bit ends the bit at the release, DO
//!   read on the last sample before it; the bit spans its rising edge up to the release;
//! - a clock edge at the very tick chip select changes belongs to neither transaction;
//! - chip select is active high, or low when configured so; a bus without chip select is one
//!   unframed stream of bits, transaction 0, with no begin or end;
//! - chip select found asserted at the first sample, or at the first sample after a gap,
//!   begins a transaction marked partial: its earlier bits were not seen. SK found high there
//!   is a bit whose rising edge was not seen, and is not reported;
//! - a gap or the end of the samples while chip select is asserted aborts the transaction; a
//!   bit whose falling edge they cut off is not reported;
//! - the transaction's end reports the level DO had on its last sample, where a device's
//!   ready or busy state shows;
//! - progress is passed on, but never past the rising edge of a bit whose falling edge is
//!   still to come.
//!
//! The decoder follows edges: the sample rate does not enter, and every clock level needs a
//! sample of its own. When the clock's high phase is shorter than the device's output delay,
//! DO changes after the falling edge and is read one bit late.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{EndReason, MicrowireEvent, TxId};

/// How the bus is wired. A description may leave fields out: they take the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MicrowireConfig {
    /// The chip select channel, if the bus has one.
    pub cs: Option<u8>,
    /// The clock channel.
    pub sk: u8,
    /// The channel of DI, the data the controller sends.
    pub di: u8,
    /// The channel of DO, the data the device sends, if connected.
    pub r#do: Option<u8>,
    /// Whether chip select is asserted low; the 93Cxx EEPROMs assert it high.
    pub cs_active_low: bool,
}

impl Default for MicrowireConfig {
    fn default() -> Self {
        Self {
            cs: Some(0),
            sk: 1,
            di: 2,
            r#do: Some(3),
            cs_active_low: false,
        }
    }
}

impl MicrowireConfig {
    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> {
        [self.cs, Some(self.sk), Some(self.di), self.r#do]
            .into_iter()
            .flatten()
    }

    /// Whether `levels` show chip select asserted; always, without a chip select line.
    pub fn selected(&self, levels: Levels) -> bool {
        match self.cs {
            Some(cs) => levels.high(cs) != self.cs_active_low,
            None => true,
        }
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), MicrowireConfigError> {
        let mut seen = ChannelMask::EMPTY;
        for channel in self.channels() {
            if channel >= MAX_CHANNELS {
                return Err(MicrowireConfigError::Channel(channel));
            }
            if seen.has(channel) {
                return Err(MicrowireConfigError::DuplicateChannel(channel));
            }
            seen = seen.with(channel);
        }
        Ok(())
    }

    /// What the decoder's output port declares: which lines the bus has.
    pub fn props(&self) -> Props {
        Props::new()
            .with("cs", self.cs.is_some())
            .with("di", true)
            .with("do", self.r#do.is_some())
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MicrowireConfigError {
    /// A channel is used for two lines.
    #[error("channel {0} is used for two lines")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The Microwire decoder node: logic samples in, `microwire.events` out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Microwire {
    config: MicrowireConfig,
}

impl Microwire {
    /// A decoder for `config`.
    pub fn new(config: MicrowireConfig) -> Result<Self, MicrowireConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &MicrowireConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<MicrowireEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Microwire {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "microwire".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<MicrowireEvent>(
                "events",
                self.config.props(),
            )],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

/// A bit whose rising edge was seen and whose falling edge is awaited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    rise: Tick,
    di: bool,
}

/// The transaction under way.
#[derive(Debug)]
struct Transfer {
    tx: TxId,
    bits: u32,
    pending: Option<Pending>,
}

impl Transfer {
    fn new(tx: TxId) -> Self {
        Self {
            tx,
            bits: 0,
            pending: None,
        }
    }
}

struct Decoder {
    config: MicrowireConfig,
    watch: ChannelMask,
    next_tx: u64,
    transfer: Option<Transfer>,
    /// DO's level on the last tick seen.
    out_level: bool,
}

impl Decoder {
    fn new(config: MicrowireConfig) -> Self {
        let mut watch = ChannelMask::one(config.sk);
        for channel in [config.cs, config.r#do].into_iter().flatten() {
            watch = watch.with(channel);
        }
        Self {
            config,
            watch,
            next_tx: 1,
            transfer: None,
            out_level: false,
        }
    }

    fn begin(&mut self) -> TxId {
        let tx = TxId(self.next_tx);
        self.next_tx += 1;
        self.transfer = Some(Transfer::new(tx));
        tx
    }

    /// DO's level in `levels`, if connected.
    fn data_out(&self, levels: Levels) -> Option<bool> {
        self.config.r#do.map(|channel| levels.high(channel))
    }

    /// DO's level as last seen, if connected.
    fn last_out(&self) -> Option<bool> {
        self.config.r#do.map(|_| self.out_level)
    }

    /// Notes DO's level in `levels`.
    fn note(&mut self, levels: Levels) {
        if let Some(level) = self.data_out(levels) {
            self.out_level = level;
        }
    }

    /// The tick progress must not pass: the rising edge of a bit still under way.
    fn hold(&self, tick: Tick) -> Tick {
        self.transfer
            .as_ref()
            .and_then(|transfer| transfer.pending)
            .map_or(tick, |pending| pending.rise.min(tick))
    }
}

/// The events one change of the lines decides: at most two.
#[derive(Debug, Default)]
struct Decided([Option<(Provenance, MicrowireEvent)>; 2]);

impl Decided {
    fn push(&mut self, at: Provenance, event: MicrowireEvent) {
        if let Some(slot) = self.0.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some((at, event));
        }
    }
}

async fn emit(out: &mut Outbox<MicrowireEvent>, decided: Decided) {
    for (at, event) in decided.0.into_iter().flatten() {
        out.emit(at, event).await;
    }
}

impl Decoder {
    /// What the lines changing at `tick` decide.
    fn change(&mut self, tick: Tick, changed: ChannelMask, levels: Levels) -> Decided {
        let mut decided = Decided::default();
        if let Some(cs) = self.config.cs
            && changed.has(cs)
        {
            if self.config.selected(levels) {
                let tx = self.begin();
                decided.push(
                    Provenance::one(tick),
                    MicrowireEvent::Begin { tx, partial: false },
                );
            } else if let Some(mut transfer) = self.transfer.take() {
                // A bit whose falling edge did not come before the release takes DO from the
                // sample before it.
                let before = self.last_out();
                if let Some(pending) = transfer.pending.take() {
                    transfer.bits += 1;
                    let bit = MicrowireEvent::Bit {
                        tx: transfer.tx,
                        di: pending.di,
                        r#do: before,
                    };
                    decided.push(
                        Provenance::over(TickRange::new(pending.rise, tick), tick),
                        bit,
                    );
                }
                let end = MicrowireEvent::End {
                    tx: transfer.tx,
                    bits: transfer.bits,
                    reason: EndReason::Release,
                    r#do: before,
                };
                decided.push(Provenance::one(tick), end);
            }
            // A clock edge at this very tick belongs to neither transaction.
            self.note(levels);
            return decided;
        }
        if changed.has(self.config.sk)
            && let Some(transfer) = self.transfer.as_mut()
        {
            let di = levels.high(self.config.di);
            if levels.high(self.config.sk) {
                match self.config.r#do {
                    Some(_) => transfer.pending = Some(Pending { rise: tick, di }),
                    None => {
                        transfer.bits += 1;
                        let bit = MicrowireEvent::Bit {
                            tx: transfer.tx,
                            di,
                            r#do: None,
                        };
                        decided.push(Provenance::one(tick), bit);
                    }
                }
            } else if let Some(pending) = transfer.pending.take() {
                transfer.bits += 1;
                let bit = MicrowireEvent::Bit {
                    tx: transfer.tx,
                    di: pending.di,
                    r#do: self.config.r#do.map(|channel| levels.high(channel)),
                };
                decided.push(
                    Provenance::over(TickRange::new(pending.rise, tick.next()), tick),
                    bit,
                );
            }
        }
        self.note(levels);
        decided
    }

    /// The samples stop at `tick`, for a gap or for good: the transaction is cut off.
    fn cut(&mut self, tick: Tick) -> Decided {
        let mut decided = Decided::default();
        if self.config.cs.is_none() {
            if let Some(transfer) = self.transfer.as_mut() {
                transfer.pending = None;
            }
            return decided;
        }
        if let Some(transfer) = self.transfer.take() {
            let end = MicrowireEvent::End {
                tx: transfer.tx,
                bits: transfer.bits,
                reason: EndReason::Aborted,
                r#do: self.last_out(),
            };
            decided.push(Provenance::at(tick), end);
        }
        decided
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<MicrowireEvent>, config: MicrowireConfig) {
    let mut decoder = Decoder::new(config);
    let framed = config.cs.is_some();
    // Without chip select every bit belongs to transaction 0, which never begins or ends.
    if !framed {
        decoder.transfer = Some(Transfer::new(TxId(0)));
    }
    // Each round of this loop starts where levels are unknown: at the first sample and after
    // every gap.
    'resync: loop {
        let levels = loop {
            match signal.sample().await {
                Sample::At { levels, .. } => break levels,
                Sample::Gap(range) => out.gap(range).await,
                Sample::Progress(tick) => out.progress(tick).await,
                Sample::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        };
        decoder.note(levels);
        if framed && config.selected(levels) {
            let tx = decoder.begin();
            out.emit(
                Provenance::one(signal.position()),
                MicrowireEvent::Begin { tx, partial: true },
            )
            .await;
        }
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
        assert_eq!(MicrowireConfig::default().validate(), Ok(()));
        let bad = |config: MicrowireConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(MicrowireConfig {
                r#do: Some(1),
                ..MicrowireConfig::default()
            }),
            MicrowireConfigError::DuplicateChannel(1)
        );
        assert_eq!(
            bad(MicrowireConfig {
                cs: Some(64),
                ..MicrowireConfig::default()
            }),
            MicrowireConfigError::Channel(64)
        );
        assert!(
            Microwire::new(MicrowireConfig {
                cs: None,
                r#do: None,
                ..MicrowireConfig::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn descriptions_may_leave_fields_out() {
        let config: MicrowireConfig =
            serde_json::from_str(r#"{"do": null, "cs": 5}"#).expect("parses");
        assert_eq!(
            config,
            MicrowireConfig {
                cs: Some(5),
                r#do: None,
                ..MicrowireConfig::default()
            }
        );
        let whole: MicrowireConfig = serde_json::from_str("{}").expect("parses");
        assert_eq!(whole, MicrowireConfig::default());
        assert!(serde_json::from_str::<MicrowireConfig>(r#"{"clk": 0}"#).is_err());
        assert_eq!(
            serde_json::to_string(&MicrowireConfig::default()).expect("serializes"),
            r#"{"cs":0,"sk":1,"di":2,"do":3,"cs_active_low":false}"#
        );
    }

    #[test]
    fn props_say_which_lines_the_bus_has() {
        let props = MicrowireConfig {
            r#do: None,
            ..MicrowireConfig::default()
        }
        .props();
        assert_eq!(props.get("cs"), Some(&true.into()));
        assert_eq!(props.get("di"), Some(&true.into()));
        assert_eq!(props.get("do"), Some(&false.into()));
        let spec = Microwire::new(MicrowireConfig {
            cs: None,
            sk: 7,
            ..MicrowireConfig::default()
        })
        .expect("valid")
        .spec();
        assert_eq!(spec.outputs[0].props.get("cs"), Some(&false.into()));
        let source = |channels: i64| {
            spec.inputs[0]
                .requires
                .unmet(&Props::new().with("channels", channels))
                .map(|(key, _)| key)
        };
        assert_eq!(source(8), None);
        assert_eq!(source(7), Some("channels"));
    }

    #[test]
    fn chip_select_may_be_active_low() {
        let config = MicrowireConfig {
            cs_active_low: true,
            ..MicrowireConfig::default()
        };
        assert!(config.selected(Levels(0)));
        assert!(!config.selected(Levels(1)));
        assert!(MicrowireConfig::default().selected(Levels(1)));
    }
}
