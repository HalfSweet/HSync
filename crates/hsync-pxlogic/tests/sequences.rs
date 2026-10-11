//! What opening a device and starting a capture do on the wire, against what PXView's own
//! pxlogic driver does for the same cases (`tools/golden`, `tests/fixtures/sequences.txt`).

use std::collections::HashMap;
use std::sync::Arc;

use hsync_core::TriggerCondition;
use hsync_driver::mock::{MockDevice, MockTransport, UsbCall};
use hsync_driver::usb::UsbError;
use hsync_driver::{CapturePlan, Device, DriverError, FirmwareDir, OptionKey, OptionValue};
use hsync_pxlogic::PxlogicDevice;
use hsync_pxlogic::profiles::{FIRMWARE_VERSION, PROFILES};
use hsync_pxlogic::protocol::{Control, EP_REGISTER_IN, EP_REGISTER_OUT, regs};

/// A device whose registers answer as `harness.c` makes them answer.
struct Registers {
    logic_mode: u32,
    firmware: u32,
    values: HashMap<u32, u32>,
    pending: [u32; 4],
}

impl MockDevice for Registers {
    fn control_out(&mut self, _: u8, _: u16, _: u16, _: &[u8]) -> Result<(), UsbError> {
        Ok(())
    }

    fn control_in(&mut self, _: u8, _: u16, _: u16, length: u16) -> Result<Vec<u8>, UsbError> {
        Ok(vec![0; usize::from(length)])
    }

    fn bulk_out(&mut self, endpoint: u8, data: &[u8]) -> Result<(), UsbError> {
        if endpoint == EP_REGISTER_OUT {
            let [command, _, address, value] = words(data);
            let answer = if command == 0xfefe_0000 {
                self.values.insert(address, value);
                0xfefe_fefe
            } else if address == regs::LOGIC_MODE {
                self.logic_mode
            } else if address == regs::FIRMWARE_VERSION {
                self.firmware
            } else {
                self.values.get(&address).copied().unwrap_or(0)
            };
            self.pending = [command, 8, address, answer];
        }
        Ok(())
    }

    fn bulk_in(&mut self, _: u8, _: usize) -> Option<Result<Vec<u8>, UsbError>> {
        None
    }

    fn bulk_read(&mut self, endpoint: u8, _: usize) -> Result<Vec<u8>, UsbError> {
        assert_eq!(endpoint, EP_REGISTER_IN);
        Ok(self
            .pending
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect())
    }
}

/// The four words of a register request.
fn words(data: &[u8]) -> [u32; 4] {
    let mut words = [0; 4];
    for (word, bytes) in words.iter_mut().zip(data.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*bytes);
    }
    words
}

fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

/// The recorded calls as the harness prints them.
fn operations(calls: &[UsbCall]) -> Vec<String> {
    calls
        .iter()
        .filter_map(|call| match call {
            UsbCall::BulkOut { endpoint, data } if *endpoint == EP_REGISTER_OUT => {
                Some(match words(data) {
                    [0xfefe_0000, _, address, value] => format!("W {address:04x} {value:08x}"),
                    [_, _, address, _] => format!("R {address:04x}"),
                })
            }
            UsbCall::BulkOut { endpoint, data } => Some(format!(
                "D {endpoint:02x} {} {:016x}",
                data.len(),
                fnv1a(data)
            )),
            UsbCall::BulkRead { .. } => None,
            UsbCall::ClearHalt { endpoint } => Some(format!("C {endpoint:02x}")),
            UsbCall::BulkInSubmit { endpoint, length } => {
                Some(format!("S {endpoint:02x} {length}"))
            }
            UsbCall::ControlIn {
                request, length, ..
            } => Some(format!("I {request:02x} {length}")),
            other => Some(format!("{other:?}")),
        })
        .collect()
}

/// Firmware files with the contents `harness.c` writes.
fn firmware_dir() -> FirmwareDir {
    let dir = std::env::temp_dir().join(format!(
        "hsync-pxlogic-sequences-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for (name, seed, length) in [
        ("SCI_LOGIC.bin", 1, 25804),
        ("hspi_ddr_RST.bin", 2, 12288),
        ("hspi_ddr.bin", 3, 12288),
    ] {
        let data: Vec<u8> = (0..length).map(|i: u32| (i * 31 + seed) as u8).collect();
        std::fs::write(dir.join(name), data).expect("firmware file");
    }
    FirmwareDir(dir)
}

struct Case<'a> {
    kind: &'a str,
    values: Vec<(&'a str, &'a str)>,
    expected: Vec<&'a str>,
}

impl Case<'_> {
    fn get(&self, key: &str) -> &str {
        self.values
            .iter()
            .find(|(name, _)| *name == key)
            .map_or("", |(_, value)| value)
    }

    fn number(&self, key: &str) -> u64 {
        let text = self.get(key);
        match text.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16).expect("hex number"),
            None => text.parse().expect("number"),
        }
    }
}

fn cases(kind: &str) -> Vec<Case<'static>> {
    let found: Vec<_> = include_str!("fixtures/sequences.txt")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (input, output) = line.split_once(" => ").expect("case output");
            let mut words = input.split_whitespace();
            Case {
                kind: words.next().expect("case kind"),
                values: words
                    .map(|word| word.split_once('=').expect("key=value"))
                    .collect(),
                expected: output.split(" | ").collect(),
            }
        })
        .filter(|case| case.kind == kind)
        .collect();
    assert!(!found.is_empty(), "no {kind} cases");
    found
}

