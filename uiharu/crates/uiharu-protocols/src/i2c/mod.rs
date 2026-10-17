// SPDX-License-Identifier: Apache-2.0

//! I²C: the decoder from logic samples to `i2c.events`, the cursor decoders built on I²C read
//! it through, and a waveform synthesizer.
//!
//! The contract is declared in the DSL (`uiharu/dsl/contracts/i2c.ts`) and its types generated
//! into `uiharu_dsl_protocols::i2c`; the decoder is written by hand (`decoder.rs`) and emits
//! those types. Decoders built on I²C read the stream through [`I2cBus`], the generic frame
//! cursor: a transaction's items are its addresses, data bytes and repeated STARTs
//! ([`TxEvent`]); a STOP ends it normally, as does the START of the next transaction when the
//! transaction was partial; running out of items anywhere but
//! [`Transaction::item_or_end`] is an error naming how the transaction ended.

mod decoder;
pub mod synth;

pub use decoder::{I2c, I2cConfig, I2cConfigError};
pub use uiharu_core::dsl::{Cut, Next, Read, ReadError, required};
pub use uiharu_dsl_protocols::i2c::{EndReason, I2cEvent, I2cEventItem as TxEvent};

use uiharu_core::{EventSchema, SchemaId};

/// The schema of I²C event streams.
pub const I2C_EVENTS: SchemaId = I2cEvent::SCHEMA;

/// A transaction: from a START to a STOP, numbered from 1 by the decoder.
pub type TxId = u64;

/// A decoder's reader on an I²C event stream.
pub type I2cBus = uiharu_core::dsl::FramedBus<I2cEvent>;

/// A cursor over the items of one transaction.
pub type Transaction<'a> = uiharu_core::dsl::Transaction<'a, I2cEvent>;

/// An address as the cursor hands it out ([`TxEvent::Address`]), for decoders that pass one
/// around.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    /// The 7-bit or 10-bit address.
    pub address: u16,
    /// Whether it is a 10-bit address.
    pub ten_bit: bool,
    /// The R/W̄ bit: whether the controller reads from the target.
    pub read: bool,
    /// Whether it was acknowledged.
    pub ack: bool,
}
