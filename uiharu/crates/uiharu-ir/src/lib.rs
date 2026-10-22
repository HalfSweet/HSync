// SPDX-License-Identifier: Apache-2.0

//! The decoder IR of uiharu: the form a decoder takes between the TypeScript DSL and the Rust
//! the backend emits over `uiharu-core`.
//!
//! A [`Module`] is one decoder with everything it needs: its types, the contracts it defines
//! and the ones it reads, its configuration, its ports, its state and its methods, and the
//! free functions they call. The body of a method is a typed, structured program whose only
//! waits are the runtime's operations ([`Op`]) and whose only collections have a declared
//! bound; the control items of the streams it reads (gaps, progress, the end) never appear in
//! it: the runtime handles them around the body and calls the decoder's hooks.
//!
//! [`validate`] checks a module against the engine's rules — those the compiler enforces for
//! DSL code, listed in `docs/dsl.md` — and names what it finds by the path of the offending
//! item. Typing is the TypeScript checker's work on the way in and rustc's on the way out;
//! the IR carries the declared types so that the backend can emit them.
//!
//! The JSON form, as `serde` writes it, is the interface between the compiler and the
//! backend: `docs/dsl.md` §4 spells it out.

#![forbid(unsafe_code)]

mod model;
mod validate;

pub use model::{
    Arm, BinaryOp, Category, Config, Const, Contract, Decoder, EnumVariant, EventRole, Expr,
    ExternalEnumVariant, Field, FieldJson, Framing, Function, Import, Input, Literal, Locale,
    Module, NumericEnumVariant, Op, Output, Param, Partial, Pattern, PropSpec, Recovery,
    RequireKind, RequireSpec, Role, SchemaRef, StateField, Stmt, Type, TypeDef, TypeKind, UnaryOp,
    UnionVariant,
};
pub use validate::{Diagnostic, validate};

/// The version of the IR this crate reads and writes; a module says which one it is in.
pub const IR_VERSION: u32 = 1;
