<!-- SPDX-License-Identifier: Apache-2.0 -->

# JTAG chain selection

`jtag-chain` selects an explicitly configured TAP window from `jtag.events@1`
and emits the same generic contract. Existing JTAG-DP and MEM-AP nodes can consume
its output. The source is TypeScript; the existing physical JTAG producer now
emits the generated contract's types without changing its wire behavior.

The concatenation model follows IEEE1149.1, also described by
[Arm's CoreSight debug introduction](https://developer.arm.com/community/arm-community-blogs/b/architectures-and-processors-blog/posts/how-to-debug-coresight-basics-part-2)
and the [Arm IHI0031A JTAG-DP register model](https://documentation-service.arm.com/static/5f900a61f86e16515cdc0610?token=).
The decoder does not infer chain topology or IR widths from bit patterns.

Configure `ir_before`, `ir_bits`, `ir_after` and their `dr_` equivalents in
**captured shift order**: the number of bits before the selected TAP, the selected
register width and the bits after it. Prefix/suffix widths are 0–4096; selected
IR width is 1–256 and selected DR width 1–4096. Both TDI and TDO are required and
promised on the output. For other TAPs in BYPASS, each contributes one DR bit;
IR widths still come from the actual device specifications.

Each selected bit is streamed with a rebased offset. Its provenance retains the
lower shift batch that proves it. The decoder forwards TAP state/reset and scan
identity. It marks the selected scan complete only when the lower scan commits,
all bit offsets are contiguous and the complete chain length exactly matches the
configured before/selected/after widths. A mismatched, truncated or aborted scan
ends aborted; downstream instruction decoders must not commit it. Reset/gaps
lose pending scan context. No whole scan is buffered: counters and at most one
bounded byte vector per line are held while emitting.

The configured DR width is fixed for this profile. If instructions select
registers of different widths (for example IDCODE32, BYPASS1 and DPACC35), scans
with another width abort until configured accordingly. Automatic per-instruction
DR layouts, dynamic chain insertion and BSDL discovery are not claimed. A
single-TAP capture can use the existing direct JTAG-DP path instead. This profile
adds explicit selection for known multi-TAP layouts, not topology discovery.

Tests independently concatenate surrounding TAP bits, check exact IR/DR
selection and rebased offsets, drive the existing JTAG-DP pipeline, reject wrong
widths and partial scans, and check block/gap/backpressure invariance. Existing
JTAG state-machine, serialization and JTAG-DP tests also run after the generated
contract migration.
