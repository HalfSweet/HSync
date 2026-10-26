//! The ATK-Logic driver against a simulated device: connecting, options and captures end to
//! end.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use hsync_atklogic::protocol::{
    BLOCK, EP_IN, EP_OUT, Link, MCU_PACKET, Timing, cmd, crc32, frame, from_lanes, mcu, to_lanes,
    u40,
};
use hsync_atklogic::{AtkLogicDevice, UPDATE_TOOL};
use hsync_capture::LogicCapture;
use hsync_core::{TriggerCondition, UiError};
use hsync_driver::mock::{MockDevice, MockTransport};
use hsync_driver::usb::UsbError;
use hsync_driver::{
    CapturePlan, Device, DriverError, EndReason, OptionKey, OptionKind, OptionValue, Packet,
};
use nusb::transfer::TransferError;

/// A command as the device received it: an MCU code and its first argument, or an FPGA code
/// and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Mcu(u8, u8),
    Fpga(u8, Vec<u8>),
}

/// Reads a command off the wire, checking its framing and CRC.
fn decode(data: &[u8]) -> Command {
    if data.len() == MCU_PACKET {
        assert_eq!(data[0], mcu::HEAD);
        assert!(data[3..].iter().all(|&byte| byte == 0), "MCU padding");
        return Command::Mcu(data[1], data[2]);
    }
    assert_eq!(data.len(), BLOCK, "FPGA commands fill a block");
    let mut ordered = Vec::new();
    from_lanes(&data.as_chunks::<BLOCK>().0[0], &mut ordered);
    assert_eq!(&ordered[..9], &[0, 0, 0, 0, 0, 0, 0, 0, 0x0a]);
    let count = usize::from(ordered[10]);
    assert_eq!(ordered[10 + count], 0x0b);
    let crc = u32::from_le_bytes(*ordered[11 + count..].first_chunk::<4>().expect("the CRC"));
    assert_eq!(crc, crc32(&ordered[9..10 + count]));
    assert!(
        ordered[15 + count..].iter().all(|&byte| byte == 0),
        "padding"
    );
    Command::Fpga(ordered[9], ordered[11..10 + count].to_vec())
}

fn frame_bytes(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x0a, kind];
    bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&[0x00, 0x0b]);
    bytes
}

fn u40_bytes(value: u64) -> [u8; 5] {
    let mut bytes = [0; 5];
    bytes.copy_from_slice(&value.to_le_bytes()[..5]);
    bytes
}

/// Sample `index` of `channel` as the simulated device sees it: a square wave whose period
/// grows with the channel.
fn level(channel: u16, index: u64) -> bool {
    (index / (u64::from(channel) + 3)) % 2 == 1
}

/// Samples `from..to` of `channel` packed eight a byte, earliest in bit 0.
fn packed(channel: u16, from: u64, to: u64) -> Vec<u8> {
    (from..to)
        .step_by(8)
        .map(|base| {
            (0..8).fold(0u8, |byte, bit| {
                byte | (u8::from(level(channel, base + bit)) << bit)
            })
        })
        .collect()
}

/// Data frames of the enabled channels, `chunk` bytes at a time, taking turns.
fn data_frames(enabled: u16, bytes: &dyn Fn(u16) -> Vec<u8>, chunk: usize) -> Vec<u8> {
    let channels: Vec<(u16, Vec<u8>)> = (0..16)
        .filter(|channel| enabled & (1 << channel) != 0)
        .map(|channel| (channel, bytes(channel)))
        .collect();
    let longest = channels
        .iter()
        .map(|(_, data)| data.len())
        .max()
        .unwrap_or(0);
    let mut out = Vec::new();
    for start in (0..longest).step_by(chunk) {
        for (channel, data) in &channels {
            if start < data.len() {
                let mut payload = vec![*channel as u8, 0];
                payload.extend_from_slice(&data[start..(start + chunk).min(data.len())]);
                out.extend(frame_bytes(frame::DATA, &payload));
            }
        }
    }
    out
}

