// SPDX-License-Identifier: Apache-2.0

//! Modbus RTU: the frames of a Modbus serial line in RTU mode, decoded from `uart.events` into
//! `modbus.events`, with the CRC of the serial line specification and a frame builder for the
//! UART synthesizer.
//!
//! RTU mode frames its messages by silences on the line, so the decoder works from the timing
//! of the characters the UART decoder reports: see [`ModbusRtu`] for the rules. The PDU is
//! reported as it is, not interpreted.

mod crc;
mod decoder;
mod events;
pub mod synth;

pub use crc::crc16;
pub use decoder::{ModbusRtu, Silences};
pub use events::{DATA_BATCH, EndReason, FrameId, MODBUS_EVENTS, ModbusEvent};
