// SPDX-License-Identifier: Apache-2.0

//! The I²S decoder: audio samples shifted out on a serial clock, one word per half-period of
//! the word select line, in the I²S format or its left- and right-justified variants.
//!
//! Implemented from Philips Semiconductors, *I²S bus specification* (February 1986, revised
//! June 5, 1996): three lines, the continuous serial clock SCK, word select WS (low for the
//! left channel, high for the right) and serial data SD, MSB first and two's complement; the
//! transmitter changes WS and SD on one clock edge and the receiver latches both on the other,
//! and WS changes one clock period before the MSB of the next word. The left- and
//! right-justified formats are common variants that align the word with the WS change or
//! with the end of the half-period instead. The rules:
//!
//! - WS and SD are read on the sampling edge of SCK, rising unless configured otherwise, at
//!   the first sample with the clock's new level, so a line that changes at that very tick is
//!   read with its new level; with `ws_inverted`, WS high is the left channel;
//! - a half-period begins at the first sampling edge that reads WS at another level than the
//!   sampling edge before it: that is the WS change that marks the boundary between the
//!   channels, and a glitch on WS between two sampling edges is none;
//! - in the I²S format, the word's first bit (its MSB) is read on the sampling edge after the
//!   one that found WS changed — one clock of delay — so a word as long as its half-period
//!   ends with the bit read on the sampling edge that finds the next change; left-justified,
//!   the MSB is read on the sampling edge that found WS changed; right-justified, the word is
//!   the last `bits` bits read before the next change;
//! - with a fixed width (`bits`), a word is whole with its `bits`-th bit and the bits after it
//!   are padding; a half-period that ends sooner yields the bits that arrived, marked
//!   incomplete. Without one, the word is every bit of its half-period, at most
//!   [`MAX_WORD_BITS`] — a longer half-period keeps its first 32 bits, as a 32-bit receiver
//!   does — and is complete;
//! - frames are counted from 1: a left half-period begins a frame, and so does the first
//!   half-period after the start of the samples or after a gap, whichever its channel; frames
//!   lost in a gap are not counted;
//! - the half-period under way at the first sample, or at the first sample after a gap, is
//!   not reported, since where its word began is unknown; the WS level at that sample is what
//!   the first sampling edge compares with;
//! - a gap or the end of the samples drops the word under way: leftover bits never make a
//!   word;
//! - progress is passed on, held back to the first bit of the word under way —
//!   right-justified, the first of the last `bits` bits read — since the word spans from there;
//! - a word covers the ticks of its first bit to its last and is settled where it is decided:
//!   at the bit that makes it whole, or else at the sampling edge that found WS changed — for
//!   an incomplete word, a word whose width WS decides and every right-justified word; in the
//!   I²S format that edge reads the word's last bit.
//!
//! Limits: words of 1 to 32 bits and two channels per WS period; TDM buses with more words
//! per period, and the DSP modes whose frame sync is a pulse rather than a level per channel,
//! are out of scope.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, LogicBlock, MAX_CHANNELS, Node, NodeFuture,
    NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires, Sample,
    SignalCursor, Tick, TickRange,
};

use super::events::{Channel, I2sEvent};

/// The most bits a word has.
pub const MAX_WORD_BITS: u8 = 32;

/// Where the word sits in its half-period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// The Philips I²S format: the MSB one clock after the WS change.
    #[default]
    I2s,
    /// Left-justified: the MSB with the WS change.
    LeftJustified,
    /// Right-justified: the LSB on the last clock before the next WS change.
    RightJustified,
}

impl Format {
    /// The name the port properties use.
    pub const fn name(self) -> &'static str {
        match self {
            Format::I2s => "i2s",
            Format::LeftJustified => "left-justified",
            Format::RightJustified => "right-justified",
        }
    }
}

/// The clock edge on which WS and SD are read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SampleEdge {
    /// The rising edge, as the specification has it.
    #[default]
    Rising,
    /// The falling edge, for buses that clock the other way round.
    Falling,
}

/// How the bus is wired and how its words are framed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct I2sConfig {
    /// The serial clock's channel.
    pub sck: u8,
    /// The word select channel.
    pub ws: u8,
    /// The serial data channel.
    pub sd: u8,
    /// Where the word sits in its half-period.
    pub format: Format,
    /// Bits per word, 1 to 32; `None` takes as many as the half-period has clocks, which
    /// right-justified words cannot do.
    pub bits: Option<u8>,
    /// The clock edge on which WS and SD are read.
    pub sample_edge: SampleEdge,
    /// Whether WS high means the left channel.
    pub ws_inverted: bool,
}