/// What the device was told before the trigger command.
#[derive(Debug, Clone, Copy)]
struct Received {
    buffer: bool,
    rle: bool,
    depth: u64,
    pre: u64,
    enabled: u16,
}

impl Received {
    fn parse(settings: &[u8], trigger: &[u8]) -> Self {
        let enabled = (0..16u16)
            .filter(|&channel| {
                let byte = trigger[usize::from(channel / 2)];
                let nibble = if channel % 2 == 0 {
                    byte >> 4
                } else {
                    byte & 15
                };
                nibble & 8 != 0
            })
            .fold(0, |mask, channel| mask | (1 << channel));
        Self {
            buffer: settings[0] & 0x80 != 0,
            rle: settings[0] & 0x40 != 0,
            depth: u40(&settings[3..8]),
            pre: u40(&settings[8..13]),
            enabled,
        }
    }
}

type Script = Box<dyn FnMut(Received) -> Vec<u8> + Send>;

/// An ATK-Logic as its MCU and FPGA answer on the bulk endpoints.
struct FakeAtk {
    /// Bytes waiting on the IN endpoint, in lane order.
    outgoing: VecDeque<u8>,
    commands: Vec<Command>,
    /// The MCU's answer to a version request, or none.
    version: Option<[u8; 9]>,
    /// The payload of the FPGA's device information, or none.
    info: Option<Vec<u8>>,
    settings: Vec<u8>,
    /// What the device sends once a capture starts.
    script: Script,
    /// Data frames the device keeps sending until stopped (Stream mode), for these channels.
    endless: Option<u16>,
    /// Samples per channel sent so far by `endless`.
    streamed: u64,
    /// IN transfers complete only once full, as the device sends no short packets.
    whole_transfers: bool,
    /// The device is gone once this many transfers have completed.
    unplug_after: Option<usize>,
    completed: usize,
}

impl Default for FakeAtk {
    fn default() -> Self {
        Self {
            outgoing: VecDeque::new(),
            commands: Vec::new(),
            version: Some([0x0a, 0x81, 0x01, 0x61, 1, 2, 3, 0, 0]),
            info: Some(info_payload(1, 2, (1, 15), (10, 66), "DL16")),
            settings: Vec::new(),
            script: Box::new(|_| Vec::new()),
            endless: None,
            streamed: 0,
            whole_transfers: true,
            unplug_after: None,
            completed: 0,
        }
    }
}

fn info_payload(ok: u8, usb: u8, fpga: (u8, u8), app: (u8, u8), name: &str) -> Vec<u8> {
    let mut payload = vec![0, 0, ok, usb, 0, fpga.0, fpga.1, app.0, app.1];
    payload.extend_from_slice(name.as_bytes());
    payload
}

impl FakeAtk {
    /// Queues frames as the FPGA sends them: zero-padded to whole blocks in lane order.
    fn send(&mut self, frames: &[u8]) {
        let mut bytes = frames.to_vec();
        bytes.resize(bytes.len().next_multiple_of(BLOCK), 0);
        self.outgoing.extend(to_lanes(&bytes));
    }

    fn take(&mut self, length: usize) -> Vec<u8> {
        let length = length.min(self.outgoing.len());
        self.outgoing.drain(..length).collect()
    }

    fn fpga_commands(&self) -> Vec<(u8, Vec<u8>)> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                Command::Fpga(code, args) => Some((*code, args.clone())),
                Command::Mcu(..) => None,
            })
            .collect()
    }
}

impl MockDevice for FakeAtk {
    fn control_out(&mut self, _: u8, _: u16, _: u16, _: &[u8]) -> Result<(), UsbError> {
        panic!("ATK-Logic devices take no control requests");
    }

    fn control_in(&mut self, _: u8, _: u16, _: u16, _: u16) -> Result<Vec<u8>, UsbError> {
        panic!("ATK-Logic devices take no control requests");
    }

