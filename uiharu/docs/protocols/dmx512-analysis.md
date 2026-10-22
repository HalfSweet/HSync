<!-- SPDX-License-Identifier: Apache-2.0 -->

# DMX512 receiver timing and slots

`dmx512-analysis` is a TypeScript stream decoder directly above `uart.events@1`
at 250 kbaud, eight data bits, no parity, two stop bits and LSB-first order.
UART owns inversion. The original implementation follows ANSI E1.11 DMX512-A
receiver framing and timing; the standard is available from the
[ESTA Technical Standards Program](https://tsp.esta.org/tsp/documents/published_docs.php).
The pinned upstream decoder was consulted for its option inventory only;
no GPL implementation source is copied or adapted.

This direct UART placement preserves every character's timestamp. Native
`dmx512.events` batches slots and cannot expose the marks between characters
inside one batch. The existing native decoder and its exact generated contract
remain available independently.

Configuration defaults are `minBreakUs = 88`, `minMabUs = 8`,
`maxMarkUs = 1000000`, `minBreakIntervalUs = 1196`,
`maxResetIntervalUs = 1250000` and `showZero = false`. These are receiver
analysis thresholds, configurable for a capture's required profile. Minimum
limits are rounded up to source ticks; maximum limits allow the same rounded
tick threshold.

`dmx512-analysis.events@1` emits a `begin` for each UART BREAK, a `start-code`,
individual numbered `slot` values, measured `timing` intervals, explicit
`error` diagnostics, and `end` on the next BREAK or capture interruption.
Start-code profiles name dimmer (0), text (0x17), RDM (0xCC), system information
(0xCF), or other. Alternate payloads remain slots, including zero-valued bytes;
the profile does not claim RDM or text-packet application decoding. Zero
filtering affects only dimmer values, with original slot indices retained.
A 513th data slot reports `too-many-slots` and later observed slots continue.
Framing errors retain the observed value and emit a separate diagnostic.

Intervals cover MAB, each inter-slot MARK, inter-packet MARK, BREAK-to-BREAK,
and RESET-to-RESET (completion of consecutive null-start-code slots). They
are measured from UART's exact character spans; interval events and diagnostics
use the current deciding character/BREAK's provenance, with previous endpoints
retained as scalar context. No output is backdated across progress. RESET
context survives alternate-code packets and resets at a gap.

Characters before the first BREAK are ignored. A packet missing its start code
reports an error when closed. Every open packet ends `aborted` at gap/EOF;
there is no fabricated timeout boundary. The decoder retains only scalar
counters, the current code and previous timing endpoints, never a complete
packet or slot batch. `p3_semantics.rs` checks independent timing/filter vectors,
compiled/interpreted equality, tiny queues, pauses and gaps.
