// SPDX-License-Identifier: Apache-2.0

//! The JTAG decoder: the TAP controller followed through its sixteen states, and the bits
//! shifted through its instruction and data registers, from logic samples.
//!
//! Written from IEEE Std 1149.1-2013, *Test Access Port and Boundary-Scan Architecture*: its
//! test access port signals and its TAP controller state diagram. The rules:
//!
//! - every line is read at the tick TCK rises: TMS and TDI are sampled on the rising edge, and
//!   TDO, which the device drives on the falling edge before it, is sampled on the same edge;
//! - each rising edge moves the controller as the standard's state diagram says; a `state`
//!   event marks every change of state, not the clocks that stay in one;
//! - the state is unknown until the controller is seen to reset — five rising edges in a row
//!   with TMS high, or TRST asserted — unless the configuration assumes a state to start
//!   from; while it is unknown, no state and no scan is reported;
//! - a scan is a passage through Shift-IR or Shift-DR: it begins on the clock that enters the
//!   shift state with no scan under way, takes one bit from TDI and TDO on every rising edge in
//!   the shift state — the one that leaves it for Exit1 with TMS high included, which shifts
//!   the last bit before the exit — and ends complete on the clock that enters Update-IR or
//!   Update-DR. Pause-IR and Pause-DR only halt shifting, as the standard has them do: a scan
//!   that pauses goes on when the controller returns to the shift state through Exit2. A path
//!   from Capture through Exit1 to Update that never shifts is no scan;
//! - bits are reported in `shift` events of at most [`SHIFT_BATCH_BITS`] bits, packed least
//!   significant bit first in the order they were shifted; a batch goes out when it is full
//!   and on the clock that leaves the shift state, settled at its last bit;
//! - TRST, when connected, is active low and asynchronous: asserting it resets the controller
//!   at once, aborting the scan under way, and holds it in Test-Logic-Reset, so TCK is ignored
//!   while it is low; a TCK edge at the very tick TRST changes does not clock the controller;
//!   TRST found low at the first sample, or at the first after a gap, is a reset there;
//! - a gap or the end of the samples aborts the scan under way (a batch it cut short goes out
//!   first, settled where it was cut); after a gap the state is unknown again — or the assumed
//!   starting state: the assumption holds wherever the samples begin, at the first sample and
//!   after every gap;
//! - events of one clock come in this order: the batch its bit completed, the end of the scan
//!   it completed, the reset, the new state, the scan it began.
//!
//! Limits: one TAP's view of the lines — a chain of devices is one long register here; the
//! decoder does not know instruction or register lengths and interprets no instruction.
//! TRST is taken as active low, as the standard defines it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uiharu_core::{
    Change, ChannelMask, InputRef, InputSpec, Io, Levels, LogicBlock, MAX_CHANNELS, Node,
    NodeFuture, NodeId, NodeSpec, Outbox, OutputRef, OutputSpec, Props, Provenance, Requires,
    Sample, SignalCursor, Tick, TickRange,
};

use super::events::{
    EndReason, JtagEvent, ResetBy, SHIFT_BATCH_BITS, SHIFT_BATCH_BYTES, ScanId, TapState,
};

/// Rising edges with TMS high that reset the controller from any state.
pub const RESET_CLOCKS: u8 = 5;

/// The state the decoder assumes where the samples begin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InitialState {
    /// Nothing: wait for a reset.
    #[default]
    Unknown,
    /// Test-Logic-Reset, where the controller is after power-up.
    TestLogicReset,
    /// Run-Test/Idle, where a test host parks the controller between operations.
    RunTestIdle,
}

impl InitialState {
    /// The state assumed, if any.
    pub const fn state(self) -> Option<TapState> {
        match self {
            InitialState::Unknown => None,
            InitialState::TestLogicReset => Some(TapState::TestLogicReset),
            InitialState::RunTestIdle => Some(TapState::RunTestIdle),
        }
    }
}

/// How the test access port is wired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JtagConfig {
    /// The test clock channel.
    pub tck: u8,
    /// The test mode select channel.
    pub tms: u8,
    /// The test data input channel, if connected.
    pub tdi: Option<u8>,
    /// The test data output channel, if connected.
    pub tdo: Option<u8>,
    /// The test reset channel (active low), if connected.
    pub trst: Option<u8>,
    /// The state assumed where the samples begin.
    pub initial_state: InitialState,
}

impl Default for JtagConfig {
    fn default() -> Self {
        Self {
            tck: 0,
            tms: 1,
            tdi: Some(2),
            tdo: Some(3),
            trst: None,
            initial_state: InitialState::Unknown,
        }
    }
}

