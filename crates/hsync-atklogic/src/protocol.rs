//! The USB protocol of ATK-Logic devices, from the ATK-Logic application (`pv/usb/usb_base.cpp`,
//! `pv/usb/usb_control.cpp`, `pv/data/analysis.cpp`, `pv/static/util.cpp`): commands to the
//! device's MCU and to the FPGA behind it, the lane order of the USB data, and the frames the
//! FPGA answers with.

use std::sync::Arc;
use std::time::Duration;

use hsync_core::{ChannelIndex, TriggerCondition};
use hsync_driver::usb::{UsbError, UsbTransport};

/// USB vendor id (`VENDOR_ID`).
pub const VENDOR_ID: u16 = 0x1a86;
/// USB product id (`PRODUCT_ID`).
pub const PRODUCT_ID: u16 = 0xffcc;
/// Bulk OUT endpoint of every command (`TO_MCU_EP`).
pub const EP_OUT: u8 = 0x02;
/// Bulk IN endpoint of answers and samples (`TO_PC_EP`).
pub const EP_IN: u8 = 0x81;

/// Bytes of the blocks whose 16-bit words the device spreads over four lanes of 512 bytes
/// (`logic_analyzer_convert_to_pc()`).
pub const BLOCK: usize = 2048;
/// Bytes of an MCU command and of its answer.
pub const MCU_PACKET: usize = 512;

/// Commands the MCU answers itself: `0x0a`, the code and its arguments, in 512 bytes.
pub mod mcu {
    /// First byte of MCU commands and answers.
    pub const HEAD: u8 = 0x0a;
    /// `GetMCUVersion()`.
    pub const VERSION: u8 = 0x81;
    /// `SetResetState()`: 1 wakes the FPGA, 0 lets it sleep.
    pub const RESET_STATE: u8 = 0x87;
    /// Byte 3 of the answer to [`VERSION`] while the application runs.
    pub const MODE_APP: u8 = 0x61;
    /// Byte 3 of the answer to [`VERSION`] while the bootloader waits for new firmware.
    pub const MODE_BOOTLOADER: u8 = 0x62;
}

/// Commands to the FPGA (`USBControl::Write(code, …)`).
pub mod cmd {
    /// `GetDeviceData()`: the FPGA answers with a [`super::frame::INFO`] frame.
    pub const DEVICE_INFO: u8 = 0x10;
    /// `ParameterSetting()`: mode, threshold, sample rate and depths
    /// ([`super::settings_args`]).
    pub const SETTINGS: u8 = 0x11;
    /// `SimpleTrigger()`: channels and trigger conditions ([`super::trigger_args`]); starts
    /// the capture.
    pub const TRIGGER: u8 = 0x12;
    /// `Stop()`.
    pub const STOP: u8 = 0x15;
    /// `PWM()`.
    pub const PWM: u8 = 0x17;
    /// `Exit()`: the host lets go of the device.
    pub const EXIT: u8 = 0x18;
}

/// Arguments of [`cmd::PWM`]. HSync drives no waveform outputs, but switches both off as
/// ATK-Logic does on connecting and on leaving (`CloseAllPWM()`).
pub mod pwm {
    /// Switches PWM1 off.
    pub const PWM1_OFF: u8 = 0x20;
    /// Switches PWM0 off.
    pub const PWM0_OFF: u8 = 0x10;
}

/// Kinds of frames the FPGA sends (`AnalysisData::order`).
pub mod frame {
    /// Samples of one channel: its number (two bytes), then its bits, eight samples a byte.
    pub const DATA: u8 = 1;
    /// What the device is ([`super::DeviceInfo`]).
    pub const INFO: u8 = 2;
    /// Where the trigger is and, in Buffer mode, how much each channel holds.
    pub const OFFSET: u8 = 3;
    /// Acknowledges a command.
    pub const REPLY: u8 = 4;
    /// How far the capture has come.
    pub const PROGRESS: u8 = 5;
    /// The capture is over.
    pub const DONE: u8 = 6;
}

