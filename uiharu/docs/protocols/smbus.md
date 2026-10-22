<!-- SPDX-License-Identifier: Apache-2.0 -->

# SMBus transactions

`smbus` consumes the generic I²C transaction contract. Its original TypeScript
implementation follows the [SMBus 3.2 specification](https://www.smbus.org/specs/SMBus_3_2_20220112.pdf),
sections 2.5.3, 6.4 and 6.5. It validates observed transaction structure and PEC;
it does not implement the SMBus electrical limits, clock-low timeout, arbitration,
Alert signaling or application-specific command meanings.

Select a transaction profile explicitly: Quick, Send/Receive Byte, Write/Read
Byte or Word, Process Call, Block Write/Read/Process Call, Host Notify, or the
32-bit and 64-bit read/write protocols. The default `raw` profile reports bytes
and checks PEC without guessing a command's transaction format. `address` may
restrict the seven-bit target address. `pec` selects a final PEC byte; Quick and
Host Notify omit PEC regardless of this option. `max_bytes` (default 512) bounds
one message's bytes; `max_block_bytes` (default 255) can impose an older device's
smaller block limit. SMBus 3.2 permits zero-length and up to 255-byte blocks;
Block Process Call additionally limits the sum of both counts to 255.

PEC is CRC-8 polynomial 0x07, initial zero, over every address/R/W byte,
command and data byte across repeated START. ACK, START and STOP do not enter
the CRC. One pending byte distinguishes final PEC from payload. An interrupted
message does not turn its final observed payload byte into a supposed PEC.
A successful end requires the selected length/direction pattern, successful
write acknowledgments, no data after a controller read NACK, and valid PEC when
selected. Read payload must end with NACK. Host Notify extracts the seven-bit
initiating device address; its least-significant wire bit is explicitly a
don't-care in the specification, so either value is accepted.

Command, count, payload bytes, numeric values, PEC and end events retain their
provenance. Scalar words use low-byte-first assembly; 64-bit values expose
`low` and `high` 32-bit halves. Every byte is emitted as it arrives, independent
of whether the eventual transaction is valid. Consumers must inspect the end
result before treating a transaction as accepted. A bounded message-wide
provenance hold permits final PEC and aggregate fields to span their source
bytes. No payload array is retained. A configured byte limit emits `limit` and
drains the rest of the transaction. Gaps, missing input and EOF abort retained
context. A partial initial transaction is drained to the next boundary.

`uiharu-interp/tests/smbus.rs` contains an independent polynomial-division PEC
oracle, the published MLX90614 Read Word vector, scalar/block profiles,
zero/maximum blocks, malformed ACK/PEC/length cases, Host Notify don't-care
values, and compiled/interpreted comparisons under block cuts, queue pressure,
budgets, reader pauses and gaps.
