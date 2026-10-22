// SPDX-License-Identifier: Apache-2.0

//! The backend of the decoder DSL: Rust from the decoder IR, over the runtime of
//! `uiharu_core::dsl` (`docs/dsl.md` §5).
//!
//! [`generate`] turns one [`Module`] into the source of one Rust module: the types of its
//! contracts, its configuration, its free functions, the node that the graph runs and the
//! state struct with the decoder's methods, hooked into the runtime's main loop through the
//! decoder trait of its kind. [`generate_lib`] writes the crate root listing the modules. The
//! output is a disposable build artifact formatted with `rustfmt`; TypeScript stays authoritative.

#![forbid(unsafe_code)]

mod emit;
mod module;
mod names;
mod types;

use uiharu_ir::Module;

pub use module::{generate, generate_lib};

/// Why a module could not be generated: an IR the validator would have refused, or a
/// construct the backend does not emit yet.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct GenError(pub String);

impl GenError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The kind of decoder, by its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Logic,
    Stream,
    Bus,
}

/// The Rust module name of an IR module.
pub fn module_name(module: &Module) -> String {
    names::snake(&module.name)
}