    fn bulk_out(&mut self, endpoint: u8, data: &[u8]) -> Result<(), UsbError> {
        assert_eq!(endpoint, EP_OUT);
        let command = decode(data);
        match &command {
            Command::Mcu(mcu::VERSION, _) => {
                if let Some(version) = self.version {
                    let mut answer = vec![0; MCU_PACKET];
                    answer[..9].copy_from_slice(&version);
                    self.outgoing.extend(answer);
                }
            }
            Command::Fpga(cmd::DEVICE_INFO, _) => {
                if let Some(info) = self.info.clone() {
                    self.send(&frame_bytes(frame::INFO, &info));
                }
            }
            Command::Fpga(cmd::SETTINGS, args) => self.settings = args.clone(),
            Command::Fpga(cmd::TRIGGER, args) => {
                let received = Received::parse(&self.settings, args);
                let frames = (self.script)(received);
                self.send(&frames);
            }
            Command::Fpga(cmd::STOP, _) => {
                self.endless = None;
                self.send(&frame_bytes(frame::REPLY, &[0, 0, cmd::STOP]));
            }
            _ => {}
        }
        self.commands.push(command);
        Ok(())
    }

    fn bulk_in(&mut self, endpoint: u8, length: usize) -> Option<Result<Vec<u8>, UsbError>> {
        assert_eq!(endpoint, EP_IN);
        if self
            .unplug_after
            .is_some_and(|transfers| self.completed >= transfers)
        {
            return Some(Err(UsbError::Transfer(TransferError::Disconnected)));
        }
        if let Some(enabled) = self.endless
            && self.outgoing.len() < length
        {
            let (from, to) = (self.streamed, self.streamed + 4096);
            let frames = data_frames(enabled, &|channel| packed(channel, from, to), 512);
            self.streamed = to;
            self.send(&frames);
        }
        if self.outgoing.is_empty() || (self.whole_transfers && self.outgoing.len() < length) {
            return None;
        }
        self.completed += 1;
        Some(Ok(self.take(length)))
    }

    fn bulk_read(&mut self, endpoint: u8, length: usize) -> Result<Vec<u8>, UsbError> {
        assert_eq!(endpoint, EP_IN);
        Ok(self.take(length))
    }

    fn cancel_in(&mut self, _endpoint: u8, length: usize) -> Vec<u8> {
        self.take(length)
    }
}

/// A device that sends what `script` makes of the settings once a capture starts.
fn scripted(script: impl FnMut(Received) -> Vec<u8> + Send + 'static) -> FakeAtk {
    FakeAtk {
        script: Box::new(script),
        ..FakeAtk::default()
    }
}

fn connect(fake: FakeAtk) -> (MockTransport<FakeAtk>, Result<AtkLogicDevice, DriverError>) {
    let transport = MockTransport::new(fake);
    let link = Link::new(Arc::new(transport.clone())).with_timing(Timing::IMMEDIATE);
    let device = AtkLogicDevice::open(link, "test".into());
    (transport, device)
}

