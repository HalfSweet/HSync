<!-- SPDX-License-Identifier: Apache-2.0 -->

# MIDI semantic fields

`midi-semantics` consumes the exact `midi.events@1` contract above the existing
MIDI 1.0 UART transport. `percussionChannel` is a zero-based channel index
(default 9, MIDI channel 10). The original implementation uses the MIDI
Association's [message summary](https://midi.org/summary-of-midi-1-0-messages),
[controller table](https://midi.org/midi-1-0-control-change-messages),
[General MIDI percussion mapping](https://midi.org/general-midi) and
[manufacturer identifiers](https://midi.org/SysExIDtable). No reference decoder
source is copied or adapted.

`midi-semantics.events@1` provides these incremental observations:

| Event           | Fields and meaning                                                                                                                                                                                                                                        |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `note`          | Original message/channel/note, pitch class, scientific octave as magnitude plus sign, on/off/pressure action, velocity or pressure, and an optional standard GM percussion name on the configured channel. Note On with velocity zero is reported as off. |
| `controller`    | Original controller number/value, optional assigned controller name, and whether the number selects the LSB half of a controller. Undefined and unassigned numbers keep a null name.                                                                      |
| `pitch-bend`    | Exact 14-bit value, offset magnitude from the center 8192, and sign. No bend range or semitone tuning is assumed.                                                                                                                                         |
| `song-position` | The exact 14-bit MIDI-beat count and its equivalent number of MIDI clocks (six clocks per beat).                                                                                                                                                          |
| `quarter-frame` | Time-code piece number and four-bit value. No complete time code or absolute time is inferred from an incomplete set of pieces.                                                                                                                           |
| `manufacturer`  | One- or three-byte SysEx identifier, extended-ID flag, and a recognized name where included in the bounded built-in table. Other identifiers retain their numeric value and a null name.                                                                  |

The manufacturer table names Sequential, Moog, Kawai, Roland, Korg, Yamaha,
Casio, Akai, educational/noncommercial, the two universal namespaces and
Novation's extended identifier. It is intentionally not a claim that every
assigned manufacturer or vendor SysEx command is understood. One-byte and
extended namespaces remain distinct, even when their assembled numbers match.
A three-byte identifier can cross native SysEx batches; only a counter and its
three-byte accumulator are retained. Gaps and SysEx ends clear that state.

Derived events reuse the complete lower message's provenance; manufacturer
identification uses the lower batch that completes its identifier. Running
status and interleaved real-time messages are handled by the lower decoder.
No note duration, instrument state, tuning, SysEx payload application, drum
layout outside the configured channel, or full MTC clock reconstruction is
invented. Uninterpreted messages remain available on the lower stream.
