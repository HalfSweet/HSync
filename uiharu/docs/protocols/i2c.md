# I²C

The `i2c` decoder (`uiharu_protocols::i2c::I2c`) reads SCL and SDA from logic samples and emits `i2c.events` version 1: the transactions on the bus, from START to STOP, with their addresses, data bytes and acknowledge bits. It is implemented from NXP UM10204, "I²C-bus specification and user manual", Rev. 7 (2021). Decoders built on I²C read the stream through `i2c::I2cBus`; `i2c::synth` builds waveforms for tests and tools.

## Configuration

| Field | Meaning           | Default |
| ----- | ----------------- | ------- |
| `scl` | The clock channel | `0`     |
| `sda` | The data channel  | `1`     |

Both lines are required, on two different channels below 64. There is nothing else to set: 10-bit addresses are recognised from their first byte, and the sample rate does not enter.

## Events

Every event names the transaction it belongs to, `tx`, numbered from 1.

| Kind      | Fields                                   | On the bus                                                                        |
| --------- | ---------------------------------------- | --------------------------------------------------------------------------------- |
| `begin`   | `tx`, `partial`                          | A START condition; with `partial: true`, a transaction found under way            |
| `address` | `tx`, `address`, `tenBit`, `read`, `ack` | The address after a START or repeated START                                       |
| `data`    | `tx`, `byte`, `ack`                      | A data byte                                                                       |
| `restart` | `tx`, `droppedBits`                      | A repeated START: the transaction goes on with another address                    |
| `end`     | `tx`, `reason`, `droppedBits`            | The end of the transaction: `stop`, `restart` (partial transactions) or `aborted` |

- `address` is the 7-bit address, or the 10-bit address when `tenBit` is true; `read` is the R/W̄ bit (true: the controller reads from the target).
- `ack` is true when SDA was low on the ninth clock. The receiver drives it: the target when the controller writes, the controller when it reads. For a 10-bit write address, both its bytes must have been acknowledged.
- `byte` is the data byte, sent most significant bit first.
- `droppedBits` counts the bits of an unfinished byte that the repeated START, STOP, gap or end of the samples cut short; they make no byte.
- `reason` is `stop` for a STOP condition (the transaction is whole); `restart` for a START condition that ended a partial transaction, the next transaction beginning at the same tick; `aborted` for a gap or the end of the samples while the transaction was under way.

Within a transaction whose START was seen, the first event, and the first after every `restart`, is an `address`, and addresses come nowhere else. A partial transaction carries no `address`, `data` or `restart`.

### Provenance

| Event                      | Span                                                                         | Settled                                                                            |
| -------------------------- | ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `begin`                    | the START's tick; for a partial transaction, the tick the bus was found busy | that tick                                                                          |
| `address`, `data`          | the first bit's SCL rising edge … the acknowledge bit's rising edge + 1      | the acknowledge edge                                                               |
| `address` (10-bit write)   | both address bytes                                                           | the second byte's acknowledge edge                                                 |
| `address` (7-bit fallback) | the `1111 0xx` byte                                                          | where its other part was known to be missing: the STOP, repeated START, gap or end |
| `restart`                  | the repeated START's tick                                                    | that tick                                                                          |
| `end` (`stop`, `restart`)  | the condition's tick                                                         | that tick                                                                          |
| `end` (`aborted`)          | empty, at the gap's start or the end of the samples                          | that tick                                                                          |

As NDJSON lines (see `design.md` §8), a write of `0x12` to `0x50` sampled at eight samples per SCL period, on a stream named `i2c`:

```json
{"stream":"i2c","seq":0,"span":[10,11],"settled":10,"event":{"kind":"begin","tx":1,"partial":false}}
{"stream":"i2c","seq":1,"span":[16,81],"settled":80,"event":{"kind":"address","tx":1,"address":80,"tenBit":false,"read":false,"ack":true}}
{"stream":"i2c","seq":2,"span":[88,153],"settled":152,"event":{"kind":"data","tx":1,"byte":18,"ack":true}}
{"stream":"i2c","seq":3,"span":[162,163],"settled":162,"event":{"kind":"end","tx":1,"reason":"stop","droppedBits":0}}
```

## Props

The output declares:

