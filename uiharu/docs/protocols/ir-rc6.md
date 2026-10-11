# RC-6 infrared

The `ir-rc6` decoder reads the mode 0 frames of a Philips RC-6 infrared remote control from the output of an infrared receiver module and emits them as `rc6.events`. It follows the RC-6 format as Philips published it and as the SB-Projects description of RC-6 sets it out: a 36 kHz carrier and bi-phase coding, with a unit `t` of 16 carrier periods, 444.4 µs. Mode 0 is the mode of consumer electronics.

The receiver demodulates the carrier: its output is at **mark** while it receives the carrier and at **space** otherwise. One decoder reads one receiver.

## Configuration

`Rc6Config`, checked by `Rc6::new`:

| Field       | Default      | Meaning                                                                                   |
| ----------- | ------------ | ----------------------------------------------------------------------------------------- |
| `channel`   | 0            | The channel of the receiver's output                                                      |
| `polarity`  | `active-low` | `active-low`: a mark is low, as receiver modules put it out; `active-high`: it is high    |
| `tolerance` | 25           | How far an interval between two edges may be from its nominal length, in percent, 1 to 50 |

The input port `logic` requires of the source `channels >= channel + 1` and a rate at which the tolerance of the unit spans at least one tick: `rateHz >= ⌈100 / (tolerance × 444.4 µs)⌉`, 9000 at 25%. Below it sampling alone could push an interval out of its window, so the graph refuses the connection, for example `ir-rc6#0.logic requires rateHz >= 9000 of logic#0.samples, which declares 8999`.

The output port `events` declares `protocol = "rc6"`.

## The stream: `rc6.events` v1

The stream is unframed: a frame, or a frame that went wrong, is one event, decided when its last half bit is certain. There is no `begin` or `end`, and nothing is ever aborted: a frame cut off by a gap or by the end of the samples is not reported.

| Kind    | Fields                                                   | Meaning                                                                   |
| ------- | -------------------------------------------------------- | ------------------------------------------------------------------------- |
| `frame` | `mode: u8`, `toggle: bool`, `address: u8`, `command: u8` | A whole frame                                                             |
| `error` | `reason`, `bits: u8`, `raw: u32`                         | A frame whose leader mark was seen but which could not be read to its end |

A frame is a leader — a mark of 6 `t` and a space of 2 `t` — and 21 bits, most significant bit first: the start bit, always 1; three mode bits; the trailer bit; eight control bits; eight information bits.

| Field     | Meaning                                                                                       |
| --------- | --------------------------------------------------------------------------------------------- |
| `mode`    | The mode bits: always 0, the only mode the decoder reads                                      |
| `toggle`  | The trailer bit, which the remote control inverts at every new key press and keeps while held |
| `address` | The control field: the address of the device                                                  |
| `command` | The information field: the command                                                            |

An `error` names in `reason` where the frame went wrong, and carries the bits read before it: `bits` of them (0 to 21; a bit is read at its middle), in `raw` in the order received, the start bit as bit `bits − 1` and the last bit read as bit 0.

| Reason             | Where                                                                                          |
| ------------------ | ---------------------------------------------------------------------------------------------- |
| `leader`           | The leader mark, longer than its window, or the leader space outside its own                   |
| `start`            | The start bit                                                                                  |
| `mode`             | A mode bit                                                                                     |
| `toggle`           | The trailer bit                                                                                |
| `address`          | A control bit                                                                                  |
| `command`          | An information bit, the last one's second half included                                        |
| `unsupported-mode` | The mode bits name a mode other than 0, which the decoder does not read: the mode is `raw & 7` |

```json
{"kind":"frame","mode":0,"toggle":false,"address":0,"command":12}
{"kind":"error","reason":"unsupported-mode","bits":4,"raw":14}
```

