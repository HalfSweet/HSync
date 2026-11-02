// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub use uiharu_core::*;

/// Built-in protocol decoders, their typed events and waveform synthesizers.
#[cfg(feature = "protocols")]
pub use uiharu_protocols as protocols;

/// Decoder discovery, configuration metadata and pipelines assembled from descriptions.
#[cfg(feature = "registry")]
pub use uiharu_registry as registry;
