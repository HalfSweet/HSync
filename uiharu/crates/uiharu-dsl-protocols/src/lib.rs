// SPDX-License-Identifier: Apache-2.0

//! The protocol decoders generated from the decoder DSL (`uiharu/dsl/decoders/`), by
//! `uiharu-codegen` from `uiharu/dsl/ir/`; do not edit. Each module is one decoder: its
//! contract's types, its configuration and its node.

#![forbid(unsafe_code)]

pub mod pwm;
pub mod spi;
pub mod spi_nor;
pub mod uart;
