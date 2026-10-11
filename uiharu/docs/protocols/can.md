# CAN

`uiharu_protocols::can::Can` decodes classic CAN frames (CAN 2.0A and 2.0B) and CAN FD frames from the receive line of a transceiver and emits them as `can.events`, version 1. It follows ISO 11898-1:2015, the Bosch CAN Specification 2.0 (parts A and B) for the classic frame, and the Bosch CAN with Flexible Data-Rate Specification 1.0 for the FD frame, with the ISO additions to it: the stuff count and the CRC initial value.

## Configuration

| Field          | Default | Meaning                                                                                                                                                 |
| -------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rx`           | 0       | The channel carrying the receive line, as the transceiver puts it out.                                                                                  |
| `dominant_low` | `true`  | Whether the dominant level is low, as on a transceiver's RX pin.                                                                                        |
| `bitrate`      | 500 000 | The nominal bit rate in bit/s: the arbitration phase of every frame, and all of classic frames and of FD frames without bit rate switching.             |
| `fd_bitrate`   | none    | The data bit rate of FD frames with BRS recessive; at least `bitrate`. Without it such frames end `unsupported` at BRS.                                 |
| `sample_point` | 87      | Where a bit is sampled, in percent of the bit time from its start, 1 to 99; the same in both phases.                                                    |
| `fd`           | `true`  | Whether FD frames are recognised by their FDF bit. When off, every frame is classic and the bit is the reserved bit `r0` (`r1` in the extended format). |

The input requires `channels` of at least `rx + 1` and `rateHz` of at least four samples per bit at the fastest bit rate the decoder follows: `fd_bitrate` when `fd` is on and it is set, `bitrate` otherwise. The graph refuses a source that does not meet them, naming the property.

## Events

The stream is framed. Every frame has a `begin` and an `end` with the same `frame` id, numbered from 1 in the order frames begin; between them come the frame's fields in bus order, each once, except `data`, which comes in batches.

| `kind`    | Fields                                                 | Meaning                                                                                                                                                                                                                                                            |
| --------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `begin`   | `frame`, `partial`                                     | A frame began: at its SOF, or at the dominant bit of an overload frame. `partial` marks traffic the decoder did not decode because the bus was busy when the samples began or resumed after a gap; only its `end` follows.                                         |
| `id`      | `frame`, `id`, `extended`                              | The identifier: 11 bits, or 29 when `extended` (IDE recessive).                                                                                                                                                                                                    |
| `control` | `frame`, `remote`, `fd`, `brs`, `esi`, `dlc`, `length` | `remote`: a classic remote frame (RTR recessive), which has no data field. `fd`: FDF recessive. `brs`, `esi`: the FD bits, recessive. `dlc`: the data length code. `length`: the bytes the code stands for; in a remote frame, the length requested.               |
| `data`    | `frame`, `offset`, `bytes`                             | Data bytes in bus order, at most 32 per event; `offset` is the position of the first in the data field.                                                                                                                                                            |
| `crc`     | `frame`, `value`, `ok`, `stuffCount`                   | The CRC sequence as received (15 bits in classic frames, 17 or 21 in FD frames) and whether it matches. In FD frames `stuffCount` is the stuff count decoded from its Gray code, and `ok` also requires it and its parity to be right; classic frames have `null`. |
| `ack`     | `frame`, `acked`                                       | Whether a receiver acknowledged the frame with a dominant ACK slot.                                                                                                                                                                                                |
| `end`     | `frame`, `reason`                                      | The frame ended, for the reason below.                                                                                                                                                                                                                             |

`length` is 0 to 8 in classic frames (codes 9 to 15 mean 8 bytes) and 0 to 8, 12, 16, 20, 24, 32, 48 or 64 in FD frames. A frame that ended early lacks the events of the fields it did not reach.

| `reason`      | The frame ended because                                                                                                                                                                       |
| ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `eof`         | Its end of frame was received. Receivers accept it when `crc.ok` is true as well.                                                                                                             |
| `error-frame` | Six consecutive dominant bits, an active error flag, destroyed it.                                                                                                                            |
| `overload`    | It is an overload frame: a dominant bit at the last bit of end of frame, or in the first or second bit of the intermission.                                                                   |
| `stuff-error` | A sixth equal recessive bit came where a stuff bit was due, or a fixed stuff bit of an FD CRC field equalled the bit before it.                                                               |
| `form-error`  | A dominant bit came where its form calls for a recessive one: the CRC delimiter, the ACK delimiter or the first six bits of end of frame.                                                     |
| `unsupported` | The decoder cannot follow it: an FD frame with a recessive `res` bit (a protocol exception; CAN XL frames look like this), or one that switches bit rates when no `fd_bitrate` is configured. |
| `aborted`     | The samples ended or were lost while it was under way, or it was already under way when they began (`partial`).                                                                               |

A classic frame with ID 0x123 and the data AB CD, as serde writes the events:

```json
{"kind":"begin","frame":1,"partial":false}
{"kind":"id","frame":1,"id":291,"extended":false}
{"kind":"control","frame":1,"remote":false,"fd":false,"brs":false,"esi":false,"dlc":2,"length":2}
{"kind":"data","frame":1,"offset":0,"bytes":[171,205]}
{"kind":"crc","frame":1,"value":32572,"ok":true,"stuffCount":null}
{"kind":"ack","frame":1,"acked":true}
{"kind":"end","frame":1,"reason":"eof"}
```

### Provenance

A field's event covers its bits, from the start of the first to the end of the last as the bit timing placed them when the last was sampled, stuff bits inside the field included, and settles at the last bit's sample point.

| Event                               | Span                                                                        | Settled                                                                                                                                  |
| ----------------------------------- | --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `begin`                             | the SOF bit                                                                 | its sample point                                                                                                                         |
| `begin` (partial)                   | the first sample of the stream or after the gap                             | where the bus was first seen dominant                                                                                                    |
| `id`                                | the identifier bits (ID-28 to ID-18, or to ID-0 with SRR and IDE inside)    | the last bit; a base identifier at IDE, which says it is base                                                                            |
| `control`                           | RTR or RRS through the last DLC bit                                         | the last DLC bit                                                                                                                         |
| `data`                              | the bytes of the batch                                                      | the last byte's last bit, or the end that cut the field short                                                                            |
| `crc`                               | the CRC sequence; in FD frames the CRC field from its first fixed stuff bit | the last CRC bit                                                                                                                         |
| `ack`                               | the dominant ACK bit, or the ACK slot when there is none                    | its sample point; one bit later for an FD frame not acknowledged                                                                         |
| `end` (`eof`)                       | the seven bits of end of frame                                              | the last one                                                                                                                             |
| `end` (`error-frame`)               | the six dominant bits                                                       | the sixth                                                                                                                                |
| `end` (other errors, `unsupported`) | the offending bit                                                           | its sample point; when the offending bit was dominant, the bit where the line turned recessive again, or where the samples stopped first |
| `begin`, `end` (`overload`)         | the dominant bit                                                            | its sample point                                                                                                                         |
| `end` (`aborted`)                   | empty, where the samples stopped or the bus was found idle                  | that tick                                                                                                                                |

## Props

| Prop          | Value                                                                  |
| ------------- | ---------------------------------------------------------------------- |
| `bitrate`     | The nominal bit rate, bit/s.                                           |
| `samplePoint` | The sample point, percent.                                             |
| `fd`          | Whether FD frames are recognised.                                      |
| `fdBitrate`   | The data bit rate, bit/s; declared only when `fd` is on and it is set. |

## Decoding rules

- **Bit timing.** A bit lasts `R / B` ticks (`R` the source's sample rate, `B` the bit rate), kept as an exact fraction: counted from the last synchronisation at tick `s`, bit `k` starts at `s + ⌊k · R / B⌋` and is sampled at `s + ⌊(100k + p) · R / (100 · B)⌋` for a sample point of `p` percent. Rounding down happens once per position, so it never accumulates.
- **Synchronisation.** A hard synchronisation on the edge of SOF; after it, every recessive-to-dominant edge becomes the start of the bit being read, however far from where that bit was expected — a simplified resynchronisation, without phase segments or a jump width limit. As the specifications require, an edge synchronises only when the bit sampled before it was recessive, and only once between two sample points. A dominant pulse that is over by the sample point is no SOF.
- **Integration.** At the first sample and after a gap the decoder waits for eleven consecutive recessive bits before it accepts a SOF, as a CAN node joining the bus does; traffic meanwhile is one `partial` frame.
- **Stuffing.** From SOF through the CRC sequence in classic frames, and through the data field in FD frames, a stuff bit of the opposite value follows five equal bits and is removed. The CRC field of FD frames has fixed stuff bits instead: one before the stuff count, then one after every fourth bit, each the inverse of the bit before it. The first stands in for a dynamic stuff bit due after the data field, which is not sent and not counted.
- **Bit rate switching.** In FD frames with BRS recessive the data bit rate applies from the sample point of BRS to the sample point of the CRC delimiter.
- **CRC.** Classic frames: CRC-15 over the destuffed bits from SOF through the data field. FD frames: CRC-17 for up to 16 data bytes, CRC-21 above, over the same bits with their dynamic stuff bits and then the stuff count (three Gray-coded bits and an even parity bit), from an initial value of 1 followed by zeros; the fixed stuff bits are left out. The stuff count is the number of dynamic stuff bits modulo 8.
- **ACK.** A dominant ACK slot acknowledges the frame. FD frames accept the acknowledgement one bit late, after a CRC delimiter two bits long, and two bits long.
- **Errors.** Six consecutive dominant bits anywhere in a frame end it as `error-frame`. A dominant bit where the frame's form calls for a recessive one ends it as `form-error`, and a wrong fixed stuff bit as `stuff-error`; when that bit is dominant, the decision waits for the line to turn recessive, so that an error flag starting there is reported as `error-frame`. After an error or overload flag the decoder waits for the eight recessive bits of its delimiter and the intermission before it accepts a SOF; dominant bits inside a delimiter start it over and are not reported.
- **Overload.** A dominant last bit of end of frame does not affect the frame (receivers do not check that bit); it, and a dominant bit in the first two bits of the intermission, are each reported as an overload frame of its own. A dominant third bit of the intermission is a SOF.
- **Unsupported frames.** After `unsupported` the decoder integrates again; the rest of the frame is not reported as a partial one.
- **Gaps and the end of the samples.** A frame under way ends `aborted` where the samples stop, after the data bytes already received; the gap or the end is passed on. The byte under way is dropped: leftover bits never make a field.
- Events do not depend on how the samples are cut into blocks, on how often the reader pauses, or on queue sizes and budgets.

## Reading the stream

Decoders built on CAN read the stream through `can::CanBus`. `next()` waits for the next frame, gap, progress or end; a frame comes as a `Frame` cursor with these reads, each of which can wait across blocks:

| Read           | Returns                                                                                  |
| -------------- | ---------------------------------------------------------------------------------------- |
| `identifier()` | the `id` and `extended` flag                                                             |
| `control()`    | the control field                                                                        |
| `byte()`       | the next data byte                                                                       |
| `end()`        | how the frame ended: the reason, the CRC field and the acknowledgement, as far as it got |

Reads move forward through the frame and consume what they pass over. A read whose field is gone — the data used up or absent, or the frame ended before the field — returns `ReadError::Missing` and leaves what it found for the next read; only `end()` consumes the frame's end, draining whatever was not read, and `Ending::is_valid` tells whether receivers accept the frame. `consumed()` gives the provenance of what was read since it was last called. `CanBus::finish()` drains a frame its reader left open.

## Limits

- One receive line, one sample point for both phases; the resynchronisation has no phase segments and no jump width limit.
- FD frames follow ISO 11898-1:2015. Frames of the earlier, non-ISO FD format, which has no stuff count, fail their CRC check or end in a stuff error. CAN XL frames end `unsupported`.
- Error-passive flags, the suspend transmission field and dominant bits inside error and overload delimiters are not reported.
- Tested tolerance: edges moved a tenth of a bit early or late, singly or together, except late dominant edges combined with early recessive ones (which take a fifth off every dominant bit); in FD frames that switch bit rates, a tenth of a data bit everywhere, since a shift before the switch is carried into the data phase. A transmitter whose bit time is 1 % off decodes too.
- Sources need at least four samples per bit at the fastest bit rate.
