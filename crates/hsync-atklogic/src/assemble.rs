//! Per-channel samples from the device, re-cut into the cross format acquisitions send
//! ([`hsync_driver::Packet::Logic`]): groups of 64 samples, one little-endian `u64` per channel.
//!
//! The device sends each channel on its own, eight samples a byte with the earliest in bit 0.
//! A capture in Buffer mode may hold more samples than asked for; the start of each channel is
//! dropped so that the trigger lands where it was asked to (`Segment::SetStartOffsetSampling()`).

use hsync_core::ChannelIndex;

/// Samples of one group.
const GROUP: u64 = 64;

#[derive(Debug, Default)]
struct Lane {
    /// Bytes not yet forwarded, from `head` on.
    bytes: Vec<u8>,
    head: usize,
    /// Whole bytes still to drop from what arrives.
    skip_bytes: u64,
    /// Bits of the first byte kept that belong to dropped samples (0–7).
    shift: u32,
    /// The device holds nothing for the channel; it reads as low.
    absent: bool,
}

impl Lane {
    fn len(&self) -> usize {
        self.bytes.len() - self.head
    }

    /// Samples ready to forward.
    fn available(&self) -> u64 {
        if self.absent {
            return u64::MAX;
        }
        (self.len() as u64 * 8).saturating_sub(u64::from(self.shift))
    }

    /// Adds bytes, keeping no more than `room` samples ready (`None`: no limit).
    fn push(&mut self, mut data: &[u8], room: Option<u64>) {
        let dropped = usize::try_from(self.skip_bytes)
            .unwrap_or(usize::MAX)
            .min(data.len());
        data = &data[dropped..];
        self.skip_bytes -= dropped as u64;
        if let Some(room) = room {
            let wanted = (room + u64::from(self.shift)).div_ceil(8);
            let missing = wanted.saturating_sub(self.len() as u64);
            data = &data[..usize::try_from(missing)
                .unwrap_or(usize::MAX)
                .min(data.len())];
        }
        if self.head > 0 && self.head * 2 >= self.bytes.len() {
            self.bytes.drain(..self.head);
            self.head = 0;
        }
        self.bytes.extend_from_slice(data);
    }

    /// Samples `64 · group` to `64 · group + 63` of those ready.
    fn word(&self, group: usize) -> u64 {
        if self.absent {
            return 0;
        }
        let at = self.head + group * 8;
        let Some(low) = self.bytes[at..].first_chunk::<8>() else {
            unreachable!("only ready groups are read");
        };
        let low = u64::from_le_bytes(*low);
        if self.shift == 0 {
            low
        } else {
            (low >> self.shift) | (u64::from(self.bytes[at + 8]) << (64 - self.shift))
        }
    }

    fn consume(&mut self, groups: usize) {
        if !self.absent {
            self.head += groups * 8;
        }
    }
}

/// Collects the channels of a capture and forwards them in whole groups.
#[derive(Debug)]
pub(crate) struct Assembler {
    lanes: Vec<Lane>,
    /// Lane of each channel the capture holds.
    lane_of: Vec<Option<usize>>,
    /// Samples per channel to forward; `None` forwards all.
    limit: Option<u64>,
    /// Samples per channel forwarded.
    emitted: u64,
}

impl Assembler {
    /// An assembler for `channels`, ascending, forwarding at most `limit` samples of each.
    pub(crate) fn new(channels: &[ChannelIndex], limit: Option<u64>) -> Self {
        let mut lane_of = Vec::new();
        for (lane, &channel) in channels.iter().enumerate() {
            let channel = usize::from(channel);
            if lane_of.len() <= channel {
                lane_of.resize(channel + 1, None);
            }
            lane_of[channel] = Some(lane);
        }
        Self {
            lanes: channels.iter().map(|_| Lane::default()).collect(),
            lane_of,
            limit,
            emitted: 0,
        }
    }

    fn lane(&mut self, channel: ChannelIndex) -> Option<&mut Lane> {
        let lane = (*self.lane_of.get(usize::from(channel))?)?;
        self.lanes.get_mut(lane)
    }

    /// Whether the capture holds `channel`.
    pub(crate) fn has(&self, channel: ChannelIndex) -> bool {
        matches!(self.lane_of.get(usize::from(channel)), Some(Some(_)))
    }

    /// Drops the first `samples` samples of `channel`, before any of its data arrives.
    pub(crate) fn skip(&mut self, channel: ChannelIndex, samples: u64) {
        let Some(lane) = self.lane(channel) else {
            return;
        };
        lane.skip_bytes = samples / 8;
        lane.shift = (samples % 8) as u32;
        // Whatever came before is dropped first.
        let buffered = usize::try_from(lane.skip_bytes)
            .unwrap_or(usize::MAX)
            .min(lane.len());
        lane.head += buffered;
        lane.skip_bytes -= buffered as u64;
    }

