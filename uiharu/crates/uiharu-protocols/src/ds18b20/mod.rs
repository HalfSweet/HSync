// SPDX-License-Identifier: Apache-2.0

//! DS18B20: the decoder from `onewire.events` to `ds18b20.events` — the thermometer's function
//! commands, its scratchpad and its registers — and a generator of its traffic.

mod decoder;
mod events;
pub mod synth;

pub use decoder::Ds18b20;
pub use events::{
    COMMANDS, CONVERT_T, COPY_SCRATCHPAD, DS18B20_EVENTS, Ds18b20End, Ds18b20Event, FAMILY_CODE,
    READ_POWER_SUPPLY, READ_SCRATCHPAD, RECALL_E2, SCRATCHPAD_LEN, WRITE_SCRATCHPAD, resolution,
};
