// SPDX-License-Identifier: Apache-2.0

//! STM8 single-wire interface, implemented from STMicroelectronics UM0470 Rev 4,
//! sections 3.2–3.6, independently of other decoder implementations.
//!
//! [`SwimEvent`] reports physical packets separately from their inferred command fields.
//! The line alone cannot distinguish a host communication reset from a target sync pulse.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{Speed, Swim, SwimConfig, SwimConfigError};
pub use events::{Direction, ErrorReason, Field, SWIM_EVENTS, SwimEvent};
