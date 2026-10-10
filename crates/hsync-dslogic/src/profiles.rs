//! Device profiles, channel modes and sample rates of the DSLogic family.
//!
//! Transcribed from `libsigrok4DSL/hardware/DSL/dsl.h` (`supported_DSLogic`, `channel_modes`,
//! `samplerates100/400/1000`); keep the order and values in step with it.

/// USB vendor id of DreamSourceLab.
pub const VENDOR_ID: u16 = 0x2A0E;

/// Product ids of DSCope oscilloscopes, recognised but not supported in v1.
pub const DSCOPE_PRODUCT_IDS: [u16; 10] = [
    0x0002, 0x0004, 0x0022, 0x0023, 0x0024, 0x0025, 0x0026, 0x0027, 0x0028, 0x002B,
];

/// Feature bits (`CAPS_FEATURE_*`).
pub mod caps {
    /// Adjustable threshold voltage.
    pub const VTH: u32 = 1 << 0;
    /// External sample buffer (buffer mode).
    pub const BUF: u32 = 1 << 1;
    /// Small startup EEPROM: the FX2 firmware is uploaded by the host.
    pub const SEEP: u32 = 1 << 3;
    /// USB 3.0 capable.
    pub const USB30: u32 = 1 << 6;
    /// ADF4360-7 clock synthesiser.
    pub const ADF4360: u32 = 1 << 8;
    /// 32 channels.
    pub const LA_CH32: u32 = 1 << 11;
    /// Threshold DAC scaled for 2.5 V.
    pub const MAX25_VTH: u32 = 1 << 13;
    /// Security check at open.
    pub const SECURITY: u32 = 1 << 14;
}

/// USB link speed a profile applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    High,
    Super,
}

/// One entry of `channel_modes[]` (logic modes only; DAQ and DSO modes are not supported).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelMode {
    /// `DSL_CHANNEL_ID`.
    pub id: u8,
    /// Stream (true) or buffer mode.
    pub stream: bool,
    /// Channels the mode creates.
    pub channels: u16,
    /// Channels that may be enabled at once.
    pub valid_channels: u16,
    pub min_samplerate: u64,
    pub max_samplerate: u64,
    pub hw_min_samplerate: u64,
    pub hw_max_samplerate: u64,
    pub pre_div: u32,
}

#[allow(clippy::too_many_arguments)]
const fn mode(
    id: u8,
    stream: bool,
    channels: u16,
    valid_channels: u16,
    min_samplerate: u64,
    max_samplerate: u64,
    hw_max_samplerate: u64,
    pre_div: u32,
) -> ChannelMode {
    ChannelMode {
        id,
        stream,
        channels,
        valid_channels,
        min_samplerate,
        max_samplerate,
        hw_min_samplerate: KHZ * 10,
        hw_max_samplerate,
        pre_div,
    }
}

const KHZ: u64 = 1_000;
const MHZ: u64 = 1_000_000;
const GHZ: u64 = 1_000_000_000;