impl Default for I2sConfig {
    fn default() -> Self {
        Self::on(0, 1, 2)
    }
}

impl I2sConfig {
    /// A bus on the given channels, in the I²S format with the word width taken from WS.
    pub const fn on(sck: u8, ws: u8, sd: u8) -> Self {
        Self {
            sck,
            ws,
            sd,
            format: Format::I2s,
            bits: None,
            sample_edge: SampleEdge::Rising,
            ws_inverted: false,
        }
    }

    /// The channel WS stands for when it is high (`ws_high`) or low.
    pub const fn channel(&self, ws_high: bool) -> Channel {
        if ws_high != self.ws_inverted {
            Channel::Right
        } else {
            Channel::Left
        }
    }

    /// Whether WS is high for `channel`.
    pub const fn ws_high(&self, channel: Channel) -> bool {
        matches!(channel, Channel::Right) != self.ws_inverted
    }

    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> {
        [self.sck, self.ws, self.sd].into_iter()
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), I2sConfigError> {
        match self.bits {
            Some(bits) if !(1..=MAX_WORD_BITS).contains(&bits) => {
                return Err(I2sConfigError::Bits(bits));
            }
            None if self.format == Format::RightJustified => {
                return Err(I2sConfigError::WidthRequired);
            }
            _ => {}
        }
        let mut seen = ChannelMask::EMPTY;
        for channel in self.channels() {
            if channel >= MAX_CHANNELS {
                return Err(I2sConfigError::Channel(channel));
            }
            if seen.has(channel) {
                return Err(I2sConfigError::DuplicateChannel(channel));
            }
            seen = seen.with(channel);
        }
        Ok(())
    }

