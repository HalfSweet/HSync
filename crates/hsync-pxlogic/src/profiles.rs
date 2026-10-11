//! Device profiles, channel modes and sample rates of the PXLogic family.
//!
//! Transcribed from `src/hardware/pxlogic/pxlogic.h` (`supported_PX`, `samplerates`) and
//! `pxlogic.c` (`channel_modes`) of PXView's libsigrok; keep the order and values in step.

const KHZ: u64 = 1_000;
const MHZ: u64 = 1_000_000;
const GHZ: u64 = 1_000_000_000;

/// USB ids of the first 32-channel devices.
pub const OLD_IDS: (u16, u16) = (0x1a86, 0x5237);
/// USB ids of the current devices, shared by every model.
pub const NEW_IDS: (u16, u16) = (0x16c0, 0x05dc);
/// What the manufacturer string of every PXLogic starts with (`logic_check_conf_profile()`).
pub const MANUFACTURER_PREFIX: &str = "PX";

/// The MCU firmware, written into the device's flash when it runs another version.
pub const FIRMWARE: &str = "SCI_LOGIC.bin";
/// The MCU firmware version this driver works with (`FIRMWARE_VERSION`).
pub const FIRMWARE_VERSION: u32 = 0x5690_0028;
/// The FPGA bitstream, loaded at every opening.
pub const FPGA_BITSTREAM: &str = "hspi_ddr.bin";
/// The bitstream loaded just before it (`fpga_rst_bit`).
pub const FPGA_RESET_BITSTREAM: &str = "hspi_ddr_RST.bin";

/// USB link speed a profile applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    High,
    Super,
}

/// One entry of `channel_modes[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelMode {
    /// `PX_CHANNEL_ID`.
    pub id: u8,
    /// Stream (true) or Buffer mode.
    pub stream: bool,
    /// Channels the mode captures, from channel 0 on.
    pub channels: u16,
    pub min_samplerate: u64,
    pub max_samplerate: u64,
}

const fn mode(id: u8, stream: bool, channels: u16, max_samplerate: u64) -> ChannelMode {
    ChannelMode {
        id,
        stream,
        channels,
        min_samplerate: 2 * KHZ,
        max_samplerate,
    }
}

/// The channel modes, indexed by `PX_CHANNEL_ID`.
pub const CHANNEL_MODES: [ChannelMode; 15] = [
    mode(0, false, 32, 250 * MHZ),
    mode(1, false, 16, 250 * MHZ),
    mode(2, false, 16, 500 * MHZ),
    mode(3, false, 8, GHZ),
    // Stream over USB 3.0
    mode(4, true, 32, 50 * MHZ),
    mode(5, true, 16, 125 * MHZ),
    mode(6, true, 8, 250 * MHZ),
    mode(7, true, 4, 500 * MHZ),
    mode(8, true, 2, GHZ),
    // Stream over USB 2.0
    mode(9, true, 1, 200 * MHZ),
    mode(10, true, 2, 100 * MHZ),
    mode(11, true, 4, 50 * MHZ),
    mode(12, true, 8, 25 * MHZ),
    mode(13, true, 16, 10 * MHZ),
    mode(14, true, 32, 5 * MHZ),
];

/// `samplerates[]`.
pub const SAMPLERATES: [u64; 33] = [
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
    200 * MHZ,
    250 * MHZ,
    400 * MHZ,
    500 * MHZ,
    800 * MHZ,
    GHZ,
];

/// One entry of `supported_PX[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub vid: u16,
    pub pid: u16,
    pub speed: UsbSpeed,
    /// What the device reports in its logic mode register.
    pub logic_mode: u32,
    pub model: &'static str,
    /// Bit `n`: channel mode `n` is available.
    pub channel_modes: u32,
    /// Bits of sample memory.
    pub memory_bits: u64,
    pub default_channel_mode: u8,
}

const fn modes(ids: &[u8]) -> u32 {
    let mut mask = 0;
    let mut at = 0;
    while at < ids.len() {
        mask |= 1 << ids[at];
        at += 1;
    }
    mask
}

/// Buffer modes and USB 3.0 Stream modes of the 32-channel models.
const MODES_32_SUPER: u32 = modes(&[0, 2, 3, 4, 5, 6, 7, 8]);
const MODES_32_HIGH: u32 = modes(&[0, 2, 3, 9, 10, 11, 12, 13, 14]);
const MODES_PRO_SUPER: u32 = modes(&[2, 3, 5, 6, 7, 8]);
const MODES_PRO_HIGH: u32 = modes(&[2, 3, 9, 10, 11, 12, 13]);
const MODES_PLUS_SUPER: u32 = modes(&[2, 5, 6, 7]);
const MODES_PLUS_HIGH: u32 = modes(&[2, 9, 10, 11, 12, 13]);
const MODES_BASE_SUPER: u32 = modes(&[1, 5, 6]);
const MODES_BASE_HIGH: u32 = modes(&[1, 9, 10, 11, 12, 13]);

