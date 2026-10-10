//! Finding values and text in decoded data, for the search box (DESIGN.md §9.8).

use serde::Serialize;

use crate::present::{DataLine, StreamMode, StreamRow};
use crate::spec::Decoded;

/// What a search looks for: the words typed in hex, and the text typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needle {
    /// Word sequences to find; text ignores the case of ASCII letters.
    sequences: Vec<(Vec<u32>, bool)>,
    /// The query as one hex number, also matched against CAN identifiers.
    number: Option<u32>,
}

impl Needle {
    /// Reads the search box: `4F`, `0x4F` or `4F 32` are words in hex (`CAFE` both the word
    /// 0xCAFE and the bytes CA FE); any query is also text. `None` for a blank query.
    pub fn parse(query: &str) -> Option<Self> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        let mut sequences = Vec::new();
        let mut number = None;
        let tokens: Option<Vec<&str>> = query
            .split_whitespace()
            .map(|token| {
                let digits = token
                    .strip_prefix("0x")
                    .or_else(|| token.strip_prefix("0X"))
                    .unwrap_or(token);
                (!digits.is_empty()
                    && digits.len() <= 8
                    && digits.bytes().all(|b| b.is_ascii_hexdigit()))
                .then_some(digits)
            })
            .collect();
        if let Some(tokens) = tokens {
            let values: Vec<u32> = tokens
                .iter()
                .filter_map(|digits| u32::from_str_radix(digits, 16).ok())
                .collect();
            if let [single] = tokens.as_slice() {
                number = values.first().copied();
                if single.len() > 2 && single.len() % 2 == 0 {
                    let bytes = (0..single.len())
                        .step_by(2)
                        .filter_map(|at| u32::from_str_radix(&single[at..at + 2], 16).ok())
                        .collect();
                    sequences.push((bytes, false));
                }
            }
            sequences.push((values, false));
        }
        let text: Vec<u32> = query
            .bytes()
            .map(|b| u32::from(b.to_ascii_lowercase()))
            .collect();
        if !sequences.iter().any(|(words, _)| *words == text) {
            sequences.push((text, true));
        }
        Some(Self { sequences, number })
    }
}

/// Words that follow each other on one line, as a search sees them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sequence {
    pub line: Option<DataLine>,
    /// Each word's value and span.
    pub words: Vec<(u32, u64, u64)>,
}

/// What a presentation can be searched for.
#[derive(Debug, Default)]
pub(crate) struct Searchable {
    sequences: Vec<Sequence>,
    /// CAN identifiers with the spans of their frames.
    ids: Vec<(u32, u64, u64)>,
    /// For each listing (text, hex): the rows holding words of each line, in time order.
    rows: [Vec<(Option<DataLine>, Vec<u32>)>; 2],
}

/// Where a search found something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    pub start: u64,
    pub end: u64,
    /// The data stream row listing it.
    pub row: Option<u32>,
}

impl Searchable {
    pub(crate) fn new(decoded: &Decoded, text_rows: &[StreamRow], hex_rows: &[StreamRow]) -> Self {
        let mut sequences = Vec::new();
        let mut ids = Vec::new();
        match decoded {
            Decoded::I2c { trace, shifted } => {
                // One sequence per transfer: addresses as shown, then data.
                let mut words = Vec::new();
                for item in &trace.items {
                    use crate::i2c::Kind;
                    match item.kind {
                        Kind::Start if !words.is_empty() => sequences.push(Sequence {
                            line: None,
                            words: std::mem::take(&mut words),
                        }),
                        Kind::AddressRead | Kind::AddressWrite => {
                            let shown = if *shifted { item.address() } else { item.byte };
                            words.push((u32::from(shown), item.start, item.end));
                        }
                        Kind::DataRead | Kind::DataWrite => {
                            words.push((u32::from(item.byte), item.start, item.end));
                        }
                        _ => {}
                    }
                }
                if !words.is_empty() {
                    sequences.push(Sequence { line: None, words });
                }
            }
            Decoded::Uart(trace) => sequences.push(Sequence {
                line: None,
                words: trace
                    .frames
                    .iter()
                    .map(|frame| (u32::from(frame.value), frame.start, frame.end))
                    .collect(),
            }),
            Decoded::Spi(trace) => {
                for transfer in &trace.transfers {
                    let words = &trace.words[transfer.words.clone()];
                    for line in [DataLine::Miso, DataLine::Mosi] {
                        let values: Option<Vec<(u32, u64, u64)>> = words
                            .iter()
                            .map(|word| {
                                let value = match line {
                                    DataLine::Miso => word.miso,
                                    DataLine::Mosi => word.mosi,
                                };
                                value.map(|value| (value, word.start, word.end))
                            })
                            .collect();
                        if let Some(values) = values.filter(|values| !values.is_empty()) {
                            sequences.push(Sequence {
                                line: Some(line),
                                words: values,
                            });
                        }
                    }
                }
            }
            Decoded::Can(trace) => {
                for frame in &trace.frames {
                    ids.push((frame.id, frame.start, frame.end));
                    // Payload bytes share the frame's span: the decoder does not time them.
                    sequences.push(Sequence {
                        line: None,
                        words: frame
                            .data
                            .iter()
                            .map(|&byte| (u32::from(byte), frame.start, frame.end))
                            .collect(),
                    });
                }
            }
        }
        Self {
            sequences,
            ids,
            rows: [index_rows(text_rows), index_rows(hex_rows)],
        }
    }

