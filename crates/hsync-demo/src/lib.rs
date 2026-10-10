//! The demo device of HSync (DESIGN.md §4.10): replays the bundled protocol capture or
//! synthesises random and counter signals, in real time, through the same driver traits and
//! packets as hardware.

#![forbid(unsafe_code)]

mod acquisition;
mod device;
mod pattern;

pub use device::{DRIVER_NAME, DemoDevice, DemoDriver, SAMPLERATES};
pub use pattern::{CHANNELS, Pattern, PatternKind, ReplaySource};
