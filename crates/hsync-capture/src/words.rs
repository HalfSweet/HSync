//! Chunked word storage for one channel.

/// Words per chunk: `2^18` words, i.e. `2^24` samples or 2 MiB, the block size of `.dsl` files.
pub const CHUNK_WORDS: usize = 1 << 18;

/// Samples per chunk.
pub const CHUNK_SAMPLES: u64 = (CHUNK_WORDS as u64) * 64;

#[derive(Debug, Clone)]
enum Chunk {
    /// Every sample of the chunk has this level; no word data is kept.
    Constant(bool),
    Words(Vec<u64>),
}

/// Append-only storage of 64-sample words. Full chunks whose samples all have one level are
/// folded into [`Chunk::Constant`] so idle channels cost next to no memory.
#[derive(Debug, Default, Clone)]
pub(crate) struct ChunkedWords {
    chunks: Vec<Chunk>,
    len: usize,
}

impl ChunkedWords {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn get(&self, index: usize) -> u64 {
        debug_assert!(index < self.len, "word {index} out of {}", self.len);
        match &self.chunks[index / CHUNK_WORDS] {
            Chunk::Constant(true) => !0,
            Chunk::Constant(false) => 0,
            Chunk::Words(words) => words[index % CHUNK_WORDS],
        }
    }

    pub(crate) fn extend(&mut self, mut words: &[u64]) {
        while !words.is_empty() {
            if self.len.is_multiple_of(CHUNK_WORDS) {
                self.chunks
                    .push(Chunk::Words(Vec::with_capacity(CHUNK_WORDS)));
            }
            let Some(Chunk::Words(chunk)) = self.chunks.last_mut() else {
                unreachable!("the last chunk is never folded while it is being filled");
            };
            let take = words.len().min(CHUNK_WORDS - chunk.len());
            chunk.extend_from_slice(&words[..take]);
            self.len += take;
            words = &words[take..];
        }
    }

    /// Folds chunk `index` into a constant chunk.
    pub(crate) fn fold(&mut self, index: usize, level: bool) {
        debug_assert!(
            (index + 1) * CHUNK_WORDS <= self.len,
            "only full chunks fold"
        );
        self.chunks[index] = Chunk::Constant(level);
    }

    /// The words of chunk `index`, or the level of a constant chunk.
    pub(crate) fn chunk(&self, index: usize) -> ChunkView<'_> {
        match &self.chunks[index] {
            Chunk::Constant(level) => ChunkView::Constant(*level),
            Chunk::Words(words) => ChunkView::Words(words),
        }
    }

    pub(crate) fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Drops the first `count` chunks, which must be full; word `i` becomes word
    /// `i - count · CHUNK_WORDS`.
    pub(crate) fn drop_front_chunks(&mut self, count: usize) {
        debug_assert!(
            count * CHUNK_WORDS <= self.len,
            "only full chunks are dropped"
        );
        self.chunks.drain(..count.min(self.chunks.len()));
        self.len = self.len.saturating_sub(count * CHUNK_WORDS);
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        let chunks = len.div_ceil(CHUNK_WORDS);
        self.chunks.truncate(chunks);
        if let Some(Chunk::Words(words)) = self.chunks.last_mut() {
            words.truncate(len - (chunks - 1) * CHUNK_WORDS);
        }
        self.len = len;
    }

    /// Bytes of heap memory held by word data.
    pub(crate) fn heap_bytes(&self) -> usize {
        self.chunks
            .iter()
            .map(|chunk| match chunk {
                Chunk::Constant(_) => 0,
                Chunk::Words(words) => words.capacity() * 8,
            })
            .sum()
    }
}

/// Borrowed contents of one chunk.
#[derive(Debug, Clone, Copy)]
pub enum ChunkView<'a> {
    /// All samples of the chunk have this level.
    Constant(bool),
    /// The chunk's words; sample `j` of the chunk is bit `j % 64` of word `j / 64`.
    Words(&'a [u64]),
}
