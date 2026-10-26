//! Shared domain types for HSync: sample positions, channels, trigger conditions and the
//! error shape that crosses into the user interface.

#![forbid(unsafe_code)]

mod error;
mod trigger;

pub use error::UiError;
pub use trigger::TriggerCondition;

/// Position of a sample within a capture, counted from the first captured sample.
pub type SampleIndex = u64;

/// Hardware channel number (probe index), `0..32` on DSLogic devices. Analog channels number
/// from [`ANALOG_BASE`], so that one index names any channel of a capture.
pub type ChannelIndex = u16;

/// Index of analog channel A0; A`n` is `ANALOG_BASE + n`. Logic channels stay below it.
pub const ANALOG_BASE: ChannelIndex = 0x8000;

/// Whether `index` names an analog channel.
pub fn is_analog(index: ChannelIndex) -> bool {
    index >= ANALOG_BASE
}

/// How an analog channel's converter codes become volts: `offset + code × step`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalogScale {
    /// Volts per code.
    pub step: f32,
    /// Volts of code 0.
    pub offset: f32,
}

impl AnalogScale {
    /// The voltage of `code`.
    pub fn volts(&self, code: u16) -> f32 {
        self.offset + f32::from(code) * self.step
    }
}
