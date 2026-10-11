//! The PXLogic driver against a simulated device: opening, options and captures end to end.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_capture::LogicCapture;
use hsync_core::TriggerCondition;
use hsync_driver::mock::{MockDevice, MockTransport, UsbCall};
use hsync_driver::usb::UsbError;
use hsync_driver::{
    CapturePlan, Device, DriverError, EndReason, FirmwareDir, OptionKey, OptionKind, OptionValue,
    Packet,
};
use hsync_pxlogic::PxlogicDevice;
use hsync_pxlogic::profiles::{FIRMWARE_VERSION, PROFILES, UsbSpeed};
use hsync_pxlogic::protocol::{
    CMD_CTL_RD, CaptureState, Control, EP_DATA, EP_REGISTER_IN, EP_REGISTER_OUT, EP_UPLOAD,
    block_size, regs, upload,
};
use nusb::transfer::TransferError;

/// Profiles by where the scan finds them: the current ids over USB 3.0 and 2.0.
const SCANNED_SUPER: usize = 2;
const SCANNED_HIGH: usize = 3;

/// Sample `index` of `channel` as the simulated device sees it: a square wave whose period
/// grows with the channel.
fn level(channel: u16, index: u64) -> bool {
    (index / (u64::from(channel) + 3)) % 2 == 1
}

/// Samples `from..to`, whole groups of 64, of `channels` as the device sends them: per group,
/// one little-endian `u64` per channel.
fn cross(channels: &[u16], from: u64, to: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    for group in (from..to).step_by(64) {
        for &channel in channels {
            let word = (0..64).fold(0u64, |word, bit| {
                word | (u64::from(level(channel, group + bit)) << bit)
            });
            bytes.extend(word.to_le_bytes());
        }
    }
    bytes
}

/// What the device was told before the capture started.
#[derive(Debug, Clone, Copy)]
struct Received {
    control: u32,
    enabled: u32,
    /// Bytes the device sends.
    bytes: u64,
    block: u32,
    position: u32,
    rise: u32,
    fall: u32,
    one: u32,
    zero: u32,
    external: u32,
}

impl Received {
    fn read(registers: &HashMap<u32, u32>) -> Self {
        let get = |address| registers.get(&address).copied().unwrap_or(0);
        Self {
            control: get(regs::CONTROL),
            enabled: get(regs::CHANNELS),
            bytes: u64::from(get(regs::LIMIT_LOW)) | (u64::from(get(regs::LIMIT_HIGH)) << 32),
            block: get(regs::BLOCK),
            position: get(regs::TRIGGER_POSITION),
            rise: get(regs::TRIGGER_RISE),
            fall: get(regs::TRIGGER_FALL),
            one: get(regs::TRIGGER_ONE),
            zero: get(regs::TRIGGER_ZERO),
            external: get(regs::EXTERNAL_TRIGGER),
        }
    }

    fn stream(&self) -> bool {
        self.control & 2 != 0
    }

    fn channels(&self) -> Vec<u16> {
        (0..32)
            .filter(|channel| self.enabled & (1 << channel) != 0)
            .collect()
    }

    /// The samples per channel the device sends: the capture and a block more.
    fn samples(&self) -> u64 {
        self.bytes * 8 / self.channels().len() as u64
    }
}

/// Data sent to the upload endpoint, with the registers that announced it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Upload {
    target: u32,
    start: u32,
    end: u32,
    data: Vec<u8>,
}

type Script = Box<dyn FnMut(Received) -> Vec<u8> + Send>;

/// A PXLogic as its MCU and FPGA answer: registers on two bulk endpoints, the capture state
/// behind a vendor request, uploads and samples on two more.
struct FakePx {
    /// The logic mode register; none fails reading it.
    logic_mode: Option<u32>,
    firmware: u32,
    registers: HashMap<u32, u32>,
    /// The answer waiting on the register endpoint.
    answer: Vec<u8>,
    uploads: Vec<Upload>,
    restarted: bool,
    /// The capture states, one per request; the last one stays.
    states: VecDeque<CaptureState>,
    polls: usize,
    /// Samples are held back until the state has been read this often.
    hold_polls: usize,
    /// What the device sends once a capture starts; by default the samples it was asked for.
    script: Script,
    capture: Option<Received>,
    outgoing: VecDeque<u8>,
    /// Samples keep coming until the capture is stopped.
    endless: bool,
    streamed: u64,
    /// The device is gone once this many transfers have completed.
    unplug_after: Option<usize>,
    completed: usize,
}