impl JtagConfig {
    /// The channels the decoder reads.
    pub fn channels(&self) -> impl Iterator<Item = u8> {
        [
            Some(self.tck),
            Some(self.tms),
            self.tdi,
            self.tdo,
            self.trst,
        ]
        .into_iter()
        .flatten()
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), JtagConfigError> {
        if self.tdi.is_none() && self.tdo.is_none() {
            return Err(JtagConfigError::NoDataLine);
        }
        let mut seen = ChannelMask::EMPTY;
        for channel in self.channels() {
            if channel >= MAX_CHANNELS {
                return Err(JtagConfigError::Channel(channel));
            }
            if seen.has(channel) {
                return Err(JtagConfigError::DuplicateChannel(channel));
            }
            seen = seen.with(channel);
        }
        Ok(())
    }

    /// What the decoder's output port declares: which lines are connected.
    pub fn props(&self) -> Props {
        Props::new()
            .with("tdi", self.tdi.is_some())
            .with("tdo", self.tdo.is_some())
            .with("trst", self.trst.is_some())
    }
}

/// Why a configuration is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum JtagConfigError {
    /// Neither data line is connected.
    #[error("neither TDI nor TDO is connected")]
    NoDataLine,
    /// A channel is used for two lines.
    #[error("channel {0} is used for two lines")]
    DuplicateChannel(u8),
    /// A channel number is out of range.
    #[error("channel {0} is out of range; channels are 0 to {last}", last = MAX_CHANNELS - 1)]
    Channel(u8),
}

/// The JTAG decoder node: logic samples in, `jtag.events` out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jtag {
    config: JtagConfig,
}

impl Jtag {
    /// A decoder for `config`.
    pub fn new(config: JtagConfig) -> Result<Self, JtagConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// The configuration.
    pub fn config(&self) -> &JtagConfig {
        &self.config
    }

    /// The logic input of the decoder added as `node`.
    pub fn input(node: NodeId) -> InputRef<Arc<LogicBlock>> {
        InputRef::of_node(node, 0)
    }

    /// The event output of the decoder added as `node`.
    pub fn events(node: NodeId) -> OutputRef<JtagEvent> {
        OutputRef::of_node(node, 0)
    }
}

impl Node for Jtag {
    fn spec(&self) -> NodeSpec {
        let channels = self
            .config
            .channels()
            .max()
            .map_or(0, |max| i64::from(max) + 1);
        NodeSpec {
            name: "jtag".into(),
            inputs: vec![InputSpec::new::<Arc<LogicBlock>>(
                "logic",
                Requires::new().at_least("channels", channels),
            )],
            outputs: vec![OutputSpec::new::<JtagEvent>("events", self.config.props())],
        }
    }

    fn start(self: Box<Self>, mut io: Io) -> NodeFuture {
        Box::pin(run(io.signal(0), io.output(0), self.config))
    }
}

/// The bits shifted since the last batch went out: at most [`SHIFT_BATCH_BITS`].
#[derive(Debug)]
struct Batch {
    /// The position of the first bit in the scan.
    offset: u32,
    bits: u16,
    first: Tick,
    last: Tick,
    tdi: [u8; SHIFT_BATCH_BYTES],
    tdo: [u8; SHIFT_BATCH_BYTES],
}

impl Batch {
    fn new() -> Self {
        Self {
            offset: 0,
            bits: 0,
            first: Tick::ZERO,
            last: Tick::ZERO,
            tdi: [0; SHIFT_BATCH_BYTES],
            tdo: [0; SHIFT_BATCH_BYTES],
        }
    }
}

/// The scan under way.
#[derive(Debug)]
struct Scan {
    id: ScanId,
    /// Bits shifted so far.
    bits: u32,
    batch: Batch,
}

struct Decoder {
    config: JtagConfig,
    watch: ChannelMask,
    /// The controller's state; `None` while unknown.
    state: Option<TapState>,
    /// Rising edges in a row with TMS high, counted while the state is unknown.
    tms_high: u8,
    /// Whether TRST holds the controller in Test-Logic-Reset.
    held: bool,
    scan: Option<Scan>,
    next_scan: u64,
}

impl Decoder {
    fn new(config: JtagConfig) -> Self {
        let mut watch = ChannelMask::one(config.tck);
        if let Some(trst) = config.trst {
            watch = watch.with(trst);
        }
        Self {
            config,
            watch,
            state: None,
            tms_high: 0,
            held: false,
            scan: None,
            next_scan: 1,
        }
    }

    fn trst_asserted(&self, levels: Levels) -> bool {
        self.config.trst.is_some_and(|trst| !levels.high(trst))
    }

