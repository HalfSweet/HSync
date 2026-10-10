//! `DSL_setting` bytes against the ones DSView produced for the same cases
//! (`tools/golden`, `tests/fixtures/settings.txt`).

use hsync_dslogic::profiles::{CHANNEL_MODES, UsbSpeed, VENDOR_ID, find_profile};
use hsync_dslogic::setting::{
    ArmConfig, TRIGGER_CHANNELS, TriggerMode, TriggerSetup, TriggerStage, setting, setting_ext32,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pattern(text: &str) -> [u8; TRIGGER_CHANNELS] {
    text.as_bytes().try_into().expect("32 trigger characters")
}

fn number(text: &str) -> u64 {
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).expect("hex number"),
        None => text.parse().expect("number"),
    }
}

struct Case {
    line: String,
    expected: String,
    expected_ext32: String,
}

fn cases() -> Vec<Case> {
    include_str!("fixtures/settings.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (input, output) = line.split_once(" => ").expect("case output");
            let (expected, expected_ext32) = output.split_once(' ').expect("two settings");
            Case {
                line: input.to_owned(),
                expected: expected.to_owned(),
                expected_ext32: expected_ext32.to_owned(),
            }
        })
        .collect()
}

#[test]
fn settings_match_dsview() {
    let cases = cases();
    assert!(cases.len() >= 30, "fixture has {} cases", cases.len());
    for case in cases {
        let mut pid = 0;
        let mut speed = UsbSpeed::High;
        let mut mode = 0;
        let (mut rate, mut limit, mut position, mut enabled) = (0, 0, 0, 0);
        let (mut filter, mut clock, mut edge, mut rle, mut instant) =
            (false, false, false, false, false);
        let mut simple = None;
        let mut trigger = TriggerSetup::default();
        for token in case.line.split_whitespace() {
            let (key, value) = token.split_once('=').expect("key=value");
            match key {
                "pid" => pid = number(value) as u16,
                "speed" => {
                    speed = if value == "S" {
                        UsbSpeed::Super
                    } else {
                        UsbSpeed::High
                    }
                }
                "chmode" => mode = number(value) as usize,
                "rate" => rate = number(value),
                "limit" => limit = number(value),
                "pos" => position = number(value) as u8,
                "en" => enabled = number(value) as u32,
                "filter" => filter = value == "1",
                "clk" => clock = value == "1",
                "edge" => edge = value == "1",
                "rle" => rle = value == "1",
                "instant" => instant = value == "1",
                "trig" => simple = Some(pattern(value)),
                "tmode" => {
                    trigger.mode = if value == "serial" {
                        TriggerMode::Serial
                    } else {
                        TriggerMode::Advanced
                    };
                }
                "stages" => trigger.last_stage = number(value) as u8,
                _ if key.starts_with('s') => {
                    let index: usize = key[1..].parse().expect("stage");
                    let fields: Vec<&str> = value.split('/').collect();
                    trigger.stages[index] = TriggerStage {
                        pattern0: pattern(fields[0]),
                        pattern1: pattern(fields[1]),
                        logic: number(fields[2]) as u8,
                        invert0: fields[3] == "1",
                        invert1: fields[4] == "1",
                        count: number(fields[5]) as u32,
                    };
                }
                _ => panic!("unknown key {key}"),
            }
        }
        if !instant {
            trigger.position = position;
            if trigger.mode != TriggerMode::Simple {
                trigger.enabled = true;
            } else if let Some(pattern) = simple {
                trigger.stages[16].pattern0 = pattern;
                trigger.enabled = pattern.iter().any(|&c| c != b'X');
            }
        }
        let profile = find_profile(VENDOR_ID, pid, speed).expect("profile");
        let config = ArmConfig {
            profile,
            mode: &CHANNEL_MODES[mode],
            samplerate: rate,
            limit_samples: limit,
            enabled,
            filter,
            external_clock: clock,
            clock_falling_edge: edge,
            rle,
            instant,
            looping: false,
            trigger: &trigger,
        };
        assert_eq!(
            hex(&setting(&config)),
            case.expected,
            "setting for {}",
            case.line
        );
        let ext32 = if profile.has(hsync_dslogic::profiles::caps::LA_CH32) {
            hex(&setting_ext32(&config))
        } else {
            "-".to_owned()
        };
        assert_eq!(
            ext32, case.expected_ext32,
            "ext32 setting for {}",
            case.line
        );
    }
}
