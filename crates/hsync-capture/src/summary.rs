//! Hierarchical summaries over the words of one channel.
//!
//! Level 0 has one bit pair per 64-sample word, every further level one bit pair per 64 units
//! of the level below, so level `k` summarises `64^(k+1)` samples per unit. A unit is
//! *uniform* when every sample in it has the same level; `mixed` marks the others and `high`
//! holds the level of uniform units (for mixed units it is unspecified). This is the idea of
//! DSView's `LogicSnapshot` mipmaps, reduced to the two facts queries need.

/// Number of summary levels; level 3 units span `2^24` samples, one storage chunk.
pub(crate) const LEVELS: usize = 4;

const _: () = assert!(
    1u64 << (6 * LEVELS) == crate::words::CHUNK_SAMPLES,
    "a top-level unit spans one chunk"
);

#[derive(Debug, Default, Clone)]
struct Level {
    mixed: Vec<u64>,
    high: Vec<u64>,
    /// Number of valid units on this level.
    units: usize,
}

impl Level {
    fn ensure_units(&mut self, units: usize) {
        let blocks = units.div_ceil(64);
        if self.mixed.len() < blocks {
            self.mixed.resize(blocks, 0);
            self.high.resize(blocks, 0);
        }
        self.units = units;
    }

    /// Mask of the valid units in `block`.
    fn valid_mask(&self, block: usize) -> u64 {
        let first = block * 64;
        if first >= self.units {
            0
        } else if self.units - first >= 64 {
            !0
        } else {
            (1u64 << (self.units - first)) - 1
        }
    }

    fn set(&mut self, unit: usize, mixed: bool, high: bool) {
        let (block, bit) = (unit / 64, 1u64 << (unit % 64));
        if mixed {
            self.mixed[block] |= bit;
        } else {
            self.mixed[block] &= !bit;
        }
        if high {
            self.high[block] |= bit;
        } else {
            self.high[block] &= !bit;
        }
    }

    /// Units in `block` that are not uniformly `level`.
    fn not_uniform(&self, block: usize, level: bool) -> u64 {
        let high = if level {
            !self.high[block]
        } else {
            self.high[block]
        };
        (self.mixed[block] | high) & self.valid_mask(block)
    }

    /// Summarises `block` as one unit of the level above.
    fn fold(&self, block: usize) -> (bool, bool) {
        let valid = self.valid_mask(block);
        let mixed = self.mixed[block] & valid != 0;
        let high = self.high[block] & valid;
        let uniform_high = high == valid;
        (
            mixed || (high != 0 && !uniform_high),
            uniform_high && valid != 0,
        )
    }
}

/// Summary of one channel. Words are only ever appended, and the last word may be partial.
#[derive(Debug, Default, Clone)]
pub(crate) struct Summary {
    levels: [Level; LEVELS],
}

impl Summary {
    /// Records words `first..first + len` from `word(i)` with `valid_bits` valid samples in the
    /// last word, then refreshes the upper levels they belong to.
    pub(crate) fn update(
        &mut self,
        first: usize,
        len: usize,
        valid_bits_in_last: u32,
        word: impl Fn(usize) -> u64,
    ) {
        if len == 0 {
            return;
        }
        let end = first + len;
        let level0 = &mut self.levels[0];
        level0.ensure_units(end.max(level0.units));
        for unit in first..end {
            let bits = if unit + 1 == end {
                valid_bits_in_last
            } else {
                64
            };
            let mask = if bits >= 64 { !0 } else { (1u64 << bits) - 1 };
            let value = word(unit) & mask;
            let mixed = value != 0 && value != mask;
            level0.set(unit, mixed, value == mask);
        }
        self.refresh(first, end);
    }

    fn refresh(&mut self, first: usize, end: usize) {
        let (mut lo, mut hi) = (first, end);
        for level in 1..LEVELS {
            let units = self.levels[level - 1].units.div_ceil(64);
            let (below, above) = self.levels.split_at_mut(level);
            let (below, current) = (&below[level - 1], &mut above[0]);
            current.ensure_units(units);
            let (block_lo, block_hi) = (lo / 64, (hi - 1) / 64 + 1);
            for block in block_lo..block_hi {
                let (mixed, high) = below.fold(block);
                current.set(block, mixed, high);
            }
            (lo, hi) = (block_lo, block_hi);
        }
    }

