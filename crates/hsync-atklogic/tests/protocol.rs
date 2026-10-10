//! Command frames, capture settings, trigger arguments and the lane order against the bytes the
//! ATK-Logic application produces for the same cases (`tools/golden`,
//! `tests/fixtures/protocol.txt`).

use hsync_atklogic::protocol::{
    BLOCK, CaptureSettings, command, from_lanes, settings_args, to_lanes, trigger_args,
};
use hsync_core::TriggerCondition;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex"))
        .collect()
}

fn number(text: &str) -> u64 {
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).expect("hex number"),
        None => text.parse().expect("number"),
    }
}

fn condition(c: char) -> TriggerCondition {
    TriggerCondition::from_dsl_char(c as u8).expect("trigger character")
}

struct Case<'a> {
    kind: &'a str,
    values: Vec<(&'a str, &'a str)>,
    expected: &'a str,
}

impl Case<'_> {
    fn get(&self, key: &str) -> &str {
        self.values
            .iter()
            .find(|(name, _)| *name == key)
            .map_or("", |(_, value)| value)
    }
}

fn cases() -> Vec<Case<'static>> {
    include_str!("fixtures/protocol.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (input, expected) = line.split_once(" => ").expect("case output");
            let mut words = input.split_whitespace();
            let kind = words.next().expect("case kind");
            Case {
                kind,
                values: words
                    .map(|word| word.split_once('=').expect("key=value"))
                    .collect(),
                expected,
            }
        })
        .collect()
}

fn of_kind(kind: &str) -> Vec<Case<'static>> {
    let found: Vec<_> = cases()
        .into_iter()
        .filter(|case| case.kind == kind)
        .collect();
    assert!(!found.is_empty(), "no {kind} cases");
    found
}

#[test]
fn command_frames_match_atk_logic() {
    for case in of_kind("command") {
        let code = u8::try_from(number(case.get("code"))).expect("code");
        let sent = command(code, &unhex(case.get("args")));
        assert_eq!(sent.len(), BLOCK);
        let mut ordered = Vec::new();
        from_lanes(&sent.as_chunks::<BLOCK>().0[0], &mut ordered);
        let mut expected = unhex(case.expected);
        expected.resize(BLOCK, 0);
        assert_eq!(hex(&ordered), hex(&expected), "command {code:#04x}");
    }
}

#[test]
fn capture_settings_match_atk_logic() {
    for case in of_kind("settings") {
        let depth = number(case.get("depth"));
        let position = number(case.get("pos"));
        let settings = CaptureSettings {
            buffer: case.get("buffer") == "1",
            rle: case.get("rle") == "1",
            vth: case.get("vth").parse().expect("volts"),
            rate_code: u8::try_from(number(case.get("hz")) + 1).expect("rate"),
            depth,
            pre_trigger: depth / 100 * position,
        };
        assert_eq!(
            hex(&settings_args(&settings)),
            case.expected,
            "{settings:?}"
        );
    }
}

#[test]
fn trigger_arguments_match_atk_logic() {
    for case in of_kind("trigger") {
        let enabled = u16::try_from(number(case.get("en"))).expect("mask");
        let conditions: Vec<_> = case
            .get("trig")
            .chars()
            .enumerate()
            .map(|(channel, c)| (channel as u16, condition(c)))
            .collect();
        let args = trigger_args(enabled, &conditions, case.get("instant") == "1");
        assert_eq!(
            hex(&args),
            case.expected,
            "{enabled:#06x} {}",
            case.get("trig")
        );
    }
}

#[test]
fn lane_order_matches_atk_logic() {
    for case in of_kind("lanes") {
        let mut parts = case.expected.split(' ');
        let block = unhex(parts.next().expect("block"));
        let device = parts.next().expect("to device");
        let pc = parts.next().expect("to pc");
        assert_eq!(hex(&to_lanes(&block)), device);
        let mut ordered = Vec::new();
        from_lanes(&block.as_chunks::<BLOCK>().0[0], &mut ordered);
        assert_eq!(hex(&ordered), pc);
    }
}
