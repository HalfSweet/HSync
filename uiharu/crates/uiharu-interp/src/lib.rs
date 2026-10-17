// SPDX-License-Identifier: Apache-2.0

//! The interpreter of the decoder IR: a decoder module loaded at run time — the JSON the
//! compiler of `uiharu/compiler` writes for a decoder in the DSL — runs on the engine without
//! Rust generated or compiled for it.
//!
//! A [`Program`] is a module loaded: checked against the engine's rules ([`uiharu_ir::validate`])
//! and compiled into the instructions of a small stack machine, typed as the Rust backend types
//! the same IR, so that what runs here is what the generated decoder would do. An
//! [`Interpreter`] is a node of the module for one configuration: it declares the ports and
//! properties the generated node would, and runs on the main loops of `uiharu_core::dsl`
//! (`run_logic`, `run_stream`, `run_bus`), which call the module's hooks through the machine.
//! The events of an interpreted decoder are [`DynEvent`]s, the JSON of the contract; a stream
//! between an interpreted decoder and a compiled one goes through a bridge ([`ToDyn`],
//! [`FromDyn`]). `docs/dsl.md` §8 describes all of it.

#![forbid(unsafe_code)]

mod bridge;
mod compile;
mod event;
mod framer;
mod intern;
mod json;
mod node;
mod program;
mod types;
mod value;
mod vm;

pub use bridge::{FromDyn, ToDyn};
pub use event::DynEvent;
pub use framer::DynFramer;
pub use intern::intern;
pub use node::Interpreter;
pub use program::{Kind, LoadError, Program};
pub use vm::FaultHandle;
