//! The capture configuration the FPGA receives before each acquisition: `DSL_setting` and, on
//! 32-channel models, `DSL_setting_ext32` (DESIGN.md §4.7).
//!
//! Ported from `dsl_fpga_arm()` in `libsigrok4DSL/hardware/DSL/dsl.c` and the trigger encoders in
//! `libsigrok4DSL/trigger.c`. `tests/setting.rs` compares the bytes with ones DSView itself
//! produced (`tools/golden`); keep every computation, including its integer and floating point
//! quirks, as DSView has it.

use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::{AdvancedTrigger, TriggerPattern};

use crate::profiles::{ChannelMode, Profile, caps};

/// Size of `DSL_setting` in bytes.
pub const SETTING_LEN: usize = 372;
/// Size of `DSL_setting_ext32` in bytes.
pub const SETTING_EXT32_LEN: usize = 204;

/// Trigger stages of the FPGA.
pub const TRIGGER_STAGES: usize = 16;
/// Channels a trigger stage covers.
pub const TRIGGER_CHANNELS: usize = 32;
/// The stage DSView keeps the simple (per-channel) trigger in, past the 16 hardware stages.
const SIMPLE_STAGE: usize = TRIGGER_STAGES;
/// The stage that holds the data pattern of a serial trigger.
const SERIAL_DATA_STAGE: usize = 3;

/// Samples the hardware handles at once.
pub const ATOMIC_SAMPLES: u64 = 64;
/// Sample counts are rounded to this alignment plus one (`SAMPLES_ALIGN`).
pub const SAMPLES_ALIGN: u64 = 1023;
/// Highest trigger position in buffer mode, in percent of the channel depth.
const MAX_TRIGGER_PERCENT: u64 = 90;
/// Highest trigger position in stream mode, in percent of the channel depth.
const MAX_STREAM_TRIGGER_PERCENT: u64 = 10;

mod mode_bit {
    pub const TRIG_EN: u16 = 1 << 0;
    pub const CLK_TYPE: u16 = 1 << 1;
    pub const CLK_EDGE: u16 = 1 << 2;
    pub const RLE: u16 = 1 << 3;
    pub const HALF: u16 = 1 << 5;
    pub const QUARTER: u16 = 1 << 6;
    pub const LOOP: u16 = 1 << 7;
    pub const FILTER: u16 = 1 << 8;
    pub const INSTANT: u16 = 1 << 9;
    pub const SLOW_ACQ: u16 = 1 << 10;
    pub const SERIAL_TRIGGER: u16 = 1 << 11;
    pub const STREAM: u16 = 1 << 12;
}

/// How the trigger stages are used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TriggerMode {
    /// One condition per channel, all of which must hold at once.
    #[default]
    Simple,
    /// Up to 16 stages of pattern conditions.
    Advanced,
    /// Start, stop, clock and data stages of a serial pattern.
    Serial,
}

/// One trigger stage: two condition patterns (`X 0 1 R F C` per channel) combined by `logic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerStage {
    pub pattern0: [u8; TRIGGER_CHANNELS],
    pub pattern1: [u8; TRIGGER_CHANNELS],
    /// Bit 0: both patterns must hold (and) rather than either (or); bit 1: contiguous.
    pub logic: u8,
    pub invert0: bool,
    pub invert1: bool,
    pub count: u32,
}

impl Default for TriggerStage {
    /// As `ds_trigger_init()` leaves a stage.
    fn default() -> Self {
        Self {
            pattern0: [b'X'; TRIGGER_CHANNELS],
            pattern1: [b'X'; TRIGGER_CHANNELS],
            logic: 1,
            invert0: false,
            invert1: false,
            count: 0,
        }
    }
}

/// The complete trigger (`struct ds_trigger`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerSetup {
    pub enabled: bool,
    pub mode: TriggerMode,
    /// Share of the capture before the trigger, in percent.
    pub position: u8,
    /// Last stage in use (advanced and serial triggers).
    pub last_stage: u8,
    /// Stages `0..16`, then the simple trigger.
    pub stages: [TriggerStage; TRIGGER_STAGES + 1],
}

impl Default for TriggerSetup {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: TriggerMode::Simple,
            position: 0,
            last_stage: 0,
            stages: [TriggerStage::default(); TRIGGER_STAGES + 1],
        }
    }
}