fn open(fake: FakeAtk) -> (MockTransport<FakeAtk>, AtkLogicDevice) {
    let (transport, device) = connect(fake);
    let device = device.expect("open");
    transport.device().commands.clear();
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
        ..
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

fn run(device: &mut AtkLogicDevice, plan: CapturePlan) -> Vec<Packet> {
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(plan, sink).expect("start");
    let packets = collect(&packets);
    acquisition.wait();
    packets
}

fn set(device: &mut AtkLogicDevice, key: OptionKey, value: OptionValue) {
    device.set_option(key, &value).expect("option");
}

fn rates(device: &AtkLogicDevice) -> Vec<u64> {
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

fn last_failure(packets: &[Packet]) -> Option<UiError> {
    match packets.last() {
        Some(Packet::End(EndReason::Failed(error))) => Some(error.clone()),
        _ => None,
    }
}

#[test]
fn connects_as_atk_logic_does() {
    let (transport, device) = connect(FakeAtk {
        version: Some([0x0a, 0x81, 0x01, 0x61, 1, 2, 3, 0, 1]),
        ..FakeAtk::default()
    });
    let device = device.expect("open");
    let info = device.info();
    assert_eq!(info.model, "DL16 Plus");
    assert_eq!(info.vendor, "ALIENTEK");
    assert_eq!(info.driver, "ATK-Logic");
    assert_eq!(info.firmware.as_deref(), Some("1.2 / 1.15"));
    assert_eq!(info.usb_speed.as_deref(), Some("high"));
    assert!(device.is_plus());
    assert_eq!(
        transport.device().commands,
        vec![
            Command::Mcu(mcu::VERSION, 0),
            Command::Mcu(mcu::VERSION, 0),
            Command::Mcu(mcu::RESET_STATE, 0),
            Command::Mcu(mcu::RESET_STATE, 1),
            Command::Fpga(cmd::DEVICE_INFO, Vec::new()),
            Command::Fpga(cmd::PWM, vec![0x20]),
            Command::Fpga(cmd::PWM, vec![0x10]),
            Command::Mcu(mcu::RESET_STATE, 0),
        ]
    );
}

#[test]
fn lets_go_of_the_device_on_closing() {
    let (transport, device) = open(FakeAtk::default());
    assert_eq!(device.info().model, "DL16");
    drop(device);
    assert_eq!(
        transport.device().commands,
        vec![
            Command::Mcu(mcu::RESET_STATE, 1),
            Command::Fpga(cmd::PWM, vec![0x20]),
            Command::Fpga(cmd::PWM, vec![0x10]),
            Command::Fpga(cmd::EXIT, Vec::new()),
            Command::Mcu(mcu::RESET_STATE, 0),
        ]
    );
}

#[test]
fn leaves_firmware_updates_to_atk_logic() {
    let (_, bootloader) = connect(FakeAtk {
        version: Some([0x0a, 0x81, 0x01, 0x62, 1, 0, 1, 0, 0]),
        ..FakeAtk::default()
    });
    assert!(matches!(bootloader, Err(DriverError::FirmwareUpdate(tool)) if tool == UPDATE_TOOL));
    let (_, old) = connect(FakeAtk {
        info: Some(info_payload(1, 2, (1, 14), (10, 66), "DL16")),
        ..FakeAtk::default()
    });
    assert!(matches!(old, Err(DriverError::FirmwareUpdate(_))));
    let (_, newer) = connect(FakeAtk {
        info: Some(info_payload(1, 2, (2, 0), (10, 67), "DL16")),
        ..FakeAtk::default()
    });
    assert!(
        matches!(newer, Err(DriverError::FirmwareIncompatible(version)) if version == "FPGA 2.00")
    );
    let (_, faulty) = connect(FakeAtk {
        info: Some(info_payload(0, 2, (1, 15), (10, 66), "DL16")),
        ..FakeAtk::default()
    });
    assert!(matches!(faulty, Err(DriverError::Fault)));
}

#[test]
fn gives_up_on_a_silent_device() {
    let (transport, device) = connect(FakeAtk {
        info: None,
        ..FakeAtk::default()
    });
    assert!(matches!(device, Err(DriverError::Timeout(_))));
    let infos = transport
        .device()
        .fpga_commands()
        .iter()
        .filter(|(code, _)| *code == cmd::DEVICE_INFO)
        .count();
    assert_eq!(infos, 6, "six attempts");
}

#[test]
fn offers_rates_by_mode_and_channels() {
    let (_, mut device) = open(FakeAtk {
        version: Some([0x0a, 0x81, 0x01, 0x61, 1, 2, 3, 0, 1]),
        ..FakeAtk::default()
    });
    // Stream mode at first, all 16 channels at the fastest rate USB carries for them.
    assert_eq!(rates(&device).last(), Some(&20_000_000));
    assert_eq!(device.settings().samplerate, 20_000_000);
    assert_eq!(device.hardware_depth(), None);
    assert!(device.can_loop());
    assert_eq!(device.trigger_caps().max_position, 1);
    for channel in 3..16 {
        device.set_channel_enabled(channel, false).expect("disable");
    }
    assert_eq!(rates(&device).last(), Some(&100_000_000));
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(100_000_000),
    );
    // A fourth channel takes 100 MHz beyond USB; the rate falls back.
    device.set_channel_enabled(3, true).expect("enable");
    assert_eq!(device.settings().samplerate, 50_000_000);

    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("buffer".into()),
    );
    assert_eq!(rates(&device).last(), Some(&1_000_000_000));
    assert_eq!(device.hardware_depth(), Some(875_000_000));
    assert!(!device.can_loop());
    assert_eq!(device.trigger_caps().max_position, 90);
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(1_000_000_000),
    );
    for channel in 4..16 {
        device.set_channel_enabled(channel, true).expect("enable");
    }
    // Sixteen channels go up to 500 MHz.
    assert_eq!(device.settings().samplerate, 500_000_000);
    assert!(
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(3_000_000))
            .is_err()
    );
    set(&mut device, OptionKey::Vth, OptionValue::Number(-2.5));
    assert!(
        device
            .set_option(OptionKey::Vth, &OptionValue::Number(5.5))
            .is_err()
    );
    assert!(
        device
            .set_option(OptionKey::Filter, &OptionValue::Bool(true))
            .is_err()
    );
}

