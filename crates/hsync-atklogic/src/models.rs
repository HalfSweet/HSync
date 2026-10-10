//! What the DL16 and DL16 Plus can do, from the capture settings of the ATK-Logic application
//! (`qml/session/content/SetContent.qml`: `resetHzComboBox()`, `resetTimeComboBox()`) and its
//! manual.

const MHZ: u64 = 1_000_000;

/// Channels of every model (`DataService::m_channelCount`).
pub const CHANNELS: u16 = 16;

/// Sample rates in the order of ATK-Logic's rate list; the code [`crate::protocol::cmd::SETTINGS`]
/// carries for a rate is its position plus one (`selectHzIndex + 1`).
pub const SAMPLERATES: [u64; 14] = [
    MHZ,
    2 * MHZ,
    4 * MHZ,
    5 * MHZ,
    10 * MHZ,
    20 * MHZ,
    25 * MHZ,
    40 * MHZ,
    50 * MHZ,
    100 * MHZ,
    200 * MHZ,
    250 * MHZ,
    500 * MHZ,
    1000 * MHZ,
];

/// Fastest rate in Stream mode.
const STREAM_MAX_RATE: u64 = 100 * MHZ;
/// Samples per second over all channels that Stream mode carries (`maxDepth = 320000000`).
const STREAM_BANDWIDTH: u64 = 320 * MHZ;
/// Fastest rate in Buffer mode on the DL16.
const BUFFER_MAX_RATE: u64 = 250 * MHZ;
/// Fastest rate in Buffer mode on the DL16 Plus with more than [`GIGA_CHANNELS`] channels.
const BUFFER_MAX_RATE_PLUS: u64 = 500 * MHZ;
/// Channels the DL16 Plus samples at 1 GHz.
const GIGA_CHANNELS: u32 = 8;

/// Bits of sample memory of the DL16 (`maxDepth = 1000000000 * 1000` per millisecond).
const MEMORY_BITS: u64 = 1_000_000_000;
/// Bits of sample memory of the DL16 Plus.
const MEMORY_BITS_PLUS: u64 = 3_500_000_000;
/// How far RLE compression lets a Buffer capture run beyond the memory
/// (`rleMaxDepth = maxDepth * 10`).
pub const RLE_FACTOR: u64 = 10;

/// The sample rates available, ascending: in Stream mode as far as USB carries them for
/// `enabled` channels, in Buffer mode up to the model's limit.
pub fn samplerates(plus: bool, stream: bool, enabled: u32) -> Vec<u64> {
    let channels = u64::from(enabled.max(1));
    SAMPLERATES
        .iter()
        .copied()
        .filter(|&rate| {
            if stream {
                rate <= STREAM_MAX_RATE && rate * channels <= STREAM_BANDWIDTH
            } else if plus {
                rate <= BUFFER_MAX_RATE_PLUS || enabled <= GIGA_CHANNELS
            } else {
                rate <= BUFFER_MAX_RATE
            }
        })
        .collect()
}

/// The code of a sample rate in the capture settings.
pub fn rate_code(rate: u64) -> Option<u8> {
    SAMPLERATES
        .iter()
        .position(|&candidate| candidate == rate)
        .and_then(|index| u8::try_from(index + 1).ok())
}

/// Samples per channel the memory holds with `enabled` channels.
pub fn memory_depth(plus: bool, enabled: u32) -> u64 {
    let bits = if plus { MEMORY_BITS_PLUS } else { MEMORY_BITS };
    bits / u64::from(enabled.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_rates_follow_the_usb_bandwidth() {
        assert_eq!(samplerates(false, true, 16).last(), Some(&(20 * MHZ)));
        assert_eq!(samplerates(true, true, 6).last(), Some(&(50 * MHZ)));
        assert_eq!(samplerates(false, true, 3).last(), Some(&(100 * MHZ)));
        assert_eq!(samplerates(false, true, 1).last(), Some(&(100 * MHZ)));
        assert_eq!(samplerates(false, true, 16).first(), Some(&MHZ));
    }

    #[test]
    fn buffer_rates_follow_the_model() {
        assert_eq!(samplerates(false, false, 16).last(), Some(&(250 * MHZ)));
        assert_eq!(samplerates(false, false, 1).last(), Some(&(250 * MHZ)));
        assert_eq!(samplerates(true, false, 16).last(), Some(&(500 * MHZ)));
        assert_eq!(samplerates(true, false, 9).last(), Some(&(500 * MHZ)));
        assert_eq!(samplerates(true, false, 8).last(), Some(&(1000 * MHZ)));
        assert_eq!(samplerates(true, false, 8).len(), SAMPLERATES.len());
    }

    #[test]
    fn codes_rates_by_their_place() {
        assert_eq!(rate_code(MHZ), Some(1));
        assert_eq!(rate_code(20 * MHZ), Some(6));
        assert_eq!(rate_code(100 * MHZ), Some(10));
        assert_eq!(rate_code(1000 * MHZ), Some(14));
        assert_eq!(rate_code(3 * MHZ), None);
    }

    #[test]
    fn shares_the_memory_between_channels() {
        assert_eq!(memory_depth(false, 16), 62_500_000);
        assert_eq!(memory_depth(true, 1), 3_500_000_000);
        assert_eq!(memory_depth(true, 0), 3_500_000_000);
    }
}
