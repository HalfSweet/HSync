// SPDX-License-Identifier: Apache-2.0

//! DP/AP request interpretation generated from `dsl/decoders/arm-memap.ts`.
//! Planned addresses are explicitly conditional; JTAG OK/FAULT never proves success.

pub use uiharu_dsl_protocols::arm_memap::{ArmMemap, ArmMemapConfig, ArmMemapEvent, RegisterName};