/// `channels` and `max_samplerate` of each rate tier.
fn tiers(device: &AtkLogicDevice) -> Vec<(u16, u64)> {
    device
        .rate_tiers()
        .iter()
        .map(|tier| {
            assert!(!tier.leading);
            assert_eq!(tier.span, 16);
            assert_eq!(tier.samplerates.last(), Some(&tier.max_samplerate));
            (tier.channels, tier.max_samplerate)
        })
        .collect()
}

#[test]
fn offers_rate_tiers_by_channel_count() {
    // A DL16 Plus.
    let (_, mut device) = open(FakeAtk {
        version: Some([0x0a, 0x81, 0x01, 0x61, 1, 2, 3, 0, 1]),
        ..FakeAtk::default()
    });
    assert_eq!(
        tiers(&device),
        [
            (3, 100_000_000),
            (6, 50_000_000),
            (8, 40_000_000),
            (12, 25_000_000),
            (16, 20_000_000),
        ]
    );
    assert_eq!(device.rate_tier(), Some(4));

    device.select_rate_tier(0, &[2, 9]).expect("two channels");
    assert_eq!(device.enabled_channels(), [2, 9]);
    assert_eq!(device.rate_tier(), Some(0));
    assert_eq!(device.settings().samplerate, 20_000_000);
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(100_000_000),
    );
    assert!(
        device
            .select_rate_tier(0, &[0, 1, 2, 3])
            .is_err_and(|error| matches!(error, DriverError::InvalidRequest(_)))
    );
    assert!(device.select_rate_tier(5, &[0]).is_err());
    assert_eq!(device.enabled_channels(), [2, 9]);
    assert_eq!(device.settings().samplerate, 100_000_000);
    device
        .select_rate_tier(2, &(0..8).collect::<Vec<_>>())
        .expect("eight channels");
    assert_eq!(device.rate_tier(), Some(2));
    assert_eq!(device.settings().samplerate, 40_000_000);

    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("buffer".into()),
    );
    assert_eq!(tiers(&device), [(8, 1_000_000_000), (16, 500_000_000)]);
    assert_eq!(device.rate_tier(), Some(0));
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(1_000_000_000),
    );
    device.set_enabled_channels(&[0, 15]).expect("two channels");
    assert_eq!(device.settings().samplerate, 1_000_000_000);
    device
        .select_rate_tier(1, &(0..16).collect::<Vec<_>>())
        .expect("every channel");
    assert_eq!(device.settings().samplerate, 500_000_000);
}

