//! De-interleaving of DSLogic "cross" data.
//!
//! The FPGA sends groups of 64 samples: for every enabled channel in ascending order one
//! little-endian `u64` whose bit `j` is the channel's sample `j` of the group.
//! USB transfers end at arbitrary byte offsets, so the de-interleaver carries the current
//! channel and an incomplete word across calls.

/// Splits cross-format bytes into per-channel words.
#[derive(Debug, Clone)]
pub struct CrossDeinterleaver {
    channels: usize,
    cursor: usize,
    partial: [u8; 8],
    partial_len: usize,
    staged: Vec<Vec<u64>>,
}

impl CrossDeinterleaver {
    /// Creates a de-interleaver for `channels` enabled channels.
    ///
    /// # Panics
    ///
    /// Panics if `channels` is zero.
    pub fn new(channels: usize) -> Self {
        assert!(channels > 0, "at least one channel must be enabled");
        Self {
            channels,
            cursor: 0,
            partial: [0; 8],
            partial_len: 0,
            staged: vec![Vec::new(); channels],
        }
    }

    /// Number of enabled channels.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Feeds bytes and returns the complete words per channel (in cross order). Channels may
    /// receive different numbers of words when `bytes` ends inside a group.
    pub fn push(&mut self, mut bytes: &[u8]) -> &[Vec<u64>] {
        for words in &mut self.staged {
            words.clear();
        }
        if self.partial_len > 0 {
            let take = bytes.len().min(8 - self.partial_len);
            self.partial[self.partial_len..self.partial_len + take].copy_from_slice(&bytes[..take]);
            self.partial_len += take;
            bytes = &bytes[take..];
            if self.partial_len < 8 {
                return &self.staged;
            }
            let word = u64::from_le_bytes(self.partial);
            self.partial_len = 0;
            self.stage(word);
        }
        let (whole, rest) = bytes.as_chunks::<8>();
        for word in whole {
            self.stage(u64::from_le_bytes(*word));
        }
        self.partial[..rest.len()].copy_from_slice(rest);
        self.partial_len = rest.len();
        &self.staged
    }

    fn stage(&mut self, word: u64) {
        self.staged[self.cursor].push(word);
        self.cursor = (self.cursor + 1) % self.channels;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cross(groups: &[[u64; 3]]) -> Vec<u8> {
        groups
            .iter()
            .flat_map(|group| group.iter().flat_map(|word| word.to_le_bytes()))
            .collect()
    }

    #[test]
    fn splits_groups_by_channel() {
        let bytes = cross(&[[1, 2, 3], [4, 5, 6]]);
        let mut deinterleaver = CrossDeinterleaver::new(3);
        let staged = deinterleaver.push(&bytes);
        assert_eq!(staged, &[vec![1, 4], vec![2, 5], vec![3, 6]]);
    }

    #[test]
    fn carries_partial_words_and_groups_across_pushes() {
        let bytes = cross(&[[1, 2, 3], [4, 5, 6]]);
        let mut deinterleaver = CrossDeinterleaver::new(3);
        let mut got = vec![Vec::new(); 3];
        // 13 is neither a multiple of 8 nor of the 24-byte group.
        for piece in bytes.chunks(13) {
            for (channel, words) in deinterleaver.push(piece).iter().enumerate() {
                got[channel].extend_from_slice(words);
            }
        }
        assert_eq!(got, vec![vec![1, 4], vec![2, 5], vec![3, 6]]);
    }
}