impl TriggerSetup {
    /// A simple trigger, as DSView's trigger dock and per-channel trigger buttons set it up.
    /// An instant capture ignores conditions and position.
    pub fn simple(
        conditions: &[(ChannelIndex, TriggerCondition)],
        position: u8,
        instant: bool,
    ) -> Self {
        let mut setup = Self::default();
        if instant {
            return setup;
        }
        setup.position = position.min(100);
        for &(channel, condition) in conditions {
            if let Some(slot) = setup.stages[SIMPLE_STAGE]
                .pattern0
                .get_mut(usize::from(channel))
            {
                *slot = condition.to_dsl_char();
                setup.enabled |= condition != TriggerCondition::None;
            }
        }
        setup
    }
}

/// A pattern of trigger characters, one per channel, `X` for channels left out.
fn pattern_chars(pattern: &TriggerPattern) -> [u8; TRIGGER_CHANNELS] {
    let mut chars = [b'X'; TRIGGER_CHANNELS];
    for &(channel, condition) in pattern {
        if let Some(slot) = chars.get_mut(usize::from(channel)) {
            *slot = condition.to_dsl_char();
        }
    }
    chars
}

impl TriggerSetup {
    /// A multi-stage or serial trigger as DSView's trigger dock commits it
    /// (`TriggerDock::commit_trigger`). Its serial tab borrows a few settings from the
    /// multi-stage tab; they take that tab's defaults here: one stage, patterns combined with
    /// "and", nothing inverted, and don't-care second patterns of stages 1 and 3.
    pub fn advanced(trigger: &AdvancedTrigger, position: u8) -> Self {
        let mut setup = Self {
            enabled: true,
            position: position.min(100),
            ..Self::default()
        };
        match trigger {
            AdvancedTrigger::Stages { stages } => {
                setup.mode = TriggerMode::Advanced;
                setup.last_stage = stages.len().clamp(1, TRIGGER_STAGES) as u8 - 1;
                for (slot, stage) in setup.stages.iter_mut().zip(stages) {
                    *slot = TriggerStage {
                        pattern0: pattern_chars(&stage.pattern0),
                        pattern1: pattern_chars(&stage.pattern1),
                        logic: (u8::from(stage.contiguous) << 1) | u8::from(stage.both),
                        invert0: stage.invert0,
                        invert1: stage.invert1,
                        count: stage.count,
                    };
                }
            }
            AdvancedTrigger::Serial(serial) => {
                setup.mode = TriggerMode::Serial;
                let [start, clock, data_channel, data, ..] = &mut setup.stages;
                start.pattern0 = pattern_chars(&serial.start);
                start.pattern1 = pattern_chars(&serial.stop);
                clock.pattern0 = pattern_chars(&serial.clock);
                clock.count = 1;
                // DSView writes 16 channels of stage 2 at a time, the upper 16 into the second
                // pattern.
                let channel = usize::from(serial.data_channel);
                if channel < 16 {
                    data_channel.pattern0[channel] = b'0';
                } else if channel < TRIGGER_CHANNELS {
                    data_channel.pattern1[channel - 16] = b'0';
                }
                for (bit, slot) in data.pattern0.iter_mut().take(16).enumerate() {
                    *slot = match (serial.mask >> bit & 1, serial.value >> bit & 1) {
                        (0, _) => b'X',
                        (_, 0) => b'0',
                        _ => b'1',
                    };
                }
                data.count = u32::from(serial.bits.clamp(1, 16) - 1);
            }
        }
        setup
    }
}

/// Everything `dsl_fpga_arm()` reads to configure a logic capture.
#[derive(Debug, Clone, Copy)]
pub struct ArmConfig<'a> {
    pub profile: &'static Profile,
    pub mode: &'static ChannelMode,
    pub samplerate: u64,
    pub limit_samples: u64,
    /// Bit `n`: channel `n` is captured.
    pub enabled: u32,
    /// One-sample-clock glitch filter.
    pub filter: bool,
    pub external_clock: bool,
    pub clock_falling_edge: bool,
    pub rle: bool,
    pub instant: bool,
    /// Stream without end, keeping the latest samples.
    pub looping: bool,
    pub trigger: &'a TriggerSetup,
}

