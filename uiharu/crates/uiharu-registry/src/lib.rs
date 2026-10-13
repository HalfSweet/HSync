// SPDX-License-Identifier: Apache-2.0

//! The registry of uiharu's node types, for programs that assemble pipelines from names
//! rather than from types: the command line and the applications that embed the engine.
//!
//! - [`KINDS`] lists every node type by the name a pipeline description uses, with how to
//!   build it from a JSON configuration, its default configuration, its [`Manifest`] (ports,
//!   configuration fields, event kinds — what a user interface needs to offer it) and how to
//!   read its output as [`DynItem`]s, events as JSON values, without the event type.
//! - [`Description`] is a pipeline as a file or a session stores it: a source, named nodes
//!   with their type and configuration, connections between ports, outputs to expose;
//!   [`build`] turns it into a running [`uiharu_core::Pipeline`] with a reader per exposed
//!   output.
//!
//! The registry is the one place a decoder is made available by name; adding a decoder means
//! adding its [`Kind`] here and its manifest in `manifests/`.

#![forbid(unsafe_code)]

mod build;
mod description;
mod dynamic;
mod kinds;
mod manifest;

pub use build::{BuildError, Built, Stream, build};
pub use description::{Connection, Description, NodeDescription, SourceDescription};
pub use dynamic::{DynItem, DynReader, TypedReader};
pub use kinds::{KINDS, Kind, kind};
pub use manifest::{Category, EventKind, Field, FieldKind, LOGIC, Manifest, Port, manifests};
