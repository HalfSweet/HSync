// SPDX-License-Identifier: Apache-2.0

//! The registry of uiharu's node types, for programs that assemble pipelines from names
//! rather than from types: the command line and the applications that embed the engine.
//!
//! - [`KINDS`] lists every node type by the name a pipeline description uses, with how to
//!   build it from a JSON configuration, its default configuration, its [`Manifest`] (ports,
//!   configuration fields, event kinds — what a user interface needs to offer it) and how to
//!   read its output as [`DynItem`]s, events as JSON values, without the event type.
//! - A [`Catalog`] adds decoder modules loaded at run time — the IR of decoders written in
//!   the DSL, run by `uiharu-interp` — to the engine's node types, under the module's name,
//!   with a manifest from its declarations; a loaded module stands in for a node type of the
//!   same name.
//! - [`Description`] is a pipeline as a file or a session stores it: a source, named nodes
//!   with their type and configuration, connections between ports, outputs to expose;
//!   [`build`] turns it into a running [`uiharu_core::Pipeline`] with a reader per exposed
//!   output, [`build_with`] does so with a catalog's node types, bridging the streams between
//!   the engine's decoders and loaded modules.
//!
//! The registry is the one place a decoder is made available by name; adding a decoder means
//! adding its [`Kind`] here and its manifest in `manifests/`.

#![forbid(unsafe_code)]

mod build;
mod catalog;
mod description;
mod dynamic;
mod kinds;
mod manifest;

pub use build::{BuildError, Built, Fault, Stream, build, build_with};
pub use catalog::{Catalog, CatalogError, KindRef, LoadedKind};
pub use description::{Connection, Description, NodeDescription, SourceDescription};
pub use dynamic::{DynItem, DynReader, TypedReader};
pub use kinds::{Bridge, KINDS, Kind, kind};
pub use manifest::{
    Category, EventKind, Field, FieldKind, LOGIC, Locale, Manifest, Port, manifests,
};
