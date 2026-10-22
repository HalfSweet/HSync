# ADXL345 I²C

`adxl345` is a TypeScript DSL decoder over `i2c.events`, implemented independently
from the [manufacturer datasheet](https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf) (Rev G, I²C and register map sections). Its source is
[`adxl345.ts`](../../dsl/decoders/adxl345.ts); generated Rust and IR are never edited by hand.

## Configuration and contract

`address` is a seven-bit target address, default `0x53`. Ten-bit addresses and
other targets are ignored. Input is `i2c.events` version 1; output is
`adxl345.events` version 1 with output property `address`.

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

The pointer advances through sequential register bytes. Registers 0x32–0x37 pair little-endian axis bytes within one addressed segment. Fields x-raw/y-raw/z-raw contain the magnitude of the signed 16-bit wire value and a separate negative flag. DEVID, BW_RATE, POWER_CTL, INT_ENABLE, INT_MAP, INT_SOURCE, DATA_FORMAT and FIFO registers receive explicit names; other registers remain raw. Values are streamed one byte at a time; only one low axis byte is retained.

## Streaming and limits

Every byte is reported before later data is read. There is no transaction-sized
buffer: state is a pointer, counters, constant-size mode flags, and at most one
unfinished byte with its provenance. Multi-byte field spans cover only their
observed bytes and settle on their final byte; a held first byte holds progress
back to its start. Byte/field events preserve consumed I²C event provenance;
segment ends use the item that decided them. Bounded queues and awaited reads/writes
provide backpressure and cancellation.

No g/mg conversion is inferred without known DATA_FORMAT/justification settings; raw axis values retain the wire representation. No SPI transport, FIFO reconstruction, activity/tap physical interpretation or orientation inference.
