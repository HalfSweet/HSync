//! Shared domain types for HSync: sample positions, channels, trigger conditions and the
//! error shape that crosses into the user interface.

#![forbid(unsafe_code)]

mod error;
mod trigger;

pub use error::UiError;
pub use trigger::TriggerCondition;

/// Position of a sample within a capture, counted from the first captured sample.
pub type SampleIndex = u64;

/// Hardware channel number (probe index), `0..32` on DSLogic devices.
pub type ChannelIndex = u16;
