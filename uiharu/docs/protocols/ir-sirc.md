# SIRC infrared

The `ir-sirc` decoder reads the frames of a Sony SIRC infrared remote control from the output of an infrared receiver module and emits them as `ir_sirc.events`. It follows the SIRC format as public descriptions give it, the SB-Projects description of the Sony SIRC protocol among them: a 40 kHz carrier and pulse width coding, with a unit `T` of 600 µs, 24 carrier periods.

The receiver demodulates the carrier: its output is at **mark** while it receives the carrier and at **space** otherwise. One decoder reads one receiver.

## Configuration

`SircConfig`, checked by `Sirc::new`:

| Field       | Default      | Meaning                                                                                |
| ----------- | ------------ | -------------------------------------------------------------------------------------- |
| `channel`   | 0            | The channel of the receiver's output                                                   |
| `polarity`  | `active-low` | `active-low`: a mark is low, as receiver modules put it out; `active-high`: it is high |
| `tolerance` | 25           | How far a mark or space may be from its nominal length, in percent of it, 1 to 50      |

The input port `logic` requires of the source `channels >= channel + 1` and a rate at which the tolerance of the unit spans at least one tick: `rateHz >= ⌈100 / (tolerance × 600 µs)⌉`, 6667 at 25%. Below it sampling alone could push a duration out of its window, so the graph refuses the connection, for example `ir-sirc#0.logic requires rateHz >= 6667 of logic#0.samples, which declares 6666`.

The output port `events` declares `protocol = "sirc"`.

## The stream: `ir_sirc.events` v1

The stream is unframed: a frame, or a frame that went wrong, is one event, decided once the silence after its last bit, or what went wrong, is certain. There is no `begin` or `end`, and nothing is ever aborted: a frame cut off by a gap or by the end of the samples before its silence was seen is not reported.

| Kind    | Fields                                                           | Meaning                                                                  |
| ------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `frame` | `bits: u8`, `command: u8`, `address: u8`, `extended: u8 \| null` | A whole frame of 12, 15 or 20 bits                                       |
| `error` | `reason`, `bits: u8`, `raw: u32`                                 | A frame whose start mark was seen but which could not be read to its end |

A frame is a start mark of 4 `T` and a space of `T`, then its bits, least significant bit first, each a mark of `T` for a 0 or 2 `T` for a 1 followed by a space of `T`.

| Field      | Meaning                                                                                       |
| ---------- | --------------------------------------------------------------------------------------------- |
| `bits`     | The length of the frame: 12, 15 or 20 bits                                                    |
| `command`  | The first seven bits                                                                          |
| `address`  | The next five bits in 12 and 20-bit frames, eight in 15-bit frames                            |
| `extended` | The last eight bits of a 20-bit frame; `null` in the shorter frames, which have no such field |

An `error` names in `reason` where the frame went wrong, and carries the bits read before it: `bits` of them (0 to 21; a bit is read at the end of its mark) in `raw`, the first as bit 0.

| Reason  | What                                                                                        |
| ------- | ------------------------------------------------------------------------------------------- |
| `start` | The start mark, longer than its window, or the space after it outside its window            |
| `bit`   | A bit's mark, neither a 0's nor a 1's, or the space after it, shorter than its window       |
| `count` | The frame ended after other than 12, 15 or 20 bits, or a 21st bit began (`bits` is then 20) |

```json
{"kind":"frame","bits":12,"command":18,"address":1,"extended":null}
{"kind":"frame","bits":20,"command":127,"address":26,"extended":226}
{"kind":"error","reason":"count","bits":13,"raw":4101}
```

| Event              | Span                                                                                                 | Settled                                           |
| ------------------ | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| `frame`            | the first tick of the start mark … ⌊`T`⌋ ticks after the last mark ends, the end of the last bit     | the last tick of the longest space a bit may have |
| `error` (an edge)  | the first tick of the start mark … the edge that decided it                                          | that edge                                         |
| `error` (too long) | the first tick of the start mark … the last tick of the element that outlasted its window, inclusive | that last tick                                    |

A frame is settled after the end of its span: the silence that tells its length outlasts the last bit's own space.

### Progress

- Progress never passes the first tick of a start mark whose frame is still being read, its closing silence included: after progress at tick _t_, no event begins before _t_.
- Before a gap or the end of the stream is passed on, progress is announced up to the gap's start or the end, or up to the start of the frame they cut off.

## Decoding rules

- The idle line is at space. Marks and spaces are measured in ticks, from the edge that begins one to the edge that ends it, and compared with a window around their nominal length _d_: from ⌈`d × rate × (100 − tolerance) / 100`⌉ to ⌊`d × rate × (100 + tolerance) / 100`⌋ ticks, both included, computed exactly in integer arithmetic. Where the windows of a 0's and a 1's mark overlap (above 33%), a mark in both belongs to the length it is nearer to, a 0 when it is as near to both.

| Element                 | Nominal | Window at 25% |
| ----------------------- | ------- | ------------- |
| Start mark, 4 `T`       | 2.4 ms  | 1.8 to 3 ms   |
| A 1's mark, 2 `T`       | 1.2 ms  | 0.9 to 1.5 ms |
| A 0's mark, every space | 600 µs  | 450 to 750 µs |

- On the idle line a mark of 4 `T` begins a frame. A shorter mark is no start and is passed over, so glitches and the shorter marks of other formats leave no trace; a longer one is a `start` error as soon as it has outlasted its window.
- A frame says nothing of its length: it ends when the line stays at space longer than a bit's space allows, `T` with its tolerance (750 µs at 25%). That silence is known once the samples up to its last tick have been seen without an edge — in the block that holds them, in one that arrives after the decoder announced progress at the end of a used-up block, or right before the next frame begins — and the frame is decided at that computed tick, whichever told the decoder, so that the result does not depend on how the samples were cut.
- A frame of 12, 15 or 20 bits is reported. Any other count is a `count` error, decided with the silence; a 21st bit is one as soon as its mark begins.
- A mark or space outside its window ends the frame as an `error`.
- At the first sample, after a gap and after every frame or error, the decoder waits for the line to **idle** — to stay at space longer than the longest space a frame has, the same `T` with its tolerance that ends a frame — before it takes a mark for a start. After a frame that condition is already met; a frame already under way when the samples begin or resume cannot be aligned and is passed over, and so is the rest of a frame after an error.
- A frame whose silence was not seen before a gap or the end of the samples is not reported, since more bits could have followed; the gap or the end is passed on.
- Should the decoder start on a source slower than its input requires, it reports nothing and only passes gaps, progress and the end on.

## Limits

- One receiver output, demodulated: the 40 kHz bursts themselves are not read.
- The repetition of frames every 45 ms while a key is held is not checked, and frames are not tied to one another.