impl ArmConfig<'_> {
    /// Channels captured, at least one (`dsl_en_ch_num`).
    pub fn enabled_count(&self) -> u32 {
        (self.enabled & channel_mask(self.mode.channels))
            .count_ones()
            .max(1)
    }

    /// Samples the capture really takes: the limit rounded up to 1024.
    pub fn actual_samples(&self) -> u64 {
        actual_samples(self.limit_samples)
    }

    /// Bytes of sample data the capture produces (`actual_bytes`).
    pub fn actual_bytes(&self) -> u64 {
        self.actual_samples() / ATOMIC_SAMPLES * u64::from(self.enabled_count()) * 8
    }

    /// Bytes per millisecond at the current rate (`to_bytes_per_ms`).
    pub fn bytes_per_ms(&self) -> u32 {
        bytes_per_ms(self.samplerate, self.enabled_count())
    }

    /// Samples each channel can keep in the hardware buffer (`dsl_channel_depth`).
    pub fn channel_depth(&self) -> u64 {
        channel_depth(self.profile, self.enabled_count())
    }

    fn mode_bits(&self) -> u16 {
        let mut mode = 0;
        let mut set = |bit: u16, on: bool| {
            if on {
                mode |= bit;
            }
        };
        set(mode_bit::TRIG_EN, self.trigger.enabled);
        set(mode_bit::CLK_TYPE, self.external_clock);
        set(mode_bit::CLK_EDGE, self.clock_falling_edge);
        set(mode_bit::RLE, self.rle);
        set(
            mode_bit::HALF,
            self.samplerate == self.profile.half_samplerate,
        );
        set(
            mode_bit::QUARTER,
            self.samplerate == self.profile.quarter_samplerate,
        );
        set(mode_bit::LOOP, self.looping);
        set(mode_bit::FILTER, self.filter);
        set(mode_bit::INSTANT, self.instant);
        set(mode_bit::SLOW_ACQ, self.bytes_per_ms() < 1024);
        set(
            mode_bit::SERIAL_TRIGGER,
            self.trigger.mode == TriggerMode::Serial,
        );
        set(mode_bit::STREAM, self.mode.stream);
        mode
    }
}

/// `limit` rounded up to a multiple of 1024.
pub fn actual_samples(limit: u64) -> u64 {
    (limit + SAMPLES_ALIGN) & !SAMPLES_ALIGN
}

/// `to_bytes_per_ms()` for logic captures.
pub fn bytes_per_ms(samplerate: u64, enabled: u32) -> u32 {
    (samplerate as f64 / 1000.0 * f64::from(enabled) / 8.0).ceil() as u32
}

/// `dsl_channel_depth()`.
pub fn channel_depth(profile: &Profile, enabled: u32) -> u64 {
    (profile.hw_depth / u64::from(enabled.max(1))) & !SAMPLES_ALIGN
}

fn channel_mask(channels: u16) -> u32 {
    if channels >= 32 {
        u32::MAX
    } else {
        (1 << channels) - 1
    }
}

/// How the trigger patterns of one stage are folded for half and quarter sample rates, where
/// the FPGA sees two or four samples of fewer channels at once.
#[derive(Debug, Clone, Copy)]
struct Fold {
    quarter: bool,
    half: bool,
}

impl Fold {
    fn for_stage(config: &ArmConfig<'_>, mode: u16, stage: usize) -> Self {
        if mode & mode_bit::SERIAL_TRIGGER != 0 && stage == SERIAL_DATA_STAGE {
            return Self {
                quarter: false,
                half: false,
            };
        }
        let adf4360 = config.profile.has(caps::ADF4360);
        let half_rate = mode & mode_bit::HALF != 0;
        let quarter_rate = mode & mode_bit::QUARTER != 0;
        Self {
            quarter: !adf4360 && quarter_rate,
            half: (!adf4360 && half_rate) || (adf4360 && quarter_rate),
        }
    }

    fn apply(self, bits: u16) -> u16 {
        const QUARTER_MASK: u16 = 0xffff >> 12;
        const HALF_MASK: u16 = 0xffff >> 8;
        if self.quarter {
            let low = bits & QUARTER_MASK;
            (low << 12) | (low << 8) | (low << 4) | low
        } else if self.half {
            let low = bits & HALF_MASK;
            (low << 8) | low
        } else {
            bits
        }
    }
}

/// `ds_trigger_get_*()`: one bit per channel `low..low + 16`, set where `test` holds.
fn pattern_bits(
    pattern: &[u8; TRIGGER_CHANNELS],
    low: usize,
    fold: Fold,
    test: fn(u8) -> bool,
) -> u16 {
    let bits = (low..low + 16).rev().fold(0u16, |bits, channel| {
        (bits << 1) | u16::from(test(pattern[channel]))
    });
    fold.apply(bits)
}

fn is_mask(c: u8) -> bool {
    c == b'X' || c == b'C'
}

fn is_value(c: u8) -> bool {
    c == b'1' || c == b'R'
}

