# NEC infrared

The `ir-nec` decoder reads the frames and repeat codes of an NEC infrared remote control from the output of an infrared receiver module and emits them as `ir_nec.events`. It follows the NEC transmission format as its users publish it — Vishay's application note _Data Formats for IR Remote Control_ (document 80071), Altium's _NEC Infrared Transmission Protocol_ and the SB-Projects description of the NEC protocol: a 38 kHz carrier, pulse distance coding, every duration a multiple of 562.5 µs.

The receiver demodulates the carrier: its output is at **mark** while it receives the carrier and at **space** otherwise. One decoder reads one receiver.

## Configuration

`NecConfig`, checked by `Nec::new`:

| Field       | Default      | Meaning                                                                                |
| ----------- | ------------ | -------------------------------------------------------------------------------------- |
| `channel`   | 0            | The channel of the receiver's output                                                   |
| `polarity`  | `active-low` | `active-low`: a mark is low, as receiver modules put it out; `active-high`: it is high |
| `tolerance` | 25           | How far a mark or space may be from its nominal length, in percent of it, 1 to 50      |

The input port `logic` requires of the source `channels >= channel + 1` and a rate at which the tolerance of the shortest duration, 562.5 µs, spans at least one tick: `rateHz >= ⌈100 / (tolerance × 562.5 µs)⌉`, 7112 at 25%. Below it sampling alone could push a duration out of its window, so the graph refuses the connection, for example `ir-nec#0.logic requires rateHz >= 7112 of logic#0.samples, which declares 7111`.

The output port `events` declares `protocol = "nec"`.

## The stream: `ir_nec.events` v1

The stream is unframed: a frame, a repeat code or a frame that went wrong is one event, decided when its last mark or space is certain. There is no `begin` or `end`, and nothing is ever aborted: a frame cut off by a gap or by the end of the samples is not reported.

| Kind     | Fields                                                             | Meaning                                                                                  |
| -------- | ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| `frame`  | `address: u16`, `command: u8`, `extended`, `inverseOk`, `raw: u32` | A whole frame: the leader, 32 data bits and the final mark                               |
| `repeat` | —                                                                  | A repeat code: the leader mark, the short leader space and the final mark                |
| `error`  | `reason`, `bits: u8`, `raw: u32`                                   | A frame or repeat code whose leader mark was seen but which could not be read to its end |

The four bytes of a frame are sent in order, each least significant bit first: the address, its inverse, the command and its inverse.

| Field       | Meaning                                                                                                                         |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `address`   | The first byte; with `extended`, the first two bytes, the first one the low byte                                                |
| `command`   | The third byte                                                                                                                  |
| `extended`  | The second byte is not the inverse of the first, so the two make a 16-bit address (extended NEC)                                |
| `inverseOk` | The fourth byte is the inverse of the command                                                                                   |
| `raw`       | The 32 data bits as received, the first as bit 0: the four bytes from the low byte up, the fourth one whatever `inverseOk` says |

An `error` names in `reason` where the frame went wrong, and carries the data bits read before it: `bits` of them (0 to 32) in `raw`, the first as bit 0.

| Reason    | What was outside its window                                                                           |
| --------- | ----------------------------------------------------------------------------------------------------- |
| `leader`  | The leader mark, longer than its window; or the leader space, neither a frame's nor a repeat code's   |
| `bit`     | A data bit's mark, too short or too long; or its space, shorter than a 0's or between a 0's and a 1's |
| `count`   | A space longer than a 1's before the 32nd bit: the frame stopped short                                |
| `trailer` | The final mark of a frame or a repeat code, too short or too long                                     |

```json
{"kind":"frame","address":0,"command":69,"extended":false,"inverseOk":true,"raw":3125149440}
{"kind":"repeat"}
{"kind":"error","reason":"count","bits":16,"raw":65280}
```

| Event              | Span                                                                                                  | Settled                   |
| ------------------ | ----------------------------------------------------------------------------------------------------- | ------------------------- |
| `frame`, `repeat`  | the first tick of the leader mark … the end of the final mark                                         | the end of the final mark |
| `error` (an edge)  | the first tick of the leader mark … the edge that ended an element too soon or outside its windows    | that edge                 |
| `error` (too long) | the first tick of the leader mark … the last tick of the element that outlasted its window, inclusive | that last tick            |

### Progress

- Progress never passes the first tick of a leader mark whose frame is still being read: after progress at tick _t_, no event begins before _t_.
- Before a gap or the end of the stream is passed on, progress is announced up to the gap's start or the end, or up to the start of the frame they cut off.

## Decoding rules

- The idle line is at space. Marks and spaces are measured in ticks, from the edge that begins one to the edge that ends it.
- A nominal length _d_ becomes `d × rate` ticks at the source's rate, and its window runs from ⌈`d × rate × (100 − tolerance) / 100`⌉ to ⌊`d × rate × (100 + tolerance) / 100`⌋ ticks, both included, computed exactly in integer arithmetic. Where the windows of two lengths the decoder may meet at one point overlap (above 33%), a duration in both belongs to the length it is nearer to, the shorter one when it is as near to both.

| Element                                    | Nominal   | Window at 25%           |
| ------------------------------------------ | --------- | ----------------------- |
| Leader mark                                | 9 ms      | 6.75 to 11.25 ms        |
| Leader space of a frame                    | 4.5 ms    | 3.375 to 5.625 ms       |
| Leader space of a repeat code              | 2.25 ms   | 1.6875 to 2.8125 ms     |
| Data bit mark, a 0's space, the final mark | 562.5 µs  | 421.875 to 703.125 µs   |
| A 1's space                                | 1687.5 µs | 1265.625 to 2109.375 µs |

- A frame begins with the leader mark. A shorter mark on the idle line is no leader and is passed over, so glitches and the shorter marks of other formats leave no trace; a longer one is a `leader` error as soon as it has outlasted its window, without waiting for its end.
- A 4.5 ms space after the leader mark begins a frame: 32 data bits, each a mark and a space whose length is the bit, then the final mark. A 2.25 ms space begins a repeat code: the final mark follows at once. A repeat code is reported on its own; neither its 108 ms rhythm nor the frame before it is checked.
- A frame or a repeat code is decided at the end of its final mark, as soon as it is seen. Nothing after the final mark is looked at: a frame of more than 32 bits reads as an NEC frame of its first 32.
- At the first sample, after a gap and after every frame, repeat code or error, the decoder waits for the line to **idle** — to stay at space longer than the longest space a frame has, the 4.5 ms leader space with its tolerance (5.625 ms at 25%) — before it takes a mark for a leader. A frame already under way when the samples begin or resume cannot be aligned and is passed over, and so is whatever follows a final mark or an error until the line idles.
- A frame or repeat code cut off by a gap or by the end of the samples is not reported; the gap or the end is passed on.
- Should the decoder start on a source slower than its input requires, it reports nothing and only passes gaps, progress and the end on.

## Limits

- One receiver output, demodulated: the 38 kHz bursts themselves are not read.
- Repeat codes are not tied to the frame before them, and their timing is not checked.
- Frames longer than 32 bits, as some other formats send, are not told apart from NEC frames.
