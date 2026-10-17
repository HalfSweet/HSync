// SPDX-License-Identifier: Apache-2.0

//! The events of an interpreted decoder, as its streams carry them.

use serde::{Deserialize, Serialize};

/// An event of a contract known only at run time: its JSON form, as the contract's Rust type
/// would serialize it — a `kind` tag and the fields by name, enums as their names, missing
/// values as `null`. Every stream between interpreted decoders carries these; a stream between
/// an interpreted decoder and a compiled one goes through a bridge ([`crate::ToDyn`],
/// [`crate::FromDyn`]) that converts through the same JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DynEvent(pub serde_json::Value);
