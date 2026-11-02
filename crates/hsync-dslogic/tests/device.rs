//! The DSLogic driver against a simulated device: opening, FPGA configuration, options and
//! acquisitions end to end.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use hsync_core::TriggerCondition;
use hsync_driver::mock::{MockDevice, MockTransport, UsbCall};
use hsync_driver::usb::UsbError;
use hsync_driver::{
    AdvancedTrigger, CapturePlan, Device, DriverError, EndReason, OptionKey, OptionKind,
    OptionValue, Packet, RateTier, SerialTriggerSpec, TriggerCaps, TriggerStageSpec,
};
use hsync_dslogic::profiles::{CHANNEL_MODES, UsbSpeed, VENDOR_ID, find_profile};
use hsync_dslogic::protocol::{
    CMD_CTL_RD, CMD_CTL_RD_PRE, CMD_CTL_WR, Control, ctr0, dest, reg, status,
};
use hsync_dslogic::setting::{ArmConfig, TriggerSetup, setting};
use hsync_dslogic::{DslogicDevice, FirmwareDir};

const HEADER_LEN: usize = 512;

/// A DSLogic as its firmware behaves on the control and bulk endpoints.
#[derive(Default)]
struct FakeDslogic {
    fpga_configured: bool,
    registers: HashMap<u16, u8>,
    announced: Option<(u8, u16, u8)>,
    progress: [u8; 4],
    outgoing: VecDeque<Vec<u8>>,
    on_start: Vec<Vec<u8>>,
    on_force_stop: Vec<Vec<u8>>,
    settings: Vec<Vec<u8>>,
    bitstream: Option<Vec<u8>>,
}

impl MockDevice for FakeDslogic {
    fn control_out(
        &mut self,
        request: u8,
        _value: u16,
        _index: u16,
        data: &[u8],
    ) -> Result<(), UsbError> {
        let target = data[0];
        let offset = u16::from_le_bytes([data[1], data[2]]);
        match request {
            CMD_CTL_WR => match target {
                dest::I2C_REG => {
                    self.registers.insert(offset, data[4]);
                    if offset == reg::CTR0 && data[4] == ctr0::FORCE_STOP {
                        self.outgoing.extend(self.on_force_stop.drain(..));
                    }
                }
                dest::START => self.outgoing.extend(self.on_start.drain(..)),
                _ => {}
            },
            CMD_CTL_RD_PRE => self.announced = Some((target, offset, data[3])),
            _ => {}
        }
        Ok(())
    }

    fn control_in(
        &mut self,
        request: u8,
        _value: u16,
        _index: u16,
        length: u16,
    ) -> Result<Vec<u8>, UsbError> {
        assert_eq!(request, CMD_CTL_RD);
        let (target, offset, size) = self.announced.take().expect("a read was announced");
        assert_eq!(u16::from(size), length);
        Ok(match (target, offset, size) {
            (dest::FW_VERSION, _, 2) => vec![2, 0],
            (dest::HW_STATUS, _, 1) => {
                let done = if self.fpga_configured {
                    status::FPGA_DONE
                } else {
                    0
                };
                vec![status::FPGA_INIT_B | status::SYS_CLR | status::GPIF_DONE | done]
            }
            (dest::I2C_STATUS, 0, 5) => vec![0, 0, 0, 0, 0x0e],
            (dest::I2C_STATUS, 0, 4) => self.progress.to_vec(),
            (dest::I2C_STATUS, register, 1) => {
                vec![self.registers.get(&register).copied().unwrap_or(0)]
            }
            (dest::NVM, _, size) => vec![0; usize::from(size)],
            other => panic!("unexpected read {other:?}"),
        })
    }

    fn bulk_out(&mut self, endpoint: u8, data: &[u8]) -> Result<(), UsbError> {
        assert_eq!(endpoint, 0x02);
        if data.len() == 372 || data.len() == 204 {
            self.settings.push(data.to_vec());
        } else {
            self.bitstream = Some(data.to_vec());
            self.fpga_configured = true;
        }
        Ok(())
    }

