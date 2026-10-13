//! Finding events by what they carry.
//!
//! A query is words separated by blanks. A number (decimal, or hexadecimal with `0x`) finds
//! an event with a numeric field of that value; two or more numbers in a row find an event
//! with an array holding them one after the other, such as the bytes of a frame; any other
//! word finds an event whose kind, or whose enumerated field, has that name (the interface
//! translates the names it shows into the contracts' keys). An event matches when every
//! word does. Field names never match.

use std::fmt::Write as _;

/// A query, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Needle {
    /// Runs of numbers: a run of one is a value, a longer run a sequence inside an array.
    sequences: Vec<Vec<Number>>,
    /// Kind and enumerated-value names, lower case.
    names: Vec<String>,
}

/// A number as a query spells it or a payload holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Number {
    Integer(i128),
    Float(f64),
}

impl Number {
    fn parse(text: &str) -> Option<Self> {
        if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            return i128::from_str_radix(hex, 16).ok().map(Number::Integer);
        }
        if let Ok(integer) = text.parse::<i128>() {
            return Some(Number::Integer(integer));
        }
        text.parse::<f64>().ok().map(Number::Float)
    }

    fn same(self, other: Number) -> bool {
        match (self, other) {
            (Number::Integer(a), Number::Integer(b)) => a == b,
            (Number::Float(a), Number::Float(b)) => a == b,
            (Number::Integer(a), Number::Float(b)) | (Number::Float(b), Number::Integer(a)) => {
                a as f64 == b
            }
        }
    }
}

impl Needle {
    /// Parses `query`; `None` when it has no words.
    pub fn parse(query: &str) -> Option<Self> {
        let mut sequences: Vec<Vec<Number>> = Vec::new();
        let mut names = Vec::new();
        let mut run = Vec::new();
        for word in query.split_whitespace() {
            match Number::parse(word) {
                Some(number) => run.push(number),
                None => {
                    if !run.is_empty() {
                        sequences.push(std::mem::take(&mut run));
                    }
                    names.push(word.to_lowercase());
                }
            }
        }
        if !run.is_empty() {
            sequences.push(run);
        }
        (!sequences.is_empty() || !names.is_empty()).then_some(Self { sequences, names })
    }

    /// Whether the event whose payload is `json` (compact, as the store keeps it) matches.
    pub fn matches(&self, json: &[u8]) -> bool {
        let mut scan = Scan {
            needle: self,
            found_sequences: vec![false; self.sequences.len()],
            found_names: vec![false; self.names.len()],
            arrays: Vec::new(),
        };
        scan.run(json);
        scan.found_sequences.iter().all(|&found| found)
            && scan.found_names.iter().all(|&found| found)
    }

    /// The query as it would be written back.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for sequence in &self.sequences {
            for number in sequence {
                match number {
                    Number::Integer(value) => {
                        let _ = write!(out, "{value} ");
                    }
                    Number::Float(value) => {
                        let _ = write!(out, "{value} ");
                    }
                }
            }
        }
        for name in &self.names {
            out.push_str(name);
            out.push(' ');
        }
        out.trim_end().to_string()
    }
}

/// One pass over a payload's JSON text.
struct Scan<'a> {
    needle: &'a Needle,
    found_sequences: Vec<bool>,
    found_names: Vec<bool>,
    /// The numbers of the arrays being read, innermost last.
    arrays: Vec<Vec<Number>>,
}

