# I²S

The `i2s` decoder reads the audio samples of an I²S bus from logic samples — in the Philips I²S format or its left- and right-justified variants — and writes them to an `i2s.events` stream. It is written from the Philips Semiconductors _I²S bus specification_ (February 1986, revised June 1996).

## Configuration

| Option            | Values                                                                                                                            | Default  |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------- | -------- |
| `sck`, `ws`, `sd` | The channels of the serial clock, word select and serial data; all three are required and distinct                                | 0, 1, 2  |
| `format`          | `i2s`, `left-justified`, `right-justified`                                                                                        | `i2s`    |
| `bits`            | The word width, 1 to 32; or none, for as many bits as the half-period has clocks (at most 32). Right-justified words need a width | none     |
| `sample_edge`     | `rising` or `falling`: the SCK edge that reads WS and SD                                                                          | `rising` |
| `ws_inverted`     | Whether WS high is the left channel                                                                                               | `false`  |

## Events (`i2s.events` v1)

| Kind     | Fields                                          | Meaning                                  |
| -------- | ----------------------------------------------- | ---------------------------------------- |
| `sample` | `frame`, `channel`, `value`, `bits`, `complete` | One word: an audio sample of one channel |

| Field      | Meaning                                                                                                                                               |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `frame`    | The WS period, counted from 1: a left half-period and the right one after it make a frame                                                             |
| `channel`  | `left` or `right`, from the level of WS                                                                                                               |
| `value`    | The bits received, the first one most significant, as an unsigned integer of `bits` bits. Audio samples are two's complement: sign-extend from `bits` |
| `bits`     | The bits in `value`, 1 to 32                                                                                                                          |
| `complete` | `false` when the half-period ended before the configured width arrived; `value` then holds the bits that did                                          |

A sample covers the ticks of its first bit to its last. It is settled at the bit that makes it whole, or else at the sampling edge that found WS changed: for an incomplete word, a word whose width WS decides and every right-justified word.

## Props

| Prop     | Value                                                  |
| -------- | ------------------------------------------------------ |
| `format` | `"i2s"`, `"left-justified"` or `"right-justified"`     |
| `bits`   | The fixed word width; absent when WS decides the width |

The logic input requires `channels` of at least the highest of the three channels plus one.

## Decoding rules

- WS and SD are read on the sampling edge of SCK, at the first sample with the clock's new level, as a receiver latches them: a line that changes at that very tick is read with its new level.
- A half-period begins at the first sampling edge that reads WS at another level than the sampling edge before it. A glitch on WS between two sampling edges is no change.
- Where the word sits in its half-period:
  - `i2s`: the MSB is read on the sampling edge after the one that found WS changed (one clock of delay), so a word as long as its half-period ends with the bit read on the edge that finds the next change;
  - `left-justified`: the MSB is read on the sampling edge that found WS changed;
  - `right-justified`: the word is the last `bits` bits read before the next change.
- With a fixed width, the bits after the word are padding. Without one, the word is every bit of the half-period; a half-period of more than 32 clocks keeps its first 32 bits, as a 32-bit receiver does.
- Frames are counted from 1: a left half-period begins a frame, and so does the first half-period after the start of the samples or after a gap, whichever its channel. Frames lost in a gap are not counted.
- The half-period under way at the first sample, or at the first sample after a gap, is not reported: where its word began is unknown. The WS level at that sample is what the first sampling edge compares with.
- A gap or the end of the samples drops the word under way: leftover bits never make a word.

## Limits

- Two channels per WS period: TDM buses with more slots per frame are not decoded.
- Words of at most 32 bits.