#[allow(clippy::too_many_arguments)]
const fn profile(
    ids: (u16, u16),
    speed: UsbSpeed,
    logic_mode: u32,
    model: &'static str,
    channel_modes: u32,
    memory_bits: u64,
    default_channel_mode: u8,
) -> Profile {
    Profile {
        vid: ids.0,
        pid: ids.1,
        speed,
        logic_mode,
        model,
        channel_modes,
        memory_bits,
        default_channel_mode,
    }
}

/// Every PXLogic, in the order of `supported_PX[]`.
pub const PROFILES: [Profile; 10] = [
    profile(
        OLD_IDS,
        UsbSpeed::Super,
        0,
        "PX-Logic 32",
        MODES_32_SUPER,
        4 * GHZ,
        0,
    ),
    profile(
        OLD_IDS,
        UsbSpeed::High,
        0,
        "PX-Logic 32",
        MODES_32_HIGH,
        4 * GHZ,
        2,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::Super,
        0,
        "PX-Logic 32",
        MODES_32_SUPER,
        4 * GHZ,
        0,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::High,
        0,
        "PX-Logic 32",
        MODES_32_HIGH,
        4 * GHZ,
        2,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::Super,
        1,
        "PX-Logic 16 Pro",
        MODES_PRO_SUPER,
        4 * GHZ,
        2,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::High,
        1,
        "PX-Logic 16 Pro",
        MODES_PRO_HIGH,
        4 * GHZ,
        2,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::Super,
        2,
        "PX-Logic 16 Plus",
        MODES_PLUS_SUPER,
        2 * GHZ,
        2,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::High,
        2,
        "PX-Logic 16 Plus",
        MODES_PLUS_HIGH,
        2 * GHZ,
        2,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::Super,
        3,
        "PX-Logic 16 Base",
        MODES_BASE_SUPER,
        GHZ,
        1,
    ),
    profile(
        NEW_IDS,
        UsbSpeed::High,
        3,
        "PX-Logic 16 Base",
        MODES_BASE_HIGH,
        GHZ,
        1,
    ),
];

/// The profile of a device, by its USB ids, link speed and logic mode.
pub fn find_profile(
    vid: u16,
    pid: u16,
    speed: UsbSpeed,
    logic_mode: u32,
) -> Option<&'static Profile> {
    PROFILES.iter().find(|profile| {
        profile.vid == vid
            && profile.pid == pid
            && profile.speed == speed
            && profile.logic_mode == logic_mode
    })
}

/// Whether the USB ids are a PXLogic's.
pub fn is_pxlogic(vid: u16, pid: u16) -> bool {
    (vid, pid) == OLD_IDS || (vid, pid) == NEW_IDS
}

impl Profile {
    /// The channel modes of the profile for Stream or Buffer mode, in table order.
    pub fn channel_modes(&self, stream: bool) -> impl Iterator<Item = &'static ChannelMode> {
        let available = self.channel_modes;
        CHANNEL_MODES
            .iter()
            .filter(move |mode| available & (1 << mode.id) != 0 && mode.stream == stream)
    }

    /// The channel mode the device starts in.
    pub fn default_mode(&self) -> &'static ChannelMode {
        &CHANNEL_MODES[usize::from(self.default_channel_mode)]
    }

    /// Whether the ids alone name the model: the first devices came in 32 channels only.
    pub fn known_by_ids(vid: u16, pid: u16) -> bool {
        (vid, pid) == OLD_IDS
    }
}

impl ChannelMode {
    /// The rates of `samplerates[]` the mode allows (`adjust_samplerate()`).
    pub fn samplerates(&self) -> Vec<u64> {
        SAMPLERATES
            .iter()
            .copied()
            .filter(|&rate| (self.min_samplerate..=self.max_samplerate).contains(&rate))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_modes_sit_at_their_ids() {
        for (index, mode) in CHANNEL_MODES.iter().enumerate() {
            assert_eq!(usize::from(mode.id), index);
        }
    }

    #[test]
    fn finds_every_model_by_its_logic_mode() {
        let super32 = find_profile(0x16c0, 0x05dc, UsbSpeed::Super, 0).expect("32");
        assert_eq!(super32.model, "PX-Logic 32");
        assert_eq!(super32.default_mode().id, 0);
        let plus = find_profile(0x16c0, 0x05dc, UsbSpeed::High, 2).expect("16 Plus");
        assert_eq!(plus.model, "PX-Logic 16 Plus");
        assert_eq!(
            plus.channel_modes(true)
                .map(|mode| mode.id)
                .collect::<Vec<_>>(),
            vec![9, 10, 11, 12, 13]
        );
        assert_eq!(
            plus.channel_modes(false)
                .map(|mode| mode.id)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert!(find_profile(0x1a86, 0x5237, UsbSpeed::Super, 1).is_none());
        assert!(is_pxlogic(0x1a86, 0x5237) && !is_pxlogic(0x1a86, 0xffcc));
    }

    #[test]
    fn limits_rates_to_the_mode() {
        let rates = CHANNEL_MODES[5].samplerates();
        assert_eq!(rates.first(), Some(&(2 * KHZ)));
        assert_eq!(rates.last(), Some(&(125 * MHZ)));
        assert_eq!(CHANNEL_MODES[8].samplerates().last(), Some(&GHZ));
    }
}
