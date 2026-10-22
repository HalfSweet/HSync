// SPDX-License-Identifier: Apache-2.0

use super::{EndReason, QspiEvent};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, Node, NodeFuture, NodeId,
    NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample, SignalCursor,
    Tick, TickRange,
};

/// A fixed-lane SPI SDR view; data lists channels in IO0, IO1, IO2, IO3 order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QspiConfig {
    /// Optional CS channel; absent means continuous words in transaction zero.
    pub cs: Option<u8>,
    /// Clock channel.
    pub clk: u8,
    /// One, two or four data channels, least significant lane first.
    pub data: Vec<u8>,
    /// SPI mode, 0 through 3.
    pub mode: u8,
    /// Word width, 1 through 32, divisible by lane count.
    pub word_bits: u8,
    /// Low asserted CS when true.
    pub cs_active_low: bool,
    /// First lane group occupies the least significant bits when true.
    pub lsb_first: bool,
}
impl Default for QspiConfig {
    fn default() -> Self {
        Self {
            cs: Some(0),
            clk: 1,
            data: vec![2, 3, 4, 5],
            mode: 0,
            word_bits: 8,
            cs_active_low: true,
            lsb_first: false,
        }
    }
}
/// Invalid wiring or transport options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QspiConfigError {
    /// Unsupported lane count.
    #[error("qspi.config.lanes")]
    Lanes,
    /// Unsupported clock mode.
    #[error("qspi.config.mode")]
    Mode,
    /// Unsupported word width.
    #[error("qspi.config.wordBits")]
    WordBits,
    /// Channel outside 0..64.
    #[error("qspi.config.channel: {0}")]
    Channel(u8),
    /// A channel appears twice.
    #[error("qspi.config.duplicateChannel: {0}")]
    DuplicateChannel(u8),
}
impl QspiConfig {
    /// All connected channels.
    pub fn channels(&self) -> impl Iterator<Item = u8> + '_ {
        self.cs
            .into_iter()
            .chain([self.clk])
            .chain(self.data.iter().copied())
    }
    /// Validate wiring and options.
    pub fn validate(&self) -> Result<(), QspiConfigError> {
        if !matches!(self.data.len(), 1 | 2 | 4) {
            return Err(QspiConfigError::Lanes);
        }
        if self.mode > 3 {
            return Err(QspiConfigError::Mode);
        }
        if self.word_bits == 0
            || self.word_bits > 32
            || !usize::from(self.word_bits).is_multiple_of(self.data.len())
        {
            return Err(QspiConfigError::WordBits);
        }
        let mut seen = ChannelMask::EMPTY;
        for c in self.channels() {
            if c >= 64 {
                return Err(QspiConfigError::Channel(c));
            }
            if seen.has(c) {
                return Err(QspiConfigError::DuplicateChannel(c));
            }
            seen = seen.with(c);
        }
        Ok(())
    }
    /// Properties downstream consumers can require.
    pub fn props(&self) -> Props {
        Props::new()
            .with("cs", self.cs.is_some())
            .with("lanes", self.data.len() as i64)
            .with("wordBits", i64::from(self.word_bits))
            .with("mode", i64::from(self.mode))
            .with(
                "bitOrder",
                if self.lsb_first {
                    "lsb-first"
                } else {
                    "msb-first"
                },
            )
            .with("sdr", true)
    }
    pub(super) fn selected(&self, levels: Levels) -> bool {
        self.cs
            .is_none_or(|cs| levels.high(cs) != self.cs_active_low)
    }
    pub(super) fn idle_high(&self) -> bool {
        self.mode & 2 != 0
    }
    fn sample_high(&self) -> bool {
        self.idle_high() == (self.mode & 1 != 0)
    }
}
/// A bounded streaming SDR decoder.
#[derive(Clone, Debug)]
pub struct Qspi {
    config: QspiConfig,
}
impl Qspi {
    /// Construct after validating the wiring.
    pub fn new(config: QspiConfig) -> Result<Self, QspiConfigError> {
        config.validate()?;
        Ok(Self { config })
    }
    /// Typed logic input.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }
    /// Typed transport output.
    pub fn events(node: NodeId) -> OutputRef<QspiEvent> {
        OutputRef::of_node(node, 0)
    }
}
impl Node for Qspi {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            name: "qspi".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least(
                    "channels",
                    i64::from(self.config.channels().max().expect("clock")) + 1,
                ),
            )],
            outputs: vec![OutputSpec::new::<QspiEvent>("events", self.config.props())],
        }
    }
    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}