/// Bytes before an FPGA frame in a command (`Write()`'s `offset`); the MCU passes on what does
/// not start like one of its own commands.
const COMMAND_PREFIX: usize = 8;
/// First byte of a frame.
const FRAME_HEAD: u8 = 0x0a;
/// Last byte of a command frame.
const COMMAND_TAIL: u8 = 0x0b;
/// The two bytes that end a frame from the FPGA (`analysis_check_data_bottom()`).
const FRAME_TAIL: [u8; 2] = [0x00, 0x0b];
/// Header bytes of a frame from the FPGA: head, kind and the payload length.
const FRAME_HEADER: usize = 4;

/// `gCRC32()`: the reflected CRC-32 polynomial, but started from 0 rather than all ones, then
/// inverted. Empty data gives all ones.
pub fn crc32(data: &[u8]) -> u32 {
    if data.is_empty() {
        return u32::MAX;
    }
    let mut crc = 0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// `logic_analyzer_convert_to_device()`: in every block, 16-bit word `4j + l` becomes word `j`
/// of lane `l`, the lanes following each other. `data` must be whole blocks.
pub fn to_lanes(data: &[u8]) -> Vec<u8> {
    debug_assert!(data.len().is_multiple_of(BLOCK));
    let mut out = Vec::with_capacity(data.len());
    let (blocks, _) = data.as_chunks::<BLOCK>();
    for block in blocks {
        let (words, _) = block.as_chunks::<2>();
        for lane in 0..4 {
            for word in words.iter().skip(lane).step_by(4) {
                out.extend_from_slice(word);
            }
        }
    }
    out
}

/// `logic_analyzer_convert_to_pc()`: puts one block back in order, appending it to `out`.
pub fn from_lanes(block: &[u8; BLOCK], out: &mut Vec<u8>) {
    let (words, _) = block.as_chunks::<2>();
    let lane = BLOCK / 8;
    out.reserve(BLOCK);
    for word in 0..lane {
        for first in [0, lane, 2 * lane, 3 * lane] {
            out.extend_from_slice(&words[first + word]);
        }
    }
}

/// FPGA command `code` with `args` as it goes over USB (`USBControl::Write()` and
/// `SendToDevice()`): eight zero bytes, `0x0a`, the code, the argument count plus one, the
/// arguments, `0x0b` and the CRC-32 of code, count and arguments (little-endian), zero-padded to
/// whole blocks and put in lane order.
///
/// # Panics
///
/// Panics with more than 254 argument bytes, which no command has.
pub fn command(code: u8, args: &[u8]) -> Vec<u8> {
    let count = u8::try_from(args.len() + 1)
        .ok()
        .filter(|&count| count < u8::MAX)
        .expect("command arguments fit in a byte");
    let mut body = Vec::with_capacity(args.len() + 2);
    body.extend_from_slice(&[code, count]);
    body.extend_from_slice(args);
    let mut frame = vec![0; COMMAND_PREFIX];
    frame.push(FRAME_HEAD);
    frame.extend_from_slice(&body);
    frame.push(COMMAND_TAIL);
    frame.extend_from_slice(&crc32(&body).to_le_bytes());
    frame.resize(frame.len().next_multiple_of(BLOCK), 0);
    to_lanes(&frame)
}

/// MCU command `code` with `args` (`SendToMCU()`). ATK-Logic sends the rest of an
/// uninitialised stack buffer after the arguments; HSync sends zeros.
pub fn mcu_command(code: u8, args: &[u8]) -> Vec<u8> {
    let mut packet = vec![0; MCU_PACKET];
    packet[0] = mcu::HEAD;
    packet[1] = code;
    packet[2..2 + args.len()].copy_from_slice(args);
    packet
}

/// A 40-bit little-endian number (`intToBytes(value, 5)`); missing bytes count as zero.
pub fn u40(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .take(5)
        .rev()
        .fold(0, |value, &byte| (value << 8) | u64::from(byte))
}

fn put_u40(out: &mut [u8], value: u64) {
    out[..5].copy_from_slice(&value.to_le_bytes()[..5]);
}

/// What [`cmd::SETTINGS`] carries (`SessionController::start()`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureSettings {
    /// Buffer rather than Stream mode.
    pub buffer: bool,
    /// Run-length compression, for Buffer captures longer than the memory.
    pub rle: bool,
    /// Threshold voltage, −5 to 5 V in steps of 0.1 V.
    pub vth: f64,
    /// Sample rate code ([`crate::models::rate_code`]).
    pub rate_code: u8,
    /// Samples per channel.
    pub depth: u64,
    /// Samples before the trigger.
    pub pre_trigger: u64,
}

