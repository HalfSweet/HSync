<!-- SPDX-License-Identifier: Apache-2.0 -->

# Arm ITM / DWT

`arm-itm` consumes the generic `trace-byte.events@1` byte/error contract.
Compose `UART → arm-trace-transport → arm-itm`; configure the transport as `raw`
for unformatted asynchronous SWO or `tpiu` for a selected formatter source.
It is implemented in `uiharu/dsl/decoders/arm-itm.ts` from
[Arm DDI0553B.y, Part F1](https://community.arm.com/cfs-file/__key/communityserver-discussions-components-files/471/DDI0553B_5F00_y_5F00_armv8m_5F00_arm.pdf).

The default `start_aligned: false` waits for five or more zero bytes followed by
`80`. Enable `start_aligned` only when the capture starts at a known packet
boundary. A stream without synchronization can therefore legitimately produce no
events. The UART baud rate and physical line belong to the preceding UART node.

Source headers select one, two or four little-endian payload bytes. Software
packets report stimulus port, optional observed extension page and raw value.
Hardware packets always retain source number and raw value. The supported DWT
interpretations additionally report event-counter flags, exception number/action,
PC samples and sleep indication. Local timestamp formats 1 and 2, extension
packets, overflow and synchronization are decoded. Timestamp values remain local
deltas; they do not assert an absolute capture clock or attach an instruction to
a sample.

A page is unknown until an applicable single-byte software extension is observed.
Overflow invalidates the observed page because extension packets might have been lost.
Long software extensions are exposed but invalidate the page interpretation.
Global timestamp packets follow Arm DDI0403E.e §D4.2.5
([original Arm manual](https://www.pjrc.com/teensy/DDI0403Ee_arm_v7m_ref_manual.pdf)).
`global-low` reports the transmitted 7, 14, 21 or 26 significant low bits plus
Wrap/ClkCh flags when present. `global-high` reports the transmitted bits above
bit 25 and the observed 48- or 64-bit timestamp width. Short low fragments omit
unchanged bits; their missing bits are not zero-filled into an absolute time.
The decoder deliberately reports components rather than associating them with
source packets or inventing a reconstructed clock after loss. Reserved headers
produce an error and stop packet interpretation until synchronization, preventing
unknown payload lengths from becoming false source packets. Hardware source
formats beyond the listed DWT fields remain raw hardware events; there is no
instruction reconstruction, symbol lookup or memory-access correlation.

The parser keeps only scalar counters, one four-byte source value, one eight-byte continuation value and provenance. It
never buffers a complete trace. Pending source and continuation packets retain
the header span until completion; gaps and end-of-input report a pending packet
as aborted. Gaps, UART framing/parity errors and malformed synchronization lose
alignment and page context. Overflow is an observed target condition and does not
invent missing data. Partial synchronization prefixes do not produce a complete
packet or fabricated data.

Tests contain independent packet-table vectors, payload byte order, extension
context, local timestamps, 48/64-bit global timestamp fragments and reserved-bit/continuation errors, DWT fields, malformed/unsupported headers, overlong
continuations, truncated payloads, recovery and block/budget/gap invariance.