    /// What the decoder's output port declares.
    pub fn props(&self) -> Props {
        let props = Props::new().with("format", self.format.name());
        match self.bits {
            Some(bits) => props.with("bits", i64::from(bits)),
            None => props,
        }
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum I2sConfigError {
    /// The word width is not 1 to 32 bits.
    #[error("{0} bits per word; 1 to {MAX_WORD_BITS} are supported")]
    Bits(u8),
    /// Right-justified words need a fixed width.
    #[error("right-justified words need a fixed width: only it says where a word begins")]
    WidthRequired,
    /// A channel is used for two lines.
    #[error("channel {0} is used for two lines")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The I²S decoder node: logic samples in, `i2s.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct I2s {
    config: I2sConfig,
}

impl I2s {
    /// A decoder for `config`.
    pub fn new(config: I2sConfig) -> Result<Self, I2sConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &I2sConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<I2sEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for I2s {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "i2s".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<I2sEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

/// The ticks a word keeps of its latest bits: enough for the widest word.
const RING: usize = MAX_WORD_BITS as usize;

/// Where bit `n` of a half-period, counted from 0, keeps its tick.
fn ring(n: u64) -> usize {
    (n % RING as u64) as usize
}

/// The word of the half-period under way.
#[derive(Debug)]
struct Word {
    frame: u64,
    channel: Channel,
    /// Bits read: every bit of the half-period right-justified, the word's bits otherwise.
    count: u64,
    /// The bits read, the latest least significant; the last 32 stay.
    value: u32,
    /// The ticks of the last bits read, by [`ring`].
    ticks: [Tick; RING],
    /// Whether the word went out already: the rest of the half-period is padding.
    reported: bool,
}

impl Word {
    fn new(frame: u64, channel: Channel) -> Self {
        Self {
            frame,
            channel,
            count: 0,
            value: 0,
            ticks: [Tick::ZERO; RING],
            reported: false,
        }
    }

    fn push(&mut self, tick: Tick, bit: bool) {
        self.value = (self.value << 1) | u32::from(bit);
        self.ticks[ring(self.count)] = tick;
        self.count += 1;
    }

    /// Where the word of at most `width` bits will start: the first of the bits it would have
    /// if the half-period ended now, as later bits only move a right-justified word's start
    /// forward; `None` before its first bit and once it went out.
    fn start(&self, width: u8) -> Option<Tick> {
        if self.reported || self.count == 0 {
            return None;
        }
        let bits = self.count.min(u64::from(width));
        Some(self.ticks[ring(self.count - bits)])
    }

    /// The word made of the last `bits` bits read, 1 to `count` and at most 32, settled at
    /// `settled`.
    fn event(&self, bits: u8, complete: bool, settled: Tick) -> (Provenance, I2sEvent) {
        let first = self.ticks[ring(self.count - u64::from(bits))];
        let last = self.ticks[ring(self.count - 1)];
        let value = if bits >= MAX_WORD_BITS {
            self.value
        } else {
            self.value & ((1 << bits) - 1)
        };
        let sample = I2sEvent::Sample {
            frame: self.frame,
            channel: self.channel,
            value,
            bits,
            complete,
        };
        (
            Provenance::over(TickRange::new(first, last.next()), settled),
            sample,
        )
    }
}

struct Decoder {
    config: I2sConfig,
    /// The bits of a whole word.
    width: u8,
    /// The frame of the latest half-period.
    frame: u64,
    /// The half-period under way, unless it began before the samples did or before a gap.
    word: Option<Word>,
    /// Whether no half-period began since the samples began or resumed after a gap.
    fresh: bool,
}

impl Decoder {
    fn new(config: I2sConfig) -> Self {
        Self {
            width: config.bits.unwrap_or(MAX_WORD_BITS),
            config,
            frame: 0,
            word: None,
            fresh: true,
        }
    }

    /// Forgets the half-period under way: the samples begin, or resume after a gap.
    fn resync(&mut self) {
        self.word = None;
        self.fresh = true;
    }

    /// Progress to `tick`, held back to where the word under way will start, which promises
    /// that nothing starting earlier follows.
    fn hold(&self, tick: Tick) -> Tick {
        let start = self.word.as_ref().and_then(|word| word.start(self.width));
        start.map_or(tick, |start| start.min(tick))
    }

    /// Begins the half-period of `channel`.
    fn begin(&mut self, channel: Channel) {
        if channel == Channel::Left || self.fresh {
            self.frame += 1;
        }
        self.fresh = false;
        self.word = Some(Word::new(self.frame, channel));
    }

    /// Takes the bit read at `tick` into the word under way; the word, if the bit made it
    /// whole.
    fn take(&mut self, tick: Tick, bit: bool) -> Option<(Provenance, I2sEvent)> {
        let width = self.width;
        let justified_right = self.config.format == Format::RightJustified;
        let word = self.word.as_mut()?;
        if word.reported {
            return None;
        }
        word.push(tick, bit);
        if justified_right || word.count < u64::from(width) {
            return None;
        }
        word.reported = true;
        Some(word.event(width, true, tick))
    }

    /// Ends the half-period under way at `tick`, the sampling edge that found WS changed; its
    /// word, unless it went out already.
    fn close(&mut self, tick: Tick) -> Option<(Provenance, I2sEvent)> {
        let word = self.word.take()?;
        if word.reported || word.count == 0 {
            return None;
        }
        let bits = u8::try_from(word.count).map_or(self.width, |count| count.min(self.width));
        let complete = bits == self.width || self.config.bits.is_none();
        Some(word.event(bits, complete, tick))
    }
}

async fn run(mut signal: SignalCursor, mut out: Outbox<I2sEvent>, config: I2sConfig) {
    let watch = ChannelMask::one(config.sck);
    // The clock's level after a sampling edge.
    let sampled = config.sample_edge == SampleEdge::Rising;
    let mut decoder = Decoder::new(config.clone());
    // Each round of this loop starts where levels are unknown: at the first sample and after
    // every gap.
    'resync: loop {
        decoder.resync();
        let mut latched = loop {
            match signal.sample().await {
                Sample::At { levels, .. } => break levels.high(config.ws),
                Sample::Gap(range) => out.gap(range).await,
                Sample::Progress(tick) => out.progress(decoder.hold(tick)).await,
                Sample::End(tick) => {
                    out.end(tick).await;
                    return;
                }
            }
        };
        loop {
            match signal.next_change(watch).await {
                Change::At { tick, levels, .. } => {
                    if levels.high(config.sck) != sampled {
                        continue;
                    }
                    let (ws, bit) = (levels.high(config.ws), levels.high(config.sd));
                    if ws == latched {
                        if let Some((at, sample)) = decoder.take(tick, bit) {
                            out.emit(at, sample).await;
                        }
                        continue;
                    }
                    latched = ws;
                    // In the I²S format the edge that finds WS changed still carries the last
                    // bit of the word that ends.
                    if config.format == Format::I2s
                        && let Some((at, sample)) = decoder.take(tick, bit)
                    {
                        out.emit(at, sample).await;
                    }
                    if let Some((at, sample)) = decoder.close(tick) {
                        out.emit(at, sample).await;
                    }
                    decoder.begin(config.channel(ws));
                    if config.format != Format::I2s
                        && let Some((at, sample)) = decoder.take(tick, bit)
                    {
                        out.emit(at, sample).await;
                    }
                }
                Change::Gap(range) => {
                    out.gap(range).await;
                    continue 'resync;
                }
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => out.progress(decoder.hold(tick)).await,
                Change::End(tick) => {
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
    use crate::i2s::I2S_EVENTS;

    #[test]
    fn ws_levels_name_the_channels() {
        let plain = I2sConfig::default();
        assert_eq!(plain.channel(false), Channel::Left);
        assert_eq!(plain.channel(true), Channel::Right);
        assert!(!plain.ws_high(Channel::Left));
        assert!(plain.ws_high(Channel::Right));
        let inverted = I2sConfig {
            ws_inverted: true,
            ..I2sConfig::default()
        };
        assert_eq!(inverted.channel(true), Channel::Left);
        assert_eq!(inverted.channel(false), Channel::Right);
        assert!(inverted.ws_high(Channel::Left));
        assert!(!inverted.ws_high(Channel::Right));
    }

    #[test]
    fn configurations_are_checked() {
        let config = I2sConfig::default();
        assert_eq!(config, I2sConfig::on(0, 1, 2));
        assert_eq!(config.validate(), Ok(()));
        let bad = |config: I2sConfig| config.validate().expect_err("rejected");
        for bits in [0, 33, 255] {
            assert_eq!(
                bad(I2sConfig {
                    bits: Some(bits),
                    ..config.clone()
                }),
                I2sConfigError::Bits(bits)
            );
        }
        for bits in [1, 16, 24, 32] {
            let fixed = I2sConfig {
                bits: Some(bits),
                ..config.clone()
            };
            assert_eq!(fixed.validate(), Ok(()));
        }
        assert_eq!(
            bad(I2sConfig {
                format: Format::RightJustified,
                ..config.clone()
            }),
            I2sConfigError::WidthRequired
        );
        assert_eq!(
            I2sConfig {
                format: Format::RightJustified,
                bits: Some(24),
                ..config.clone()
            }
            .validate(),
            Ok(())
        );
        assert_eq!(
            bad(I2sConfig::on(0, 1, 0)),
            I2sConfigError::DuplicateChannel(0)
        );
        assert_eq!(bad(I2sConfig::on(0, 64, 2)), I2sConfigError::Channel(64));
        assert_eq!(
            I2s::new(I2sConfig::on(5, 5, 6)),
            Err(I2sConfigError::DuplicateChannel(5))
        );
    }

    #[test]
    fn props_and_requirements_describe_the_bus() {
        let props = I2sConfig::default().props();
        assert_eq!(props.get("format"), Some(&"i2s".into()));
        assert_eq!(props.get("bits"), None);
        let config = I2sConfig {
            format: Format::RightJustified,
            bits: Some(24),
            ..I2sConfig::on(7, 2, 4)
        };
        let props = config.props();
        assert_eq!(props.get("format"), Some(&"right-justified".into()));
        assert_eq!(props.get("bits"), Some(&24i64.into()));
        let spec = I2s::new(config).expect("valid").spec();
        let requires = &spec.inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 8)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 7)).is_some());
        assert_eq!(spec.outputs[0].schema, I2S_EVENTS);
    }

    #[test]
    fn words_keep_the_ticks_of_their_bits() {
        let mut word = Word::new(3, Channel::Right);
        for n in 0..40u64 {
            word.push(Tick(100 + 2 * n), n % 3 == 0);
        }
        // The last eight bits read: n = 32 … 39, of which 33, 36 and 39 are ones.
        let (at, sample) = word.event(8, true, Tick(500));
        assert_eq!(at.span, TickRange::new(Tick(164), Tick(179)));
        assert_eq!(at.settled, Tick(500));
        assert_eq!(
            sample,
            I2sEvent::Sample {
                frame: 3,
                channel: Channel::Right,
                value: 0b0100_1001,
                bits: 8,
                complete: true,
            }
        );
        let (at, sample) = word.event(32, false, Tick(501));
        assert_eq!(at.span, TickRange::new(Tick(116), Tick(179)));
        assert!(matches!(sample, I2sEvent::Sample { bits: 32, .. }));
    }

    #[test]
    fn a_word_under_way_starts_where_its_event_would() {
        let mut word = Word::new(1, Channel::Left);
        assert_eq!(word.start(8), None);
        for n in 0..5u64 {
            word.push(Tick(100 + 2 * n), true);
        }
        assert_eq!(word.start(8), Some(Tick(100)));
        for n in 5..40u64 {
            word.push(Tick(100 + 2 * n), true);
        }
        // The first of the last eight bits, and of the last 32, as the events above.
        assert_eq!(word.start(8), Some(Tick(164)));
        assert_eq!(word.start(32), Some(Tick(116)));
        word.reported = true;
        assert_eq!(word.start(8), None);
    }
}