/// The logic channel modes, indexed by `DSL_CHANNEL_ID`.
pub const CHANNEL_MODES: [ChannelMode; 26] = [
    // Stream, USB 2.0 devices
    mode(0, true, 16, 16, 50 * KHZ, 20 * MHZ, 100 * MHZ, 1),
    mode(1, true, 16, 12, 50 * KHZ, 25 * MHZ, 100 * MHZ, 1),
    mode(2, true, 16, 6, 50 * KHZ, 50 * MHZ, 100 * MHZ, 1),
    mode(3, true, 16, 3, 50 * KHZ, 100 * MHZ, 100 * MHZ, 1),
    // Stream, USB 3.0 devices on a USB 2.0 port
    mode(4, true, 16, 16, 100 * KHZ, 20 * MHZ, 500 * MHZ, 5),
    mode(5, true, 16, 12, 100 * KHZ, 25 * MHZ, 500 * MHZ, 5),
    mode(6, true, 16, 6, 100 * KHZ, 50 * MHZ, 500 * MHZ, 5),
    mode(7, true, 16, 3, 100 * KHZ, 100 * MHZ, 500 * MHZ, 5),
    mode(8, true, 32, 32, 100 * KHZ, 10 * MHZ, 500 * MHZ, 5),
    mode(9, true, 32, 16, 100 * KHZ, 20 * MHZ, 500 * MHZ, 5),
    mode(10, true, 32, 12, 100 * KHZ, 25 * MHZ, 500 * MHZ, 5),
    mode(11, true, 32, 6, 100 * KHZ, 50 * MHZ, 500 * MHZ, 5),
    mode(12, true, 32, 3, 100 * KHZ, 100 * MHZ, 500 * MHZ, 5),
    // Stream, USB 3.0
    mode(13, true, 32, 32, MHZ, 50 * MHZ, 500 * MHZ, 5),
    mode(14, true, 32, 30, MHZ, 100 * MHZ, 500 * MHZ, 5),
    mode(15, true, 32, 12, MHZ, 250 * MHZ, 500 * MHZ, 5),
    mode(16, true, 16, 16, MHZ, 125 * MHZ, 500 * MHZ, 5),
    mode(17, true, 16, 12, MHZ, 250 * MHZ, 500 * MHZ, 5),
    mode(18, true, 16, 6, MHZ, 500 * MHZ, 500 * MHZ, 5),
    mode(19, true, 8, 3, MHZ, GHZ, 500 * MHZ, 5),
    // Buffer
    mode(20, false, 16, 16, 50 * KHZ, 100 * MHZ, 100 * MHZ, 1),
    mode(21, false, 8, 8, 50 * KHZ, 200 * MHZ, 100 * MHZ, 1),
    mode(22, false, 4, 4, 50 * KHZ, 400 * MHZ, 100 * MHZ, 1),
    mode(23, false, 32, 32, MHZ, 250 * MHZ, 500 * MHZ, 5),
    mode(24, false, 16, 16, MHZ, 500 * MHZ, 500 * MHZ, 5),
    mode(25, false, 8, 8, MHZ, GHZ, 500 * MHZ, 5),
];

/// `samplerates100`.
pub const SAMPLERATES_100: &[u64] = &[
    10,
    20,
    50,
    100,
    200,
    500,
    KHZ,
    2 * KHZ,
    5 * KHZ,
    10 * KHZ,
    20 * KHZ,
    40 * KHZ,
    50 * KHZ,
    100 * KHZ,
    200 * KHZ,
    400 * KHZ,
    500 * KHZ,
    MHZ,
    2 * MHZ,
    4 * MHZ,
    5 * MHZ,
    10 * MHZ,
    20 * MHZ,
    25 * MHZ,
    50 * MHZ,
    100 * MHZ,
];

/// `samplerates400`.
pub const SAMPLERATES_400: &[u64] = &[
    10,
    20,
    50,
    100,
    200,
    500,
    KHZ,
    2 * KHZ,
    5 * KHZ,
    10 * KHZ,
    20 * KHZ,
    40 * KHZ,
    50 * KHZ,
    100 * KHZ,
    200 * KHZ,
    400 * KHZ,
    500 * KHZ,
    MHZ,
    2 * MHZ,
    4 * MHZ,
    5 * MHZ,
    10 * MHZ,
    20 * MHZ,
    25 * MHZ,
    50 * MHZ,
    100 * MHZ,
    200 * MHZ,
    400 * MHZ,
];

/// `samplerates1000`.
pub const SAMPLERATES_1000: &[u64] = &[
    10,
    20,
    50,
    100,
    200,
    500,
    KHZ,
    2 * KHZ,
    5 * KHZ,
    10 * KHZ,
    20 * KHZ,
    40 * KHZ,
    50 * KHZ,
    100 * KHZ,
    200 * KHZ,
    400 * KHZ,
    500 * KHZ,
    MHZ,
    2 * MHZ,
    4 * MHZ,
    5 * MHZ,
    10 * MHZ,
    20 * MHZ,
    25 * MHZ,
    50 * MHZ,
    100 * MHZ,
    125 * MHZ,
    250 * MHZ,
    500 * MHZ,
    GHZ,
];