/// The arguments of [`cmd::SETTINGS`]: the mode bits (`0x80` Buffer, `0x40` RLE), the
/// threshold in tenths of a volt with `0x80` for a negative one, the rate code, then the depth
/// and the samples before the trigger as 40-bit numbers.
pub fn settings_args(settings: &CaptureSettings) -> [u8; 13] {
    let mut args = [0; 13];
    if settings.buffer {
        args[0] |= 0x80;
    }
    if settings.rle {
        args[0] |= 0x40;
    }
    // Qt's qRound() rounds halves up; the sign comes from the voltage, not the rounded value.
    let tenths = (settings.vth * 10.0 + 0.5).floor().abs().min(127.0) as u8;
    args[1] = tenths | if settings.vth < 0.0 { 0x80 } else { 0 };
    args[2] = settings.rate_code;
    put_u40(&mut args[3..8], settings.depth);
    put_u40(&mut args[8..13], settings.pre_trigger);
    args
}

/// The bits of a trigger condition in its channel's nibble: `0x1` rise, `0x2` fall, `0x4` high;
/// low is none of them, and no condition all three.
const fn condition_bits(condition: TriggerCondition) -> u8 {
    match condition {
        TriggerCondition::None => 0x7,
        TriggerCondition::Rise => 0x1,
        TriggerCondition::Fall => 0x2,
        TriggerCondition::Both => 0x3,
        TriggerCondition::High => 0x4,
        TriggerCondition::Low => 0x0,
    }
}

/// The arguments of [`cmd::TRIGGER`]: a nibble for each of the 16 channels, channel 0 in the
/// high nibble of the first byte, holding `0x8` when the channel is captured plus its condition
/// bits; then 1 to start at once, ignoring the conditions.
pub fn trigger_args(
    enabled: u16,
    conditions: &[(ChannelIndex, TriggerCondition)],
    instant: bool,
) -> [u8; 9] {
    let mut args = [0; 9];
    for channel in 0..16u16 {
        if enabled & (1 << channel) == 0 {
            continue;
        }
        let condition = conditions
            .iter()
            .rev()
            .find(|(index, _)| *index == channel)
            .map_or(TriggerCondition::None, |(_, condition)| *condition);
        let nibble = 0x8 | condition_bits(condition);
        let shift = if channel % 2 == 0 { 4 } else { 0 };
        args[usize::from(channel / 2)] |= nibble << shift;
    }
    args[8] = u8::from(instant);
    args
}

/// A frame from the FPGA: `0x0a`, its kind, the payload length (16-bit little-endian), the
/// payload and `0x00 0x0b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// One of [`frame`].
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// Cuts what the device sends into frames (`Analysis`). USB data goes in as it arrives; whole
/// blocks are put back in order, and frames come out with the bytes between them skipped.
///
/// ATK-Logic puts each USB transfer back in order on its own and drops what is short of a
/// block; this reader carries such bytes over to the next transfer. Likewise, a frame cut off
/// at the end waits for the rest, where ATK-Logic searches its payload for further frames.
#[derive(Debug, Clone, Default)]
pub struct FrameReader {
    /// USB bytes short of a whole block.
    partial: Vec<u8>,
    /// Bytes in order; those before `start` are read.
    data: Vec<u8>,
    start: usize,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds data the device sent, in lane order.
    pub fn push(&mut self, mut usb: &[u8]) {
        self.compact();
        if !self.partial.is_empty() {
            let take = usb.len().min(BLOCK - self.partial.len());
            self.partial.extend_from_slice(&usb[..take]);
            usb = &usb[take..];
            let Some(block) = self.partial.as_chunks::<BLOCK>().0.first() else {
                return;
            };
            from_lanes(block, &mut self.data);
            self.partial.clear();
        }
        let (blocks, rest) = usb.as_chunks::<BLOCK>();
        for block in blocks {
            from_lanes(block, &mut self.data);
        }
        self.partial.extend_from_slice(rest);
    }