fn is_edge(c: u8) -> bool {
    c == b'R' || c == b'F' || c == b'C'
}

/// The six pattern words of one stage for channels `low..low + 16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StageWords {
    mask0: u16,
    mask1: u16,
    value0: u16,
    value1: u16,
    edge0: u16,
    edge1: u16,
}

impl StageWords {
    /// What the unused stages of a simple trigger hold.
    const UNUSED: Self = Self {
        mask0: 0xffff,
        mask1: 0xffff,
        value0: 0,
        value1: 0,
        edge0: 0,
        edge1: 0,
    };

    fn new(stage: &TriggerStage, low: usize, fold: Fold) -> Self {
        Self {
            mask0: pattern_bits(&stage.pattern0, low, fold, is_mask),
            mask1: pattern_bits(&stage.pattern1, low, fold, is_mask),
            value0: pattern_bits(&stage.pattern0, low, fold, is_value),
            value1: pattern_bits(&stage.pattern1, low, fold, is_value),
            edge0: pattern_bits(&stage.pattern0, low, fold, is_edge),
            edge1: pattern_bits(&stage.pattern1, low, fold, is_edge),
        }
    }
}

/// The trigger stages as the FPGA receives them.
struct EncodedTrigger {
    low: [StageWords; TRIGGER_STAGES],
    high: [StageWords; TRIGGER_STAGES],
    logic0: [u16; TRIGGER_STAGES],
    logic1: [u16; TRIGGER_STAGES],
    count: [u32; TRIGGER_STAGES],
}

fn logic_word(stage: &TriggerStage, invert: bool) -> u16 {
    (u16::from(stage.logic) << 1) + u16::from(invert)
}

fn encode_trigger(config: &ArmConfig<'_>, mode: u16) -> EncodedTrigger {
    let trigger = config.trigger;
    let mut encoded = EncodedTrigger {
        low: [StageWords::UNUSED; TRIGGER_STAGES],
        high: [StageWords::UNUSED; TRIGGER_STAGES],
        logic0: [2; TRIGGER_STAGES],
        logic1: [2; TRIGGER_STAGES],
        count: [0; TRIGGER_STAGES],
    };
    if trigger.mode == TriggerMode::Simple {
        let stage = &trigger.stages[SIMPLE_STAGE];
        let fold = Fold::for_stage(config, mode, 0);
        encoded.low[0] = StageWords::new(stage, 0, fold);
        encoded.high[0] = StageWords::new(stage, 16, fold);
        encoded.logic0[0] = logic_word(stage, stage.invert0);
        encoded.logic1[0] = logic_word(stage, stage.invert1);
        encoded.count[0] = stage.count;
    } else {
        for (index, stage) in trigger.stages[..TRIGGER_STAGES].iter().enumerate() {
            let fold = Fold::for_stage(config, mode, index);
            encoded.low[index] = StageWords::new(stage, 0, fold);
            encoded.high[index] = StageWords::new(stage, 16, fold);
            encoded.logic0[index] = logic_word(stage, stage.invert0);
            encoded.logic1[index] = logic_word(stage, stage.invert1);
            encoded.count[index] = stage.count;
        }
    }
    encoded
}

struct Writer<'a> {
    bytes: &'a mut [u8],
    at: usize,
}

impl Writer<'_> {
    fn u16(&mut self, value: u16) {
        self.bytes[self.at..self.at + 2].copy_from_slice(&value.to_le_bytes());
        self.at += 2;
    }

    fn u32(&mut self, value: u32) {
        self.bytes[self.at..self.at + 4].copy_from_slice(&value.to_le_bytes());
        self.at += 4;
    }

    fn words(&mut self, stages: &[StageWords; TRIGGER_STAGES], field: fn(&StageWords) -> u16) {
        for stage in stages {
            self.u16(field(stage));
        }
    }

    fn stage_words(&mut self, stages: &[StageWords; TRIGGER_STAGES]) {
        self.words(stages, |s| s.mask0);
        self.words(stages, |s| s.mask1);
        self.words(stages, |s| s.value0);
        self.words(stages, |s| s.value1);
        self.words(stages, |s| s.edge0);
        self.words(stages, |s| s.edge1);
    }
}

const SYNC: u32 = 0xf5a5_f5a5;
const END_SYNC: u32 = 0xfa5a_fa5a;

