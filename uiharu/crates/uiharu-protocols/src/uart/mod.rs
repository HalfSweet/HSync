// SPDX-License-Identifier: Apache-2.0

//! UART: the decoder from the logic samples of one asynchronous serial line to `uart.events`,
//! and a waveform synthesizer.
//!
//! The character format is the asynchronous start-stop framing of TIA/EIA-232-F as 16550-class
//! UARTs receive it. One decoder reads one line; a link with a receive and a transmit line takes
//! two, and the events know nothing of direction. The stream is unframed, so decoders built on
//! it read the [`UartEvent`]s straight from their inbox.

mod decoder;
mod events;
pub mod synth;

pub use decoder::{BitOrder, Parity, StopBits, Uart, UartConfig, UartConfigError};
pub use events::{ParityCheck, UART_EVENTS, UartEvent};