    /// Drops the bytes already read once they are most of the buffer.
    fn compact(&mut self) {
        if self.start > 0 && self.start * 2 >= self.data.len() {
            self.data.drain(..self.start);
            self.start = 0;
        }
    }

    /// The next whole frame, if one has arrived.
    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            let found = self.data[self.start..]
                .iter()
                .position(|&byte| byte == FRAME_HEAD);
            let Some(found) = found else {
                self.start = self.data.len();
                return None;
            };
            let at = self.start + found;
            let rest = &self.data[at..];
            if rest.len() < 2 {
                self.start = at;
                return None;
            }
            // `analysis_check_data_header()`: kinds 1 to 6 only.
            if !(frame::DATA..=frame::DONE).contains(&rest[1]) {
                self.start = at + 1;
                continue;
            }
            if rest.len() < FRAME_HEADER {
                self.start = at;
                return None;
            }
            let len = usize::from(u16::from_le_bytes([rest[2], rest[3]]));
            // `analysis_get_data()` takes no empty frames.
            if len == 0 {
                self.start = at + 1;
                continue;
            }
            let end = FRAME_HEADER + len + FRAME_TAIL.len();
            if rest.len() < end {
                self.start = at;
                return None;
            }
            if rest[FRAME_HEADER + len..end] != FRAME_TAIL {
                self.start = at + 1;
                continue;
            }
            let frame = Frame {
                kind: rest[1],
                payload: rest[FRAME_HEADER..FRAME_HEADER + len].to_vec(),
            };
            self.start = at + end;
            return Some(frame);
        }
    }
}

/// The MCU's answer to [`mcu::VERSION`] (`ConnectDevice::CheckDeviceCreanInfo()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McuVersion {
    /// The bootloader runs, waiting for a firmware update, rather than the application.
    pub bootloader: bool,
    pub major: u8,
    pub minor: u8,
    /// Hardware revision (`deviceVersion`); 0 is not a valid one.
    pub hardware: u8,
    /// 1 for the Plus models (`level`).
    pub level: u8,
}

impl McuVersion {
    /// Reads `0a 81 01 <mode> <major> <minor> <hardware> _ <level>`; other answers give `None`.
    pub fn parse(answer: &[u8]) -> Option<Self> {
        let answer = answer.get(..9)?;
        if answer[..3] != [mcu::HEAD, mcu::VERSION, 0x01] {
            return None;
        }
        let bootloader = match answer[3] {
            mcu::MODE_APP => false,
            mcu::MODE_BOOTLOADER => true,
            _ => return None,
        };
        Some(Self {
            bootloader,
            major: answer[4],
            minor: answer[5],
            hardware: answer[6],
            level: answer[8],
        })
    }
}

/// What the FPGA says about the device in an [`frame::INFO`] frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The FPGA works.
    pub ok: bool,
    /// USB generation of the link: 2 or 3.
    pub usb: u8,
    /// FPGA logic version, major and minor (`fpgaVersions = major · 100 + minor`).
    pub fpga: (u8, u8),
    /// Oldest ATK-Logic version that may drive this device, e.g. `(10, 66)` for 1.0.6.6.
    pub min_app: (u8, u8),
    /// Model name, e.g. `DL16`.
    pub name: String,
}

impl DeviceInfo {
    /// Reads `_ _ <ok> <usb> _ <fpga major> <fpga minor> <app major> <app minor> <name…>`.
    pub fn parse(payload: &[u8]) -> Option<Self> {
        let head = payload.get(..9)?;
        let name = &payload[9..];
        let name = name
            .iter()
            .position(|&byte| byte == 0)
            .map_or(name, |end| &name[..end]);
        Some(Self {
            ok: head[2] == 1,
            usb: head[3],
            fpga: (head[5], head[6]),
            min_app: (head[7], head[8]),
            name: String::from_utf8_lossy(name).trim().to_owned(),
        })
    }

    /// `fpgaVersions`.
    pub fn fpga_version(&self) -> u32 {
        u32::from(self.fpga.0) * 100 + u32::from(self.fpga.1)
    }

