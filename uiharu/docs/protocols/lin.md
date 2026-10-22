# LIN

The LIN decoder reads the frames of a LIN (Local Interconnect Network) bus from the characters of a UART stream and emits them as `lin.events`. It follows the LIN Specification Package Revision 2.2A (LIN Consortium; LIN is now standardized as ISO 17987): the frame of §2.3.1, a receiving node's acceptance of a frame up to the next break field (§2.3.2) and the diagnostic frames of §2.3.3.4. The data length LIN 1.x may code into the identifier comes from the LIN Specification Package Revision 1.3, §3.1.2.

A frame is a header the master sends — a break field, the sync byte 0x55 and the protected identifier — and a response of one to eight data bytes and a checksum, sent by the master or a slave. Its bytes are UART characters of eight data bits without parity, so the decoder is stacked on the [UART decoder](uart.md), whose stream carries breaks as well as characters.

```rust
let uart = graph.add(Uart::new(UartConfig {
    line: 0,
    baud: 19_200,
    ..UartConfig::default()
})?);
let lin = graph.add(Lin::new(LinConfig::default())?);
graph.connect(logic.samples, Uart::input(uart))?;
graph.connect(Uart::events(uart), Lin::input(lin))?;
let frames = graph.expose("lin", Lin::events(lin))?;
```

The master and the slaves share LIN's single wire, so one UART stream carries the whole bus.

## Configuration

`LinConfig`, checked by `Lin::new`:

| Field            | Default | Meaning                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `length_by_id`   | `false` | End a frame with its checksum after the number of data bytes bits 4 and 5 of its identifier code, as LIN 1.x allowed: 2 for identifiers 0 to 31, 4 for 32 to 47, 8 for 48 to 63. Otherwise a frame ends at the next break field                                                                                                                                                                                                              |
| `min_break_bits` | 11      | The shortest UART break taken as a break field, in nominal bit times, 9 to 13. 11 is the threshold LIN slaves detect a break with (§2.3.1.1): a master sends at least 13, but one whose clock runs a little fast sends breaks that measure just under 13 at the nominal bit time. 13 takes only breaks as long as a master must send them; 9 takes every break the UART reports, as receivers with an accurate clock may, from 9.5 bit times |

## Requirements

The input port `uart` requires of the UART stream LIN's byte field, a start bit, eight data bits least significant bit first and a stop bit:

| Property   | Requirement                                                                |
| ---------- | -------------------------------------------------------------------------- |
| `dataBits` | `= 8`                                                                      |
| `parity`   | `= "none"`                                                                 |
| `stopBits` | `= "one"`                                                                  |
| `bitOrder` | `= "lsb-first"`                                                            |
| `baud`     | `in 1000..=20000`, the bit rates LIN allows; the bit time is taken from it |

A connection that does not meet them is refused, for example `lin#1.uart requires parity = "none" of uart#0.events, which declares "even"`.

## The stream: `lin.events` v1

Frames are numbered from 1. A frame's first event is its `header`, or an `error` with the reason `sync` when the byte after the break field is not a sync byte; then come its `data`, `checksum` and `error`s, and an `end`, all carrying the frame's id.

| Kind       | Fields                                          | Meaning                                                                                                                               |
| ---------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `header`   | `frame`, `pid`, `id`, `pidOk`                   | The protected identifier as sent, its frame identifier (bits 0 to 5) and whether its parity bits (6 and 7) match                      |
| `data`     | `frame`, `offset: u8`, `bytes`                  | Data bytes of the response in the order they were sent, data 1 first; at most 8 per event. A frame's data normally comes in one event |
| `checksum` | `frame`, `value: u8`, `classicOk`, `enhancedOk` | The response's last byte, and whether it is the classic checksum and whether it is the enhanced one                                   |
| `error`    | `frame`, `reason`                               | Something wrong in the frame, reported as soon as it is seen; the frame then ends `error`                                             |
| `end`      | `frame`, `reason`                               | The frame ended                                                                                                                       |

| Error reason | When                                                                                                                                                      |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `sync`       | the byte after the break field is not 0x55 with its stop bit: the header is not recognized, and the frame ends at once                                    |
| `parity`     | the parity bits of the protected identifier do not match its frame identifier                                                                             |
| `framing`    | a byte of the frame, its identifier or a response byte, has a framing error: its stop bit was dominant. Its value is taken as read                        |
| `checksum`   | the checksum is neither the classic nor the enhanced one; in a diagnostic frame (60 or 61), which always carries the classic checksum, it is not that one |
| `too-short`  | the response is a single byte, with no room for a data byte and a checksum, or, with `length_by_id`, it ended before its data and checksum                |
| `too-long`   | a tenth response byte came, more than eight data bytes and a checksum: the frame ends at once                                                             |

| End reason    | When                                                        |
| ------------- | ----------------------------------------------------------- |
| `complete`    | the response, data and checksum, came without an error      |
| `no-response` | no byte followed the header before the next break field     |
| `error`       | an `error` was reported for the frame                       |
| `aborted`     | a gap or the end of the samples came before the frame ended |

```json
{"kind":"header","frame":1,"pid":80,"id":16,"pidOk":true}
{"kind":"data","frame":1,"offset":0,"bytes":[74,85,147,229]}
{"kind":"checksum","frame":1,"value":230,"classicOk":true,"enhancedOk":false}
{"kind":"end","frame":1,"reason":"complete"}
```

