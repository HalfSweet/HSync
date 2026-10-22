// SPDX-License-Identifier: Apache-2.0

//! Fixed-width SDR dual/quad SPI transport, implemented from Winbond W25Q128JV
//! revision F, §§4.2, 6.1–6.3 and figures 18 and 20:
//! <https://www.winbond.com/resource-files/w25q128jv%20revf%2003272018%20plus.pdf>.
//!
//! - Samples one lane group on each mode-selected clock edge; IO0 is its low bit.
//! - Lane width is fixed: no opcode, direction, dummy-clock or phase inference.
//! - CS changes take priority over a simultaneous clock edge.
//! - Busy initial samples start partial frames; gaps/end abort open frames.
//! - Incomplete words are dropped and counted at frame end; progress is held at
//!   the first sample of the incomplete word. State holds one u32 word only.

mod decoder;
mod events;
pub mod synth;
pub use decoder::{Qspi, QspiConfig, QspiConfigError};
pub use events::{EndReason, QSPI_EVENTS, QspiBus, QspiEvent, Word};
