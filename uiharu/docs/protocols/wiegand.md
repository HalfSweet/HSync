# Wiegand

The frames an access-control reader sends on its DATA0 and DATA1 lines, decoded from logic samples: each bit a short low pulse on one of the lines, each frame ended by a silence. Node type `wiegand`; module `crates/uiharu-protocols/src/wiegand/`. Implemented from the Security Industry Association's SIA AC-01-1996.10, _Access Control Standard Protocol for the 26-Bit Wiegand Reader Interface_, and reader datasheets.

## Configuration

| Field          | Default | Meaning                                                                                   |
| -------------- | ------- | ----------------------------------------------------------------------------------------- |
| `d0`           | 0       | The DATA0 channel, whose pulses are 0 bits                                                |
| `d1`           | 1       | The DATA1 channel, whose pulses are 1 bits                                                |
| `frame_gap_us` | 10000   | How long the lines stay high after a frame's last pulse before the frame ends, above zero |
| `pulse_min_us` | 20      | The shortest pulse, above zero                                                            |
| `pulse_max_us` | 100     | The longest pulse, at least `pulse_min_us`                                                |

The standard's pulses last 20 to 100 µs (nominally 50 µs) and follow each other 200 µs to 20 ms apart (nominally 1 to 2 ms); the pulse window can be widened for readers outside it.

The input port `logic` requires of the source `channels >= max(d0, d1) + 1` and two samples in the shortest pulse: `rateHz >= ⌈2 · 10⁶ / pulse_min_us⌉`, 100 kHz by default. The output port `events` declares `frameGapUs` (the configured frame gap) and `maxBits` (64).

## The stream: `wiegand.events` v1

The stream is unframed: a frame is one event, decided once the silence after its last pulse has passed. A frame that could not be read is an `error` instead.

| Kind    | Fields                                                                     | Meaning                                                                                |
| ------- | -------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `frame` | `frame`, `bits: u16`, `data: [u8]`, `parityOk: bool \| null`               | A frame of 1 to 64 bits                                                                |
| `error` | `reason`: `both-lines`, `short-pulse`, `long-pulse`, `too-long`, `aborted` | A frame that could not be read; the pulses after it up to the next silence are skipped |

- `frame` numbers the reported frames from 1.
- `data` holds the bits packed most significant bit first: the first bit on the lines is the top bit of the first byte, and the last byte is padded with zeros; ⌈`bits` / 8⌉ bytes.
- `parityOk` is set for 26-bit frames only: whether the first bit makes the first 13 bits even and the last bit makes the last 13 odd, the parities of the standard 26-bit format (even parity, 8-bit facility code, 16-bit card number, odd parity). Other lengths — 34 and 37 bits are common — are reported as they are, with `null`: their formats are the reader's.
- `both-lines`: both lines were low at once; `short-pulse` and `long-pulse`: a pulse outside the width window; `too-long`: a 65th bit; `aborted`: a gap or the end of the samples came before the frame's silence had passed.

```json
{"kind":"frame","frame":1,"bits":26,"data":[61,152,28,192],"parityOk":true}
{"kind":"error","reason":"short-pulse"}
```

| Event                               | Span                                                    | Settled                                     |
| ----------------------------------- | ------------------------------------------------------- | ------------------------------------------- |
| `frame`                             | the first pulse's fall through the last pulse's release | `last release + frame_gap`                  |
| `error` (`both-lines`)              | the frame from its first pulse through the tick         | the first tick both lines were low          |
| `error` (`short-pulse`, `too-long`) | the frame from its first pulse through the release      | the offending pulse's release               |
| `error` (`long-pulse`)              | the frame from its first pulse through the decision     | `fall + pulse_max`, while the pulse goes on |
| `error` (`aborted`)                 | the frame from its first pulse through its last edge    | the gap's start or the end of the samples   |

Progress never passes the first pulse of a frame still being read: after progress at tick _t_, no event begins before _t_.

## Decoding rules

- A pulse runs from the tick a line falls to the tick it rises. A bound of "at least _t_ µs" is ⌈_t_ · rate / 10⁶⌉ ticks, one of "at most _t_ µs" ⌊_t_ · rate / 10⁶⌋ ticks, in integer arithmetic; the frame gap is ⌊`frame_gap_us` · rate / 10⁶⌋ ticks.
- A frame is a run of pulses. It ends once the lines have stayed high for more than the frame gap after its last pulse; a pulse falling later begins the next frame. Whether the next frame's first pulse, the samples reaching the end of the silence or the source's progress tell the decoder, the frame is settled at `last release + frame_gap`, so the result does not depend on how the samples were cut.
- Both lines low at once, a pulse shorter than `pulse_min_us`, a pulse longer than `pulse_max_us` and a 65th bit end the frame with an `error`. A long pulse is decided once it has outlasted `pulse_max_us`, before it ends. The pulses after the error up to the next silence belong to the broken frame and are skipped.
- The decoder knows nothing of what came before the first sample, or before the first sample after a gap: a frame may be under way there. It begins reading frames once the lines have been seen high for more than the frame gap, and skips the pulses before that — so a capture should begin with the lines idle for longer than the frame gap.
- A gap or the end of the samples before a frame's silence has passed is an `aborted` error; the gap or the end is passed on after it. A frame whose last pulse comes less than the frame gap before the end of a capture is reported this way.

## Limits

- The spacing of the pulses is not checked: a frame is any run of pulses closer together than the frame gap.
- Only the 26-bit format's parities are checked; what the bits mean — facility codes, card numbers, other formats — is the reader's business.
- Frames carry at most 64 bits.