    /// Marks `channel` as one the device holds nothing for; it reads as low.
    pub(crate) fn absent(&mut self, channel: ChannelIndex) {
        if let Some(lane) = self.lane(channel) {
            lane.absent = true;
        }
    }

    /// Adds samples of `channel`; those of channels the capture does not hold are dropped.
    pub(crate) fn push(&mut self, channel: ChannelIndex, data: &[u8]) {
        let room = self.limit.map(|limit| limit.saturating_sub(self.emitted));
        if let Some(lane) = self.lane(channel) {
            lane.push(data, room);
        }
    }

    /// The groups every channel is ready with, in cross format.
    pub(crate) fn take(&mut self) -> Vec<u8> {
        let ready = self
            .lanes
            .iter()
            .filter(|lane| !lane.absent)
            .map(Lane::available)
            .min()
            .unwrap_or(0);
        let mut groups = ready / GROUP;
        if let Some(limit) = self.limit {
            groups = groups.min(limit.saturating_sub(self.emitted) / GROUP);
        }
        let groups = usize::try_from(groups).unwrap_or(0);
        if groups == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(groups * self.lanes.len() * 8);
        for group in 0..groups {
            for lane in &self.lanes {
                out.extend_from_slice(&lane.word(group).to_le_bytes());
            }
        }
        for lane in &mut self.lanes {
            lane.consume(groups);
        }
        self.emitted += groups as u64 * GROUP;
        out
    }

    /// Samples per channel forwarded.
    pub(crate) fn emitted(&self) -> u64 {
        self.emitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words of `cross` for each of `channels` channels.
    fn split(cross: &[u8], channels: usize) -> Vec<Vec<u64>> {
        let mut words = vec![Vec::new(); channels];
        for (at, word) in cross.as_chunks::<8>().0.iter().enumerate() {
            words[at % channels].push(u64::from_le_bytes(*word));
        }
        words
    }

    fn samples(words: &[u64], count: usize) -> Vec<bool> {
        (0..count)
            .map(|sample| (words[sample / 64] >> (sample % 64)) & 1 == 1)
            .collect()
    }

    #[test]
    fn interleaves_channels_into_groups() {
        let mut assembler = Assembler::new(&[0, 3], None);
        assembler.push(0, &[0x11; 12]);
        assert!(assembler.take().is_empty(), "channel 3 has nothing yet");
        assembler.push(3, &[0x22; 20]);
        let cross = assembler.take();
        assert_eq!(
            split(&cross, 2),
            vec![vec![0x1111_1111_1111_1111], vec![0x2222_2222_2222_2222]]
        );
        assembler.push(0, &[0x33; 4]);
        let cross = assembler.take();
        assert_eq!(
            split(&cross, 2),
            vec![vec![0x3333_3333_1111_1111], vec![0x2222_2222_2222_2222]]
        );
        assert_eq!(assembler.emitted(), 128);
        // Channel 5 is not captured.
        assembler.push(5, &[0xff; 64]);
        assert!(!assembler.has(5));
    }

    #[test]
    fn drops_the_start_of_channels() {
        // Sample i of the source is bit i % 8 of byte i / 8: every third sample high.
        let source: Vec<u8> = (0..64)
            .map(|byte| {
                (0..8).fold(0u8, |acc, bit| {
                    acc | (u8::from((byte * 8 + bit) % 3 == 0) << bit)
                })
            })
            .collect();
        let expected = |skip: usize| (skip..skip + 256).map(|i| i % 3 == 0).collect::<Vec<_>>();
        for skip in [0, 5, 8, 13, 64, 77] {
            let mut assembler = Assembler::new(&[1], Some(256));
            assembler.skip(1, skip as u64);
            for piece in source.chunks(7) {
                assembler.push(1, piece);
            }
            let cross = assembler.take();
            assert_eq!(
                samples(&split(&cross, 1)[0], 256),
                expected(skip),
                "skipping {skip}"
            );
        }
    }

    #[test]
    fn stops_at_the_limit() {
        let mut assembler = Assembler::new(&[0], Some(128));
        assembler.push(0, &[0xaa; 100]);
        assert_eq!(assembler.take().len(), 16);
        assembler.push(0, &[0xbb; 100]);
        assert!(assembler.take().is_empty());
        assert_eq!(assembler.emitted(), 128);
    }

    #[test]
    fn reads_absent_channels_as_low() {
        let mut assembler = Assembler::new(&[0, 1], None);
        assembler.absent(1);
        assert!(assembler.take().is_empty());
        assembler.push(0, &[0xff; 16]);
        assert_eq!(split(&assembler.take(), 2), vec![vec![!0, !0], vec![0, 0]]);
    }
}
