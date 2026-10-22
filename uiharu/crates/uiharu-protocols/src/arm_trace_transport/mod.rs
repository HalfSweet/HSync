// SPDX-License-Identifier: Apache-2.0

//! Raw SWO and formatted TPIU transport for generic trace byte consumers.
pub use uiharu_dsl_protocols::arm_trace_transport::{
    ArmTraceTransport, ArmTraceTransportConfig, TraceByteError, TraceByteEvent, TraceFormat,
};
