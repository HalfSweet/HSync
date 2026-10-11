# RC-5 infrared

The `ir-rc5` decoder reads the frames of a Philips RC-5 infrared remote control, and of its extension RC-5X, from the output of an infrared receiver module and emits them as `rc5.events`. It follows Philips Semiconductors' data sheet of the SAA3010 RC-5 transmitter and the SB-Projects description of RC-5 and its extension: a 36 kHz carrier and bi-phase coding, each bit 64 carrier periods (1.778 ms) long, in two halves of 888.9 µs at opposite levels.

The receiver demodulates the carrier: its output is at **mark** while it receives the carrier and at **space** otherwise. One decoder reads one receiver.

## Configuration

`Rc5Config`, checked by `Rc5::new`:

| Field       | Default      | Meaning                                                                                   |
| ----------- | ------------ | ----------------------------------------------------------------------------------------- |
| `channel`   | 0            | The channel of the receiver's output                                                      |
| `polarity`  | `active-low` | `active-low`: a mark is low, as receiver modules put it out; `active-high`: it is high    |
| `tolerance` | 25           | How far an interval between two edges may be from its nominal length, in percent, 1 to 50 |

The input port `logic` requires of the source `channels >= channel + 1` and a rate at which the tolerance of a half bit spans at least one tick: `rateHz >= ⌈100 / (tolerance × 888.9 µs)⌉`, 4500 at 25%. Below it sampling alone could push an interval out of its window, so the graph refuses the connection, for example `ir-rc5#0.logic requires rateHz >= 4500 of logic#0.samples, which declares 4499`.

The output port `events` declares `protocol = "rc5"`.

## The stream: `rc5.events` v1

The stream is unframed: a frame, or a frame that went wrong, is one event, decided when its last half bit is certain. There is no `begin` or `end`, and nothing is ever aborted: a frame cut off by a gap or by the end of the samples is not reported.

| Kind    | Fields                                                   | Meaning                                                                  |
| ------- | -------------------------------------------------------- | ------------------------------------------------------------------------ |
| `frame` | `toggle: bool`, `address: u8`, `command: u8`, `extended` | A whole frame of 14 bits                                                 |
| `error` | `reason`, `bits: u8`, `raw: u16`                         | A frame whose first edge was seen but which could not be read to its end |

A frame's 14 bits are sent most significant bit first: two start bits, the toggle bit, five address bits and six command bits. The first start bit is always 1; the second is 1 in RC-5 and, in RC-5X, the inverse of bit 6 of the command.

| Field      | Meaning                                                                                               |
| ---------- | ----------------------------------------------------------------------------------------------------- |
| `toggle`   | The toggle bit, which the remote control inverts at every new key press and keeps while a key is held |
| `address`  | The address (system), 0 to 31                                                                         |
| `command`  | The command, 0 to 127: the six command bits, and as bit 6 the inverse of the second start bit         |
| `extended` | The second start bit is 0, so the command is one of 64 to 127, which only RC-5X sends                 |

An `error` names in `reason` the field of the bit being read when the frame went wrong — `start`, `toggle`, `address` or `command` (the last bit's second half included) — and carries the bits read before it: `bits` of them (1 to 14; a bit is read at its middle), in `raw` in the order received, the first start bit as bit `bits − 1` and the last bit read as bit 0.

```json
{"kind":"frame","toggle":false,"address":0,"command":12,"extended":false}
{"kind":"error","reason":"address","bits":6,"raw":49}
```

| Event                     | Span                                                                                                        | Settled                                                      |
| ------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `frame` ending in a mark  | ⌊half bit⌋ ticks before its first edge … the end of its final mark                                          | the end of the final mark                                    |
| `frame` ending in a space | ⌊half bit⌋ ticks before its first edge … ⌊half bit⌋ ticks after its last middle                             | the last tick of the shortest half bit after its last middle |
| `error` (an edge)         | ⌊half bit⌋ ticks before its first edge … the edge that decided it                                           | that edge                                                    |
| `error` (too long)        | ⌊half bit⌋ ticks before its first edge … the last tick of the interval that outlasted its window, inclusive | that last tick                                               |

The first half of the first start bit is a space, which cannot be told from the idle line: a frame is therefore placed from half a bit before its first edge, computed from the nominal half bit.

### Progress

- Progress never passes the start of a frame still being read: after progress at tick _t_, no event begins before _t_. On the idle line it is held back by half a bit from the samples seen, since the next frame begins half a bit before its first edge.
- Before a gap or the end of the stream is passed on, progress is announced up to the gap's start or the end, or up to the start of the frame they cut off.

## Decoding rules

- The idle line is at space. A 1 is a space then a mark, a 0 a mark then a space: every bit has an edge in its middle, whose direction is the bit, and two bits have an edge between them only when they are equal.
- The bits are read from the edges, never from a clock of the decoder's own. The first edge of a frame is the middle of its first start bit. From each middle the next edge comes one half bit later — an edge between two equal bits, the next middle following one half bit after it — or two half bits later, the middle of a bit unequal to the last.
- Every interval between two edges is judged on its own against a window around its nominal length _d_: from ⌈`d × rate × (100 − tolerance) / 100`⌉ to ⌊`d × rate × (100 + tolerance) / 100`⌋ ticks, both included, computed exactly in integer arithmetic. At 25% one half bit is read from 666.7 to 1111.1 µs and two from 1333.3 to 2222.2 µs. Where the two windows overlap (above 33%), an interval in both belongs to the length it is nearer to, one half bit when it is as near to both.
- Every middle re-anchors the timing, so a remote control whose clock is off by less than the tolerance is followed bit after bit without the error adding up.
- On the idle line an edge to mark is the middle of a first start bit. When the mark after it is shorter than a half bit's window, it was a glitch: it is passed over and the next edge to mark is looked at.
- A frame is decided as soon as its last half bit is certain: a last bit of 1 ends with a mark, decided at the mark's end; a last bit of 0 ends with a space, decided once the space has lasted the shortest half bit its window allows, without waiting for an edge.
- An interval outside its windows, or an edge that does not come within the longest of them, ends the frame as an `error`.
- At the first sample, after a gap and after every frame or error, the decoder waits for the line to **idle** — to stay at space longer than the longest space a frame has, two half bits with their tolerance (2.222 ms at 25%) — before it takes an edge to mark for the start of a frame. A frame already under way when the samples begin or resume cannot be aligned and is passed over, and so is the rest of a frame after an error.
- A frame cut off by a gap or by the end of the samples is not reported; the gap or the end is passed on.
- Should the decoder start on a source slower than its input requires, it reports nothing and only passes gaps, progress and the end on.

## Limits

- One receiver output, demodulated: the 36 kHz bursts themselves are not read.
- The repetition of a frame every 64 bit times (113.8 ms) while a key is held is not checked; a reader tells a held key from a new press by `toggle`.
