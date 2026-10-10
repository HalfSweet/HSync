//! What an acquisition sends to the session.

use hsync_core::{ChannelIndex, UiError};

/// One message of a running acquisition.
#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    /// Samples follow for these channels: cross-format groups of 64 samples, one little-endian
    /// `u64` per channel in this order.
    Start {
        channels: Vec<ChannelIndex>,
        samplerate: u64,
    },
    /// The trigger fired at this sample.
    Trigger { sample: u64 },
    /// Cross-format sample bytes; boundaries are arbitrary.
    Logic(Vec<u8>),
    /// While the hardware buffer fills: whether the trigger fired and how many samples are in.
    Progress { triggered: bool, captured: u64 },
    /// The device lost data because the host did not keep up (stream mode).
    Overflow,
    /// The acquisition is over; no packets follow.
    End(EndReason),
}

/// Why an acquisition ended.
#[derive(Debug, Clone, PartialEq)]
pub enum EndReason {
    /// Every requested sample arrived.
    Completed,
    /// The user stopped it; the samples received so far are valid.
    Stopped,
    /// The device was unplugged.
    Detached,
    /// The device or the USB link failed.
    Failed(UiError),
}
