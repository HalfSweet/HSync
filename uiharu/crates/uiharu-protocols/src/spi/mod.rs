// SPDX-License-Identifier: Apache-2.0

//! SPI: the decoder from logic samples to `spi.events`, the cursor decoders built on SPI read
//! it through, and a waveform synthesizer.
//!
//! The decoder is written in the DSL (`uiharu/dsl/decoders/spi.ts`) and generated into
//! `uiharu_dsl_protocols::spi`; this module re-exports it with the names decoders built on SPI
//! use. Its rules are in the module documentation there and in `docs/protocols/spi.md`.
//!
//! A transaction is one chip select period, numbered from 1 by the decoder; `0` stands for the
//! unframed stream of a bus without chip select. Decoders built on SPI read the stream through
//! [`SpiBus`], the generic frame cursor over `spi.events`: each read consumes one [`Word`], with
//! both lines' values, and can wait across blocks; the normal end is only ever returned by
//! [`Transaction::item_or_end`], and running out of words anywhere else is an error naming how
//! the transaction ended, so that the decoder can close what it had opened.

pub mod synth;

pub use uiharu_core::dsl::{Cut, Next, Read, ReadError, required};
pub use uiharu_dsl_protocols::spi::{
    BitOrder, EndReason, Spi, SpiConfig, SpiEvent, SpiEventItem as Word, cpha, cpol,
    samples_on_rising, selected,
};

use uiharu_core::{EventSchema, SchemaId};

/// The schema of SPI event streams.
pub const SPI_EVENTS: SchemaId = SpiEvent::SCHEMA;

/// Why a configuration is rejected.
pub type SpiConfigError = uiharu_core::dsl::ConfigError;

/// A transaction: one chip select period. Numbered from 1 by the decoder; 0 stands for the
/// unframed stream of a bus without chip select.
pub type TxId = u64;

/// A decoder's reader on an SPI event stream.
pub type SpiBus = uiharu_core::dsl::FramedBus<SpiEvent>;

/// A cursor over the words of one transaction.
pub type Transaction<'a> = uiharu_core::dsl::Transaction<'a, SpiEvent>;