    fn bulk_in(&mut self, endpoint: u8, length: usize) -> Option<Result<Vec<u8>, UsbError>> {
        assert_eq!(endpoint, 0x86);
        let mut chunk = self.outgoing.pop_front()?;
        if chunk.len() > length {
            let rest = chunk.split_off(length);
            self.outgoing.push_front(rest);
        }
        Some(Ok(chunk))
    }
}

fn header(trigger: Option<u32>, remain: u64) -> Vec<u8> {
    let mut data = vec![0u8; HEADER_LEN];
    data[0..4].copy_from_slice(&0x5555_5555u32.to_le_bytes());
    data[4..8].copy_from_slice(&trigger.unwrap_or(0).to_le_bytes());
    data[12..16].copy_from_slice(&(remain as u32).to_le_bytes());
    data[16..20].copy_from_slice(&((remain >> 32) as u32).to_le_bytes());
    data[20..24].copy_from_slice(&u32::from(trigger.is_some()).to_le_bytes());
    data
}

fn firmware_dir(files: &[(&str, &[u8])]) -> FirmwareDir {
    let dir = std::env::temp_dir().join(format!(
        "hsync-dslogic-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for (name, contents) in files {
        std::fs::write(dir.join(name), contents).expect("firmware file");
    }
    FirmwareDir(dir)
}

fn open(fake: FakeDslogic, firmware: FirmwareDir) -> (MockTransport<FakeDslogic>, DslogicDevice) {
    let transport = MockTransport::new(fake);
    let ctl = Control::new(Arc::new(transport.clone())).without_read_delay();
    let profile = find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("DSLogic Plus");
    let device = DslogicDevice::open(ctl, profile, firmware, "test".into()).expect("open");
    (transport, device)
}

/// A DSLogic U3Pro16 on a USB 3.0 port.
fn open_u3pro16() -> DslogicDevice {
    open_u3pro16_with(configured(), UsbSpeed::Super).1
}

fn open_u3pro16_with(
    fake: FakeDslogic,
    speed: UsbSpeed,
) -> (MockTransport<FakeDslogic>, DslogicDevice) {
    let transport = MockTransport::new(fake);
    let ctl = Control::new(Arc::new(transport.clone())).without_read_delay();
    let profile = find_profile(VENDOR_ID, 0x002A, speed).expect("U3Pro16");
    let device = DslogicDevice::open(ctl, profile, firmware_dir(&[]), "test".into()).expect("open");
    (transport, device)
}

fn configured() -> FakeDslogic {
    FakeDslogic {
        fpga_configured: true,
        ..FakeDslogic::default()
    }
}

fn wrote(transport: &MockTransport<FakeDslogic>, target: u8, offset: u16, value: &[u8]) -> bool {
    transport.calls().iter().any(|call| {
        matches!(call, UsbCall::ControlOut { request: CMD_CTL_WR, data, .. }
            if data[0] == target && u16::from_le_bytes([data[1], data[2]]) == offset && &data[4..] == value)
    })
}

fn wait_for_force_stop(transport: &MockTransport<FakeDslogic>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !wrote(transport, dest::I2C_REG, reg::CTR0, &[ctr0::FORCE_STOP]) {
        assert!(Instant::now() < deadline, "the first stop starts uploading");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn assert_buffer_abort_sequence(transport: &MockTransport<FakeDslogic>) {
    let commands: Vec<_> = transport
        .calls()
        .iter()
        .filter_map(|call| match call {
            UsbCall::ControlOut {
                request: CMD_CTL_WR,
                data,
                ..
            } if data[0] == dest::STOP => Some((dest::STOP, Vec::new())),
            UsbCall::ControlOut {
                request: CMD_CTL_WR,
                data,
                ..
            } if data[0] == dest::I2C_REG
                && u16::from_le_bytes([data[1], data[2]]) == reg::CTR0
                && matches!(data[4], ctr0::FORCE_STOP | ctr0::FORCE_RDY) =>
            {
                Some((dest::I2C_REG, data[4..].to_vec()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        commands,
        vec![
            (dest::STOP, Vec::new()),
            (dest::I2C_REG, vec![ctr0::FORCE_STOP]),
            (dest::I2C_REG, vec![ctr0::FORCE_RDY]),
            (dest::STOP, Vec::new()),
        ],
        "STOP before arming, FORCE_STOP to upload, FORCE_RDY then STOP to abort"
    );
}

fn collect(packets: &Receiver<Packet>) -> Vec<Packet> {
    let mut all = Vec::new();
    loop {
        let packet = packets
            .recv_timeout(Duration::from_secs(5))
            .expect("the acquisition ends");
        let end = matches!(packet, Packet::End(_));
        all.push(packet);
        if end {
            return all;
        }
    }
}

fn logic_bytes(packets: &[Packet]) -> usize {
    packets
        .iter()
        .map(|packet| match packet {
            Packet::Logic(data) => data.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn opens_a_configured_device() {
    let (transport, device) = open(configured(), firmware_dir(&[]));
    let info = device.info();
    assert_eq!(info.model, "DSLogic Plus");
    assert_eq!(info.firmware.as_deref(), Some("2.0"));
    // 1.0 V threshold: 1.0 / 3.3 × 1.5 / 2.5 × 255.
    assert!(wrote(&transport, dest::I2C_REG, reg::VTH, &[46]));
    assert!(wrote(&transport, dest::I2C_REG, reg::CTR0, &[0]));
    assert_eq!(device.channel_count(), 16);
    assert_eq!(device.enabled_channels().len(), 16);
}

#[test]
fn configures_a_blank_fpga() {
    let bitstream: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
    let (transport, _device) = open(
        FakeDslogic::default(),
        firmware_dir(&[("DSLogicPlus.bin", &bitstream)]),
    );
    assert_eq!(
        transport.device().bitstream.as_deref(),
        Some(bitstream.as_slice())
    );
    assert!(wrote(&transport, dest::BULK_WR, 0, &[0xe8, 0x03, 0x00]));
    assert!(wrote(&transport, dest::PROG_B, 0, &[0xfb]));
    assert!(wrote(&transport, dest::LED, 0, &[0x01]));
}

#[test]
fn reports_a_missing_bitstream() {
    let transport = MockTransport::new(FakeDslogic::default());
    let ctl = Control::new(Arc::new(transport)).without_read_delay();
    let profile = find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("DSLogic Plus");
    let error = DslogicDevice::open(ctl, profile, firmware_dir(&[]), "test".into())
        .expect_err("no bitstream");
    assert!(
        matches!(error, hsync_driver::DriverError::FirmwareMissing(name) if name == "DSLogicPlus.bin")
    );
}

#[test]
fn lists_options_for_the_current_mode() {
    let (_transport, mut device) = open(configured(), firmware_dir(&[]));
    let options = device.options();
    let rates = options
        .iter()
        .find(|option| option.key == OptionKey::Samplerate)
        .expect("sample rate");
    let OptionKind::Integers { values } = &rates.kind else {
        panic!("sample rates are integers");
    };
    assert_eq!(values.last(), Some(&20_000_000));
    assert!(options.iter().any(|option| option.key == OptionKey::Vth));
    assert!(
        !options
            .iter()
            .any(|option| option.key == OptionKey::Threshold)
    );

    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    let options = device.options();
    let mode = options
        .iter()
        .find(|option| option.key == OptionKey::ChannelMode)
        .expect("channel mode");
    assert_eq!(mode.value, OptionValue::Choice("20".into()));
    device
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(100_000_000))
        .expect("100 MHz in buffer mode");
    assert!(
        device
            .set_option(OptionKey::Samplerate, &OptionValue::Integer(400_000_000))
            .is_err()
    );
}

#[test]
fn limits_channels_to_the_mode() {
    let (_transport, mut device) = open(configured(), firmware_dir(&[]));
    device
        .set_option(OptionKey::ChannelMode, &OptionValue::Choice("1".into()))
        .expect("12 channels at 25 MHz");
    assert_eq!(device.enabled_channels(), (0..12).collect::<Vec<_>>());
    assert!(device.set_channel_enabled(12, true).is_err());
    device.set_channel_enabled(0, false).expect("disable");
    device
        .set_channel_enabled(12, true)
        .expect("enable another");
    assert_eq!(device.enabled_channels().len(), 12);
}

/// `max_samplerate` and `channels`, `leading`, `span` of each tier.
fn tier_shapes(tiers: &[RateTier]) -> Vec<(u16, bool, u16, u64)> {
    tiers
        .iter()
        .map(|tier| {
            assert_eq!(tier.samplerates.last(), Some(&tier.max_samplerate));
            (tier.channels, tier.leading, tier.span, tier.max_samplerate)
        })
        .collect()
}

#[test]
fn offers_channel_modes_as_rate_tiers() {
    let mut device = open_u3pro16();
    assert_eq!(
        tier_shapes(&device.rate_tiers()),
        [
            (3, false, 8, 1_000_000_000),
            (6, false, 16, 500_000_000),
            (12, false, 16, 250_000_000),
            (16, true, 16, 125_000_000),
        ]
    );
    assert_eq!(device.rate_tiers()[3].samplerates.first(), Some(&1_000_000));
    assert_eq!(device.rate_tier(), Some(3));

    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    assert_eq!(
        tier_shapes(&device.rate_tiers()),
        [(8, true, 8, 1_000_000_000), (16, true, 16, 500_000_000)]
    );
    assert_eq!(device.rate_tier(), Some(1));
}

#[test]
fn selects_a_rate_tier_with_its_channels() {
    let mut device = open_u3pro16();
    device
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(100_000_000))
        .expect("100 MHz");
    device
        .select_rate_tier(0, &[1, 5, 7])
        .expect("3 of D0–D7 at up to 1 GHz");
    assert_eq!(device.rate_tier(), Some(0));
    assert_eq!(device.channel_count(), 8);
    assert_eq!(device.enabled_channels(), [1, 5, 7]);
    assert_eq!(device.settings().samplerate, 100_000_000);

    device
        .set_option(OptionKey::Samplerate, &OptionValue::Integer(1_000_000_000))
        .expect("1 GHz");
    device
        .select_rate_tier(3, &(0..16).collect::<Vec<_>>())
        .expect("all 16 channels");
    assert_eq!(device.enabled_channels().len(), 16);
    assert_eq!(device.settings().samplerate, 125_000_000);
}

#[test]
fn refuses_channels_that_do_not_fit_a_tier() {
    let mut device = open_u3pro16();
    device.select_rate_tier(1, &[0, 9]).expect("6-channel tier");
    let enabled = device.enabled_channels();
    for (index, channels) in [
        (0, vec![1, 9]),
        (1, (0..7).collect::<Vec<_>>()),
        (1, vec![16]),
        (4, vec![0]),
    ] {
        assert!(matches!(
            device.select_rate_tier(index, &channels),
            Err(DriverError::InvalidRequest(_))
        ));
        assert_eq!(device.rate_tier(), Some(1));
        assert_eq!(device.enabled_channels(), enabled);
    }
}

#[test]
fn sets_all_enabled_channels_within_the_mode() {
    let mut device = open_u3pro16();
    device
        .set_option(OptionKey::ChannelMode, &OptionValue::Choice("17".into()))
        .expect("12 channels at 250 MHz");
    device.set_enabled_channels(&[0, 15]).expect("two channels");
    assert_eq!(device.enabled_channels(), [0, 15]);
    assert!(
        device
            .set_enabled_channels(&(0..13).collect::<Vec<_>>())
            .is_err()
    );
    assert!(device.set_enabled_channels(&[16]).is_err());
    assert_eq!(device.enabled_channels(), [0, 15]);
    device.set_enabled_channels(&[]).expect("none");
    assert!(device.enabled_channels().is_empty());
}

#[test]
fn captures_in_stream_mode() {
    let mut fake = configured();
    let limit: u64 = 4096;
    let data_len = (limit / 64 * 16 * 8) as usize;
    fake.on_start = vec![header(Some(1234), 0), vec![0xa5; data_len + 1000]];
    let (transport, mut device) = open(fake, firmware_dir(&[]));
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
        .expect("limit");
    let (sink, packets) = crossbeam_channel::unbounded();
    let plan = CapturePlan {
        triggers: vec![(0, TriggerCondition::Rise)],
        trigger_position: 50,
        instant: false,
        advanced: None,
        looping: false,
    };
    let acquisition = device.start(plan.clone(), sink).expect("start");
    let packets = collect(&packets);
    acquisition.wait();

    assert_eq!(
        packets[0],
        Packet::Start {
            channels: (0..16).collect(),
            samplerate: 1_000_000,
            analog: Vec::new(),
        }
    );
    assert_eq!(packets[1], Packet::Trigger { sample: 1234 });
    assert_eq!(logic_bytes(&packets), data_len);
    assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));

    // Stream mode pins the trigger position to 1 %, as DSView's trigger dock does.
    let trigger = TriggerSetup::simple(&plan.triggers, 1, false);
    let expected = setting(&ArmConfig {
        profile: find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("profile"),
        mode: &CHANNEL_MODES[0],
        samplerate: 1_000_000,
        limit_samples: limit,
        enabled: 0xffff,
        filter: false,
        external_clock: false,
        clock_falling_edge: false,
        rle: false,
        looping: false,
        trigger: &trigger,
    });
    assert_eq!(transport.device().settings, vec![expected.to_vec()]);
    // A capture that ends by itself is left with FORCE_RDY, as DSView does.
    assert!(wrote(
        &transport,
        dest::I2C_REG,
        reg::CTR0,
        &[ctr0::FORCE_RDY]
    ));
    assert!(wrote(&transport, dest::WORDWIDE, 0, &[1]));
}

#[test]
fn instant_u3pro16_captures_ignore_triggers_without_the_dso_mode_bit() {
    let serial = AdvancedTrigger::Serial(SerialTriggerSpec {
        start: vec![(0, TriggerCondition::Rise)],
        stop: vec![(0, TriggerCondition::Fall)],
        clock: vec![(1, TriggerCondition::Rise)],
        data_channel: 2,
        bits: 8,
        value: 0xa5,
        mask: 0xff,
    });
    for speed in [UsbSpeed::High, UsbSpeed::Super] {
        for (mode, rate, mode_bits) in [
            (24, 500_000_000, 0x0020),
            (25, 1_000_000_000, 0x0040),
            (18, 500_000_000, 0x1020),
            (19, 1_000_000_000, 0x1040),
        ] {
            let stream = CHANNEL_MODES[mode].stream;
            if stream && speed == UsbSpeed::High {
                continue;
            }
            let mut triggers = vec![None];
            if !stream {
                triggers.extend([
                    Some(AdvancedTrigger::Stages {
                        stages: vec![stage()],
                    }),
                    Some(serial.clone()),
                ]);
            }
            for advanced in triggers {
                let channels: Vec<_> = (0..if stream { 3 } else { 4 }).collect();
                let limit = 1024;
                let data_len = limit as usize / 8 * channels.len();
                let mut trigger_header = header(None, 0);
                trigger_header.resize(1024, 0);
                let mut fake = configured();
                fake.on_start = vec![trigger_header, vec![0xa5; data_len]];
                let (transport, mut device) = open_u3pro16_with(fake, speed);
                device
                    .set_option(
                        OptionKey::OperationMode,
                        &OptionValue::Choice(if stream { "stream" } else { "buffer" }.into()),
                    )
                    .expect("operation mode");
                device
                    .set_option(
                        OptionKey::ChannelMode,
                        &OptionValue::Choice(mode.to_string()),
                    )
                    .expect("channel mode");
                device.set_enabled_channels(&channels).expect("channels");
                device
                    .set_option(OptionKey::Samplerate, &OptionValue::Integer(rate))
                    .expect("rate");
                device
                    .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
                    .expect("limit");
                let plan = CapturePlan {
                    triggers: vec![(0, TriggerCondition::Rise)],
                    trigger_position: 75,
                    instant: true,
                    advanced,
                    looping: false,
                };
                let (sink, packets) = crossbeam_channel::unbounded();
                let acquisition = device.start(plan, sink).expect("start");
                let packets = collect(&packets);
                acquisition.wait();
                assert_eq!(logic_bytes(&packets), data_len);
                assert_eq!(packets.last(), Some(&Packet::End(EndReason::Completed)));

                let expected = setting(&ArmConfig {
                    profile: find_profile(VENDOR_ID, 0x002A, speed).expect("profile"),
                    mode: &CHANNEL_MODES[mode],
                    samplerate: rate,
                    limit_samples: limit,
                    enabled: (1 << channels.len()) - 1,
                    filter: false,
                    external_clock: false,
                    clock_falling_edge: false,
                    rle: false,
                    looping: false,
                    trigger: &TriggerSetup::default(),
                });
                let fake = transport.device();
                let actual = &fake.settings[0];
                // DSView keeps bit 9 clear in LOGIC mode, even for instant captures.
                assert_eq!(u16::from_le_bytes([actual[6], actual[7]]), mode_bits);
                assert_eq!(actual, &expected, "{speed:?}, channel mode {mode}");
            }
        }
    }
}

#[test]
fn stopping_a_buffer_capture_uploads_what_it_holds() {
    let mut fake = configured();
    let limit: u64 = 1 << 20;
    let captured: u64 = 2048;
    fake.progress = [0, 0, 0x08, 0];
    fake.on_force_stop = vec![
        header(None, limit - captured),
        vec![0x5a; (captured / 64 * 16 * 8) as usize],
    ];
    let (transport, mut device) = open(fake, firmware_dir(&[]));
    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
        .expect("limit");
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(CapturePlan::default(), sink).expect("start");
    // Wait for a progress report, then stop.
    loop {
        match packets
            .recv_timeout(Duration::from_secs(5))
            .expect("progress")
        {
            Packet::Progress {
                triggered,
                captured,
            } => {
                assert!(!triggered);
                assert_eq!(captured, 0x08 << 16);
                break;
            }
            Packet::Start { .. } => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    acquisition.stop();
    let rest = collect(&packets);
    acquisition.wait();
    assert_eq!(logic_bytes(&rest), (captured / 64 * 16 * 8) as usize);
    assert_eq!(rest.last(), Some(&Packet::End(EndReason::Stopped)));
    assert!(wrote(
        &transport,
        dest::I2C_REG,
        reg::CTR0,
        &[ctr0::FORCE_STOP]
    ));
}

#[test]
fn stopping_a_buffer_upload_again_aborts_before_or_after_the_header() {
    for speed in [UsbSpeed::High, UsbSpeed::Super] {
        for partial_upload in [false, true] {
            let limit = 1 << 20;
            let received = vec![0x5a; 1024];
            let mut fake = configured();
            if partial_upload {
                let mut trigger_header = header(None, limit - 2048);
                trigger_header.resize(1024, 0);
                // The header promises 4096 bytes, but the device stalls after 1024.
                fake.on_force_stop = vec![trigger_header, received.clone()];
            }
            let (transport, mut device) = open_u3pro16_with(fake, speed);
            device
                .set_option(
                    OptionKey::OperationMode,
                    &OptionValue::Choice("buffer".into()),
                )
                .expect("buffer mode");
            device
                .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
                .expect("limit");
            let (sink, packets) = crossbeam_channel::unbounded();
            let acquisition = device.start(CapturePlan::default(), sink).expect("start");
            acquisition.stop();
            wait_for_force_stop(&transport);
            if partial_upload {
                loop {
                    match packets
                        .recv_timeout(Duration::from_secs(5))
                        .expect("partial upload")
                    {
                        Packet::Logic(data) => {
                            assert_eq!(data, received);
                            break;
                        }
                        Packet::Start { .. } | Packet::Progress { .. } => {}
                        other => panic!("unexpected {other:?}"),
                    }
                }
            }
            acquisition.stop();
            let rest = collect(&packets);
            acquisition.wait();
            assert_eq!(rest.last(), Some(&Packet::End(EndReason::Stopped)));
            assert_eq!(logic_bytes(&rest), 0);
            assert_buffer_abort_sequence(&transport);
            device
                .set_option(OptionKey::Filter, &OptionValue::Bool(true))
                .expect("options unlock after aborting the upload");
        }
    }
}

#[test]
fn repeated_stop_requests_do_not_get_lost_or_restart_buffer_uploads() {
    let (transport, mut device) = open(configured(), firmware_dir(&[]));
    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(CapturePlan::default(), sink).expect("start");
    // Back-to-back requests can arrive before the worker has handled the first one.
    for _ in 0..300 {
        acquisition.stop();
    }
    let rest = collect(&packets);
    acquisition.wait();
    assert_eq!(rest.last(), Some(&Packet::End(EndReason::Stopped)));
    assert_buffer_abort_sequence(&transport);
}

#[test]
fn dropping_an_acquisition_aborts_a_stalled_buffer_upload() {
    let (transport, mut device) = open(configured(), firmware_dir(&[]));
    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(CapturePlan::default(), sink).expect("start");
    acquisition.stop();
    wait_for_force_stop(&transport);
    drop(acquisition);
    let rest = collect(&packets);
    assert_eq!(rest.last(), Some(&Packet::End(EndReason::Stopped)));
    assert_buffer_abort_sequence(&transport);
}

#[test]
fn stopping_a_stream_capture_discards_and_stops_the_device() {
    let (transport, mut device) = open(configured(), firmware_dir(&[]));
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(CapturePlan::default(), sink).expect("start");
    assert!(matches!(
        packets.recv_timeout(Duration::from_secs(5)),
        Ok(Packet::Start { .. })
    ));
    assert!(
        device
            .set_option(OptionKey::Filter, &OptionValue::Bool(true))
            .is_err(),
        "options are locked while capturing"
    );
    acquisition.stop();
    let rest = collect(&packets);
    acquisition.wait();
    assert_eq!(rest.last(), Some(&Packet::End(EndReason::Stopped)));
    assert!(wrote(
        &transport,
        dest::I2C_REG,
        reg::CTR0,
        &[ctr0::FORCE_RDY]
    ));
    let stops = transport
        .calls()
        .iter()
        .filter(|call| matches!(call, UsbCall::ControlOut { request: CMD_CTL_WR, data, .. } if data[0] == dest::STOP))
        .count();
    assert_eq!(stops, 2, "STOP before arming and after the stopped capture");
    device
        .set_option(OptionKey::Filter, &OptionValue::Bool(true))
        .expect("options unlock after the capture");
}

#[test]
fn loops_past_the_limit_in_stream_mode() {
    let mut fake = configured();
    let limit: u64 = 4096;
    let limit_bytes = (limit / 64 * 16 * 8) as usize;
    fake.on_start = vec![header(None, 0), vec![0x3c; 3 * limit_bytes]];
    let (transport, mut device) = open(fake, firmware_dir(&[]));
    assert!(device.can_loop());
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
        .expect("limit");
    let (sink, packets) = crossbeam_channel::unbounded();
    let plan = CapturePlan {
        looping: true,
        ..CapturePlan::default()
    };
    let acquisition = device.start(plan.clone(), sink).expect("start");
    // Everything the device sends is forwarded, three times the limit, until stopped.
    let mut forwarded = 0;
    while forwarded < 3 * limit_bytes {
        match packets.recv_timeout(Duration::from_secs(5)).expect("data") {
            Packet::Logic(data) => forwarded += data.len(),
            Packet::End(reason) => panic!("ended by itself: {reason:?}"),
            _ => {}
        }
    }
    acquisition.stop();
    let rest = collect(&packets);
    acquisition.wait();
    assert_eq!(rest.last(), Some(&Packet::End(EndReason::Stopped)));
    assert_eq!(forwarded + logic_bytes(&rest), 3 * limit_bytes);

    // The FPGA is told to loop: DSView sets the analog mode bit for it.
    let trigger = TriggerSetup::simple(&[], 1, false);
    let expected = setting(&ArmConfig {
        profile: find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("profile"),
        mode: &CHANNEL_MODES[0],
        samplerate: 1_000_000,
        limit_samples: limit,
        enabled: 0xffff,
        filter: false,
        external_clock: false,
        clock_falling_edge: false,
        rle: false,
        looping: true,
        trigger: &trigger,
    });
    assert_eq!(transport.device().settings, vec![expected.to_vec()]);
}

#[test]
fn refuses_to_loop_in_buffer_mode() {
    let (_, mut device) = open(configured(), firmware_dir(&[]));
    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    assert!(!device.can_loop());
    let (sink, _packets) = crossbeam_channel::unbounded();
    let plan = CapturePlan {
        looping: true,
        ..CapturePlan::default()
    };
    assert!(matches!(
        device.start(plan, sink),
        Err(DriverError::InvalidRequest(_))
    ));
}

#[test]
fn reports_what_the_trigger_can_do_in_each_mode() {
    let (_, mut device) = open(configured(), firmware_dir(&[]));
    // Stream mode keeps nothing in the device; Buffer mode holds its memory's share per channel.
    assert_eq!(device.hardware_depth(), None);
    assert_eq!(
        device.trigger_caps(),
        TriggerCaps {
            max_position: 1,
            advanced: false,
            advanced_other_mode: true
        }
    );
    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(1 << 20))
        .expect("limit");
    assert_eq!(
        device.trigger_caps(),
        TriggerCaps {
            max_position: 90,
            advanced: true,
            advanced_other_mode: false
        }
    );
    let profile = find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("profile");
    assert_eq!(
        device.hardware_depth(),
        Some(hsync_dslogic::setting::channel_depth(
            profile,
            device.enabled_channels().len() as u32
        ))
    );
}

#[test]
fn refuses_multi_stage_triggers_in_stream_mode() {
    let (_, mut device) = open(configured(), firmware_dir(&[]));
    let (sink, _packets) = crossbeam_channel::unbounded();
    let plan = CapturePlan {
        advanced: Some(AdvancedTrigger::Stages {
            stages: vec![stage()],
        }),
        ..CapturePlan::default()
    };
    assert!(matches!(
        device.start(plan, sink),
        Err(DriverError::InvalidRequest(_))
    ));
}

fn stage() -> TriggerStageSpec {
    TriggerStageSpec {
        pattern0: vec![(0, TriggerCondition::Rise), (1, TriggerCondition::High)],
        pattern1: Vec::new(),
        both: true,
        invert0: false,
        invert1: false,
        count: 2,
        contiguous: false,
    }
}

#[test]
fn arms_multi_stage_triggers_in_buffer_mode() {
    let mut fake = configured();
    let limit: u64 = 1 << 20;
    fake.progress = [0, 0, 0x08, 0];
    fake.on_force_stop = vec![header(None, limit), Vec::new()];
    let (transport, mut device) = open(fake, firmware_dir(&[]));
    device
        .set_option(
            OptionKey::OperationMode,
            &OptionValue::Choice("buffer".into()),
        )
        .expect("buffer mode");
    device
        .set_option(OptionKey::SampleLimit, &OptionValue::Integer(limit))
        .expect("limit");
    let advanced = AdvancedTrigger::Stages {
        stages: vec![stage(), stage()],
    };
    let plan = CapturePlan {
        trigger_position: 30,
        advanced: Some(advanced.clone()),
        ..CapturePlan::default()
    };
    let (sink, packets) = crossbeam_channel::unbounded();
    let acquisition = device.start(plan, sink).expect("start");
    acquisition.stop();
    let _ = collect(&packets);
    acquisition.wait();

    let trigger = TriggerSetup::advanced(&advanced, 30);
    let expected = setting(&ArmConfig {
        profile: find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("profile"),
        // The first Buffer mode: 16 channels up to 100 MHz.
        mode: &CHANNEL_MODES[20],
        samplerate: 1_000_000,
        limit_samples: limit,
        enabled: 0xffff,
        filter: false,
        external_clock: false,
        clock_falling_edge: false,
        rle: false,
        looping: false,
        trigger: &trigger,
    });
    assert_eq!(transport.device().settings, vec![expected.to_vec()]);
}