/// The sample rate divider: `(div_l, div_h)`.
fn divider(config: &ArmConfig<'_>) -> (u16, u16) {
    let mode = config.mode;
    let pre_div = mode.pre_div;
    let mut t = (mode.hw_max_samplerate as f64 / config.samplerate as f64).ceil() as u32;
    let low = if t >= pre_div {
        pre_div - 1
    } else {
        t.wrapping_sub(1)
    };
    let mut div_h = (low << 8) as u16;
    t = (f64::from(t) / f64::from(pre_div)).ceil() as u32;
    let div_l = (t & 0xffff) as u16;
    div_h = div_h.wrapping_add((t >> 16) as u16);
    (div_l, div_h)
}

/// Where the trigger sits in the capture, in samples.
fn trigger_position(config: &ArmConfig<'_>) -> u32 {
    let requested =
        (f64::from(config.trigger.position) / 100.0 * config.limit_samples as f64) as u32;
    let position = u64::from(requested.max(ATOMIC_SAMPLES as u32));
    let percent = if config.mode.stream {
        MAX_STREAM_TRIGGER_PERCENT
    } else {
        MAX_TRIGGER_PERCENT
    };
    position.min(config.channel_depth() * percent / 100) as u32
}

/// `DSL_setting` for a capture.
pub fn setting(config: &ArmConfig<'_>) -> [u8; SETTING_LEN] {
    let mut bytes = [0u8; SETTING_LEN];
    let mode = config.mode_bits();
    let encoded = encode_trigger(config, mode);
    let enabled = config.enabled & channel_mask(config.mode.channels);
    let actual = config.actual_samples();
    let count = actual >> 4;
    let (div_l, div_h) = divider(config);
    let position = trigger_position(config);
    let channels = config.enabled_count();

    let mut w = Writer {
        bytes: &mut bytes,
        at: 0,
    };
    w.u32(SYNC);
    w.u16(0x0001);
    w.u16(mode);
    w.u16(0x0102);
    w.u16(div_l);
    w.u16(div_h);
    w.u16(0x0302);
    w.u16((count & 0xffff) as u16);
    w.u16((count >> 16) as u16);
    w.u16(0x0502);
    w.u16((position & 0xffc0) as u16);
    w.u16((position >> 16) as u16);
    w.u16(0x0701);
    w.u16((((channels & 0x1f) << 8) + u32::from(config.trigger.last_stage)) as u16);
    w.u16(0x0802);
    w.u16((actual & 0xffff) as u16);
    w.u16((actual >> 16) as u16);
    w.u16(0x0a02);
    w.u16((enabled & 0xffff) as u16);
    w.u16((enabled >> 16) as u16);
    w.u16(0x0c01);
    w.u16(0);
    w.u16(0x40a0);
    w.stage_words(&encoded.low);
    for logic in encoded.logic0 {
        w.u16(logic);
    }
    for logic in encoded.logic1 {
        w.u16(logic);
    }
    for count in encoded.count {
        w.u32(count);
    }
    w.u32(END_SYNC);
    debug_assert_eq!(w.at, SETTING_LEN);
    bytes
}

/// `DSL_setting_ext32` for a capture on a 32-channel model.
pub fn setting_ext32(config: &ArmConfig<'_>) -> [u8; SETTING_EXT32_LEN] {
    let mut bytes = [0u8; SETTING_EXT32_LEN];
    let encoded = encode_trigger(config, config.mode_bits());
    let mut w = Writer {
        bytes: &mut bytes,
        at: 0,
    };
    w.u32(SYNC);
    w.u16(0x6060);
    w.stage_words(&encoded.high);
    w.u16(0xffff);
    w.u32(END_SYNC);
    debug_assert_eq!(w.at, SETTING_EXT32_LEN);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{CHANNEL_MODES, UsbSpeed, VENDOR_ID, find_profile};

    fn plus() -> &'static Profile {
        find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("DSLogic Plus")
    }

    #[test]
    fn simple_trigger_marks_the_channel() {
        let setup = TriggerSetup::simple(&[(3, TriggerCondition::Rise)], 50, false);
        assert!(setup.enabled);
        assert_eq!(setup.position, 50);
        assert_eq!(setup.stages[SIMPLE_STAGE].pattern0[3], b'R');
        assert_eq!(setup.stages[SIMPLE_STAGE].pattern0[2], b'X');
    }

    #[test]
    fn instant_capture_ignores_the_trigger() {
        let setup = TriggerSetup::simple(&[(3, TriggerCondition::Rise)], 50, true);
        assert_eq!(setup, TriggerSetup::default());
    }

    #[test]
    fn counts_at_least_one_channel() {
        let trigger = TriggerSetup::default();
        let config = ArmConfig {
            profile: plus(),
            mode: &CHANNEL_MODES[0],
            samplerate: 1_000_000,
            limit_samples: 1000,
            enabled: 0,
            filter: false,
            external_clock: false,
            clock_falling_edge: false,
            rle: false,
            instant: false,
            looping: false,
            trigger: &trigger,
        };
        assert_eq!(config.enabled_count(), 1);
        assert_eq!(config.actual_samples(), 1024);
        assert_eq!(config.actual_bytes(), 128);
    }
}