#[test]
fn has_one_rate_tier_in_buffer_mode_on_the_dl16() {
    let settings = hsync_atklogic::Settings {
        stream: false,
        ..hsync_atklogic::Settings::defaults(false)
    };
    let tiers = settings.rate_tiers(false);
    assert_eq!(tiers.len(), 1);
    assert_eq!(
        (tiers[0].channels, tiers[0].max_samplerate),
        (16, 250_000_000)
    );
}

#[test]
fn captures_in_buffer_mode() {
    const LIMIT: u64 = 8192;
    const POSITION: u8 = 25;
    // The device keeps capturing before the trigger until it fires, then holds this much more
    // than asked for; 77 is not a whole byte.
    const EXTRA: u64 = 1000 + 77;
    let fake = scripted(|received: Received| {
        assert!(received.buffer && !received.rle);
        assert_eq!(received.depth, LIMIT);
        let pre = received.pre;
        let held = (received.depth + EXTRA).next_multiple_of(8);
        // Counted in the device's samples: the trigger fired at sample `pre + EXTRA`.
        let after = held - (pre + EXTRA);
        let mut frames = Vec::new();
        for count in [pre / 2, pre, LIMIT] {
            let mut payload = vec![0, 0];
            payload.extend(u40_bytes(count));
            frames.extend(frame_bytes(frame::PROGRESS, &payload));
        }
        let mut offset = vec![0, 0];
        offset.extend(u40_bytes(after));
        for channel in 0..16 {
            let count = if received.enabled & (1 << channel) != 0 {
                held / 8
            } else {
                0
            };
            offset.extend(u40_bytes(count));
        }
        frames.extend(frame_bytes(frame::OFFSET, &offset));
        frames.extend(data_frames(
            received.enabled,
            &|channel| packed(channel, 0, held),
            300,
        ));
        frames.extend(frame_bytes(frame::DONE, &[0, 0, 0]));
        frames
    });
    let (transport, mut device) = open(fake);
    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("buffer".into()),
    );
    set(
        &mut device,
        OptionKey::Samplerate,
        OptionValue::Integer(200_000_000),
    );
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(LIMIT),
    );
    set(&mut device, OptionKey::Vth, OptionValue::Number(3.3));
    for channel in [1, 2, 5] {
        device.set_channel_enabled(channel, false).expect("disable");
    }
    let packets = run(
        &mut device,
        plan(vec![(0, TriggerCondition::Rise)], POSITION),
    );

    let pre = LIMIT / 100 * u64::from(POSITION);
    assert!(packets.contains(&Packet::Progress {
        triggered: false,
        captured: pre / 2
    }));
    assert!(packets.contains(&Packet::Trigger { sample: pre }));
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    let capture = capture(&packets);
    assert_eq!(capture.len(), LIMIT);
    for (channel, data) in capture.channels() {
        let expected: Vec<bool> = (0..LIMIT).map(|i| level(channel, EXTRA + i)).collect();
        let got: Vec<bool> = (0..LIMIT).map(|i| data.sample(i)).collect();
        assert_eq!(got, expected, "channel {channel}");
    }
    assert_eq!(
        capture
            .channels()
            .map(|(channel, _)| channel)
            .collect::<Vec<_>>(),
        vec![0, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );

    let commands = transport.device().commands.clone();
    let fpga: Vec<_> = transport.device().fpga_commands();
    assert_eq!(commands[0], Command::Mcu(mcu::RESET_STATE, 1));
    let mut settings = vec![0x80, 33, 11];
    settings.extend(u40_bytes(LIMIT));
    settings.extend(u40_bytes(pre));
    assert_eq!(fpga[0], (cmd::SETTINGS, settings));
    // Channel 0 rising; 1, 2 and 5 not captured; the rest without a condition.
    assert_eq!(
        fpga[1],
        (
            cmd::TRIGGER,
            vec![0x90, 0x0f, 0xf0, 0xff, 0xff, 0xff, 0xff, 0xff, 0]
        )
    );
    assert_eq!(fpga[2], (cmd::STOP, Vec::new()));
    assert_eq!(commands.last(), Some(&Command::Mcu(mcu::RESET_STATE, 0)));
}

