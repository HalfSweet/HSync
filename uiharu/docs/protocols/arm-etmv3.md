<!-- SPDX-License-Identifier: Apache-2.0 -->

# Arm ETMv3 packet stream

`arm-etmv3` consumes `trace-byte.events@1`. Compose UART →
`arm-trace-transport` → `arm-etmv3`, selecting raw SWO bytes or a TPIU source in
the transport node. The parser is independently authored from
[Arm IHI0014Q Chapter 7](https://documentation-service.arm.com/static/5f90158b4966cd7c95fd5b5e).

Configuration reflects target trace settings, which cannot be inferred safely
from arbitrary captured bytes:

| Option            | Default     | Meaning                                                                   |
| ----------------- | ----------- | ------------------------------------------------------------------------- |
| `start_aligned`   | false       | Otherwise wait for at least five zeros and `80` alignment synchronization |
| `context_bytes`   | 0           | Configured Context ID width: 0, 1, 2 or 4 bytes                           |
| `cycle_accurate`  | false       | Select cycle-accurate P-header encoding                                   |
| `minor_version`   | 5           | ETMv3 minor version 0–5; affects supported atom/VMID/timestamp headers    |
| `branch_encoding` | alternative | `original` or ETMv3.4+ `alternative` address encoding                     |
| `data_addresses`  | false       | Whether data address tracing is enabled in the target                     |
| `timestamp_bits`  | 48          | Target timestamp width, 48 or 64                                          |

Non-cycle and cycle P-header formats emit individual E, N and W atoms in captured
order. A cycle count is an observed packet field; zero means counter overflow,
not zero elapsed cycles. It is not automatically attached to capture time or to
an instruction. Instruction synchronization reports Context ID, full address,
ISA/security/AltISA/Hyp flags, reason and the load/store-in-progress flag. For an
LSiP packet its initial address belongs to the data instruction; the subsequent
compressed branch event supplies the current instruction address.

Original and alternative branch encodings reconstruct missing address bits only
when an earlier I-sync or full branch supplies them. A short branch without that
context retains its fragment and leaves `address` absent. Full addresses can
establish ARM, Thumb or Jazelle alignment. Exception information preserves the
captured exception number, cancellation, resume and ISA/security flags. The
legacy fifth-byte EEE encoding is exposed separately as `legacyException`; it is
not confused with the extended exception number.

Normal data, out-of-order placeholders and tagged returns report observed
address/value fields. Address compression uses an observed prior data address;
unknown upper bits remain unknown. A packet without an address does not invent a
contiguous address or pair a return with an unobserved request. `bytes` is the
compressed value width, not necessarily the processor's memory access width.
BE-8 is exposed as the observed flag; raw transferred value bytes retain their
little-endian packet assembly. Value-not-traced packets carry no invented value.

Context ID, VMID, trigger, store failure, data suppression, ignore and exception
entry/exit packets are decoded. Timestamp packets expose their transmitted
fragment, significant bit count and clock-change flag. They do not assume omitted
high bits are zero, select binary versus Gray coding, or fabricate an absolute
clock after loss.

State consists of bounded scalar counters, 64-bit accumulators and provenance.
Each continuation has its specified bound, including five-byte cycle/data and
branch fields, at most three exception-information bytes and seven/nine-byte
48/64-bit timestamps. Gaps, upstream errors, malformed/reserved headers and
truncation invalidate reconstruction context and require alignment synchronization.
The default therefore produces no events from an unsynchronized capture. Pending
packets hold their start provenance until their final byte or the known cut.

Tests use Arm Examples 7-1/7-2, branch and I-sync diagrams, data-address layouts,
error/truncation vectors, all configuration profiles and block/gap/backpressure
invariance. Generated and interpreted execution compare full events and
provenance.

The [pinned upstream ETMv3 decoder](https://github.com/sigrokproject/libsigrokdecode/blob/71f451443029322d57376214c330b518efd84f88/decoders/arm_etmv3/pd.py)
was consulted only to audit capabilities/options; no GPL algorithm or source was
copied or used as the implementation model. Its optional ELF/objdump integration
for instruction disassembly, symbols and source mapping is absent here. No code
image is available through the current decoder's typed stream, and no process or
file access is introduced into the DSL. Instruction-by-instruction PC stepping,
source lines and function names are consequently not claimed. This packet profile
is useful without those facilities but is not full annotation parity.