    pub(crate) fn sequences(&self) -> &[Sequence] {
        &self.sequences
    }

    pub(crate) fn ids(&self) -> &[(u32, u64, u64)] {
        &self.ids
    }

    /// Everything `needle` finds, in time order, with its row in the `mode` listing `rows`.
    pub(crate) fn find(&self, needle: &Needle, mode: StreamMode, rows: &[StreamRow]) -> Vec<Found> {
        let mut found: Vec<(u64, u64, Option<DataLine>)> = Vec::new();
        for sequence in &self.sequences {
            for (pattern, fold) in &needle.sequences {
                find_in(&sequence.words, pattern, *fold, |start, end| {
                    found.push((start, end, sequence.line));
                });
            }
        }
        if let Some(number) = needle.number {
            found.extend(
                self.ids
                    .iter()
                    .filter(|&&(id, _, _)| id == number)
                    .map(|&(_, start, end)| (start, end, None)),
            );
        }
        found.sort_unstable_by_key(|&(start, end, line)| (start, line, std::cmp::Reverse(end)));
        found.dedup_by_key(|&mut (start, _, line)| (start, line));
        let index = &self.rows[match mode {
            StreamMode::Text => 0,
            StreamMode::Hex => 1,
        }];
        found
            .into_iter()
            .map(|(start, end, line)| Found {
                start,
                end,
                row: row_of(index, rows, line, start),
            })
            .collect()
    }
}

/// Calls `hit` with the span of each match of `pattern` in `words`, not overlapping.
fn find_in(words: &[(u32, u64, u64)], pattern: &[u32], fold: bool, mut hit: impl FnMut(u64, u64)) {
    if pattern.is_empty() || words.len() < pattern.len() {
        return;
    }
    let same = |word: u32, wanted: u32| {
        if fold && (u32::from(b'A')..=u32::from(b'Z')).contains(&word) {
            word + 32 == wanted
        } else {
            word == wanted
        }
    };
    let mut at = 0;
    while at + pattern.len() <= words.len() {
        if pattern
            .iter()
            .zip(&words[at..])
            .all(|(&wanted, &(word, _, _))| same(word, wanted))
        {
            hit(words[at].1, words[at + pattern.len() - 1].2);
            at += pattern.len();
        } else {
            at += 1;
        }
    }
}

/// The rows of a listing that hold words, by line, in time order.
fn index_rows(rows: &[StreamRow]) -> Vec<(Option<DataLine>, Vec<u32>)> {
    let mut index: Vec<(Option<DataLine>, Vec<u32>)> = Vec::new();
    for (position, row) in rows.iter().enumerate() {
        let line = match row {
            StreamRow::I2c { .. } | StreamRow::UartText { .. } | StreamRow::Can { .. } => None,
            StreamRow::Hex { line, .. } | StreamRow::Run { line, .. } => *line,
            StreamRow::SpiText { line, .. } => Some(*line),
            StreamRow::Note { .. } | StreamRow::Gap { .. } => continue,
        };
        match index.iter_mut().find(|(other, _)| *other == line) {
            Some((_, positions)) => positions.push(position as u32),
            None => index.push((line, vec![position as u32])),
        }
    }
    index
}

/// The row of `line` whose span holds `sample`.
fn row_of(
    index: &[(Option<DataLine>, Vec<u32>)],
    rows: &[StreamRow],
    line: Option<DataLine>,
    sample: u64,
) -> Option<u32> {
    let (_, positions) = index.iter().find(|(other, _)| *other == line)?;
    let at = positions.partition_point(|&position| rows[position as usize].end() <= sample);
    let position = *positions.get(at)?;
    (rows[position as usize].start() <= sample).then_some(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hex_and_text() {
        let needle = Needle::parse(" 0x4F ").expect("query");
        assert_eq!(needle.number, Some(0x4F));
        assert_eq!(
            needle.sequences,
            vec![
                (vec![0x4F], false),
                (b"0x4f".iter().map(|&b| u32::from(b)).collect(), true)
            ]
        );
        let needle = Needle::parse("CAFE").expect("query");
        assert_eq!(needle.number, Some(0xCAFE));
        assert_eq!(needle.sequences[0], (vec![0xCA, 0xFE], false));
        assert_eq!(needle.sequences[1], (vec![0xCAFE], false));
        let needle = Needle::parse("HOST").expect("query");
        assert_eq!(needle.number, None);
        assert_eq!(
            needle.sequences,
            vec![(b"host".iter().map(|&b| u32::from(b)).collect(), true)]
        );
        assert_eq!(Needle::parse("   "), None);
    }

    #[test]
    fn finds_without_overlapping_and_ignores_case_in_text() {
        let words: Vec<(u32, u64, u64)> = b"aAAaB"
            .iter()
            .enumerate()
            .map(|(i, &b)| (u32::from(b), i as u64 * 10, i as u64 * 10 + 10))
            .collect();
        let mut hits = Vec::new();
        find_in(&words, &[u32::from(b'a'), u32::from(b'a')], true, |s, e| {
            hits.push((s, e))
        });
        assert_eq!(hits, vec![(0, 20), (20, 40)]);
        hits.clear();
        find_in(
            &words,
            &[u32::from(b'a'), u32::from(b'a')],
            false,
            |s, e| hits.push((s, e)),
        );
        assert_eq!(hits, Vec::new());
    }
}