impl Scan<'_> {
    fn run(&mut self, json: &[u8]) {
        let mut i = 0;
        while i < json.len() {
            match json[i] {
                b'"' => {
                    let start = i + 1;
                    let mut end = start;
                    while end < json.len() && json[end] != b'"' {
                        if json[end] == b'\\' {
                            end += 1;
                        }
                        end += 1;
                    }
                    let text = &json[start..end.min(json.len())];
                    i = end + 1;
                    let mut after = i;
                    while after < json.len() && json[after].is_ascii_whitespace() {
                        after += 1;
                    }
                    let is_key = json.get(after) == Some(&b':');
                    if !is_key {
                        self.string(text);
                    }
                }
                b'-' | b'0'..=b'9' => {
                    let start = i;
                    while i < json.len() && !matches!(json[i], b',' | b']' | b'}' | b' ') {
                        i += 1;
                    }
                    if let Some(number) = std::str::from_utf8(&json[start..i])
                        .ok()
                        .and_then(Number::parse)
                    {
                        self.number(number);
                    }
                }
                b'[' => {
                    self.arrays.push(Vec::new());
                    i += 1;
                }
                b']' => {
                    if let Some(numbers) = self.arrays.pop() {
                        self.array(&numbers);
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }

    fn string(&mut self, text: &[u8]) {
        for (name, found) in self.needle.names.iter().zip(&mut self.found_names) {
            if !*found && text.eq_ignore_ascii_case(name.as_bytes()) {
                *found = true;
            }
        }
    }

    fn number(&mut self, number: Number) {
        if let Some(array) = self.arrays.last_mut() {
            array.push(number);
        }
        for (sequence, found) in self.needle.sequences.iter().zip(&mut self.found_sequences) {
            if let [wanted] = sequence.as_slice()
                && wanted.same(number)
            {
                *found = true;
            }
        }
    }

    fn array(&mut self, numbers: &[Number]) {
        for (sequence, found) in self.needle.sequences.iter().zip(&mut self.found_sequences) {
            if !*found
                && sequence.len() > 1
                && numbers.windows(sequence.len()).any(|window| {
                    window
                        .iter()
                        .zip(sequence)
                        .all(|(held, wanted)| held.same(*wanted))
                })
            {
                *found = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn needle(query: &str) -> Needle {
        Needle::parse(query).expect("the query has words")
    }

    #[test]
    fn parses_numbers_runs_and_names() {
        assert_eq!(Needle::parse("   "), None);
        let parsed = needle("frame 0x41 65 -2 1.5 Ok");
        assert_eq!(
            parsed.sequences,
            vec![vec![
                Number::Integer(0x41),
                Number::Integer(65),
                Number::Integer(-2),
                Number::Float(1.5)
            ]]
        );
        assert_eq!(parsed.names, vec!["frame", "ok"]);
        assert_eq!(parsed.render(), "65 65 -2 1.5 frame ok");
        assert_eq!(
            needle("1 a 2").sequences,
            vec![vec![Number::Integer(1)], vec![Number::Integer(2)]]
        );
    }

    const FRAME: &[u8] = br#"{"kind":"frame","value":65,"parity":null,"framingOk":true}"#;
    const DATA: &[u8] =
        br#"{"kind":"data","tx":3,"bytes":[1,2,3,4],"nested":[[9,8],[7,6]],"ok":"err-crc"}"#;

    #[test]
    fn a_number_finds_a_numeric_field() {
        assert!(needle("65").matches(FRAME));
        assert!(needle("0x41").matches(FRAME));
        assert!(needle("65.0").matches(FRAME));
        assert!(!needle("66").matches(FRAME));
        assert!(needle("3").matches(DATA));
        assert!(needle("8").matches(DATA));
    }

    #[test]
    fn a_name_finds_a_kind_or_an_enumerated_value_but_not_a_field_name() {
        assert!(needle("frame").matches(FRAME));
        assert!(needle("Frame").matches(FRAME));
        assert!(needle("err-crc").matches(DATA));
        assert!(!needle("kind").matches(FRAME));
        assert!(!needle("value").matches(FRAME));
        assert!(!needle("framingOk").matches(FRAME));
        assert!(!needle("true").matches(FRAME));
    }

    #[test]
    fn numbers_in_a_row_find_them_one_after_the_other_in_an_array() {
        assert!(needle("2 3").matches(DATA));
        assert!(needle("1 2 3 4").matches(DATA));
        assert!(needle("0x9 8").matches(DATA));
        assert!(!needle("3 2").matches(DATA));
        assert!(!needle("1 3").matches(DATA));
        assert!(!needle("8 7").matches(DATA));
        assert!(!needle("1 2").matches(FRAME));
    }

    #[test]
    fn every_word_must_match() {
        assert!(needle("frame 65").matches(FRAME));
        assert!(!needle("frame 66").matches(FRAME));
        assert!(!needle("data 65").matches(FRAME));
        assert!(needle("data 2 3").matches(DATA));
        assert!(!needle("data 3 2").matches(DATA));
    }

    #[test]
    fn escaped_strings_are_read_whole() {
        let json = br#"{"kind":"text","text":"say \"hi\" : 65","value":7}"#;
        assert!(needle("7").matches(json));
        assert!(!needle("65").matches(json));
        assert!(!needle("hi").matches(json));
    }
}
