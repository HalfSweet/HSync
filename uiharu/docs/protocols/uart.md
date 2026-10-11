# UART

The UART decoder reads the characters of one asynchronous serial line from logic samples and emits them as `uart.events`. It follows the start-stop character format of TIA/EIA-232-F as 16550-class UARTs receive it: the line idles at mark; a character is a start bit at space, 5 to 9 data bits, an optional parity bit and 1, 1½ or 2 stop bits at mark; a line held at space through a whole character is a break.

One decoder reads one line. A link with a receive and a transmit line takes two decoders; the events know nothing of direction.

## Configuration

`UartConfig`, checked by `Uart::new`:

| Field          | Default     | Meaning                                                |
| -------------- | ----------- | ------------------------------------------------------ |
| `line`         | 0           | The channel the line is on                             |
| `baud`         | 115200      | Bits per second, above zero                            |
| `data_bits`    | 8           | Data bits per character, 5 to 9                        |
| `parity`       | `none`      | `none`, `even`, `odd`, `mark` or `space`               |
| `stop_bits`    | `one`       | `one`, `one-and-a-half` or `two`                       |
| `bit_order`    | `lsb-first` | `lsb-first` or `msb-first`                             |
| `inverted`     | `false`     | The line idles low: mark is low and space high         |
| `sample_point` | 50          | Where each bit is read, in percent of the bit, 1 to 99 |

The input port `logic` requires of the source `channels >= line + 1` and `rateHz >= 2 × baud`: with fewer than two samples per bit the bits cannot be told apart, so the graph refuses the connection, for example `uart#0.logic requires rateHz >= 200000 of logic#0.samples, which declares 150000`.

## The stream: `uart.events` v1

The stream is unframed: every character is one event, decided when its last stop bit has been read. There is no `begin` or `end`, and nothing is ever aborted: a character the decoder could not read to its end is not reported.

| Kind    | Fields                                              | Meaning                                                                                                                                                                                                                                                   |
| ------- | --------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `frame` | `value: u16`, `parity: ok/error/null`, `framing_ok` | A character. `value` holds the data bits, the first received being bit 0 (least significant bit first) or the highest bit (most significant first); `parity` is `null` on a line without a parity bit; `framing_ok` says whether every stop bit read mark |
| `break` | `ticks: u64`                                        | The line was held at space through a whole character, stop bits included, then returned to mark; `ticks` is how long it was at space                                                                                                                      |

```json
{"kind":"frame","value":65,"parity":"ok","framing_ok":true}
{"kind":"break","ticks":1234}
```

| Event   | Span                                               | Settled                          |
| ------- | -------------------------------------------------- | -------------------------------- |
| `frame` | the start edge … the computed end of the stop bits | the last stop bit's sample point |
| `break` | the start edge … the first tick back at mark       | that tick                        |

The output port `events` declares:

| Property   | Value                                              |
| ---------- | -------------------------------------------------- |
| `baud`     | the baud rate                                      |
| `dataBits` | 5 to 9                                             |
| `parity`   | `"none"`, `"even"`, `"odd"`, `"mark"` or `"space"` |
| `stopBits` | `"one"`, `"one-and-a-half"` or `"two"`             |
| `bitOrder` | `"lsb-first"` or `"msb-first"`                     |

The bit time in ticks is not among them: it depends on the rate of the source, which a reader learns from its own time base and combines with `baud`.

### Progress

Readers can rely on two promises that let them measure silences on the line:

- progress never passes the start edge of a character or break still being read: after progress at tick _t_, no event begins before _t_;
- before a gap or the end of the stream is passed on, progress is announced up to the gap's start or the end, or up to the start edge of the character they cut off.

## Decoding rules

- Mark, the idle level, is high, or low on an inverted line; space is the other level. A mark bit is a 1, a space bit a 0.
- A character begins at an edge from mark to space. The start bit is read at its sample point; if the line is back at mark there, the edge was a glitch and the search for a start edge goes on from that point.
- A character is timed from its start edge: every later bit is read at its sample point and nowhere else, so edges inside it are just its bits. Bit _n_, the start bit being bit 0, is read at `start + ⌊rate × (100 n + p) / (100 × baud)⌋`, `rate` being the source's samples per second and `p` the sample point in percent; the half stop bit of 1½ stop bits is read at `p` percent of its half bit. Every position is computed from the start edge and rounded down, so rounding never accumulates.
- The parity bit is checked against the data bits: even and odd parity count the 1 bits, mark and space parity want a fixed bit. Every stop bit must read mark, or the character has a framing error.
- A character whose every bit reads space, its parity and stop bits included, is a break: the decoder waits for the line to return to mark and reports how long it was held at space. A reader that wants a stricter rule, such as a full character time, can apply it to `ticks`.
- After a character the search for the next start edge begins at its last stop bit's sample point, so a sender somewhat faster than the configured rate is followed and characters may follow each other without idle time.
- The first sample of the stream and the first after a gap are never an edge. A line found at space there is a character under way, which is skipped: decoding starts at the next edge to space once the line is back at mark. That edge can lie inside a character whose start was not seen, so the first characters may be misaligned until the line idles; framing errors usually show it.
- A character or break cut off by a gap or by the end of the samples is not reported. After a gap the search for a start edge resumes at the gap's end, or at the sample point the decoder was seeking when it met the gap, whichever is later.
- A source with fewer than two ticks per bit is refused when it is connected. Should a decoder start on one all the same, it reports no characters and only passes gaps, progress and the end on.

## Limits

- One line per decoder; 5 to 9 data bits.
- One sample per bit, at the sample point: no majority vote over several samples.
- Nothing is reported about the idle line itself, and the configuration is not detected from the signal.