    /// Forgets the first `count` chunks' worth of words, which must be full: word `i` becomes
    /// word `i - count · CHUNK_WORDS`.
    pub(crate) fn drop_front_chunks(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        // Below the top, a chunk covers whole blocks of units: drop them.
        for depth in 0..LEVELS - 1 {
            let units = 1usize << (6 * (LEVELS - 1 - depth));
            let level = &mut self.levels[depth];
            let blocks = (units / 64 * count).min(level.mixed.len());
            level.mixed.drain(..blocks);
            level.high.drain(..blocks);
            level.units = level.units.saturating_sub(units * count);
        }
        // The top level has one unit per chunk: build it again from the level below.
        let units = self.levels[LEVELS - 2].units.div_ceil(64);
        let top = &mut self.levels[LEVELS - 1];
        top.mixed.clear();
        top.high.clear();
        top.ensure_units(units);
        for block in 0..units {
            let (mixed, high) = self.levels[LEVELS - 2].fold(block);
            self.levels[LEVELS - 1].set(block, mixed, high);
        }
    }

    /// Whether the chunk-sized unit `unit` on the top level is uniform, and its level.
    pub(crate) fn top_unit(&self, unit: usize) -> Option<bool> {
        let top = &self.levels[LEVELS - 1];
        if unit >= top.units {
            return None;
        }
        let (block, bit) = (unit / 64, 1u64 << (unit % 64));
        (top.mixed[block] & bit == 0).then_some(top.high[block] & bit != 0)
    }

    /// The first word at or after `start` that is not uniformly `level`.
    pub(crate) fn next_word_not(&self, start: usize, level: bool) -> Option<usize> {
        let mut depth = 0;
        let mut pos = start;
        loop {
            let current = &self.levels[depth];
            if pos >= current.units {
                return None;
            }
            let block = pos / 64;
            let candidates = current.not_uniform(block, level) & (!0u64 << (pos % 64));
            if candidates != 0 {
                return Some(self.descend(
                    depth,
                    block * 64 + candidates.trailing_zeros() as usize,
                    level,
                ));
            }
            if depth + 1 < LEVELS {
                depth += 1;
                pos = block + 1;
            } else {
                pos = (block + 1) * 64;
            }
        }
    }

    /// The last word at or before `end` that is not uniformly `level`.
    pub(crate) fn prev_word_not(&self, end: usize, level: bool) -> Option<usize> {
        let mut depth = 0;
        let mut pos = end.min(self.levels[0].units.checked_sub(1)?);
        loop {
            let current = &self.levels[depth];
            let block = pos / 64;
            let keep = if pos % 64 == 63 {
                !0
            } else {
                (1u64 << (pos % 64 + 1)) - 1
            };
            let candidates = current.not_uniform(block, level) & keep;
            if candidates != 0 {
                let unit = block * 64 + 63 - candidates.leading_zeros() as usize;
                return Some(self.descend_back(depth, unit, level));
            }
            if block == 0 {
                return None;
            }
            if depth + 1 < LEVELS {
                depth += 1;
                pos = block - 1;
            } else {
                pos = block * 64 - 1;
            }
        }
    }

    fn descend(&self, mut depth: usize, mut unit: usize, level: bool) -> usize {
        while depth > 0 {
            depth -= 1;
            let candidates = self.levels[depth].not_uniform(unit, level);
            debug_assert_ne!(candidates, 0, "summary levels disagree");
            unit = unit * 64 + candidates.trailing_zeros() as usize;
        }
        unit
    }

    fn descend_back(&self, mut depth: usize, mut unit: usize, level: bool) -> usize {
        while depth > 0 {
            depth -= 1;
            let candidates = self.levels[depth].not_uniform(unit, level);
            debug_assert_ne!(candidates, 0, "summary levels disagree");
            unit = unit * 64 + 63 - candidates.leading_zeros() as usize;
        }
        unit
    }
}
