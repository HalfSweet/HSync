//! The binary format of viewport queries.
//!
//! ```text
//! header (little endian, 32 bytes)
//!   u32 version = 1
//!   u32 channel count
//!   u32 bins per channel
//!   u32 reserved = 0
//!   u64 start sample
//!   u64 end sample (exclusive)
//! per channel
//!   u16 channel index
//!   u16 reserved = 0
//!   u8[bins] cells, see hsync_capture::cell
//! ```
//!
//! Analog channels ([`encode_analog_bins`]) have the same header, then
//!
//! ```text
//! per channel
//!   u16 channel index
//!   u16 reserved = 0
//!   f32 lowest and f32 highest volts of the channel's samples so far, NaN without any
//!   f32[2 × bins] lowest and highest volts of each column, NaN beyond the samples
//! ```

use hsync_capture::LogicCapture;
use hsync_core::{ChannelIndex, SampleIndex};
use serde::Deserialize;
use ts_rs::TS;

use crate::SessionError;

/// Version of the binary layout written by [`encode_bins`].
pub const BINS_FORMAT_VERSION: u32 = 1;

/// Version of the binary layout written by [`encode_analog_bins`].
pub const ANALOG_BINS_FORMAT_VERSION: u32 = 1;

/// Upper bound on bins per channel, far above any screen width.
const MAX_BINS: u32 = 1 << 16;

/// A viewport query: summarise `start..end` of some channels in `bins` columns.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BinsRequest {
    /// Capture the request was made for.
    pub capture_id: u64,
    /// Channels to summarise.
    pub channels: Vec<ChannelIndex>,
    /// First sample.
    pub start: SampleIndex,
    /// Sample after the last one.
    pub end: SampleIndex,
    /// Columns per channel; at most the number of samples in the range.
    pub bins: u32,
}

/// Checks a request and writes the header; returns the columns per channel and the output, with
/// room for `per_bin` bytes a column.
fn header(
    request: &BinsRequest,
    version: u32,
    per_bin: usize,
) -> Result<(usize, Vec<u8>), SessionError> {
    if request.end <= request.start || request.bins == 0 || request.bins > MAX_BINS {
        return Err(SessionError::InvalidRange);
    }
    let bins = u64::from(request.bins).min(request.end - request.start);
    let bins = usize::try_from(bins).map_err(|_| SessionError::InvalidRange)?;
    let channel_count =
        u32::try_from(request.channels.len()).map_err(|_| SessionError::InvalidRange)?;
    let mut out = Vec::with_capacity(32 + request.channels.len() * (12 + bins * per_bin));
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&channel_count.to_le_bytes());
    out.extend_from_slice(&u32::try_from(bins).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&request.start.to_le_bytes());
    out.extend_from_slice(&request.end.to_le_bytes());
    Ok((bins, out))
}