    /// Takes the bit of a clock in a shift state into the batch; the batch, once full.
    fn shift(&mut self, tick: Tick, levels: Levels) -> Option<(Provenance, JtagEvent)> {
        let config = &self.config;
        let scan = self.scan.as_mut()?;
        let batch = &mut scan.batch;
        if batch.bits == 0 {
            batch.offset = scan.bits;
            batch.first = tick;
        }
        let (byte, bit) = (usize::from(batch.bits / 8), batch.bits % 8);
        let level = |line: Option<u8>| line.is_some_and(|channel| levels.high(channel));
        batch.tdi[byte] |= u8::from(level(config.tdi)) << bit;
        batch.tdo[byte] |= u8::from(level(config.tdo)) << bit;
        batch.bits += 1;
        batch.last = tick;
        scan.bits = scan.bits.saturating_add(1);
        if batch.bits < SHIFT_BATCH_BITS {
            return None;
        }
        self.take_batch(tick)
    }

    /// The batch as an event, if it holds any bit, settled at `decided`: its last bit, or the
    /// tick that cut it short.
    fn take_batch(&mut self, decided: Tick) -> Option<(Provenance, JtagEvent)> {
        let scan = self.scan.as_mut()?;
        let batch = std::mem::replace(&mut scan.batch, Batch::new());
        if batch.bits == 0 {
            return None;
        }
        let bytes = usize::from(batch.bits).div_ceil(8);
        let event = JtagEvent::Shift {
            scan: scan.id,
            offset: batch.offset,
            tdi: self.config.tdi.map(|_| batch.tdi[..bytes].to_vec()),
            tdo: self.config.tdo.map(|_| batch.tdo[..bytes].to_vec()),
            bits: batch.bits,
        };
        let span = TickRange::new(batch.first, batch.last.next());
        Some((Provenance::over(span, decided.max(batch.last)), event))
    }

    fn begin_scan(&mut self, state: TapState) -> Option<JtagEvent> {
        let register = state.register()?;
        let id = ScanId(self.next_scan);
        self.next_scan += 1;
        self.scan = Some(Scan {
            id,
            bits: 0,
            batch: Batch::new(),
        });
        Some(JtagEvent::ScanBegin { scan: id, register })
    }

    /// Ends the scan under way, if any, with `reason`.
    fn end_scan(&mut self, reason: EndReason) -> Option<JtagEvent> {
        let scan = self.scan.take()?;
        Some(JtagEvent::ScanEnd {
            scan: scan.id,
            bits: scan.bits,
            reason,
        })
    }
}

/// Where the samples begin, at the first sample or after a gap: the assumed state, or the
/// reset TRST holds the controller in.
async fn start(decoder: &mut Decoder, out: &mut Outbox<JtagEvent>, tick: Tick, levels: Levels) {
    decoder.state = decoder.config.initial_state.state();
    decoder.tms_high = 0;
    decoder.held = false;
    if decoder.trst_asserted(levels) {
        trst_reset(decoder, out, tick).await;
    } else if let Some(state) = decoder.state {
        out.emit(Provenance::one(tick), JtagEvent::State { state })
            .await;
    }
}

/// TRST was asserted at `tick`: the scan under way is cut off and the controller reset.
async fn trst_reset(decoder: &mut Decoder, out: &mut Outbox<JtagEvent>, tick: Tick) {
    decoder.held = true;
    decoder.tms_high = 0;
    if let Some((at, batch)) = decoder.take_batch(tick) {
        out.emit(at, batch).await;
    }
    if let Some(end) = decoder.end_scan(EndReason::Aborted) {
        out.emit(Provenance::one(tick), end).await;
    }
    out.emit(
        Provenance::one(tick),
        JtagEvent::Reset { by: ResetBy::Trst },
    )
    .await;
    let was = decoder.state.replace(TapState::TestLogicReset);
    if was != Some(TapState::TestLogicReset) {
        let state = TapState::TestLogicReset;
        out.emit(Provenance::one(tick), JtagEvent::State { state })
            .await;
    }
}

