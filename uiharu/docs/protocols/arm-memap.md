# Arm DP/AP and MEM-AP plans

A TypeScript stream decoder above `jtag-dp.events` v1, from the [Arm Debug Interface v5 Architecture Specification, IHI0031A](https://documentation-service.arm.com/static/5f900a61f86e16515cdc0610?token=), §§4.4, 6, 8 and 11. It names DP/AP register requests and associates captured responses with the **previous** admitted scan. It reports conditional memory plans; JTAG's combined OK/FAULT acknowledgement never proves that a memory operation succeeded.

## Configuration

Stack `jtag → jtag-dp → arm-memap`. Configure `ap` (0–255, default 0) as the one MEM-AP whose candidate state to track. `tar_wrap_bits` is 10–31, default 10: the configured auto-increment boundary, at least the architecture's minimum 1 KiB window. Select the actual AP implementation's boundary. Other APs remain register observations; no unbounded AP-state map is kept.

## Stream: `arm-memap.events` v1

| Event             | Meaning                                                                                                                             |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `request`         | Admitted current request: scan, AP/DP, observed SELECT's AP index and register address where known, name, read/write and raw DATAIN |
| `response`        | Captured prior result: original request scan, current response scan, raw value, register name and `okOrFault = true`                |
| `memory-request`  | DRW access on the selected AP: conditional `plannedAddress`, CSW size where known, packed flag, direction and raw DATAIN            |
| `memory-response` | Prior DRW request's captured raw response and original conditional address; no successful memory result is claimed                  |
| `status`          | A captured CTRL/STAT value and whether STICKYORUN, STICKYERR or WDATAERR is set                                                     |
| `wait`            | Current access was not admitted; the pending request and candidate TAR do not advance                                               |
| `context-lost`    | Reset, ABORT or damaged/unsupported transport framing invalidated context                                                           |

Unknown SELECT is not assumed to select AP zero. Register addresses remain absent until the appropriate SELECT bank is observed. SELECT, CSW and TAR candidate values update when the later OK/FAULT scan returns the preceding request's result, not when its request first appears. CSW and TAR reads use captured values; writes use observed request values. They remain conditional because an AP fault cannot be excluded from that combined acknowledgement.

DRW plans retain the TAR value that applied when admitted. Address increments happen when that DRW operation's prior response arrives: CSW AddrInc 0 leaves it unchanged, single increments by the configured byte/halfword/word size, packed increments by four. The configured TAR boundary wraps only its low bits. Unsupported sizes or AddrInc values invalidate candidate TAR. Raw 32-bit words are preserved; endian-dependent byte lanes and packed subaccess values are not invented.

A sticky fault invalidates CSW/TAR candidates. An unknown acknowledgement, mismatched response association, gap, reset, ABORT or uninterpretable scan discards pending context. WAIT, including a shortened WAIT poll, retains it. At most one pending request and a fixed number of scalar registers are kept. Every event reuses the transport event's provenance; no unreported old sample span is retained.

## Limits

This layer reports request intent, register observations and planned addresses, not confirmed target effects. It does not identify STM32 families, implement manufacturer debug registers, reconstruct CPU execution, program flash, support ADIv6, infer AP topology or handle banked data registers BD0–BD3 as memory operations. One configured MEM-AP is tracked. It does not consume SWD: the existing mixed string/object ACK serialization needs a DSL representation that the current compiler does not support. The existing SWD decoder remains available unchanged.
