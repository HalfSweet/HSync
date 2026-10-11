// SPDX-License-Identifier: Apache-2.0

//! The SPI decoder: words shifted on the sampling clock edge while chip select is asserted,
//! each chip select period a transaction.
//!
//! The rules, which the mode only parametrizes:
//!
//! - a bit is taken on the sampling edge of the clock: rising when CPOL equals CPHA (modes 0
//!   and 3), falling otherwise (modes 1 and 2);
//! - bits are taken only while chip select is asserted; a bus without chip select is one
//!   unframed stream of words, transaction 0, with no begin or end;
//! - a clock edge at the very tick chip select changes belongs to neither transaction;
//! - bits left over when chip select is released, the samples end or a gap starts do not make
//!   a word and are dropped;
//! - chip select found asserted at the first sample, or at the first sample after a gap,
//!   begins a transaction marked partial: its earlier words were not seen;
//! - a gap or the end of the samples while chip select is asserted aborts the transaction.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{EndReason, SpiEvent, TxId};

/// Which bit of a word is shifted first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BitOrder {
    /// The most significant bit first.
    #[default]
    MsbFirst,
    /// The least significant bit first.
    LsbFirst,
}

impl BitOrder {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            BitOrder::MsbFirst => "msb-first",
            BitOrder::LsbFirst => "lsb-first",
        }
    }
}

/// How the bus is wired and clocked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpiConfig {
    /// The clock channel.
    pub clk: u8,
    /// The chip select channel, if the bus has one.
    pub cs: Option<u8>,
    /// The master-out channel, if connected.
    pub mosi: Option<u8>,
    /// The master-in channel, if connected.
    pub miso: Option<u8>,
    /// The SPI mode, 0 to 3: bit 1 is CPOL, bit 0 is CPHA.
    pub mode: u8,
    /// Bits per word, 1 to 8.
    pub word_bits: u8,
    /// Which bit comes first.
    pub bit_order: BitOrder,
    /// Whether chip select is asserted low.
    pub cs_active_low: bool,
}

impl Default for SpiConfig {
    fn default() -> Self {
        Self {
            clk: 0,
            cs: Some(1),
            mosi: Some(2),
            miso: Some(3),
            mode: 0,
            word_bits: 8,
            bit_order: BitOrder::MsbFirst,
            cs_active_low: true,
        }
    }
}

impl SpiConfig {
    /// Clock polarity: whether the clock idles high.
    pub const fn cpol(&self) -> bool {
        self.mode & 0b10 != 0
    }

    /// Clock phase: whether bits are taken on the second edge of each clock period.
    pub const fn cpha(&self) -> bool {
        self.mode & 0b01 != 0
    }

    /// Whether bits are taken on the rising edge of the clock.
    pub const fn samples_on_rising(&self) -> bool {
        self.cpol() == self.cpha()
    }