/// A rising edge of TCK at `tick`, with the lines at `levels`.
async fn clock(decoder: &mut Decoder, out: &mut Outbox<JtagEvent>, tick: Tick, levels: Levels) {
    let tms = levels.high(decoder.config.tms);
    let Some(state) = decoder.state else {
        decoder.tms_high = if tms {
            decoder.tms_high.saturating_add(1)
        } else {
            0
        };
        if decoder.tms_high >= RESET_CLOCKS {
            decoder.state = Some(TapState::TestLogicReset);
            out.emit(Provenance::one(tick), JtagEvent::Reset { by: ResetBy::Tms })
                .await;
            let state = TapState::TestLogicReset;
            out.emit(Provenance::one(tick), JtagEvent::State { state })
                .await;
        }
        return;
    };
    if state.is_shift()
        && let Some((at, batch)) = decoder.shift(tick, levels)
    {
        out.emit(at, batch).await;
    }
    let next = state.next(tms);
    if next == state {
        return;
    }
    decoder.state = Some(next);
    if state.is_shift()
        && let Some((at, batch)) = decoder.take_batch(tick)
    {
        out.emit(at, batch).await;
    }
    if next.is_update()
        && let Some(end) = decoder.end_scan(EndReason::Complete)
    {
        out.emit(Provenance::one(tick), end).await;
    }
    if next == TapState::TestLogicReset {
        out.emit(Provenance::one(tick), JtagEvent::Reset { by: ResetBy::Tms })
            .await;
    }
    out.emit(Provenance::one(tick), JtagEvent::State { state: next })
        .await;
    if next.is_shift()
        && decoder.scan.is_none()
        && let Some(begin) = decoder.begin_scan(next)
    {
        out.emit(Provenance::one(tick), begin).await;
    }
}

/// The samples stop at `tick`, for a gap or for good: the scan under way is aborted there and
/// the state is no longer known.
async fn cut(decoder: &mut Decoder, out: &mut Outbox<JtagEvent>, tick: Tick) {
    if let Some((at, batch)) = decoder.take_batch(tick) {
        out.emit(at, batch).await;
    }
    if let Some(end) = decoder.end_scan(EndReason::Aborted) {
        out.emit(Provenance::at(tick), end).await;
    }
    decoder.state = None;
}

async fn run(mut signal: SignalCursor, mut out: Outbox<JtagEvent>, config: JtagConfig) {
    let mut decoder = Decoder::new(config);
    let (tck, trst) = (decoder.config.tck, decoder.config.trst);
    // Each round of this loop starts where levels are unknown: at the first sample and after
    // every gap.
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
        start(&mut decoder, &mut out, tick, levels).await;
        loop {
            match signal.next_change(decoder.watch).await {
                Change::At {
                    tick,
                    changed,
                    levels,
                } => {
                    if let Some(trst) = trst
                        && changed.has(trst)
                    {
                        if decoder.trst_asserted(levels) {
                            trst_reset(&mut decoder, &mut out, tick).await;
                        } else {
                            decoder.held = false;
                        }
                        // A TCK edge at this very tick does not clock the controller.
                        continue;
                    }
                    if !decoder.held && changed.has(tck) && levels.high(tck) {
                        clock(&mut decoder, &mut out, tick, levels).await;
                    }
                }
                Change::Gap(range) => {
                    cut(&mut decoder, &mut out, range.start).await;
                    out.gap(range).await;
                    continue 'resync;
                }
                // Without a deadline the wait is never quiet.
                Change::Quiet(_) => {}
                Change::Progress(tick) => out.progress(tick).await,
                Change::End(tick) => {
                    cut(&mut decoder, &mut out, tick).await;
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
        assert_eq!(JtagConfig::default().validate(), Ok(()));
        let bad = |config: JtagConfig| config.validate().expect_err("rejected");
        assert_eq!(
            bad(JtagConfig {
                tdi: None,
                tdo: None,
                ..JtagConfig::default()
            }),
            JtagConfigError::NoDataLine
        );
        assert_eq!(
            bad(JtagConfig {
                trst: Some(1),
                ..JtagConfig::default()
            }),
            JtagConfigError::DuplicateChannel(1)
        );
        assert_eq!(
            bad(JtagConfig {
                tdo: Some(64),
                ..JtagConfig::default()
            }),
            JtagConfigError::Channel(64)
        );
        assert!(
            JtagConfig {
                tdi: None,
                ..JtagConfig::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn props_say_which_lines_are_connected() {
        let props = JtagConfig {
            tdo: None,
            trst: Some(7),
            ..JtagConfig::default()
        }
        .props();
        assert_eq!(props.get("tdi"), Some(&true.into()));
        assert_eq!(props.get("tdo"), Some(&false.into()));
        assert_eq!(props.get("trst"), Some(&true.into()));
        let jtag = Jtag::new(JtagConfig {
            trst: Some(7),
            ..JtagConfig::default()
        })
        .expect("valid");
        let requires = &jtag.spec().inputs[0].requires;
        assert!(requires.unmet(&Props::new().with("channels", 8)).is_none());
        assert!(requires.unmet(&Props::new().with("channels", 7)).is_some());
    }

    #[test]
    fn initial_states_name_a_state() {
        assert_eq!(InitialState::Unknown.state(), None);
        assert_eq!(
            InitialState::RunTestIdle.state(),
            Some(TapState::RunTestIdle)
        );
        assert_eq!(
            serde_json::to_string(&InitialState::TestLogicReset).expect("serializes"),
            r#""test-logic-reset""#
        );
    }
}
