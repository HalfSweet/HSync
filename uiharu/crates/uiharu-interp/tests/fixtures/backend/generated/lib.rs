// SPDX-License-Identifier: Apache-2.0

//! The protocol decoders generated from the decoder DSL (`uiharu/dsl/decoders/`) and the
//! contracts of decoders written by hand (`uiharu/dsl/contracts/`), by `uiharu-codegen` from
//! `uiharu/dsl/ir/`; do not edit. A module is one decoder — its contract's types, its
//! configuration and its node — or one contract: its types alone.

#![forbid(unsafe_code)]

pub mod backend_contracts;
pub mod backend_forward;
pub mod backend_probe;
