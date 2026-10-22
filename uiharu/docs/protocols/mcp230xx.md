# MCP23008 / MCP23017

`mcp230xx` is a TypeScript DSL decoder over `i2c.events`, implemented independently
from the [manufacturer datasheet](https://ww1.microchip.com/downloads/en/DeviceDoc/20001952C.pdf) (MCP23017 DS20001952C §§3.2/3.5 and MCP23008 DS21919E). Its source is
[`mcp230xx.ts`](../../dsl/decoders/mcp230xx.ts); generated Rust and IR are never edited by hand.

## Configuration and contract

`address` is a seven-bit target address, default `0x20`. Ten-bit addresses and
other targets are ignored. Input is `i2c.events` version 1; output is
`mcp230xx.events` version 1 with output property `address`.

| Kind      | Meaning                                                                                 |
| --------- | --------------------------------------------------------------------------------------- |
| `begin`   | One addressed segment: `op`, lower `tx`, `read` direction, address `ack`                |
| `pointer` | Attempted register/command pointer and its ACK; absent on pointerless devices           |
| `byte`    | Immediate raw byte, optional known `register`, `read`, `ack`                            |
| `field`   | Decoded numeric field: `register`, enum `field`, magnitude `value`, `negative`, `valid` |
| `end`     | Segment result: complete, incomplete, aborted or nack                                   |

A read-ending NACK is normal and its final byte is retained. An unacknowledged
write does not change interpreted device state and invalidates the pointer.
An unknown pointer stays unknown; raw bytes are retained. START/repeated START
closes a segment; a STOP closes the last segment. Initial partial I²C frames are
drained. Gaps and truncation abort open segments and clear pointer knowledge.

## Device rules

`model` selects mcp23008 or mcp23017 (default). `initial_bank` and `initial_seq_off` state explicitly assumed initial IOCON bits, both false by default. Observed IOCON reads/acknowledged writes update BANK and SEQOP. Register fields identify direction, inversion, interrupt enable/default/control, IOCON, pull-ups, interrupt flags/capture, GPIO and output latches; `port` is 0 for A and 1 for B. BANK=0 uses paired A/B addresses; BANK=1 uses 0x00–0x0a and 0x10–0x1a. SEQOP=1 holds the pointer on the 8-bit device or BANK=1, but toggles A/B pairs on the 16-bit device with BANK=0. Sequential access wraps after the final register. NACK does not advance the pointer. A gap loses mode knowledge, so later bytes are preserved without guessed register semantics or auto-increment.

## Streaming and limits

Every byte is reported before later data is read. There is no transaction-sized
buffer: state is a pointer, counters, constant-size mode flags, and at most one
unfinished byte with its provenance. Multi-byte field spans cover only their
observed bytes and settle on their final byte; a held first byte holds progress
back to its start. Byte/field events preserve consumed I²C event provenance;
segment ends use the item that decided them. Bounded queues and awaited reads/writes
provide backpressure and cancellation.

No SPI MCP23S devices, pin electrical levels, interrupt timing, reset-pin inference or register-cache reconstruction. Initial mode assumptions are configuration, not a detected power-on reset. Bank-1 holes remain unknown register bytes.

The 8-bit model is also specified by [Microchip MCP23008 DS21919E](https://ww1.microchip.com/downloads/en/DeviceDoc/21919e.pdf).