impl Default for FakePx {
    fn default() -> Self {
        Self {
            logic_mode: Some(0),
            firmware: FIRMWARE_VERSION,
            registers: HashMap::new(),
            answer: Vec::new(),
            uploads: Vec::new(),
            restarted: false,
            states: VecDeque::new(),
            polls: 0,
            hold_polls: 0,
            script: Box::new(|received: Received| {
                cross(&received.channels(), 0, received.samples())
            }),
            capture: None,
            outgoing: VecDeque::new(),
            endless: false,
            streamed: 0,
            unplug_after: None,
            completed: 0,
        }
    }
}

impl FakePx {
    fn register(&self, address: u32) -> u32 {
        self.registers.get(&address).copied().unwrap_or(0)
    }

    fn answer(&mut self, words: [u32; 4]) {
        self.answer = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    }
}

impl MockDevice for FakePx {
    fn control_out(&mut self, _: u8, _: u16, _: u16, _: &[u8]) -> Result<(), UsbError> {
        panic!("PXLogic devices take no control writes");
    }

    fn control_in(
        &mut self,
        request: u8,
        _: u16,
        _: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        assert_eq!((request, length), (CMD_CTL_RD, 16));
        self.polls += 1;
        let state = if self.states.len() > 1 {
            self.states.pop_front()
        } else {
            self.states.front().copied()
        }
        .unwrap_or_default();
        let mut data = state.samples.to_le_bytes().to_vec();
        data.extend(u32::from(state.triggered).to_le_bytes());
        data.extend(state.trigger_position.to_le_bytes());
        Ok(data)
    }

    fn bulk_out(&mut self, endpoint: u8, data: &[u8]) -> Result<(), UsbError> {
        if endpoint == EP_UPLOAD {
            self.uploads.push(Upload {
                target: self.register(regs::UPLOAD_TARGET),
                start: self.register(regs::UPLOAD_START),
                end: self.register(regs::UPLOAD_END),
                data: data.to_vec(),
            });
            return Ok(());
        }
        assert_eq!(endpoint, EP_REGISTER_OUT);
        let words: Vec<u32> = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes))
            .collect();
        let [command, 8, address, value] = words[..] else {
            panic!("a register request: {words:x?}");
        };
        match command {
            0xfefe_0000 => {
                self.registers.insert(address, value);
                self.answer([command, 8, address, 0xfefe_fefe]);
                if address == regs::RESET {
                    self.restarted = true;
                }
                // The setup ends releasing the hold; the capture runs.
                if address == regs::HOLD && value == 0 {
                    let received = Received::read(&self.registers);
                    self.capture = Some(received);
                    let bytes = (self.script)(received);
                    self.outgoing.extend(bytes);
                }
            }
            0xfefe_0001 => match (address, self.logic_mode) {
                (regs::LOGIC_MODE, None) => self.answer.clear(),
                (regs::LOGIC_MODE, Some(mode)) => self.answer([command, 8, address, mode]),
                (regs::FIRMWARE_VERSION, _) => {
                    self.answer([command, 8, address, self.firmware]);
                }
                _ => self.answer([command, 8, address, self.register(address)]),
            },
            other => panic!("unknown register command {other:#x}"),
        }
        Ok(())
    }

    fn bulk_in(&mut self, endpoint: u8, length: usize) -> Option<Result<Vec<u8>, UsbError>> {
        assert_eq!(endpoint, EP_DATA);
        if self
            .unplug_after
            .is_some_and(|transfers| self.completed >= transfers)
        {
            return Some(Err(UsbError::Transfer(TransferError::Disconnected)));
        }
        if self.polls < self.hold_polls {
            return None;
        }
        if self.endless
            && let Some(capture) = self.capture
            && self.outgoing.len() < length
        {
            let (from, to) = (self.streamed, self.streamed + 4096);
            self.outgoing.extend(cross(&capture.channels(), from, to));
            self.streamed = to;
        }
        if self.outgoing.is_empty() {
            return None;
        }
        self.completed += 1;
        let length = length.min(self.outgoing.len());
        Some(Ok(self.outgoing.drain(..length).collect()))
    }

    fn bulk_read(&mut self, endpoint: u8, _: usize) -> Result<Vec<u8>, UsbError> {
        assert_eq!(endpoint, EP_REGISTER_IN);
        Ok(std::mem::take(&mut self.answer))
    }
}