#[test]
fn captures_in_stream_mode() {
    const LIMIT: u64 = 16384;
    let fake = scripted(|received: Received| {
        assert!(!received.buffer);
        let mut offset = vec![0, 0];
        offset.extend(u40_bytes(100));
        let mut frames = frame_bytes(frame::OFFSET, &offset);
        frames.extend(data_frames(
            received.enabled,
            &|channel| packed(channel, 0, received.depth),
            1000,
        ));
        frames.extend(frame_bytes(frame::DONE, &[0, 0, 0]));
        frames
    });
    let (transport, mut device) = open(fake);
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(LIMIT),
    );
    let packets = run(&mut device, plan(vec![(3, TriggerCondition::Fall)], 50));
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    assert!(packets.contains(&Packet::Trigger { sample: 100 }));
    let capture = capture(&packets);
    assert_eq!(capture.len(), LIMIT);
    for (channel, data) in capture.channels() {
        assert!((0..LIMIT).all(|i| data.sample(i) == level(channel, i)));
    }
    // Stream mode pins the trigger position to 1 %.
    let fpga = transport.device().fpga_commands();
    assert_eq!(fpga[0].1[0], 0);
    assert_eq!(u40(&fpga[0].1[8..13]), LIMIT / 100);
}

#[test]
fn expands_rle_captures() {
    let fake = scripted(|received: Received| {
        assert!(received.buffer && received.rle);
        let mut offset = vec![0, 0];
        offset.extend(u40_bytes(received.depth - received.pre));
        for _ in 0..16 {
            offset.extend(u40_bytes(512));
        }
        offset.push(0);
        let mut frames = frame_bytes(frame::OFFSET, &offset);
        for channel in 0..16u8 {
            // 300 bytes of 0x0f, then 212 of 0xff: 4096 samples.
            frames.extend(frame_bytes(
                frame::DATA,
                &[channel, 0, 200, 0x0f, 100, 0x0f, 212, 0xff],
            ));
        }
        frames.extend(frame_bytes(frame::DONE, &[0, 0, 0]));
        frames
    });
    let (_, mut device) = open(fake);
    set(
        &mut device,
        OptionKey::OperationMode,
        OptionValue::Choice("buffer".into()),
    );
    // Beyond the 62.5 M samples the memory holds for 16 channels.
    set(
        &mut device,
        OptionKey::SampleLimit,
        OptionValue::Integer(100_000_768),
    );
    let packets = run(&mut device, plan(Vec::new(), 10));
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));
    let capture = capture(&packets);
    assert_eq!(capture.len(), 4096);
    let data = capture.channel(9).expect("channel 9");
    assert!(data.sample(0) && !data.sample(4) && data.sample(2395) && !data.sample(2399));
    assert!(data.sample(2400) && data.sample(4095));
}

#[test]
fn reports_compressed_captures_beyond_the_memory() {
    for (flags, key) in [
        (1, "error.device.memoryFull"),
        (3, "error.device.rleOverrun"),
    ] {
        let fake = scripted(move |_| {
            let mut offset = vec![0, 0];
            offset.extend(u40_bytes(0));
            offset.extend([0; 80]);
            offset.push(flags);
            frame_bytes(frame::OFFSET, &offset)
        });
        let (_, mut device) = open(fake);
        set(
            &mut device,
            OptionKey::OperationMode,
            OptionValue::Choice("buffer".into()),
        );
        set(
            &mut device,
            OptionKey::SampleLimit,
            OptionValue::Integer(100_000_768),
        );
        let packets = run(&mut device, plan(Vec::new(), 10));
        assert_eq!(last_failure(&packets), Some(UiError::new(key)));
    }
}