| Event                     | Span                                                                                                   | Settled                                                 |
| ------------------------- | ------------------------------------------------------------------------------------------------------ | ------------------------------------------------------- |
| `frame` ending in a mark  | the first tick of the leader mark … the end of its final mark                                          | the end of the final mark                               |
| `frame` ending in a space | the first tick of the leader mark … ⌊`t`⌋ ticks after its last middle                                  | the last tick of the shortest `t` after its last middle |
| `error` (an edge)         | the first tick of the leader mark … the edge that decided it                                           | that edge                                               |
| `error` (too long)        | the first tick of the leader mark … the last tick of the interval that outlasted its window, inclusive | that last tick                                          |

An `unsupported-mode` error is decided at the middle of the last mode bit.

### Progress

- Progress never passes the first tick of a leader mark whose frame is still being read: after progress at tick _t_, no event begins before _t_.
- Before a gap or the end of the stream is passed on, progress is announced up to the gap's start or the end, or up to the start of the frame they cut off.

## Decoding rules

- The idle line is at space. Opposite to RC-5, a 1 is a mark then a space and a 0 a space then a mark: every bit has an edge in its middle, whose direction is the bit, and two bits have an edge between them only when they are equal. A bit is two halves of `t`; the trailer bit's halves last 2 `t`.
- On the idle line a mark of 6 `t` begins a frame. A shorter mark is no leader and is passed over, so glitches and the shorter marks of other formats leave no trace; a longer one is a `leader` error as soon as it has outlasted its window.
- The bits are read from the edges, never from a clock of the decoder's own. The leader space ends at the start of the start bit, whose mark of `t` ends at its middle. From each middle the next edge comes after the rest of the bit — an edge between two equal bits, the next middle following after the first half of the next bit — or after the rest of the bit and the first half of the next, the middle of a bit unequal to the last. Within the frame that is one or two `t`; before the trailer bit's middle one or three, after it two or three. Every middle re-anchors the timing.
- Every interval is judged on its own against a window around its nominal length _d_: from ⌈`d × rate × (100 − tolerance) / 100`⌉ to ⌊`d × rate × (100 + tolerance) / 100`⌋ ticks, both included, computed exactly in integer arithmetic.

| Length        | Nominal   | Window at 25%      |
| ------------- | --------- | ------------------ |
| `t`           | 444.4 µs  | 333.3 to 555.6 µs  |
| 2 `t`         | 888.9 µs  | 666.7 to 1111.1 µs |
| 3 `t`         | 1333.3 µs | 1000 to 1666.7 µs  |
| Leader, 6 `t` | 2666.7 µs | 2000 to 3333.3 µs  |

- Where the windows of the two lengths the decoder may meet at one point overlap — two and three `t` after the trailer bit's middle from a tolerance of 20%, one and two `t` above 33% — an interval in both belongs to the length it is nearer to, the shorter one when it is as near to both. At 25% a remote control whose clock is uniformly fast by a sixth or more therefore cannot be read past the trailer bit: three units have come as near to two.
- Only mode 0 is read: once the three mode bits name another mode, the frame ends as an `unsupported-mode` error.
- A frame is decided as soon as its last half bit is certain: a last bit of 0 ends with a mark, decided at the mark's end; a last bit of 1 ends with a space, decided once the space has lasted the shortest `t` its window allows. The signal-free time of 6 `t` after a frame is not waited for.
- An interval outside its windows, or an edge that does not come within the longest of them, ends the frame as an `error`.
- At the first sample, after a gap and after every frame or error, the decoder waits for the line to **idle** — to stay at space longer than the longest space a frame has, 3 `t` with its tolerance (1.667 ms at 25%) — before it takes a mark for a leader. A frame already under way when the samples begin or resume cannot be aligned and is passed over, and so is the rest of a frame after an error.
- A frame cut off by a gap or by the end of the samples is not reported; the gap or the end is passed on.
- Should the decoder start on a source slower than its input requires, it reports nothing and only passes gaps, progress and the end on.

## Limits

- One receiver output, demodulated: the 36 kHz bursts themselves are not read.
- Mode 0 only. Mode 6 (RC-6A, which Windows Media Center remote controls send) and the other modes are reported as `unsupported-mode`; their fields after the header are not read.
- The signal-free time after a frame is not checked.