/// A firmware directory with made-up files of the names the driver loads.
fn firmware(files: &[(&str, usize)]) -> FirmwareDir {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "hsync-pxlogic-device-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for (seed, &(name, length)) in files.iter().enumerate() {
        let data: Vec<u8> = (0..length).map(|i| (i * 7 + seed * 13) as u8).collect();
        std::fs::write(dir.join(name), data).expect("firmware file");
    }
    FirmwareDir(dir)
}

fn bundled() -> FirmwareDir {
    firmware(&[
        ("SCI_LOGIC.bin", 25_804),
        ("hspi_ddr_RST.bin", 5000),
        ("hspi_ddr.bin", 300_000),
    ])
}

fn connect(
    fake: FakePx,
    scanned: usize,
    firmware: &FirmwareDir,
) -> (MockTransport<FakePx>, Result<PxlogicDevice, DriverError>) {
    let transport = MockTransport::new(fake);
    let device = PxlogicDevice::open(
        Control::new(Arc::new(transport.clone())),
        &PROFILES[scanned],
        firmware,
        "test".into(),
    );
    (transport, device)
}

/// A PX-Logic 16 Plus over USB 3.0.
fn open(fake: FakePx) -> (MockTransport<FakePx>, PxlogicDevice) {
    open_model(fake, 2, SCANNED_SUPER)
}

fn open_model(
    fake: FakePx,
    logic_mode: u32,
    scanned: usize,
) -> (MockTransport<FakePx>, PxlogicDevice) {
    let (transport, device) = connect(
        FakePx {
            logic_mode: Some(logic_mode),
            ..fake
        },
        scanned,
        &bundled(),
    );
    let device = device.expect("open");
    transport.clear_calls();
    transport.device().uploads.clear();
    (transport, device)
}

fn collect(packets: &Receiver<Packet>) -> Vec<Packet> {
    let mut all = Vec::new();
    loop {
        let packet = packets
            .recv_timeout(Duration::from_secs(10))
            .expect("the acquisition ends");
        let end = matches!(packet, Packet::End(_));
        all.push(packet);
        if end {
            return all;
        }
    }
}

/// The capture the packets build.
fn capture(packets: &[Packet]) -> LogicCapture {
    let Some(Packet::Start {
        channels,
        samplerate,
    }) = packets.first()
    else {
        panic!("the capture starts with Start");
    };
    let mut capture = LogicCapture::new(*samplerate, channels.iter().copied());
    let mut deinterleaver = capture.deinterleaver();
    for packet in packets {
        if let Packet::Logic(bytes) = packet {
            capture.append_cross(&mut deinterleaver, bytes);
        }
    }
    capture
}

fn plan(triggers: Vec<(u16, TriggerCondition)>, position: u8) -> CapturePlan {
    CapturePlan {
        triggers,
        trigger_position: position,
        instant: false,
        advanced: None,
        looping: false,
    }
}

fn run(device: &mut PxlogicDevice, plan: CapturePlan) -> Vec<Packet> {
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(plan, sink).expect("start");
    let packets = collect(&packets);
    acquisition.wait();
    packets
}

fn set(device: &mut PxlogicDevice, key: OptionKey, value: OptionValue) {
    device.set_option(key, &value).expect("option");
}

