# OpenFSI transport

`fsi` implements the two-wire OpenFSI 1.0.0 packet profile. Data is electrically
inverted by default; `inverted` is explicit wiring configuration. Host commands
sample on rising clock edges and slave responses on falling edges, matching the
POWER9 sampling profile. `turnaround` defaults to three cycles (1–32), and
`response_timeout` bounds response-start waiting (default 256, range 1–65535).

A 256-cycle logical-one BREAK establishes synchronization and clears cached
addresses and pending transactions. Initial alignment defaults to unknown;
`start_aligned` is an explicit assertion that capture begins before a command.
Gaps require a new BREAK, and truncated packets end aborted.

Commands include absolute 21-bit, signed relative 8-bit and same-word addressing,
Data Poll, Error Poll, Interrupt Poll and Terminate. The reserved Same-address
read/size encoding is recognized as Terminate. Byte/halfword/word size bits are
separated from the effective byte address. Four optional cached word addresses
retain slave-specific context. They are established only by CRC-valid matching
acknowledgements; missing bases remain unknown. Error/CRC/mismatched responses
invalidate affected context. A BUSY response preserves its pending transfer for
Data Poll, including the original read width; polling another slave cannot borrow
that context.

CRC4 uses x⁴+x²+x+1, initial zero, including the logical start bit. Each command
and response reports received/expected CRC independently. Data bytes stream
immediately and remain provisional until CRC/end results. Responses distinguish
ACK, BUSY, error-A, error-CRC and interrupt/DMA fields. `success` requires a
CRC-valid matching ACK/interrupt response and a valid host packet; it does not
establish an engine's later side effects. Unknown Data Poll response lengths
require resynchronization instead of guessing a payload width.

This profile does not implement engine-register semantics, device enable/plug
detection, programmable electrical delays or automatic error-recovery policy.
It holds one byte, fixed-width packet counters, four optional addresses and one
pending transfer, never an entire transaction or response series. BREAK counters
saturate; packet/provenance state is bounded.

Primary reference: [OpenPOWER OpenFSI 1.0.0, 2016-12-12](https://openpowerfoundation.org/specifications/fsi/),
chapters 3–4; [public specification mirror](https://wiki.raptorcs.com/w/images/9/97/OpenFSI-spec-20161212.pdf).
Direct PDF retrieval from that mirror intermittently failed with TLS EOF/timeouts;
the indexed primary tables were accessible. Command/response wire facts and the
POWER9 sampling profile were cross-checked against the attributed AGPL-3.0-or-later
libsigrokdecode FSI reference (Raptor Engineering, 2020). This parser, its state
layout and independent polynomial-division vectors are original; no AGPL body
was copied or translated. This external retrieval issue is not an IR blocker.