/// One entry of `supported_DSLogic[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub vid: u16,
    pub pid: u16,
    pub speed: UsbSpeed,
    pub model: &'static str,
    /// FX2 firmware uploaded by the host (only for `SEEP` devices).
    pub firmware: &'static str,
    /// FPGA bitstream for the 1.8/2.5/3.3 V threshold level.
    pub fpga_bit33: &'static str,
    /// FPGA bitstream for the 5 V threshold level.
    pub fpga_bit50: &'static str,
    pub caps: u32,
    /// Bit `n` set: channel mode `n` is available.
    pub channel_modes: u32,
    /// Channels of the device.
    pub total_channels: u16,
    /// Hardware buffer in samples × channels.
    pub hw_depth: u64,
    pub samplerates: &'static [u64],
    pub default_channel_mode: u8,
    pub default_samplerate: u64,
    pub default_sample_limit: u64,
    pub half_samplerate: u64,
    pub quarter_samplerate: u64,
}

impl Profile {
    pub fn has(&self, feature: u32) -> bool {
        self.caps & feature != 0
    }

    /// A USB 3.0 model connected at USB 2.0 speed (`MainWindow::check_usb_device_speed`).
    pub fn on_slow_port(&self) -> bool {
        self.has(caps::USB30) && self.speed == UsbSpeed::High
    }

    /// Channel modes available in buffer (`stream == false`) or stream mode, in table order.
    pub fn channel_modes(&self, stream: bool) -> impl Iterator<Item = &'static ChannelMode> + '_ {
        CHANNEL_MODES
            .iter()
            .filter(move |mode| mode.stream == stream && self.channel_modes & (1 << mode.id) != 0)
    }

    /// The sample rates allowed in `mode`.
    pub fn samplerates_for(&self, mode: &ChannelMode) -> Vec<u64> {
        self.samplerates
            .iter()
            .copied()
            .filter(|rate| *rate >= mode.min_samplerate && *rate <= mode.max_samplerate)
            .collect()
    }
}

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

const fn modes(ids: &[u8]) -> u32 {
    let mut bits = 0;
    let mut i = 0;
    while i < ids.len() {
        bits |= 1 << ids[i];
        i += 1;
    }
    bits
}

#[allow(clippy::too_many_arguments)]
const fn profile(
    pid: u16,
    speed: UsbSpeed,
    model: &'static str,
    firmware: &'static str,
    fpga_bit33: &'static str,
    fpga_bit50: &'static str,
    caps: u32,
    channel_modes: u32,
    total_channels: u16,
    hw_depth: u64,
    samplerates: &'static [u64],
    default_channel_mode: u8,
    half_samplerate: u64,
    quarter_samplerate: u64,
) -> Profile {
    Profile {
        vid: VENDOR_ID,
        pid,
        speed,
        model,
        firmware,
        fpga_bit33,
        fpga_bit50,
        caps,
        channel_modes,
        total_channels,
        hw_depth,
        samplerates,
        default_channel_mode,
        default_samplerate: MHZ,
        default_sample_limit: MHZ,
        half_samplerate,
        quarter_samplerate,
    }
}

const USB2_MODES: u32 = modes(&[0, 1, 2, 3, 20, 21, 22]);
const USB2_BASIC_MODES: u32 = modes(&[0, 1, 2, 3, 20]);