fn choice_ids(device: &PxlogicDevice, key: OptionKey) -> Vec<String> {
    let options = device.options();
    let option = options
        .iter()
        .find(|option| option.key == key)
        .expect("the option");
    let OptionKind::Choice { choices } = &option.kind else {
        panic!("{key:?} is a choice");
    };
    choices.iter().map(|choice| choice.id.clone()).collect()
}

fn rates(device: &PxlogicDevice) -> Vec<u64> {
    let options = device.options();
    let option = options
        .iter()
        .find(|option| option.key == OptionKey::Samplerate)
        .expect("sample rate");
    let OptionKind::Integers { values } = &option.kind else {
        panic!("sample rates are integers");
    };
    values.clone()
}

/// The FPGA bitstreams as they are uploaded: in whole 4 KiB pages.
fn fpga_upload(firmware: &FirmwareDir, name: &str) -> Upload {
    let mut data = firmware.load(name).expect("bitstream");
    let end = data.len().next_multiple_of(4096);
    data.resize(end, 0);
    Upload {
        target: upload::FPGA,
        start: 0,
        end: end as u32,
        data,
    }
}

#[test]
fn opens_as_the_model_it_reports() {
    let firmware = bundled();
    let (transport, device) = connect(
        FakePx {
            logic_mode: Some(2),
            ..FakePx::default()
        },
        SCANNED_SUPER,
        &firmware,
    );
    let device = device.expect("open");
    let info = device.info();
    assert_eq!(info.model, "PX-Logic 16 Plus");
    assert_eq!(info.vendor, "PXLogic");
    assert_eq!(info.driver, "PXLogic");
    assert_eq!(info.firmware.as_deref(), Some("56900028"));
    assert_eq!(info.usb_speed.as_deref(), Some("super"));
    assert!(!info.slow_port);
    let fake = transport.device();
    assert_eq!(
        fake.uploads,
        vec![
            fpga_upload(&firmware, "hspi_ddr_RST.bin"),
            fpga_upload(&firmware, "hspi_ddr.bin"),
        ]
    );
    assert!(!fake.restarted);
    drop(fake);

    // Buffer mode, all 16 channels at 500 MHz; 2 Gbit of memory.
    assert_eq!(device.channel_count(), 16);
    assert_eq!(device.enabled_channels(), (0..16).collect::<Vec<_>>());
    assert_eq!(device.settings().samplerate, 500_000_000);
    assert_eq!(device.hardware_depth(), Some(124_999_680));
    assert!(!device.can_loop());
    assert_eq!(device.trigger_caps().max_position, 90);
    assert!(!device.trigger_caps().advanced);
}

#[test]
fn keeps_the_scanned_model_when_the_mode_is_unknown() {
    for logic_mode in [None, Some(9)] {
        let (_, device) = connect(
            FakePx {
                logic_mode,
                ..FakePx::default()
            },
            SCANNED_HIGH,
            &bundled(),
        );
        let device = device.expect("open");
        assert_eq!(device.info().model, "PX-Logic 32");
        assert_eq!(device.info().usb_speed.as_deref(), Some("high"));
        assert!(device.info().slow_port);
        assert_eq!(device.channel_count(), 16, "500 MHz on 16 channels");
    }
}

#[test]
fn writes_its_firmware_into_the_flash() {
    let firmware = bundled();
    let (transport, device) = connect(
        FakePx {
            firmware: 0x5690_0027,
            ..FakePx::default()
        },
        SCANNED_SUPER,
        &firmware,
    );
    assert!(matches!(device, Err(DriverError::Restarting)));
    let mut image = firmware.load("SCI_LOGIC.bin").expect("firmware");
    image.resize(48 * 1024, 0xff);
    let fake = transport.device();
    assert_eq!(
        fake.uploads,
        vec![Upload {
            target: upload::MCU_FLASH,
            start: 48 * 1024,
            end: 4 * 48 * 1024,
            data: image.repeat(3),
        }]
    );
    assert!(fake.restarted);
}

