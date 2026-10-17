// SPDX-License-Identifier: Apache-2.0

//! UART: the decoder from the logic samples of one asynchronous serial line to `uart.events`,
//! and a waveform synthesizer.
//!
//! The decoder is written in the DSL (`uiharu/dsl/decoders/uart.ts`) and generated into
//! `uiharu_dsl_protocols::uart`; this module re-exports it. One decoder reads one line; a link
//! with a receive and a transmit line takes two, and the events know nothing of direction. The
//! stream is unframed, so decoders built on it read the [`UartEvent`]s straight from their
//! inbox.

pub mod synth;

pub use uiharu_dsl_protocols::uart::{
    BitOrder, Parity, ParityCheck, StopBits, Uart, UartConfig, UartEvent, half_bits, parity_bit,
};

use uiharu_core::{EventSchema, SchemaId};

/// The schema of UART event streams.
pub const UART_EVENTS: SchemaId = UartEvent::SCHEMA;

/// Why a configuration is rejected.
pub type UartConfigError = uiharu_core::dsl::ConfigError;
