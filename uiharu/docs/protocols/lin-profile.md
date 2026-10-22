<!-- SPDX-License-Identifier: Apache-2.0 -->

# LIN version and idle profile

`lin-profile` consumes `uart.events@1`, requiring eight data bits, no parity,
one stop bit and LSB-first order at the configured `baud` (default 19200).
Its independent implementation follows the LIN Consortium's
[LIN Specification Package 2.2A](https://community.nxp.com/pwmxy87654/attachments/pwmxy87654/16-bit/18786/3/LIN_Specification_Package_2.2A.pdf),
Protocol Specification §2.3, and LIN 1.3 identifier-derived data lengths.
The pinned upstream decoder was consulted for the version/idle option inventory;
no GPL implementation source is copied or adapted.

`version` is `1` or `2` (default `2`). Version 1 accepts the classic checksum;
version 2 requires the enhanced checksum for signal frames. Diagnostic IDs 60
and 61 always require the classic checksum. Both checksum comparisons remain
visible alongside `selectedOk`, so a checksum that fits the other profile is
not silently accepted. PID parity, SYNC, UART framing, short/overlong responses
and checksum mismatches are explicit errors.

`minBreakBits` defaults to 11 and accepts 9–13, rounded down to ticks as the
native receiver does. `lengthById` defaults off. When enabled it uses the LIN
1.x ID bits to expect 2, 4 or 8 data bytes and completes at the checksum.
Otherwise one pending byte is retained as the possible checksum; as soon as a
following byte proves it to be data, it is emitted. At most eight data bytes
and one pending byte are accepted; the decoder never buffers a frame.

`idleBits` is an explicit response-completion threshold, default 20 nominal
bit periods (at least 10), rounded up to ticks. It is an analysis profile and
is not claimed to be an LDF-defined response length or a mandatory LIN idle
limit. A response ends at `last-character-end + idleBits` once the lower
stream's progress, a later event or capture end proves that silence occurred.
A frame that has not reached that deadline at capture end remains aborted.
No UART idle event or signal polling is required, and the computed boundary
is independent of source blocks and scheduling. Header-only silence produces
`no-response`. A next BREAK also closes the previous response normally.

`lin-profile.events@1` emits `header`, individual `data` bytes, `checksum`,
`error` and `end`. End records whether the idle threshold completed it. Header
provenance spans BREAK through PID; a pending checksum holds progress at its
source character. A checksum finalized by silence settles at the computed
deadline, while its span remains its source byte. Gaps/early EOF emit the last
unproven byte as data and close the frame aborted. Incomplete headers are
discarded. No cluster signal map, diagnostic application layer or successful
server effect is inferred.