/// Every supported DSLogic model; the same product id appears once per USB speed.
pub const PROFILES: [Profile; 12] = [
    profile(
        0x0001,
        UsbSpeed::High,
        "DSLogic",
        "DSLogic.fw",
        "DSLogic33.bin",
        "DSLogic50.bin",
        caps::SEEP | caps::BUF,
        USB2_MODES,
        16,
        256 * MIB,
        SAMPLERATES_400,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
    profile(
        0x0003,
        UsbSpeed::High,
        "DSLogic Pro",
        "DSLogicPro.fw",
        "DSLogicPro.bin",
        "DSLogicPro.bin",
        caps::SEEP | caps::VTH | caps::BUF,
        USB2_MODES,
        16,
        256 * MIB,
        SAMPLERATES_400,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
    profile(
        0x0020,
        UsbSpeed::High,
        "DSLogic Plus",
        "DSLogicPlus.fw",
        "DSLogicPlus.bin",
        "DSLogicPlus.bin",
        caps::VTH | caps::BUF,
        USB2_MODES,
        16,
        256 * MIB,
        SAMPLERATES_400,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
    profile(
        0x0021,
        UsbSpeed::High,
        "DSLogic Basic",
        "DSLogicBasic.fw",
        "DSLogicBasic.bin",
        "DSLogicBasic.bin",
        caps::VTH,
        USB2_MODES,
        16,
        256 * 1024,
        SAMPLERATES_400,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
    profile(
        0x0029,
        UsbSpeed::High,
        "DSLogic U2Basic",
        "DSLogicU2Basic.fw",
        "DSLogicU2Basic.bin",
        "DSLogicU2Basic.bin",
        caps::VTH | caps::BUF,
        USB2_BASIC_MODES,
        16,
        64 * MIB,
        SAMPLERATES_100,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
    profile(
        0x002A,
        UsbSpeed::High,
        "DSLogic U3Pro16",
        "DSLogicU3Pro16.fw",
        "DSLogicU3Pro16.bin",
        "DSLogicU3Pro16.bin",
        caps::VTH | caps::BUF | caps::USB30 | caps::ADF4360,
        modes(&[4, 5, 6, 7, 24, 25]),
        16,
        2 * GIB,
        SAMPLERATES_1000,
        4,
        500 * MHZ,
        GHZ,
    ),
    profile(
        0x002A,
        UsbSpeed::Super,
        "DSLogic U3Pro16",
        "DSLogicU3Pro16.fw",
        "DSLogicU3Pro16.bin",
        "DSLogicU3Pro16.bin",
        caps::VTH | caps::BUF | caps::USB30 | caps::ADF4360,
        modes(&[16, 17, 18, 19, 24, 25]),
        16,
        2 * GIB,
        SAMPLERATES_1000,
        16,
        500 * MHZ,
        GHZ,
    ),
    profile(
        0x002C,
        UsbSpeed::High,
        "DSLogic U3Pro32",
        "DSLogicU3Pro32.fw",
        "DSLogicU3Pro32.bin",
        "DSLogicU3Pro32.bin",
        caps::VTH | caps::BUF | caps::USB30 | caps::ADF4360 | caps::LA_CH32,
        modes(&[8, 9, 10, 11, 12, 23, 24, 25]),
        32,
        2 * GIB,
        SAMPLERATES_1000,
        8,
        500 * MHZ,
        GHZ,
    ),
    profile(
        0x002C,
        UsbSpeed::Super,
        "DSLogic U3Pro32",
        "DSLogicU3Pro32.fw",
        "DSLogicU3Pro32.bin",
        "DSLogicU3Pro32.bin",
        caps::VTH | caps::BUF | caps::USB30 | caps::ADF4360 | caps::LA_CH32,
        modes(&[13, 14, 15, 18, 19, 23, 24, 25]),
        32,
        2 * GIB,
        SAMPLERATES_1000,
        13,
        500 * MHZ,
        GHZ,
    ),
    profile(
        0x002D,
        UsbSpeed::High,
        "DSLogic U2Pro16",
        "DSLogicU2Pro16.fw",
        "DSLogicU2Pro16.bin",
        "DSLogicU2Pro16.bin",
        caps::VTH | caps::BUF | caps::ADF4360 | caps::SECURITY,
        modes(&[4, 5, 6, 7, 24, 25]),
        16,
        4 * GIB,
        SAMPLERATES_1000,
        4,
        500 * MHZ,
        GHZ,
    ),
    profile(
        0x0030,
        UsbSpeed::High,
        "DSLogic Plus",
        "DSLogicPlus.fw",
        "DSLogicPlus-pgl12.bin",
        "DSLogicPlus-pgl12.bin",
        caps::VTH | caps::BUF | caps::MAX25_VTH | caps::SECURITY,
        USB2_MODES,
        16,
        256 * MIB,
        SAMPLERATES_400,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
    profile(
        0x0031,
        UsbSpeed::High,
        "DSLogic U2Basic",
        "DSLogicU2Basic.fw",
        "DSLogicU2Basic-pgl12.bin",
        "DSLogicU2Basic-pgl12.bin",
        caps::VTH | caps::BUF | caps::MAX25_VTH | caps::SECURITY,
        USB2_BASIC_MODES,
        16,
        64 * MIB,
        SAMPLERATES_100,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
];

/// Two later revisions share their bitstreams' naming with the `-pgl12-2` suffix.
pub const PROFILES_PGL12_2: [Profile; 2] = [
    profile(
        0x0034,
        UsbSpeed::High,
        "DSLogic Plus",
        "DSLogicPlus-pgl12-2.fw",
        "DSLogicPlus-pgl12-2.bin",
        "DSLogicPlus-pgl12-2.bin",
        caps::VTH | caps::BUF | caps::MAX25_VTH | caps::SECURITY,
        USB2_MODES,
        16,
        256 * MIB,
        SAMPLERATES_400,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
    profile(
        0x0035,
        UsbSpeed::High,
        "DSLogic U2Basic",
        "DSLogicU2Basic-pgl12-2.fw",
        "DSLogicU2Basic-pgl12-2.bin",
        "DSLogicU2Basic-pgl12-2.bin",
        caps::VTH | caps::BUF | caps::MAX25_VTH | caps::SECURITY,
        USB2_BASIC_MODES,
        16,
        64 * MIB,
        SAMPLERATES_100,
        0,
        200 * MHZ,
        400 * MHZ,
    ),
];

/// The profile of a device by product id and link speed.
pub fn find_profile(vid: u16, pid: u16, speed: UsbSpeed) -> Option<&'static Profile> {
    PROFILES
        .iter()
        .chain(PROFILES_PGL12_2.iter())
        .find(|profile| profile.vid == vid && profile.pid == pid && profile.speed == speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_modes_are_indexed_by_id() {
        for (index, mode) in CHANNEL_MODES.iter().enumerate() {
            assert_eq!(usize::from(mode.id), index);
        }
    }

    #[test]
    fn profiles_default_to_one_of_their_modes() {
        for profile in PROFILES.iter().chain(PROFILES_PGL12_2.iter()) {
            assert_ne!(
                profile.channel_modes & (1 << profile.default_channel_mode),
                0,
                "{} {:?}",
                profile.model,
                profile.speed
            );
        }
    }

    #[test]
    fn finds_profiles_by_speed() {
        let high = find_profile(VENDOR_ID, 0x002A, UsbSpeed::High).expect("U3Pro16 on USB 2");
        let fast = find_profile(VENDOR_ID, 0x002A, UsbSpeed::Super).expect("U3Pro16 on USB 3");
        assert_eq!(high.default_channel_mode, 4);
        assert_eq!(fast.default_channel_mode, 16);
        assert!(high.on_slow_port() && !fast.on_slow_port());
        let plus = find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("a USB 2.0 model");
        assert!(!plus.on_slow_port());
        assert!(find_profile(VENDOR_ID, 0x0002, UsbSpeed::High).is_none());
    }

    #[test]
    fn limits_sample_rates_to_the_channel_mode() {
        let plus = find_profile(VENDOR_ID, 0x0020, UsbSpeed::High).expect("DSLogic Plus");
        let stream16 = &CHANNEL_MODES[0];
        let rates = plus.samplerates_for(stream16);
        assert_eq!(rates.first(), Some(&(50 * KHZ)));
        assert_eq!(rates.last(), Some(&(20 * MHZ)));
        let buffer4 = &CHANNEL_MODES[22];
        assert_eq!(plus.samplerates_for(buffer4).last(), Some(&(400 * MHZ)));
    }
}