    /// `minVersion`.
    pub fn min_app_version(&self) -> u32 {
        u32::from(self.min_app.0) * 100 + u32::from(self.min_app.1)
    }
}

/// How long the exchanges with a device wait; tests run without the pauses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long a single read waits for data (`ReadSynchronous()`).
    pub read: Duration,
    /// How long a capture transfer may take to fill before it is cut short with what it holds.
    pub transfer: Duration,
    /// How long a capture may go without a frame from the device before it fails: ATK-Logic
    /// counts 20 idle rounds of 100 ms (`missCount`).
    pub silence: Duration,
    /// Pause before talking to a device just opened, and between connection attempts.
    pub settle: Duration,
    /// Pause between commands the FPGA needs time for.
    pub gap: Duration,
    /// Pause between reads that find the MCU in a bad state, or still busy.
    pub poll: Duration,
}

impl Timing {
    /// ATK-Logic's.
    pub const DEVICE: Self = Self {
        read: Duration::from_millis(50),
        transfer: Duration::from_millis(1200),
        silence: Duration::from_secs(2),
        settle: Duration::from_millis(300),
        gap: Duration::from_millis(30),
        poll: Duration::from_millis(10),
    };

    /// No pauses and short transfer timeouts, for simulated devices.
    pub const IMMEDIATE: Self = Self {
        read: Duration::ZERO,
        transfer: Duration::from_millis(5),
        silence: Duration::from_secs(1),
        settle: Duration::ZERO,
        gap: Duration::ZERO,
        poll: Duration::ZERO,
    };
}

/// How long a command may take to go out. ATK-Logic submits commands with 100 ms and does not
/// wait for them; HSync waits, and gives a busy device longer.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

/// Commands and reads over an opened device.
#[derive(Clone)]
pub struct Link {
    usb: Arc<dyn UsbTransport>,
    timing: Timing,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("timing", &self.timing)
            .finish_non_exhaustive()
    }
}

impl Link {
    pub fn new(usb: Arc<dyn UsbTransport>) -> Self {
        Self {
            usb,
            timing: Timing::DEVICE,
        }
    }