The frame's end is decided where the UART recognizes the next break, at the break's end, or at the start of a gap or the end of the samples; with `length_by_id`, by the checksum.

| Event                                 | Span                                                                                              | Settled                                                                                                                                                                                   |
| ------------------------------------- | ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `header`                              | the start of the break field … the end of the protected identifier                                | the identifier's stop bit                                                                                                                                                                 |
| `data`                                | the data bytes                                                                                    | the stop bit of the byte after the eighth, when eight go out while the frame is under way; with `length_by_id`, the last data byte's stop bit; otherwise where the frame's end is decided |
| `checksum`                            | the checksum byte                                                                                 | where the frame's end is decided; with `length_by_id`, the checksum's stop bit                                                                                                            |
| `error` (`sync`, `parity`, `framing`) | the byte                                                                                          | that byte's stop bit                                                                                                                                                                      |
| `error` (`checksum`)                  | the checksum byte                                                                                 | as the checksum                                                                                                                                                                           |
| `error` (`too-short`)                 | the bytes of the response                                                                         | where the frame's end is decided                                                                                                                                                          |
| `error` (`too-long`)                  | the ninth and tenth bytes of the response                                                         | the tenth byte's stop bit                                                                                                                                                                 |
| `end` (by a break)                    | empty, at the start of the break                                                                  | where the break is recognized, at its end                                                                                                                                                 |
| `end` (by a byte)                     | empty, at the end of that byte: the checksum with `length_by_id`, a wrong sync byte, a tenth byte | that tick                                                                                                                                                                                 |
| `end` (aborted)                       | empty, at the gap's start or the end of the samples                                               | that tick                                                                                                                                                                                 |

The output port `events` declares `lengthById`, `true` when frames end with the length their identifier codes.

## Identifiers and checksums

The protected identifier carries the frame identifier in bits 0 to 5 and two parity bits: P0 = ID0 ⊕ ID1 ⊕ ID2 ⊕ ID4 in bit 6 and P1 = ¬(ID1 ⊕ ID3 ⊕ ID4 ⊕ ID5) in bit 7, so identifier 0x10 is sent as 0x50 and the master request frame's 0x3C as 0x3C.

| Frame identifier | Frames                                                                 |
| ---------------- | ---------------------------------------------------------------------- |
| 0 to 59          | frames that carry signals: unconditional, event triggered and sporadic |
| 60 (0x3C)        | the master request frame, a diagnostic frame of eight data bytes       |
| 61 (0x3D)        | the slave response frame, a diagnostic frame of eight data bytes       |
| 62, 63           | reserved                                                               |

The checksum is the inverted eight-bit sum with carry — the bytes added, 255 subtracted whenever the sum reaches 256 — of the data bytes (the classic checksum, which LIN 1.x frames and the diagnostic frames carry) or of the protected identifier and the data bytes (the enhanced checksum of LIN 2.x frames). Which one a frame carries is in the cluster's description, so both are checked; 0x4A, 0x55, 0x93, 0xE5 sum to 0x19, and their classic checksum is 0xE6.

## Decoding rules

- A break field is a UART break at least `min_break_bits` nominal bit times long, 11 by default, `⌊min_break_bits × rate / baud⌋` ticks with `rate` the source's samples per second: rounded down, so a break of exactly that many bit times is taken whatever the phase of the samples.
- Every break ends the frame under way, since the master begins each frame with one; a break long enough begins the next frame. After a shorter one, characters are skipped up to the next break field.
- The character after the break field must be the sync byte, 0x55 without a framing error; then comes the protected identifier, and the header goes out.
- Every character after it is the response, up to the frame's end: the next break, a gap or the end of the samples, or, with `length_by_id`, the checksum after the data length the identifier codes, after which characters are skipped up to the next break field.
- The response's last byte is its checksum, which is only known when the frame ends, so the decoder holds the last byte back. The data goes out in one batch of at most eight bytes, as soon as the eighth is known to be data or when the frame ends; with `length_by_id` the data goes out with its last byte.
- A frame ends `aborted` when a gap or the end of the samples cuts it, otherwise `error` when an error was reported for it, `no-response` when no byte followed its header, and `complete` otherwise. A header with a wrong parity and no response ends `error`.
- An aborted frame reports the bytes it holds as data and no checksum; it cannot be told whether more were to come. A frame whose header had not gone out — a break, or a break and a sync byte, before a gap, the end or another break — reports nothing.
- At the start of the samples and after a gap the decoder waits for a break field: a frame under way there is skipped.
- Progress is passed on, held back to the start of a break field whose header has not gone out, then to the first byte not reported, or, with `length_by_id`, to the start of the response, which a break before its checksum reports too short as a whole.

## Limits

- The data length of a frame is not on the bus but in the cluster's description, so without `length_by_id` a response ends at the next break field, and the last frame of a capture ends `aborted`.
- Signals, diagnostic requests and the transport layer are not interpreted.
- The frame-time maximum of §2.3.2, `TFrame_Maximum`, 1.4 times the nominal time of the header and of the response, is not checked, though the specification asks tools to check it; collisions of event triggered frames are not told apart from other errors.
- A wake-up signal long enough to be a break begins a frame, which reports nothing unless a byte follows it.

The separate [LIN version and idle profile](lin-profile.md) reads the same
UART contract directly, enforces the selected classic/enhanced checksum, and
can complete the final response at a configured idle deadline without a
following BREAK. Its explicitly configured idle threshold does not infer an
LDF response length or change this native transport's limits.
