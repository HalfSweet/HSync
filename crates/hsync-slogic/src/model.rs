// SPDX-License-Identifier: GPL-3.0-or-later
// Ported from sipeed/libsigrok, slogic-dev, commit
// 0d728b3e1226a61738c99c3333ad16d5c2814728:
// src/hardware/sipeed-slogic-analyzer/slogic/{slogic16u3.c,slogic32u3.c,slogic_data.c}.
// Copyright (C) 2023-2025 Shenzhen Sipeed Technology Co., Ltd.
//! The SLogic U3 models and their link-dependent sample-rate ceilings.

const MHZ: u64 = 1_000_000;

/// One analyzer of the U3 family, which shares the register/AUX protocol (`slogic_model`).
#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    /// Model name, as the reference reports it.
    pub name: &'static str,
    /// USB product id of the capture firmware (the bootloader uses another).
    pub product_id: u16,
    /// Physical digital inputs, D0 upwards.
    pub channels: u8,
    /// Advertised discrete rates in ascending order.
    pub samplerates: &'static [u64],
    /// Wire modes in ascending order: the channels transferred from D0, and the native
    /// rate ceiling of that mode.
    pub wire_modes: &'static [(u8, u64)],
    /// Bytes the firmware sends before the first whole sample, discarded once per RUN
    /// (`first_drop_bytes`).
    pub head_bytes: usize,
}

/// Sipeed SLogic16U3: 16 inputs on a 5 Gbps link (`slogic16u3.c`).
pub static SLOGIC16U3: Model = Model {
    name: "SLogic16U3",
    product_id: 0x3031,
    channels: 16,
    samplerates: &[
        5 * MHZ,
        8 * MHZ,
        10 * MHZ,
        16 * MHZ,
        20 * MHZ,
        25 * MHZ,
        32 * MHZ,
        40 * MHZ,
        50 * MHZ,
        80 * MHZ,
        100 * MHZ,
        160 * MHZ,
        200 * MHZ,
        400 * MHZ,
        800 * MHZ,
    ],
    wire_modes: &[(4, 800 * MHZ), (8, 400 * MHZ), (16, 200 * MHZ)],
    // Its firmware still opens the gate mid-word.
    head_bytes: 4,
};

/// Sipeed SLogic32U3: 32 inputs on a 10 Gbps link (`slogic32u3.c`).
pub static SLOGIC32U3: Model = Model {
    name: "SLogic32U3",
    product_id: 0x3032,
    channels: 32,
    samplerates: &[
        5 * MHZ,
        8 * MHZ,
        10 * MHZ,
        16 * MHZ,
        20 * MHZ,
        25 * MHZ,
        32 * MHZ,
        40 * MHZ,
        50 * MHZ,
        80 * MHZ,
        100 * MHZ,
        160 * MHZ,
        200 * MHZ,
        400 * MHZ,
        800 * MHZ,
        1400 * MHZ,
    ],
    wire_modes: &[
        (4, 1400 * MHZ),
        (8, 800 * MHZ),
        (16, 400 * MHZ),
        (32, 200 * MHZ),
    ],
    // Firmware from S/N 202610120954 gates on whole words.
    head_bytes: 0,
};

/// Every supported model.
pub static MODELS: [&Model; 2] = [&SLOGIC16U3, &SLOGIC32U3];

/// The model whose capture firmware enumerates with this product id.
pub fn find(product_id: u16) -> Option<&'static Model> {
    MODELS
        .into_iter()
        .find(|model| model.product_id == product_id)
}

/// The negotiated USB link, which may carry less than the model's native rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    /// USB 2.0: about 40 MB/s of sample payload.
    High,
    /// USB 3.x at 5 Gbps: about 400 MB/s of sample payload.
    Super,
    /// USB 3.x at 10 Gbps: no lower than the device's own ceiling.
    SuperPlus,
}

impl UsbSpeed {
    /// Sustained sample payload in bits per second, if it can limit a U3 model.
    fn payload_bits(self) -> Option<u64> {
        match self {
            Self::High => Some(320 * MHZ),
            Self::Super => Some(3200 * MHZ),
            Self::SuperPlus => None,
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Super => "super",
            Self::SuperPlus => "superPlus",
        }
    }
}