    /// The same link with other timing.
    #[must_use]
    pub fn with_timing(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// The USB transport.
    pub fn usb(&self) -> &Arc<dyn UsbTransport> {
        &self.usb
    }

    /// Sends an MCU command (`SendToMCU()`).
    pub fn mcu(&self, code: u8, args: &[u8]) -> Result<(), UsbError> {
        self.usb
            .bulk_out(EP_OUT, &mcu_command(code, args), WRITE_TIMEOUT)
    }

    /// Sends an FPGA command (`Write(code, …)`).
    pub fn command(&self, code: u8, args: &[u8]) -> Result<(), UsbError> {
        self.usb
            .bulk_out(EP_OUT, &command(code, args), WRITE_TIMEOUT)
    }

    /// Reads what arrives within the read timeout, at most `length` bytes, possibly none
    /// (`ReadSynchronous()`, without its reordering of whole blocks).
    pub fn read(&self, length: usize) -> Result<Vec<u8>, UsbError> {
        self.usb.bulk_read(EP_IN, length, self.timing.read)
    }

    /// Reads and drops what the device still has to send, at most `limit` times
    /// (`while (usb->ReadSynchronous(data))`). Returns the number of reads that found data.
    pub fn drain(&self, limit: usize) -> Result<usize, UsbError> {
        for count in 0..limit {
            if self.read(MCU_PACKET)?.is_empty() {
                return Ok(count);
            }
        }
        Ok(limit)
    }

    /// Waits `duration`, if any.
    pub fn pause(&self, duration: Duration) {
        if !duration.is_zero() {
            std::thread::sleep(duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![FRAME_HEAD, kind];
        bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(&FRAME_TAIL);
        bytes
    }

    /// `bytes` as the device sends them: zero-padded to whole blocks in lane order.
    fn usb(bytes: &[u8]) -> Vec<u8> {
        let mut padded = bytes.to_vec();
        padded.resize(padded.len().next_multiple_of(BLOCK), 0);
        to_lanes(&padded)
    }

    fn frames(reader: &mut FrameReader) -> Vec<Frame> {
        std::iter::from_fn(|| reader.next_frame()).collect()
    }

    #[test]
    fn computes_atk_logic_crc() {
        assert_eq!(crc32(&[]), 0xffff_ffff);
        // Started from 0, the CRC of a zero byte is 0, inverted.
        assert_eq!(crc32(&[0]), 0xffff_ffff);
        // The first entries of `table[]` in util.cpp, inverted.
        assert_eq!(crc32(&[1]), !0x7707_3096);
        assert_eq!(crc32(&[2]), !0xee0e_612c);
    }

    #[test]
    fn lanes_round_trip() {
        let data: Vec<u8> = (0..BLOCK * 2).map(|i| (i * 7 + i / 256) as u8).collect();
        let lanes = to_lanes(&data);
        // Word 1 of the block goes to the front of lane 1, word 4 next to word 0.
        assert_eq!(&lanes[..2], &data[..2]);
        assert_eq!(&lanes[2..4], &data[8..10]);
        assert_eq!(&lanes[512..514], &data[2..4]);
        assert_eq!(
            &lanes[BLOCK + 1536..BLOCK + 1538],
            &data[BLOCK + 6..BLOCK + 8]
        );
        let mut back = Vec::new();
        for block in lanes.as_chunks::<BLOCK>().0 {
            from_lanes(block, &mut back);
        }
        assert_eq!(back, data);
    }

    #[test]
    fn frames_commands() {
        let bytes = command(cmd::STOP, &[]);
        assert_eq!(bytes.len(), BLOCK);
        let mut ordered = Vec::new();
        from_lanes(&bytes.as_chunks::<BLOCK>().0[0], &mut ordered);
        let crc = crc32(&[cmd::STOP, 1]).to_le_bytes();
        assert_eq!(
            &ordered[..16],
            &[
                0, 0, 0, 0, 0, 0, 0, 0, 0x0a, 0x15, 0x01, 0x0b, crc[0], crc[1], crc[2], crc[3]
            ]
        );
        assert!(ordered[16..].iter().all(|&byte| byte == 0));
        // The MCU tells commands for the FPGA by their start: two zero bytes, then 0x0a.
        assert_eq!(&bytes[..4], &[0, 0, 0x0a, 0x15]);

        let packet = mcu_command(mcu::RESET_STATE, &[1]);
        assert_eq!(packet.len(), MCU_PACKET);
        assert_eq!(&packet[..4], &[0x0a, 0x87, 0x01, 0x00]);
    }

    fn negative_settings() -> CaptureSettings {
        CaptureSettings {
            buffer: false,
            rle: false,
            vth: -2.5,
            rate_code: 1,
            depth: 1024,
            pre_trigger: 10,
        }
    }

    #[test]
    fn encodes_capture_settings() {
        let args = settings_args(&CaptureSettings {
            buffer: true,
            rle: true,
            vth: 1.6,
            rate_code: 12,
            depth: 0x12_3456_789a,
            pre_trigger: 1000,
        });
        assert_eq!(
            args,
            [
                0xc0, 16, 12, 0x9a, 0x78, 0x56, 0x34, 0x12, 0xe8, 0x03, 0, 0, 0
            ]
        );
        let negative = settings_args(&negative_settings());
        // qRound(-25.0) is -25; the sign bit comes on top.
        assert_eq!(negative[..3], [0x00, 0x80 | 25, 1]);
        // qRound(-2.5) rounds up to -2.
        let half = settings_args(&CaptureSettings {
            vth: -0.25,
            ..negative_settings()
        });
        assert_eq!(half[1], 0x80 | 2);
    }

    #[test]
    fn encodes_trigger_nibbles() {
        let args = trigger_args(
            0b1000_0000_0000_0111,
            &[
                (0, TriggerCondition::Rise),
                (1, TriggerCondition::Low),
                (5, TriggerCondition::High),
                (15, TriggerCondition::Both),
            ],
            false,
        );
        // Channel 0 rising, 1 low, 2 without a condition; 5 is not captured; 15 any edge.
        assert_eq!(args, [0x98, 0xf0, 0, 0, 0, 0, 0, 0x0b, 0]);
        assert_eq!(trigger_args(0xffff, &[], true)[8], 1);
        assert_eq!(trigger_args(0xffff, &[], true)[..8], [0xff; 8]);
        assert_eq!(
            trigger_args(0x0002, &[(1, TriggerCondition::Fall)], false)[0],
            0x0a
        );
    }

    #[test]
    fn reads_frames_across_transfers() {
        let mut bytes = frame(frame::PROGRESS, &[0, 0, 1, 2, 3, 4, 5]);
        bytes.extend_from_slice(&[0; 7]);
        bytes.extend_from_slice(&frame(frame::DATA, &[3, 0, 0xaa, 0x55]));
        let data = usb(&bytes);
        let mut reader = FrameReader::new();
        // Neither a multiple of 512 nor of a block.
        for piece in data.chunks(700) {
            reader.push(piece);
        }
        assert_eq!(
            frames(&mut reader),
            vec![
                Frame {
                    kind: frame::PROGRESS,
                    payload: vec![0, 0, 1, 2, 3, 4, 5]
                },
                Frame {
                    kind: frame::DATA,
                    payload: vec![3, 0, 0xaa, 0x55]
                },
            ]
        );
    }

    #[test]
    fn waits_for_frames_cut_off_between_blocks() {
        let payload: Vec<u8> = (0..3000).map(|i| i as u8).collect();
        let mut bytes = vec![0; BLOCK - 100];
        bytes.extend_from_slice(&frame(frame::DATA, &payload));
        let data = usb(&bytes);
        let mut reader = FrameReader::new();
        reader.push(&data[..BLOCK]);
        assert_eq!(reader.next_frame(), None);
        reader.push(&data[BLOCK..]);
        let got = reader.next_frame().expect("the frame");
        assert_eq!(got.payload, payload);
        assert_eq!(reader.next_frame(), None);
    }

    #[test]
    fn skips_what_is_not_a_frame() {
        let mut bytes = vec![0x0a, 0x07, 0x0a, 0x01, 0x00, 0x00];
        // A header whose tail does not match.
        bytes.extend_from_slice(&[0x0a, 0x02, 0x02, 0x00, 0xaa, 0xbb, 0x01, 0x0b]);
        bytes.extend_from_slice(&frame(frame::DONE, &[0, 0, 0]));
        let data = usb(&bytes);
        let mut reader = FrameReader::new();
        reader.push(&data);
        assert_eq!(
            frames(&mut reader),
            vec![Frame {
                kind: frame::DONE,
                payload: vec![0, 0, 0]
            }]
        );
    }

    #[test]
    fn parses_answers() {
        let mut answer = vec![0u8; MCU_PACKET];
        answer[..9].copy_from_slice(&[0x0a, 0x81, 0x01, 0x61, 1, 3, 2, 0, 1]);
        assert_eq!(
            McuVersion::parse(&answer),
            Some(McuVersion {
                bootloader: false,
                major: 1,
                minor: 3,
                hardware: 2,
                level: 1
            })
        );
        answer[3] = 0x62;
        assert!(McuVersion::parse(&answer).is_some_and(|version| version.bootloader));
        answer[3] = 0x63;
        assert_eq!(McuVersion::parse(&answer), None);
        assert_eq!(McuVersion::parse(&answer[..8]), None);

        let info = DeviceInfo::parse(b"\0\0\x01\x02\0\x01\x0f\x0a\x42DL16\0\0").expect("info");
        assert_eq!(
            info,
            DeviceInfo {
                ok: true,
                usb: 2,
                fpga: (1, 15),
                min_app: (10, 66),
                name: "DL16".into()
            }
        );
        assert_eq!(info.fpga_version(), 115);
        assert_eq!(info.min_app_version(), 1066);
        assert_eq!(DeviceInfo::parse(&[0; 8]), None);
    }

    #[test]
    fn reads_40_bit_numbers() {
        assert_eq!(u40(&[0x01, 0x02, 0x03, 0x04, 0x05, 0xff]), 0x05_0403_0201);
        assert_eq!(u40(&[0x01]), 1);
    }
}