    /// Whether `levels` show chip select asserted; always, without a chip select line.
    pub fn selected(&self, levels: Levels) -> bool {
        match self.cs {
            Some(cs) => levels.high(cs) != self.cs_active_low,
            None => true,
        }
    }

    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> {
        [Some(self.clk), self.cs, self.mosi, self.miso]
            .into_iter()
            .flatten()
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), SpiConfigError> {
        if self.mode > 3 {
            return Err(SpiConfigError::Mode(self.mode));
        }
        if !(1..=8).contains(&self.word_bits) {
            return Err(SpiConfigError::WordBits(self.word_bits));
        }
        if self.mosi.is_none() && self.miso.is_none() {
            return Err(SpiConfigError::NoDataLine);
        }
        let mut seen = ChannelMask::EMPTY;
        for channel in self.channels() {
            if channel >= MAX_CHANNELS {
                return Err(SpiConfigError::Channel(channel));
            }
            if seen.has(channel) {
                return Err(SpiConfigError::DuplicateChannel(channel));
            }
            seen = seen.with(channel);
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        Props::new()
            .with("wordBits", i64::from(self.word_bits))
            .with("bitOrder", self.bit_order.name())
            .with("mode", i64::from(self.mode))
            .with("cs", self.cs.is_some())
            .with("mosi", self.mosi.is_some())
            .with("miso", self.miso.is_some())
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpiConfigError {
    /// The mode is not 0 to 3.
    #[error("SPI mode {0} does not exist; modes are 0 to 3")]
    Mode(u8),
    /// The word width is not 1 to 8 bits.
    #[error("{0} bits per word; 1 to 8 are supported")]
    WordBits(u8),
    /// Neither data line is connected.
    #[error("neither MOSI nor MISO is connected")]
    NoDataLine,
    /// A channel is used for two lines.
    #[error("channel {0} is used for two lines")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The SPI decoder node: logic samples in, `spi.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spi {
    config: SpiConfig,
}

impl Spi {
    /// A decoder for `config`.
    pub fn new(config: SpiConfig) -> Result<Self, SpiConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &SpiConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<SpiEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Spi {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "spi".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<SpiEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

/// The transaction under way.
#[derive(Debug)]
struct Transfer {
    tx: TxId,
    /// Bits taken into the current word.
    bits: u8,
    mosi: u8,
    miso: u8,
    /// The tick of the current word's first bit.
    first_bit: Tick,
}

impl Transfer {
    fn new(tx: TxId) -> Self {
        Self {
            tx,
            bits: 0,
            mosi: 0,
            miso: 0,
            first_bit: Tick::ZERO,
        }
    }
}

struct Decoder {
    config: SpiConfig,
    watch: ChannelMask,
    next_tx: u64,
    transfer: Option<Transfer>,
}

impl Decoder {
    fn new(config: SpiConfig) -> Self {
        let mut watch = ChannelMask::one(config.clk);
        if let Some(cs) = config.cs {
            watch = watch.with(cs);
        }
        Self {
            config,
            watch,
            next_tx: 1,
            transfer: None,
        }
    }

    fn begin(&mut self) -> TxId {
        let tx = TxId(self.next_tx);
        self.next_tx += 1;
        self.transfer = Some(Transfer::new(tx));
        tx
    }

    /// Takes a bit from the data lines; the word, once whole.
    fn shift(&mut self, tick: Tick, levels: Levels) -> Option<(Provenance, SpiEvent)> {
        let config = &self.config;
        let transfer = self.transfer.as_mut()?;
        if transfer.bits == 0 {
            transfer.first_bit = tick;
        }
        let bit = |line: Option<u8>| line.is_some_and(|channel| levels.high(channel));
        match config.bit_order {
            BitOrder::MsbFirst => {
                transfer.mosi = (transfer.mosi << 1) | u8::from(bit(config.mosi));
                transfer.miso = (transfer.miso << 1) | u8::from(bit(config.miso));
            }
            BitOrder::LsbFirst => {
                transfer.mosi |= u8::from(bit(config.mosi)) << transfer.bits;
                transfer.miso |= u8::from(bit(config.miso)) << transfer.bits;
            }
        }
        transfer.bits += 1;
        if transfer.bits < config.word_bits {
            return None;
        }
        let word = SpiEvent::Word {
            tx: transfer.tx,
            mosi: config.mosi.map(|_| transfer.mosi),
            miso: config.miso.map(|_| transfer.miso),
        };
        let at = Provenance::over(TickRange::new(transfer.first_bit, tick.next()), tick);
        transfer.bits = 0;
        transfer.mosi = 0;
        transfer.miso = 0;
        Some((at, word))
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<SpiEvent>, config: SpiConfig) {
    let mut decoder = Decoder::new(config);
    let framed = decoder.config.cs.is_some();
    // Without chip select every word belongs to transaction 0, which never begins or ends.
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
        let mut selected = decoder.config.selected(levels);
        if framed && selected {
            let tx = decoder.begin();
            out.emit(
                Provenance::one(signal.position()),
                SpiEvent::Begin { tx, partial: true },
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
                    if let Some(cs) = decoder.config.cs
                        && changed.has(cs)
                    {
                        selected = decoder.config.selected(levels);
                        if selected {
                            let tx = decoder.begin();
                            out.emit(
                                Provenance::one(tick),
                                SpiEvent::Begin { tx, partial: false },
                            )
                            .await;
                        } else if let Some(transfer) = decoder.transfer.take() {
                            let end = SpiEvent::End {
                                tx: transfer.tx,
                                reason: EndReason::CsRelease,
                            };
                            out.emit(Provenance::one(tick), end).await;
                        }
                        // A clock edge at this very tick belongs to neither transaction.
                        continue;
                    }
                    if selected
                        && changed.has(decoder.config.clk)
                        && levels.high(decoder.config.clk) == decoder.config.samples_on_rising()
                        && let Some((at, word)) = decoder.shift(tick, levels)
                    {
                        out.emit(at, word).await;
                    }
                }
                Change::Gap(range) => {
                    if framed && let Some(transfer) = decoder.transfer.take() {
                        let end = SpiEvent::End {
                            tx: transfer.tx,
                            reason: EndReason::Aborted,
                        };
                        out.emit(Provenance::at(range.start), end).await;
                    } else if let Some(transfer) = decoder.transfer.as_mut() {
                        transfer.bits = 0;
                        transfer.mosi = 0;
                        transfer.miso = 0;
                    }
                    out.gap(range).await;
                    continue 'resync;
                }
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => out.progress(tick).await,
                Change::End(tick) => {
                    if framed && let Some(transfer) = decoder.transfer.take() {
                        let end = SpiEvent::End {
                            tx: transfer.tx,
                            reason: EndReason::Aborted,
                        };
                        out.emit(Provenance::at(tick), end).await;
                    }
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
    fn modes_pick_the_sampling_edge() {
        let mode = |mode| SpiConfig {
            mode,
            ..SpiConfig::default()
        };
        assert!(mode(0).samples_on_rising());
        assert!(!mode(1).samples_on_rising());
        assert!(!mode(2).samples_on_rising());
        assert!(mode(3).samples_on_rising());
        assert!(mode(2).cpol());
        assert!(mode(1).cpha());
    }

    #[test]
    fn configurations_are_checked() {
        assert_eq!(SpiConfig::default().validate(), Ok(()));
        let bad = |config: SpiConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(SpiConfig {
                mode: 4,
                ..SpiConfig::default()
            }),
            SpiConfigError::Mode(4)
        );
        assert_eq!(
            bad(SpiConfig {
                word_bits: 9,
                ..SpiConfig::default()
            }),
            SpiConfigError::WordBits(9)
        );
        assert_eq!(
            bad(SpiConfig {
                word_bits: 0,
                ..SpiConfig::default()
            }),
            SpiConfigError::WordBits(0)
        );
        assert_eq!(
            bad(SpiConfig {
                mosi: None,
                miso: None,
                ..SpiConfig::default()
            }),
            SpiConfigError::NoDataLine
        );
        assert_eq!(
            bad(SpiConfig {
                miso: Some(0),
                ..SpiConfig::default()
            }),
            SpiConfigError::DuplicateChannel(0)
        );
        assert_eq!(
            bad(SpiConfig {
                miso: Some(64),
                ..SpiConfig::default()
            }),
            SpiConfigError::Channel(64)
        );
    }

    #[test]
    fn props_describe_the_bus() {
        let props = SpiConfig {
            miso: None,
            ..SpiConfig::default()
        }
        .props();
        assert_eq!(props.get("wordBits"), Some(&8i64.into()));
        assert_eq!(props.get("bitOrder"), Some(&"msb-first".into()));
        assert_eq!(props.get("cs"), Some(&true.into()));
        assert_eq!(props.get("miso"), Some(&false.into()));
        let spi = Spi::new(SpiConfig {
            clk: 5,
            cs: None,
            ..SpiConfig::default()
        })
        .expect("valid");
        let spec = spi.spec();
        assert!(
            spec.inputs[0]
                .requires
                .unmet(&Props::new().with("channels", 6))
                .is_none()
        );
        assert!(
            spec.inputs[0]
                .requires
                .unmet(&Props::new().with("channels", 5))
                .is_some()
        );
    }
}