/// Summarises the requested analog channels in the analog layout above.
pub fn encode_analog_bins(
    capture: &LogicCapture,
    request: &BinsRequest,
) -> Result<Vec<u8>, SessionError> {
    fn put(out: &mut Vec<u8>, (low, high): (f32, f32)) {
        out.extend_from_slice(&low.to_le_bytes());
        out.extend_from_slice(&high.to_le_bytes());
    }
    let (bins, mut out) = header(request, ANALOG_BINS_FORMAT_VERSION, 8)?;
    let none = (f32::NAN, f32::NAN);
    for &channel in &request.channels {
        let data = capture
            .analog_channel(channel)
            .ok_or(SessionError::UnknownChannel(channel))?;
        out.extend_from_slice(&channel.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        put(&mut out, data.volt_extent(0, data.len()).unwrap_or(none));
        for column in data.bins(request.start, request.end, bins) {
            put(&mut out, column.unwrap_or(none));
        }
    }
    Ok(out)
}

/// Summarises the requested channels in the binary layout above.
pub fn encode_bins(capture: &LogicCapture, request: &BinsRequest) -> Result<Vec<u8>, SessionError> {
    let (bins, mut out) = header(request, BINS_FORMAT_VERSION, 4 + 1)?;
    for &channel in &request.channels {
        let data = capture
            .channel(channel)
            .ok_or(SessionError::UnknownChannel(channel))?;
        out.extend_from_slice(&channel.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&data.bins(request.start, request.end, bins));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use hsync_capture::cell;

    use super::*;

    fn capture() -> LogicCapture {
        let mut capture = LogicCapture::new(1_000, [2, 7]);
        // Channel 2 is high for the first half of each word, channel 7 stays low.
        if let Some(data) = capture.channel_mut(2) {
            data.append_words(&[0x0000_0000_FFFF_FFFF; 2]);
        }
        if let Some(data) = capture.channel_mut(7) {
            data.append_words(&[0; 2]);
        }
        capture
    }

    #[test]
    fn writes_header_and_cells() {
        let request = BinsRequest {
            capture_id: 1,
            channels: vec![7, 2],
            start: 0,
            end: 128,
            bins: 4,
        };
        let out = encode_bins(&capture(), &request).expect("valid request");
        assert_eq!(out.len(), 32 + 2 * (4 + 4));
        assert_eq!(&out[0..4], &1u32.to_le_bytes());
        assert_eq!(&out[4..8], &2u32.to_le_bytes());
        assert_eq!(&out[8..12], &4u32.to_le_bytes());
        assert_eq!(&out[24..32], &128u64.to_le_bytes());
        // Channel 7 first, as requested.
        assert_eq!(&out[32..34], &7u16.to_le_bytes());
        assert_eq!(&out[36..40], &[0, 0, 0, 0]);
        assert_eq!(&out[40..42], &2u16.to_le_bytes());
        let high = cell::FIRST_HIGH | cell::LAST_HIGH;
        assert_eq!(&out[44..48], &[high, 0, high, 0]);
    }

    #[test]
    fn clamps_bins_to_the_number_of_samples() {
        let request = BinsRequest {
            capture_id: 1,
            channels: vec![2],
            start: 10,
            end: 13,
            bins: 100,
        };
        let out = encode_bins(&capture(), &request).expect("valid request");
        assert_eq!(&out[8..12], &3u32.to_le_bytes());
        assert_eq!(out.len(), 32 + 4 + 3);
    }

    fn floats(bytes: &[u8]) -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect()
    }

    #[test]
    fn writes_analog_extents_and_empty_columns_as_nan() {
        let a0 = hsync_core::ANALOG_BASE;
        let scale = hsync_core::AnalogScale {
            step: 0.5,
            offset: -1.0,
        };
        let mut capture = LogicCapture::new(1_000, []).with_analog([(a0, scale)]);
        capture.append_analog(&[0, 4, 2, 10]);
        let request = BinsRequest {
            capture_id: 1,
            channels: vec![a0],
            start: 0,
            end: 6,
            bins: 3,
        };
        let out = encode_analog_bins(&capture, &request).expect("valid request");
        assert_eq!(&out[0..4], &ANALOG_BINS_FORMAT_VERSION.to_le_bytes());
        assert_eq!(&out[8..12], &3u32.to_le_bytes());
        assert_eq!(out.len(), 32 + 4 + 8 + 3 * 8);
        assert_eq!(&out[32..34], &a0.to_le_bytes());
        let values = floats(&out[36..]);
        assert_eq!(&values[..6], &[-1.0, 4.0, -1.0, 1.0, 0.0, 4.0]);
        assert!(values[6].is_nan() && values[7].is_nan());
        assert!(matches!(
            encode_analog_bins(
                &capture,
                &BinsRequest {
                    channels: vec![0],
                    ..request
                }
            ),
            Err(SessionError::UnknownChannel(0))
        ));
    }

    #[test]
    fn rejects_bad_requests() {
        let mut request = BinsRequest {
            capture_id: 1,
            channels: vec![3],
            start: 0,
            end: 64,
            bins: 8,
        };
        assert!(matches!(
            encode_bins(&capture(), &request),
            Err(SessionError::UnknownChannel(3))
        ));
        request.channels = vec![2];
        request.end = 0;
        assert!(matches!(
            encode_bins(&capture(), &request),
            Err(SessionError::InvalidRange)
        ));
    }
}
