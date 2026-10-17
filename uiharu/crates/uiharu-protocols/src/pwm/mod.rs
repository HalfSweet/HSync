// SPDX-License-Identifier: Apache-2.0

//! PWM: the decoder from the logic samples of one pulse-width modulated line to `pwm.events`,
//! and a waveform synthesizer.
//!
//! The decoder is written in the DSL (`uiharu/dsl/decoders/pwm.ts`) and generated into
//! `uiharu_dsl_protocols::pwm`; this module re-exports it.

pub mod synth;

pub use uiharu_dsl_protocols::pwm::{Polarity, Pwm, PwmConfig, PwmEvent, is_active};

use uiharu_core::{EventSchema, SchemaId};

/// The schema of PWM event streams.
pub const PWM_EVENTS: SchemaId = PwmEvent::SCHEMA;

/// Why a configuration is rejected.
pub type PwmConfigError = uiharu_core::dsl::ConfigError;
