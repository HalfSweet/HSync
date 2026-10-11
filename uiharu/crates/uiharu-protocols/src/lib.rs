// SPDX-License-Identifier: Apache-2.0

//! The reference protocols of uiharu: SPI, decoded from logic samples, and SPI NOR flash
//! commands, decoded from SPI events.

#![forbid(unsafe_code)]

pub mod spi;
pub mod spi_nor;
pub mod timing;