fn transport(logic_mode: u32, firmware: u32) -> MockTransport<Registers> {
    MockTransport::new(Registers {
        logic_mode,
        firmware,
        values: HashMap::new(),
        pending: [0; 4],
    })
}

#[test]
fn opening_matches_pxview() {
    let firmware = firmware_dir();
    for case in cases("open") {
        let profile = &PROFILES[case.number("profile") as usize];
        let usb = transport(
            case.number("lm") as u32,
            u32::from_str_radix(case.get("fw"), 16).expect("version"),
        );
        let opened = PxlogicDevice::open(
            Control::new(Arc::new(usb.clone())),
            profile,
            &firmware,
            "test".into(),
        );
        let (outcome, steps) = case.expected.split_last().expect("an outcome");
        assert_eq!(operations(&usb.calls()), steps, "{}", case.get("profile"));
        let mut outcome = outcome.split_whitespace();
        let ret = outcome.nth(1).expect("ret");
        let logic_mode: u32 = outcome.nth(1).expect("lm").parse().expect("lm");
        match opened {
            Ok(device) => {
                assert_eq!(ret, "0");
                let opened = PROFILES
                    .iter()
                    .find(|profile| profile.model == device.info().model)
                    .expect("a profile");
                assert_eq!(opened.logic_mode, logic_mode);
            }
            Err(DriverError::Restarting) => assert_eq!(ret, "-7", "SR_ERR_DEV_CLOSED"),
            Err(error) => panic!("opening failed: {error}"),
        }
    }
}

fn condition(c: char) -> TriggerCondition {
    TriggerCondition::from_dsl_char(c as u8).expect("trigger character")
}

#[test]
fn starting_captures_matches_pxview() {
    let firmware = firmware_dir();
    for case in cases("start") {
        let profile = &PROFILES[case.number("profile") as usize];
        let usb = transport(profile.logic_mode, FIRMWARE_VERSION);
        let mut device = PxlogicDevice::open(
            Control::new(Arc::new(usb.clone())),
            profile,
            &firmware,
            "test".into(),
        )
        .expect("open");
        usb.clear_calls();

        let mut set = |key, value| device.set_option(key, &value).expect("option");
        set(
            OptionKey::OperationMode,
            OptionValue::Choice(case.get("mode").to_owned()),
        );
        set(
            OptionKey::ChannelMode,
            OptionValue::Choice(case.get("chmode").to_owned()),
        );
        set(
            OptionKey::Samplerate,
            OptionValue::Integer(case.number("rate")),
        );
        set(
            OptionKey::SampleLimit,
            OptionValue::Integer(case.number("limit")),
        );
        set(
            OptionKey::Vth,
            OptionValue::Number(case.get("vth").parse().expect("volts")),
        );
        set(
            OptionKey::Filter,
            OptionValue::Bool(case.get("filter") == "1"),
        );
        set(
            OptionKey::ClockFallingEdge,
            OptionValue::Bool(case.get("edge") == "1"),
        );
        let external = ["off", "rise", "high", "fall", "low", "both"][case.number("ext") as usize];
        set(
            OptionKey::ExternalTrigger,
            OptionValue::Choice(external.to_owned()),
        );
        let enabled = case.number("en");
        for channel in 0..device.channel_count() {
            if enabled & (1 << channel) == 0 {
                device.set_channel_enabled(channel, false).expect("disable");
            }
        }
        let plan = CapturePlan {
            triggers: case
                .get("trig")
                .chars()
                .enumerate()
                .map(|(channel, c)| (channel as u16, condition(c)))
                .collect(),
            trigger_position: case.number("pos") as u8,
            instant: false,
            advanced: None,
            looping: case.get("loop") == "1",
        };
        let (sink, _packets) = crossbeam_channel::unbounded();
        let acquisition = device.start(plan, sink).expect("start");
        acquisition.stop();
        acquisition.wait();

        // The capture goes on to poll the trigger; PXView's setup ends with the transfers.
        let mut got = operations(&usb.calls());
        let last = got
            .iter()
            .rposition(|step| step.starts_with("S "))
            .expect("transfers");
        got.truncate(last + 1);
        let (got, transfers) = split_transfers(&got);
        let (expected, blocks) = split_transfers(&case.expected);
        assert_eq!(got, expected, "{:?}", case.values);
        // PXView reads whole blocks; HSync reads them in transfers of at most 1 MiB, whole
        // packets, keeping at least as many bytes in flight.
        let (asked, in_blocks) = (transfers.iter().sum::<usize>(), blocks.iter().sum());
        assert!(
            transfers
                .iter()
                .all(|&size| size <= MAX_TRANSFER && size % 1024 == 0),
            "{transfers:?}"
        );
        assert!(
            (in_blocks..in_blocks + MAX_TRANSFER).contains(&asked),
            "{asked} for {blocks:?}"
        );
    }
}

/// Longest transfer HSync asks for.
const MAX_TRANSFER: usize = 1 << 20;

/// The steps but the transfers on the data endpoint, and the sizes of those.
fn split_transfers(steps: &[impl AsRef<str>]) -> (Vec<String>, Vec<usize>) {
    let mut rest = Vec::new();
    let mut transfers = Vec::new();
    for step in steps {
        match step.as_ref().strip_prefix("S 82 ") {
            Some(size) => transfers.push(size.parse().expect("transfer size")),
            None => rest.push(step.as_ref().to_owned()),
        }
    }
    (rest, transfers)
}