#[test]
fn leaves_the_device_alone_without_its_firmware() {
    let (transport, device) = connect(
        FakePx {
            firmware: 0,
            ..FakePx::default()
        },
        SCANNED_SUPER,
        &firmware(&[]),
    );
    assert!(matches!(device, Err(DriverError::FirmwareMissing(name)) if name == "SCI_LOGIC.bin"));
    assert!(transport.device().uploads.is_empty());
    assert!(!transport.device().restarted);

    let (transport, device) = connect(
        FakePx {
            firmware: 0,
            ..FakePx::default()
        },
        SCANNED_SUPER,
        &firmware(&[("SCI_LOGIC.bin", 48 * 1024 + 1)]),
    );
    assert!(matches!(device, Err(DriverError::FirmwareIncompatible(_))));
    assert!(!transport.device().restarted);

    let (_, device) = connect(
        FakePx::default(),
        SCANNED_SUPER,
        &firmware(&[("SCI_LOGIC.bin", 100)]),
    );
    assert!(
        matches!(device, Err(DriverError::FirmwareMissing(name)) if name == "hspi_ddr_RST.bin")
    );
}

#[test]
fn offers_the_modes_of_the_model() {
    // A PX-Logic 16 Pro over USB 3.0.
    let (_, mut device) = open_model(FakePx::default(), 1, SCANNED_SUPER);
    assert_eq!(device.info().model, "PX-Logic 16 Pro");
    assert_eq!(choice_ids(&device, OptionKey::ChannelMode), ["2", "3"]);
    assert_eq!(
        choice_ids(&device, OptionKey::ExternalTrigger),
        ["off", "rise", "high", "fall", "low", "both"]
    );
    set(
        &mut device,
        OptionKey::ChannelMode,
        OptionValue::Choice("3".into()),
    );
    assert_eq!(device.channel_count(), 8);
    assert_eq!(rates(&device).last(), Some(&1_000_000_000));
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(1_000_000_000),
    );
    assert_eq!(
        device.hardware_depth(),
        Some(4_000_000_000 / 8 / 1024 * 1024)
    );

    // Stream mode starts at its first channel mode, the rate kept within it.
    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("stream".into()),
    );
    assert_eq!(
        choice_ids(&device, OptionKey::ChannelMode),
        ["5", "6", "7", "8"]
    );
    assert_eq!(device.channel_count(), 16);
    assert_eq!(device.settings().samplerate, 125_000_000);
    assert_eq!(device.hardware_depth(), None);
    assert!(device.can_loop());
    assert_eq!(device.trigger_caps().max_position, 1);
    assert!(
        device
            .set_option(OptionKey::ChannelMode, &OptionValue::Choice("2".into()))
            .is_err(),
        "a Buffer mode"
    );
    assert!(
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(250_000_000))
            .is_err()
    );
    assert!(
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(3_000_000))
            .is_err()
    );
    assert!(
        device
            .set_option(OptionKey::Vth, &OptionValue::Number(6.5))
            .is_err()
    );
    assert!(
        device
            .set_option(OptionKey::Rle, &OptionValue::Bool(true))
            .is_err()
    );
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(100_001),
    );
    assert_eq!(device.settings().limit_samples, 100_032);
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(10),
    );
    assert_eq!(device.settings().limit_samples, 1024);

    // Over USB 2.0 the Stream modes are slower.
    let (_, mut device) = open_model(FakePx::default(), 1, SCANNED_HIGH);
    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("stream".into()),
    );
    assert_eq!(
        choice_ids(&device, OptionKey::ChannelMode),
        ["9", "10", "11", "12", "13"]
    );
    assert_eq!(device.channel_count(), 1);
    assert_eq!(rates(&device).last(), Some(&200_000_000));
}

#[test]
fn holds_more_samples_on_fewer_channels() {
    let (_, mut device) = open(FakePx::default());
    for channel in 8..16 {
        device.set_channel_enabled(channel, false).expect("disable");
    }
    assert_eq!(device.hardware_depth(), Some(249_999_360));
    assert!(device.set_channel_enabled(16, true).is_err());
}