#[derive(Default)]
struct Shift {
    value: u32,
    bits: u8,
    first: Option<Tick>,
}
async fn close(
    out: &mut Outbox<QspiEvent>,
    active: &mut Option<u64>,
    shift: &mut Shift,
    tick: Tick,
    reason: EndReason,
    framed: bool,
) {
    if let Some(tx) = active.take()
        && framed
    {
        out.emit(
            if reason == EndReason::Aborted {
                Provenance::at(tick)
            } else {
                Provenance::one(tick)
            },
            QspiEvent::End {
                tx,
                reason,
                dropped_bits: shift.bits,
            },
        )
        .await;
    }
    *shift = Shift::default();
}
async fn run(mut signal: SignalCursor, mut out: Outbox<QspiEvent>, config: QspiConfig) {
    let framed = config.cs.is_some();
    let mut watch = ChannelMask::one(config.clk);
    if let Some(cs) = config.cs {
        watch = watch.with(cs);
    }
    let mut next_tx = 1u64;
    let mut active = None;
    let mut shift = Shift::default();
    'resync: loop {
        let (tick, levels) = loop {
            match signal.sample().await {
                Sample::At { tick, levels } => break (tick, levels),
                Sample::Gap(range) => out.gap(range).await,
                Sample::Progress(tick) => out.progress(tick).await,
                Sample::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        };
        if config.selected(levels) {
            let tx = if framed { next_tx } else { 0 };
            if framed {
                next_tx += 1;
                out.emit(
                    Provenance::one(tick),
                    QspiEvent::Begin { tx, partial: true },
                )
                .await;
            }
            active = Some(tx);
        }
        loop {
            match signal.next_change(watch).await {
                Change::At {
                    tick,
                    changed,
                    levels,
                } => {
                    if config.cs.is_some_and(|cs| changed.has(cs)) {
                        if config.selected(levels) {
                            let tx = next_tx;
                            next_tx += 1;
                            active = Some(tx);
                            shift = Shift::default();
                            out.emit(
                                Provenance::one(tick),
                                QspiEvent::Begin { tx, partial: false },
                            )
                            .await;
                        } else {
                            close(
                                &mut out,
                                &mut active,
                                &mut shift,
                                tick,
                                EndReason::Release,
                                framed,
                            )
                            .await;
                        }
                        continue;
                    }
                    if changed.has(config.clk)
                        && levels.high(config.clk) == config.sample_high()
                        && let Some(tx) = active
                    {
                        let lanes = config.data.len() as u8;
                        let group = config.data.iter().enumerate().fold(0u32, |value, (i, &c)| {
                            value | (u32::from(levels.high(c)) << i)
                        });
                        shift.first.get_or_insert(tick);
                        if config.lsb_first {
                            shift.value |= group << shift.bits;
                        } else {
                            shift.value = (shift.value << lanes) | group;
                        }
                        shift.bits += lanes;
                        if shift.bits == config.word_bits {
                            let first = shift.first.expect("first group");
                            out.emit(
                                Provenance::over(TickRange::new(first, tick.next()), tick),
                                QspiEvent::Word {
                                    tx,
                                    value: shift.value,
                                },
                            )
                            .await;
                            shift = Shift::default();
                        }
                    }
                }
                Change::Gap(range) => {
                    close(
                        &mut out,
                        &mut active,
                        &mut shift,
                        range.start,
                        EndReason::Aborted,
                        framed,
                    )
                    .await;
                    out.gap(range).await;
                    continue 'resync;
                }
                Change::Progress(tick) => {
                    out.progress(shift.first.map_or(tick, |first| first.min(tick)))
                        .await
                }
                Change::Quiet(_) => {}
                Change::End(tick) => {
                    close(
                        &mut out,
                        &mut active,
                        &mut shift,
                        tick,
                        EndReason::Aborted,
                        framed,
                    )
                    .await;
                    out.end(tick).await;
                    return;
                }
            }
        }
    }
}
