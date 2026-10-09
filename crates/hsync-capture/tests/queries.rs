//! Checks the storage and its queries against a run-length model of the same samples.

use hsync_capture::{CHUNK_SAMPLES, ChannelData, EdgeCount, LogicCapture, cell};
use proptest::prelude::*;

/// Samples described as runs of equal levels; `edges[k]` is the first sample of run `k + 1`.
struct Model {
    first: bool,
    edges: Vec<u64>,
    len: u64,
}

impl Model {
    fn from_runs(runs: &[(bool, u64)]) -> Self {
        let mut edges = Vec::new();
        let mut len = 0;
        let mut level = runs.first().is_some_and(|run| run.0);
        let first = level;
        for &(run_level, run_len) in runs {
            if len > 0 && run_level != level {
                edges.push(len);
            }
            level = run_level;
            len += run_len;
        }
        Self { first, edges, len }
    }

    fn sample(&self, index: u64) -> bool {
        let flips = self.edges.partition_point(|&edge| edge <= index);
        self.first ^ (flips % 2 == 1)
    }

    fn next_change(&self, index: u64) -> Option<u64> {
        let at = self.edges.partition_point(|&edge| edge <= index);
        self.edges.get(at).copied()
    }

    fn prev_change(&self, index: u64) -> Option<u64> {
        let at = self.edges.partition_point(|&edge| edge <= index);
        at.checked_sub(1).map(|k| self.edges[k] - 1)
    }

    fn bins(&self, start: u64, end: u64, bins: usize) -> Vec<u8> {
        let span = u128::from(end - start);
        let bound = |i: usize| start + (span * i as u128 / bins as u128) as u64;
        (0..bins)
            .map(|i| {
                let (lo, hi) = (bound(i), bound(i + 1).min(self.len));
                if lo >= hi {
                    return cell::EMPTY;
                }
                let transition = self.edges.iter().any(|&edge| lo < edge && edge < hi);
                (u8::from(self.sample(lo)) * cell::FIRST_HIGH)
                    | (u8::from(self.sample(hi - 1)) * cell::LAST_HIGH)
                    | (u8::from(transition) * cell::TRANSITION)
            })
            .collect()
    }

    fn count_edges(&self, start: u64, end: u64) -> EdgeCount {
        let mut count = EdgeCount::default();
        for &edge in &self.edges {
            if start < edge && edge < end.min(self.len) {
                if self.sample(edge) {
                    count.rising += 1;
                } else {
                    count.falling += 1;
                }
            }
        }
        count
    }

    /// Whole words of the samples, padded with the last level.
    fn words(&self) -> Vec<u64> {
        let count = self.len.div_ceil(64) as usize;
        let mut words = Vec::with_capacity(count);
        let mut edge = 0;
        let mut level = self.first;
        for word in 0..count as u64 {
            let (lo, hi) = (word * 64, word * 64 + 64);
            let mut value = 0u64;
            let mut at = lo;
            while at < hi {
                let until = self
                    .edges
                    .get(edge)
                    .copied()
                    .filter(|&e| e < hi)
                    .unwrap_or(hi);
                if level {
                    let width = until - at;
                    let ones = if width == 64 { !0 } else { (1u64 << width) - 1 };
                    value |= ones << (at - lo);
                }
                at = until;
                if until < hi {
                    level = !level;
                    edge += 1;
                }
            }
            words.push(value);
        }
        words
    }

    fn channel(&self, split: usize) -> ChannelData {
        let words = self.words();
        let mut channel = ChannelData::new();
        let split = split.min(words.len());
        channel.append_words(&words[..split]);
        channel.append_words(&words[split..]);
        channel.truncate(self.len);
        channel
    }
}

fn run_length() -> impl Strategy<Value = u64> {
    prop_oneof![
        8 => 1u64..64,
        4 => 64u64..5_000,
        2 => 5_000u64..300_000,
    ]
}