#[test]
fn captures_in_buffer_mode() {
    const LIMIT: u64 = 8192;
    const RATE: u64 = 1_000_000;
    let (transport, mut device) = open(FakePx {
        states: VecDeque::from([
            CaptureState {
                samples: 1000,
                triggered: false,
                trigger_position: 0,
            },
            CaptureState {
                samples: 3000,
                triggered: true,
                trigger_position: 2050,
            },
        ]),
        hold_polls: 2,
        ..FakePx::default()
    });
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(RATE),
    );
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(LIMIT),
    );
    set(&mut device, OptionKey::Vth, OptionValue::Number(3.3));
    set(&mut device, OptionKey::Filter, OptionValue::Bool(true));
    set(
        &mut device,
        OptionKey::ExternalTrigger,
        OptionValue::Choice("fall".into()),
    );
    for channel in [1, 2, 5] {
        device.set_channel_enabled(channel, false).expect("disable");
    }
    let packets = run(
        &mut device,
        plan(
            vec![
                (0, TriggerCondition::Rise),
                (3, TriggerCondition::Both),
                (4, TriggerCondition::Low),
                (5, TriggerCondition::High),
            ],
            25,
        ),
    );

    assert_eq!(
        packets[..4],
        [
            Packet::Start {
                channels: vec![0, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                samplerate: RATE,
            },
            Packet::Progress {
                triggered: false,
                captured: 1000,
            },
            Packet::Progress {
                triggered: true,
                captured: 2048,
            },
            Packet::Trigger { sample: 2050 },
        ]
    );
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    let capture = capture(&packets);
    assert_eq!(capture.len(), LIMIT);
    for (channel, data) in capture.channels() {
        let expected: Vec<bool> = (0..LIMIT).map(|i| level(channel, i)).collect();
        let got: Vec<bool> = (0..LIMIT).map(|i| data.sample(i)).collect();
        assert_eq!(got, expected, "channel {channel}");
    }

    let fake = transport.device();
    let received = fake.capture.expect("a capture");
    assert_eq!(received.enabled, 0b1111_1111_1101_1001);
    assert_eq!(received.control, 1 << 3, "Buffer mode, filtering");
    assert_eq!(received.block, block_size(RATE, 13, UsbSpeed::Super));
    assert_eq!(received.bytes, LIMIT * 13 / 8 + u64::from(received.block));
    assert_eq!(received.position, 2048);
    // Conditions of channel 5 are dropped with it.
    assert_eq!(
        (received.rise, received.fall, received.zero, received.one),
        (0b1001, 0b1000, 0b1_0000, 0)
    );
    assert_eq!(received.external, 3);
    assert_eq!(fake.register(regs::VTH_DUTY), 5938);
    assert_eq!(fake.register(regs::DIVIDER), 99);
}

#[test]
fn captures_in_stream_mode() {
    const LIMIT: u64 = 16384;
    let (transport, mut device) = open(FakePx {
        states: VecDeque::from([CaptureState {
            samples: 0,
            triggered: true,
            trigger_position: 163,
        }]),
        hold_polls: 1,
        ..FakePx::default()
    });
    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("stream".into()),
    );
    set(
        &mut device,
        OptionKey::ChannelMode,
        OptionValue::Choice("6".into()),
    );
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(2_000_000),
    );
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(LIMIT),
    );
    let packets = run(&mut device, plan(vec![(3, TriggerCondition::Fall)], 50));
    assert!(packets.contains(&Packet::Trigger { sample: 163 }));
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    let capture = capture(&packets);
    assert_eq!(capture.len(), LIMIT);
    for (channel, data) in capture.channels() {
        assert!((0..LIMIT).all(|i| data.sample(i) == level(channel, i)));
    }
    // Stream mode pins the trigger position to 1 %.
    let received = transport.device().capture.expect("a capture");
    assert!(received.stream());
    assert_eq!(received.position, 163);
    assert_eq!(received.channels(), (0..8).collect::<Vec<_>>());
}

