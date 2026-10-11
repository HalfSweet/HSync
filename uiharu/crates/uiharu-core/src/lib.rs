// SPDX-License-Identifier: Apache-2.0

//! uiharu, a streaming protocol decode engine: decoders run as resumable state machines over
//! blocks of logic samples, exchange typed events through bounded streams with backpressure,
//! and are wired into a checked decode graph.

#![forbid(unsafe_code)]