| Key                | Value                                                        |
| ------------------ | ------------------------------------------------------------ |
| `bus`              | `"i2c"`                                                      |
| `framing`          | `"start-stop"`: transactions are framed by `begin` and `end` |
| `tenBitAddressing` | `true`: 10-bit addresses may occur                           |

The input requires the source to have at least `max(scl, sda) + 1` channels.

## Decoding rules

- A START condition is SDA falling while SCL is high, a STOP condition SDA rising while SCL is high. An SDA change at the very tick SCL changes is a data change, never a condition: the order of two changes within one sample cannot be seen.
- A bit is SDA's level at SCL's rising edge. The rising edge on which a START or STOP then happens belongs to the condition, not to a byte: it is the clock a controller raises to make one.
- A byte is eight bits, most significant first, then the acknowledge bit on the ninth rising edge: SDA low is ACK, high is NACK. The byte is reported at that edge, so a START or STOP during the acknowledge clock comes after it; eight bits followed directly by a STOP read as an acknowledged byte, as a STOP raises SCL with SDA low.
- The first byte after a START or repeated START is the address. `1111 0xx` with R/W̄ = 0 begins a 10-bit address that the next byte completes (A9 and A8 from the first byte, A7–A0 from the second); one `address` covers both. `1111 0xx` with R/W̄ = 1 reads from the 10-bit address the transaction last addressed for writing, if its two high bits match and no other address came between.
- A `1111 0xx` byte whose other part was not seen — the first byte of a write address followed by a STOP, a repeated START, a gap or the end of the samples; a read with no matching write address before it — is reported as the 7-bit address it is (`0x78`–`0x7b`, `tenBit: false`).
- Reserved addresses — the general call (`0x00`, write), the START byte (`0x00`, read), CBUS, Hs-mode controller codes, device ID (`0x7c`–`0x7f`) — are reported as addresses like any other; what the bytes after them mean is for the decoders above.
- A repeated START stays in the transaction: `restart`, then another `address`.
- Bits clocked since the last whole byte when a START, STOP, gap or the end of the samples interrupts them make no byte; the `restart` or `end` counts them. At a gap or the end of the samples, a bit whose clock is still high is not counted: it could have turned into a condition.
- Clock stretching, SCL held low by a target, needs nothing: only rising edges are sampled.

## Partial and aborted transactions

The bus is busy when the samples begin, or resume after a gap, with SCL or SDA low, and when SCL changes while no transaction is open (clocks before any START). The transaction under way then begins with `partial: true` at that tick. Its bytes cannot be aligned — the decoder cannot know which bit of which byte it came in on — so it reports none. It ends at the next STOP, `stop`, or at the next START, which ends it with `restart` and begins the next transaction at the same tick.

A gap or the end of the samples while a transaction is open ends it with `aborted`; the gap or the end follows. After a gap the bus state is learnt again from the first sample.

## The cursor

`I2cBus::next()` waits for the next transaction, gap, progress or end and hands out a `Transaction`:

| Method                               | What it does                                                                                      |
| ------------------------------------ | ------------------------------------------------------------------------------------------------- |
| `id()`, `is_partial()`, `is_ended()` | The transaction's id; whether its beginning was missed; whether it has ended                      |
| `event()`                            | The next `address`, `data` or `restart`; the normal end is the error `Incomplete`                 |
| `event_or_end()`                     | The next event, or `Read::End` at the normal end (`stop`, or `restart` for a partial transaction) |
| `drain()`                            | Consumes the rest of the transaction up to its normal end                                         |
| `consumed()`                         | The ticks of what was read since it was last called, for the provenance of what a decoder emits   |

An aborted transaction is the error `Truncated`; a gap or the end of the stream under an open transaction is `Gap` or `EndOfInput`, the item itself following from `next()`. Which addresses a protocol answers to, how wide its register addresses are and its byte order are the decoder's own code.

## Limits

- There is no spike filter: a glitch on SCL counts as a clock. The decoder needs each SCL level, and each SDA change between two SCL edges, on a sample of its own.
- Arbitration between controllers is not detected; the bus shows the winner's bits. Hs-mode and SMBus extensions (timeouts, packet error checking) are not decoded: their bytes decode as bytes.