fn runs() -> impl Strategy<Value = Vec<(bool, u64)>> {
    prop::collection::vec((any::<bool>(), run_length()), 1..40)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn point_queries_match_the_model(
        runs in runs(),
        split in 0usize..20_000,
        probes in prop::collection::vec(any::<u64>(), 32),
    ) {
        let model = Model::from_runs(&runs);
        let channel = model.channel(split);
        prop_assert_eq!(channel.len(), model.len);
        for probe in probes {
            let index = probe % model.len;
            prop_assert_eq!(channel.sample(index), model.sample(index), "sample {}", index);
            prop_assert_eq!(channel.next_change(index), model.next_change(index), "next {}", index);
            prop_assert_eq!(channel.prev_change(index), model.prev_change(index), "prev {}", index);
        }
    }

    #[test]
    fn range_queries_match_the_model(
        runs in runs(),
        a in any::<u64>(),
        b in any::<u64>(),
        bins in 1usize..300,
    ) {
        let model = Model::from_runs(&runs);
        let channel = model.channel(usize::MAX);
        let (mut start, mut end) = (a % model.len, b % (model.len + 1));
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        prop_assert_eq!(channel.count_edges(start, end), model.count_edges(start, end));
        if end > start {
            let bins = bins.min((end - start) as usize);
            prop_assert_eq!(channel.bins(start, end, bins), model.bins(start, end, bins));
        }
    }
}

#[test]
fn edges_are_found_across_constant_chunks() {
    // Two and a half chunks of silence around a short burst: exercises every summary level
    // and folds the silent chunks into constant chunks.
    let silent = 2 * CHUNK_SAMPLES + CHUNK_SAMPLES / 2;
    let model = Model::from_runs(&[(true, silent), (false, 3), (true, 10), (false, silent)]);
    let channel = model.channel(usize::MAX);

    assert!(
        channel.heap_bytes() < 3 * 2 * 1024 * 1024,
        "silent chunks were not folded"
    );
    for index in [
        0,
        5,
        CHUNK_SAMPLES,
        silent - 1,
        silent,
        silent + 2,
        silent + 3,
        model.len - 1,
    ] {
        assert_eq!(channel.sample(index), model.sample(index), "sample {index}");
        assert_eq!(
            channel.next_change(index),
            model.next_change(index),
            "next {index}"
        );
        assert_eq!(
            channel.prev_change(index),
            model.prev_change(index),
            "prev {index}"
        );
    }
    assert_eq!(
        channel.count_edges(0, model.len),
        EdgeCount {
            rising: 1,
            falling: 2
        }
    );
    assert_eq!(
        channel.bins(0, model.len, 400),
        model.bins(0, model.len, 400)
    );
}

#[test]
fn pulses_report_capture_bounds() {
    let model = Model::from_runs(&[(false, 100), (true, 50), (false, 30)]);
    let channel = model.channel(usize::MAX);
    let pulse = channel.pulse(120).expect("in range");
    assert!(pulse.high);
    assert_eq!((pulse.start, pulse.end), (100, 150));
    assert!(pulse.start_is_edge && pulse.end_is_edge);
    let head = channel.pulse(10).expect("in range");
    assert_eq!((head.start, head.end, head.start_is_edge), (0, 100, false));
    let tail = channel.pulse(179).expect("in range");
    assert_eq!((tail.end, tail.end_is_edge), (180, false));
    assert_eq!(channel.pulse(180), None);
}

#[test]
fn cross_data_reaches_the_right_channels() {
    let mut capture = LogicCapture::new(1_000_000, [0, 3, 5]);
    let mut deinterleaver = capture.deinterleaver();
    let groups: Vec<u8> = (0..4u64)
        .flat_map(|group| [group, !group, 0xAA].into_iter().flat_map(u64::to_le_bytes))
        .collect();
    for piece in groups.chunks(20) {
        capture.append_cross(&mut deinterleaver, piece);
    }
    assert_eq!(capture.len(), 4 * 64);
    let d0 = capture.channel(0).expect("channel 0");
    let d3 = capture.channel(3).expect("channel 3");
    assert!(d0.sample(64)); // group 1 of channel 0 is the word 1
    assert!(!d3.sample(64));
    assert!(capture.channel(1).is_none());
}
