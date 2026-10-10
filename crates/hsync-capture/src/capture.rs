//! A logic capture: per-channel samples plus the facts that place them in time.

use hsync_core::{ChannelIndex, SampleIndex};
use serde::Serialize;
use ts_rs::TS;

use crate::channel::ChannelData;
use crate::ingest::CrossDeinterleaver;

/// What the UI needs to know about a capture to lay out its timeline.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureInfo {
    /// Samples per second; `0` for captures clocked externally.
    pub samplerate: u64,
    /// Number of samples every channel holds.
    pub sample_count: u64,
    /// Sample at which the trigger fired, if it did.
    pub trigger_sample: Option<u64>,
    /// When the trigger fired (or the capture started), in milliseconds since the Unix epoch.
    pub trigger_time_ms: Option<i64>,
    /// Channels that hold data, in ascending order.
    pub channels: Vec<ChannelIndex>,
}

/// Samples of the channels of one capture.
#[derive(Debug, Clone)]
pub struct LogicCapture {
    samplerate: u64,
    trigger_sample: Option<SampleIndex>,
    trigger_time_ms: Option<i64>,
    channels: Vec<(ChannelIndex, ChannelData)>,
}

impl LogicCapture {
    /// Creates an empty capture of the given channels.
    pub fn new(samplerate: u64, channels: impl IntoIterator<Item = ChannelIndex>) -> Self {
        let mut channels: Vec<_> = channels
            .into_iter()
            .map(|index| (index, ChannelData::new()))
            .collect();
        channels.sort_by_key(|(index, _)| *index);
        channels.dedup_by_key(|(index, _)| *index);
        Self {
            samplerate,
            trigger_sample: None,
            trigger_time_ms: None,
            channels,
        }
    }

    /// Samples per second.
    pub fn samplerate(&self) -> u64 {
        self.samplerate
    }

    /// Number of samples every channel holds.
    pub fn len(&self) -> SampleIndex {
        self.channels
            .iter()
            .map(|(_, data)| data.len())
            .min()
            .unwrap_or(0)
    }

    /// Whether the capture holds no samples.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Records where and when the trigger fired.
    pub fn set_trigger(&mut self, sample: Option<SampleIndex>, time_ms: Option<i64>) {
        self.trigger_sample = sample;
        self.trigger_time_ms = time_ms;
    }

    /// Sample at which the trigger fired.
    pub fn trigger_sample(&self) -> Option<SampleIndex> {
        self.trigger_sample
    }

    /// The samples of channel `index`.
    pub fn channel(&self, index: ChannelIndex) -> Option<&ChannelData> {
        self.channels
            .binary_search_by_key(&index, |(i, _)| *i)
            .ok()
            .map(|at| &self.channels[at].1)
    }

    /// Mutable samples of channel `index`.
    pub fn channel_mut(&mut self, index: ChannelIndex) -> Option<&mut ChannelData> {
        self.channels
            .binary_search_by_key(&index, |(i, _)| *i)
            .ok()
            .map(|at| &mut self.channels[at].1)
    }

    /// Channels in ascending order.
    pub fn channels(&self) -> impl Iterator<Item = (ChannelIndex, &ChannelData)> {
        self.channels.iter().map(|(index, data)| (*index, data))
    }

    /// A de-interleaver matching this capture's channels.
    pub fn deinterleaver(&self) -> CrossDeinterleaver {
        CrossDeinterleaver::new(self.channels.len().max(1))
    }

    /// Appends cross-format bytes; the capture's channels are the enabled channels in the
    /// order the device sends them.
    pub fn append_cross(&mut self, deinterleaver: &mut CrossDeinterleaver, bytes: &[u8]) {
        debug_assert_eq!(deinterleaver.channels(), self.channels.len());
        let staged = deinterleaver.push(bytes);
        for ((_, data), words) in self.channels.iter_mut().zip(staged) {
            data.append_words(words);
        }
    }

    /// Samples `from..to` of `channels` as a capture of their own, for saving or exporting part of
    /// a capture. Channels the capture lacks are left out; the trigger stays when it falls inside.
    pub fn slice(&self, from: SampleIndex, to: SampleIndex, channels: &[ChannelIndex]) -> Self {
        /// Words copied at once, to bound the staging buffer.
        const BATCH: usize = 1 << 16;
        let to = to.min(self.len());
        let from = from.min(to);
        let len = to - from;
        let kept: Vec<ChannelIndex> = channels
            .iter()
            .copied()
            .filter(|&index| self.channel(index).is_some())
            .collect();
        let mut out = Self::new(self.samplerate, kept);
        out.trigger_time_ms = self.trigger_time_ms;
        out.trigger_sample = self
            .trigger_sample
            .filter(|&sample| sample >= from && sample < to)
            .map(|sample| sample - from);
        let first = (from / 64) as usize;
        let shift = from % 64;
        let words = len.div_ceil(64) as usize;
        for (index, data) in &mut out.channels {
            let Some(source) = self.channel(*index) else {
                continue;
            };
            let available = source.word_count();
            let word = |at: usize| if at < available { source.word(at) } else { 0 };
            let mut batch = Vec::with_capacity(words.min(BATCH));
            for at in 0..words {
                let low = word(first + at) >> shift;
                let high = if shift == 0 {
                    0
                } else {
                    word(first + at + 1) << (64 - shift)
                };
                batch.push(low | high);
                if batch.len() == BATCH {
                    data.append_words(&batch);
                    batch.clear();
                }
            }
            data.append_words(&batch);
            data.truncate(len);
        }
        out
    }

    /// Forgets the first `samples` samples of every channel, a multiple of 64, as loop
    /// captures do once they hold their limit. A trigger among them is forgotten too.
    ///
    /// # Panics
    ///
    /// Panics if `samples` is not a multiple of 64 or exceeds the capture.
    pub fn drop_front(&mut self, samples: SampleIndex) {
        for (_, data) in &mut self.channels {
            data.drop_front(samples);
        }
        self.trigger_sample = self
            .trigger_sample
            .and_then(|sample| sample.checked_sub(samples));
    }

    /// Lines the chunks of every channel up with its samples again after
    /// [`Self::drop_front`], one channel at a time.
    pub fn compact(&mut self) {
        for (_, data) in &mut self.channels {
            data.compact();
        }
    }

    /// Shortens every channel to `len` samples.
    pub fn truncate(&mut self, len: SampleIndex) {
        for (_, data) in &mut self.channels {
            data.truncate(len);
        }
    }

    /// Bytes of heap memory held by sample data.
    pub fn heap_bytes(&self) -> usize {
        self.channels
            .iter()
            .map(|(_, data)| data.heap_bytes())
            .sum()
    }

    /// Summary for the UI.
    pub fn info(&self) -> CaptureInfo {
        CaptureInfo {
            samplerate: self.samplerate,
            sample_count: self.len(),
            trigger_sample: self.trigger_sample,
            trigger_time_ms: self.trigger_time_ms,
            channels: self.channels.iter().map(|(index, _)| *index).collect(),
        }
    }
}
