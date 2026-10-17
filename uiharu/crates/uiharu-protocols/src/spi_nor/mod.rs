// SPDX-License-Identifier: Apache-2.0

//! SPI NOR flash: the decoder from `spi.events` to `nor.events`, and generators of its command
//! sequences for the SPI synthesizer.
//!
//! The decoder is written in the DSL (`uiharu/dsl/decoders/spi-nor.ts`) and generated into
//! `uiharu_dsl_protocols::spi_nor`; this module re-exports it.

pub mod synth;

pub use uiharu_dsl_protocols::spi_nor::{
    AddressBytes, Command, Direction, IdScheme, NorEnd, NorEvent, SpiNor, SpiNorConfig,
    address_width, command_of, layout_of,
};

use uiharu_core::{EventSchema, SchemaId};

/// The schema of SPI NOR event streams.
pub const NOR_EVENTS: SchemaId = NorEvent::SCHEMA;

/// The most bytes one `data` or `id` event carries.
pub const DATA_BATCH: usize = uiharu_dsl_protocols::spi_nor::DATA_BATCH as usize;

/// An operation on the flash, numbered from 1 by the decoder.
pub type OpId = u64;
