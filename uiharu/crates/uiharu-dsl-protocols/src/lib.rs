// SPDX-License-Identifier: Apache-2.0

//! Decoder modules compiled from the TypeScript DSL. Generate the disposable implementation
//! under `src/generated/` with `pnpm dsl:generate` before building.
//! This handwritten crate root keeps a Cargo target available while codegen bootstraps.

#![forbid(unsafe_code)]

#[path = "generated/lib.rs"]
mod generated;

// Generated modules use `crate::<module>` for shared contracts; keep those paths public.
pub use generated::*;
