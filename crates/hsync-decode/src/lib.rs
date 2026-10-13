//! Protocol decoding: a sigrok-style wait engine over the bit-packed samples of
//! `hsync-capture`, and native decoders for the buses the design presents.

#![forbid(unsafe_code)]

pub mod can;
pub mod i2c;
pub mod sampler;
pub mod spec;
pub mod spi;
pub mod uart;

pub use spec::{Bus, Decoded, Settings, Spec};
