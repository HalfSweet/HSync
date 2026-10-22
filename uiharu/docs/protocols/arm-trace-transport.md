<!-- SPDX-License-Identifier: Apache-2.0 -->

# Arm trace transport

`arm-trace-transport` converts UART bytes into the generic `trace-byte.events@1`
contract. It preserves observed UART break, parity and framing errors as explicit
transport errors. It requires eight data bits and least-significant-bit-first
UART configuration. It never invents UART frames for extracted formatter data.

`format: "raw"` passes valid bytes through for unformatted asynchronous SWO. In
`format: "tpiu"` it implements the CoreSight formatter from
[Arm IHI0029D, Chapter D4](https://documentation-service.arm.com/static/5f9009d5f86e16515cdc0417?token=).
It extracts one configured `trace_id` (1–111) from sixteen-byte formatter frames.
Each mixed-use byte is interpreted using its corresponding auxiliary bit,
including an ID change that takes effect after the following ordinary byte. A
new ID in byte fourteen applies to the following frame; its reserved auxiliary
bit is ignored. Unselected and unknown source IDs produce no payload bytes.

The default `start_aligned: false` waits for the full-frame synchronization
sequence `FF FF FF 7F`. Known frame-boundary captures can enable `start_aligned`.
Halfword synchronization `FF 7F` is removed at halfword boundaries, including
inside a frame. A full synchronization packet found during a partial frame
reports an alignment error and discards the partial frame. Gaps and UART errors
lose formatter alignment and source ID context; data is not guessed after them.

The formatter keeps a fixed fifteen-byte frame payload in two scalar words,
plus one halfword and counters. The auxiliary byte is required before any frame
payload is exposed. This is a fixed physical framing bound, not transaction or
capture buffering. Every extracted byte covers the formatter frame that proves
it and settles at its auxiliary byte. Pending frames hold progress at the first
captured byte. Truncated frames report errors without leaking tentative payload.
The raw profile retains each source UART byte's own provenance.

Compose `UART → arm-trace-transport → arm-itm`. Other trace packet consumers can
read the same generic byte/error contract. The node does not reconstruct the
parallel TPIU pin interface or Manchester SWO, does not expose all source IDs at
once, and does not infer a source ID before one is observed. Replicate the node
with different `trace_id` values to inspect multiple sources.

Tests include the independent D4.2.1 two-source formatter example, reconstructed
auxiliary low bits, delayed/final-byte ID changes, synchronization insertion,
truncation, UART error propagation, both profiles and block/gap/budget invariance.