#[test]
fn reports_stream_overflow() {
    let fake = scripted(|_| frame_bytes(frame::DONE, &[0, 0, 1]));
    let (_, mut device) = open(fake);
    let packets = run(&mut device, plan(Vec::new(), 1));
    assert!(packets.contains(&Packet::Overflow));
    assert_eq!(
        last_failure(&packets),
        Some(UiError::new("error.device.overflow"))
    );
}

#[test]
fn reports_refused_settings() {
    let fake = scripted(|_| frame_bytes(frame::REPLY, &[0, 0, cmd::TRIGGER, 2]));
    let (_, mut device) = open(fake);
    let packets = run(&mut device, plan(Vec::new(), 1));
    assert_eq!(
        last_failure(&packets).map(|error| error.code),
        Some("error.device.invalidRequest".to_owned())
    );
}

#[test]
fn fails_when_the_device_falls_silent() {
    let (_, mut device) = open(FakeAtk::default());
    let packets = run(&mut device, plan(Vec::new(), 1));
    assert_eq!(
        last_failure(&packets).map(|error| error.code),
        Some("error.device.timeout".to_owned())
    );
}

#[test]
fn stops_on_request() {
    let fake = scripted(|_| Vec::new());
    let (transport, mut device) = open(fake);
    transport.device().endless = Some(0xffff);
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(plan(Vec::new(), 1), sink).expect("start");
    loop {
        match packets.recv_timeout(Duration::from_secs(5)) {
            Ok(Packet::Logic(_)) => break,
            Ok(Packet::End(reason)) => panic!("ended early: {reason:?}"),
            Ok(_) => {}
            Err(error) => panic!("no data: {error}"),
        }
    }
    acquisition.stop();
    let packets = collect(&packets);
    acquisition.wait();
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Stopped)));
    let fpga = transport.device().fpga_commands();
    assert_eq!(fpga.last(), Some(&(cmd::STOP, Vec::new())));
}

#[test]
fn ends_when_the_device_is_unplugged() {
    let fake = FakeAtk {
        unplug_after: Some(0),
        ..scripted(|received: Received| {
            data_frames(
                received.enabled,
                &|channel| packed(channel, 0, received.depth),
                1000,
            )
        })
    };
    let (transport, mut device) = open(fake);
    let packets = run(&mut device, plan(Vec::new(), 1));
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Detached)));
    assert!(
        !transport
            .device()
            .fpga_commands()
            .iter()
            .any(|(code, _)| *code == cmd::STOP),
        "a device that is gone is not stopped"
    );
}

#[test]
fn loops_in_stream_mode_until_stopped() {
    let fake = scripted(|received: Received| {
        assert_eq!(received.depth, ((1u64 << 40) - 1) / 16);
        Vec::new()
    });
    let (transport, mut device) = open(fake);
    transport.device().endless = Some(0xffff);
    let (sink, packets) = crossbeam_channel::unbounded();
    let mut looping = plan(Vec::new(), 1);
    looping.looping = true;
    let acquisition = device.start(looping, sink).expect("start");
    let mut samples = 0;
    while samples < 3 * 4096 {
        match packets.recv_timeout(Duration::from_secs(5)) {
            Ok(Packet::Logic(bytes)) => samples += bytes.len() as u64 / 16 / 8 * 64,
            Ok(Packet::End(reason)) => panic!("ended early: {reason:?}"),
            Ok(_) => {}
            Err(error) => panic!("no data: {error}"),
        }
    }
    acquisition.stop();
    let packets = collect(&packets);
    acquisition.wait();
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Stopped)));
}
