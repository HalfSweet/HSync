//! Protocol decoding (DESIGN.md §7): a sigrok-style wait engine over the bit-packed samples of
//! `hsync-capture`, and native decoders for the buses the design presents.

#![forbid(unsafe_code)]

pub mod can;
pub mod i2c;
pub mod present;
pub mod sampler;
pub mod search;
pub mod spec;
pub mod spi;
pub mod uart;

pub use present::{
    DataLine, DecoderView, I2cCondition, I2cMeaning, I2cToken, LaneItem, LaneView, Presentation,
    StreamCounts, StreamMode, StreamNote, StreamRow,
};
pub use search::{Found, Needle};
pub use spec::{Bus, Decoded, Settings, Spec};