#[cfg(test)]
mod advanced_tests {
    use hsync_driver::{SerialTriggerSpec, TriggerStageSpec};

    use super::*;

    fn chars(text: &str) -> [u8; TRIGGER_CHANNELS] {
        let mut out = [b'X'; TRIGGER_CHANNELS];
        out[..text.len()].copy_from_slice(text.as_bytes());
        out
    }

    #[test]
    fn stages_keep_their_patterns_logic_and_counts() {
        let trigger = AdvancedTrigger::Stages {
            stages: vec![
                TriggerStageSpec {
                    pattern0: vec![(0, TriggerCondition::High), (2, TriggerCondition::Rise)],
                    pattern1: vec![(1, TriggerCondition::Low)],
                    both: false,
                    invert0: false,
                    invert1: true,
                    count: 3,
                    contiguous: true,
                },
                TriggerStageSpec {
                    pattern0: vec![(17, TriggerCondition::Both)],
                    pattern1: Vec::new(),
                    both: true,
                    invert0: false,
                    invert1: false,
                    count: 1,
                    contiguous: false,
                },
            ],
        };
        let setup = TriggerSetup::advanced(&trigger, 50);
        assert!(setup.enabled);
        assert_eq!(setup.mode, TriggerMode::Advanced);
        assert_eq!(setup.last_stage, 1);
        assert_eq!(setup.position, 50);
        assert_eq!(setup.stages[0].pattern0, chars("1XR"));
        assert_eq!(setup.stages[0].pattern1, chars("X0"));
        assert_eq!(setup.stages[0].logic, 2);
        assert!(setup.stages[0].invert1);
        assert_eq!(setup.stages[0].count, 3);
        assert_eq!(setup.stages[1].pattern0, chars("XXXXXXXXXXXXXXXXXC"));
        assert_eq!(setup.stages[1].logic, 1);
        assert_eq!(setup.stages[2], TriggerStage::default());
    }

    #[test]
    fn serial_fills_the_four_stages_as_dsview() {
        let trigger = AdvancedTrigger::Serial(SerialTriggerSpec {
            start: vec![(0, TriggerCondition::Fall)],
            stop: vec![(0, TriggerCondition::Rise)],
            clock: vec![(1, TriggerCondition::Rise)],
            data_channel: 2,
            bits: 8,
            value: 0x00a5,
            mask: 0x00f0,
        });
        let setup = TriggerSetup::advanced(&trigger, 10);
        assert_eq!(setup.mode, TriggerMode::Serial);
        assert_eq!(setup.last_stage, 0);
        assert_eq!(setup.stages[0].pattern0, chars("F"));
        assert_eq!(setup.stages[0].pattern1, chars("R"));
        assert_eq!(setup.stages[1].pattern0, chars("XR"));
        assert_eq!(setup.stages[1].count, 1);
        assert_eq!(setup.stages[2].pattern0, chars("XX0"));
        // Bit k of the value in position k: 0xA5 masked to its upper nibble.
        assert_eq!(setup.stages[3].pattern0, chars("XXXX0101"));
        assert_eq!(setup.stages[3].count, 7);
        assert_eq!(setup.stages[0].logic, 1);
        assert_eq!(setup.stages[0].count, 0);
    }

    #[test]
    fn serial_data_channels_past_16_go_to_the_second_pattern() {
        let trigger = AdvancedTrigger::Serial(SerialTriggerSpec {
            start: Vec::new(),
            stop: Vec::new(),
            clock: vec![(20, TriggerCondition::Rise)],
            data_channel: 18,
            bits: 1,
            value: 1,
            mask: 1,
        });
        let setup = TriggerSetup::advanced(&trigger, 10);
        assert_eq!(setup.stages[2].pattern0, [b'X'; TRIGGER_CHANNELS]);
        assert_eq!(setup.stages[2].pattern1, chars("XX0"));
        assert_eq!(setup.stages[3].count, 0);
    }
}