impl Model {
    /// All physical inputs as a channel mask.
    pub fn all_channels(&self) -> u32 {
        u32::MAX >> (32 - self.channels)
    }

    /// The narrowest wire mode that carries channel `highest`.
    pub fn wire_mode(&self, highest: u8) -> Option<u8> {
        self.wire_modes
            .iter()
            .map(|&(channels, _)| channels)
            .find(|&channels| highest < channels)
    }

    /// The fastest rate of a wire mode on this link (`slogic_link_max_rate()`), or 0 for
    /// a mode the model does not have.
    pub fn max_samplerate(&self, channels: u8, speed: UsbSpeed) -> u64 {
        let Some(&(_, native)) = self.wire_modes.iter().find(|mode| mode.0 == channels) else {
            return 0;
        };
        speed
            .payload_bits()
            .map_or(native, |bits| native.min(bits / u64::from(channels)))
    }

    /// The advertised rates a wire mode offers on this link (`slogic_rate_list()`).
    pub fn samplerates(&self, channels: u8, speed: UsbSpeed) -> Vec<u64> {
        let maximum = self.max_samplerate(channels, speed);
        self.samplerates
            .iter()
            .copied()
            .take_while(|&rate| rate <= maximum)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_only_the_capture_firmware_of_the_u3_models() {
        assert_eq!(find(0x3031), Some(&SLOGIC16U3));
        assert_eq!(find(0x3032), Some(&SLOGIC32U3));
        // Combo 8 (another protocol) and the bootloader.
        assert_eq!(find(0x0300), None);
        assert_eq!(find(0x30f1), None);
    }

    #[test]
    fn caps_the_native_ceilings_by_what_the_link_carries() {
        for (model, speed, ceilings) in [
            (&SLOGIC16U3, UsbSpeed::SuperPlus, &[800, 400, 200][..]),
            (&SLOGIC16U3, UsbSpeed::Super, &[800, 400, 200]),
            (&SLOGIC16U3, UsbSpeed::High, &[80, 40, 20]),
            (&SLOGIC32U3, UsbSpeed::SuperPlus, &[1400, 800, 400, 200]),
            (&SLOGIC32U3, UsbSpeed::Super, &[800, 400, 200, 100]),
            (&SLOGIC32U3, UsbSpeed::High, &[80, 40, 20, 10]),
        ] {
            assert_eq!(model.wire_modes.len(), ceilings.len());
            for (&(channels, _), &mhz) in model.wire_modes.iter().zip(ceilings) {
                assert_eq!(
                    model.max_samplerate(channels, speed),
                    mhz * MHZ,
                    "{} {channels} channels on {speed:?}",
                    model.name
                );
                let rates = model.samplerates(channels, speed);
                assert_eq!(rates.first(), Some(&(5 * MHZ)));
                assert_eq!(rates.last(), Some(&(mhz * MHZ)));
            }
        }
        assert_eq!(SLOGIC16U3.max_samplerate(32, UsbSpeed::SuperPlus), 0);
        assert_eq!(SLOGIC32U3.max_samplerate(3, UsbSpeed::SuperPlus), 0);
    }

    #[test]
    fn selects_the_narrowest_wire_mode_covering_a_channel() {
        for (model, modes) in [
            (
                &SLOGIC16U3,
                &[(0, 4), (3, 4), (4, 8), (7, 8), (8, 16), (15, 16)][..],
            ),
            (&SLOGIC32U3, &[(3, 4), (7, 8), (15, 16), (16, 32), (31, 32)]),
        ] {
            for &(highest, wire) in modes {
                assert_eq!(model.wire_mode(highest), Some(wire), "D{highest}");
            }
            assert_eq!(model.wire_mode(model.channels), None);
        }
        assert_eq!(SLOGIC16U3.all_channels(), 0xffff);
        assert_eq!(SLOGIC32U3.all_channels(), u32::MAX);
    }
}
