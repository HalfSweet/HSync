//! The demo device of HSync: replays the bundled protocol capture or
//! synthesises random and counter signals, with two sine waves on analog channels, in real
//! time, through the same driver traits and packets as hardware.

#![forbid(unsafe_code)]

mod acquisition;
mod analog;
mod device;
mod pattern;

pub use device::{DRIVER_NAME, DemoDevice, DemoDriver, SAMPLERATES};
pub use pattern::{CHANNELS, Pattern, PatternKind, ReplaySource};
