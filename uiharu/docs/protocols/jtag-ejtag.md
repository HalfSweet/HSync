<!-- SPDX-License-Identifier: Apache-2.0 -->

# MIPS32 EJTAG

`jtag-ejtag` consumes `jtag.events@1`, directly from a single TAP or an explicitly
configured `jtag-chain` selection. It requires both TDI and TDO. The TypeScript
implementation follows the original MIPS Technologies
[M4K Software User's Manual MD00249, revision 02.03](https://hades.mech.northwestern.edu/images/c/c1/M4K_Software_Manual.pdf),
§§8.4–8.5, and Microchip's
[PIC32 programming specification, §20.2](https://onlinedocs.microchip.com/oxy/GUID-85C6D874-3EAA-4549-9EBC-46E827CEF407-en-US-1/GUID-41934FF6-0FCE-443E-A483-93C7B14FA778.html).
No reference decoder implementation is copied.

The MIPS32 profile decodes IDCODE, IMPCODE, ADDRESS, DATA and CONTROL 32-bit
registers, ALL's 96-bit Control/Data/Address shift order, FASTDATA's leading flag
and 32-bit data, and the one-bit BYPASS path selected by BYPASS/EJTAGBOOT/NORMALBOOT.
`ir_bits` defaults to five; explicit five- through eight-bit wrapper widths are
supported. Reset selects IDCODE according to this profile. A capture beginning
without an observed reset or committed instruction retains unknown instruction
context. Capture-IR's required low `01` pattern is checked before committing an
instruction.

Both input and captured output values remain visible. IDCODE reports numeric
version, part and manufacturer fields; IMPCODE exposes the EJTAG version while
leaving implementation-dependent feature bits raw. CONTROL adds captured PrAcc,
PrNW, reset, debug-mode, doze and halt flags. ADDRESS/DATA context is retained only
while a processor access is observed pending. A `processor-completion-request`
means the probe shifted zero into PrAcc while a pending access was captured; it
is not a verified target memory operation. New non-pending status clears the
context. ALL obtains the address/data/control context from the same captured
register chain.

FASTDATA reports its raw completion request and captured success flag. It marks
transport completion only when completion was requested and that flag was set.
The payload still does not identify a target memory address: target code performs
the actual upload/download, and that code is not reconstructed. EJTAGBOOT and
NORMALBOOT alter future reset behavior; observing them does not invent a CPU
reset or completed boot.

Only fully committed, contiguous scans of the expected width produce register
semantics. Invalid lengths, missing lines, invalid IR capture, unknown
instructions, aborted scans and gaps produce errors or lose context. Optional
TCB/PDtrace registers, MIPS64 register widths, DMA/debug-memory operations and
instruction disassembly are outside this profile; unknown instructions do not
become guessed register accesses. The fixed-DR-width chain selector must match
the register selected in a given capture profile.

At most six 32-bit scalar words, counters and provenance are retained for a
physical register chain. No arbitrary transaction is buffered. Events cover the
scan that proves them and settle at its committed end. Gaps and end-of-input
abort pending scans using captured bounds. Tests use independent register/flag
vectors for ALL, FASTDATA, processor requests and context loss, malformed widths,
IR capture, reset/default ID and block/gap/backpressure invariance.