#[test]
fn ignores_every_trigger_in_instant_captures() {
    let (transport, mut device) = open(FakePx {
        states: VecDeque::from([CaptureState {
            samples: 500,
            triggered: true,
            trigger_position: 600,
        }]),
        hold_polls: 1,
        ..FakePx::default()
    });
    let mut instant = plan(vec![(0, TriggerCondition::Rise)], 50);
    instant.instant = true;
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(4096),
    );
    set(
        &mut device,
        OptionKey::ExternalTrigger,
        OptionValue::Choice("rise".into()),
    );
    let packets = run(&mut device, instant);
    assert!(
        !packets
            .iter()
            .any(|packet| matches!(packet, Packet::Trigger { .. }))
    );
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    let received = transport.device().capture.expect("a capture");
    assert_eq!(
        (received.rise, received.fall, received.zero, received.one),
        (0, 0, 0, 0)
    );
    assert_eq!(received.external, 0);
}

#[test]
fn loops_in_stream_mode_until_stopped() {
    let (transport, mut device) = open(FakePx {
        states: VecDeque::from([CaptureState {
            samples: 0,
            triggered: true,
            trigger_position: 64,
        }]),
        script: Box::new(|_| Vec::new()),
        endless: true,
        ..FakePx::default()
    });
    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("stream".into()),
    );
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(1_000_000),
    );
    let (sink, packets) = crossbeam_channel::unbounded();
    let mut looping = plan(vec![(0, TriggerCondition::Rise)], 1);
    looping.looping = true;
    let acquisition = device.start(looping, sink).expect("start");
    // Five times the sample limit.
    let mut samples = 0;
    while samples < 5 * 1_000_000 {
        match packets.recv_timeout(Duration::from_secs(5)) {
            Ok(Packet::Logic(bytes)) => samples += bytes.len() as u64 * 8 / 16,
            Ok(Packet::Trigger { .. }) => panic!("loop captures have no trigger"),
            Ok(Packet::End(reason)) => panic!("ended early: {reason:?}"),
            Ok(_) => {}
            Err(error) => panic!("no data: {error}"),
        }
    }
    acquisition.stop();
    let packets = collect(&packets);
    acquisition.wait();
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Stopped)));
    assert!(transport.device().capture.expect("a capture").stream());
}

#[test]
fn stops_on_request() {
    let (transport, mut device) = open(FakePx {
        hold_polls: usize::MAX,
        ..FakePx::default()
    });
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device
        .start(plan(vec![(0, TriggerCondition::Rise)], 50), sink)
        .expect("start");
    assert!(
        device
            .set_option(OptionKey::Filter, &OptionValue::Bool(true))
            .is_err(),
        "busy capturing"
    );
    loop {
        match packets.recv_timeout(Duration::from_secs(5)) {
            Ok(Packet::Progress { .. }) => break,
            Ok(Packet::End(reason)) => panic!("ended early: {reason:?}"),
            Ok(_) => {}
            Err(error) => panic!("no progress: {error}"),
        }
    }
    let before = transport.calls().len();
    acquisition.stop();
    let packets = collect(&packets);
    acquisition.wait();
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Stopped)));
    // PXView tells the device nothing when a capture ends; a poll may have been under way.
    assert!(
        transport.calls()[before..]
            .iter()
            .all(|call| matches!(call, UsbCall::ControlIn { .. }))
    );
    set(&mut device, OptionKey::Filter, OptionValue::Bool(true));
}

#[test]
fn ends_when_the_device_is_unplugged() {
    let (_, mut device) = open(FakePx {
        unplug_after: Some(0),
        ..FakePx::default()
    });
    let packets = run(&mut device, plan(Vec::new(), 1));
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Detached)));
}

#[test]
fn refuses_what_pxlogic_cannot_do() {
    let (_, mut device) = open(FakePx::default());
    let (sink, _packets) = crossbeam_channel::unbounded();
    let mut looping = plan(Vec::new(), 1);
    looping.looping = true;
    assert!(matches!(
        device.start(looping, sink.clone()),
        Err(DriverError::InvalidRequest(_))
    ));
    for channel in 0..16 {
        device.set_channel_enabled(channel, false).expect("disable");
    }
    assert!(matches!(
        device.start(plan(Vec::new(), 1), sink),
        Err(DriverError::InvalidRequest(_))
    ));
}
