# PS/2

The frames a PS/2 keyboard or mouse and its host exchange over the port's clock and data lines, in both directions, decoded from logic samples. Node type `ps2`; module `crates/uiharu-protocols/src/ps2/`. Implemented from the IBM PS/2 keyboard and auxiliary device interface, as IBM's Personal System/2 hardware technical references describe it and the widely reproduced application note _The PS/2 Mouse/Keyboard Protocol_ (Adam Chapweske) restates it.

Both lines are open collector and idle high, and the device generates the clock: high and low for 30 to 50 µs each. A frame is a start bit (0), eight data bits least significant first, an odd parity bit and a stop bit (1).

- **Device to host**: the device clocks eleven pulses; the host reads each bit while the clock is low.
- **Host to device**: the host inhibits the bus by holding the clock low for at least 100 µs, pulls the data line low — its request to send, which is the start bit — and releases the clock. The device clocks the data bits, parity and stop bit in, reading each on a rising edge, then acknowledges by pulling the data line low for one last clock pulse: the host's own inhibit pulse and the device's eleven, twelve clock pulses in all.

## Configuration

| Field  | Default | Meaning               |
| ------ | ------- | --------------------- |
| `clk`  | 0       | The clock channel     |
| `data` | 1       | The data line channel |

The input port `logic` requires of the source `channels >= max(clk, data) + 1` and `rateHz >= 100000`: three samples in the shortest half period of the clock. The output port `events` declares `wordBits = 8` and `bitOrder = "lsb-first"`.

## The stream: `ps2.events` v1

The stream is unframed: a frame is one event, decided at its last clock edge. A frame that could not be read is an `error` instead.

| Kind    | Fields                                                                          | Meaning                                                      |
| ------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| `frame` | `direction`, `value: u8`, `parityOk: bool`, `stopOk: bool`, `ack: bool \| null` | A frame                                                      |
| `error` | `reason`: `start` or `aborted`                                                  | A frame with a start bit of 1, or one cut off before its end |

- `direction` is `device-to-host` or `host-to-device`.
- `value` holds the data bits, the first on the line as bit 0.
- `parityOk` says whether the number of 1s among the data and parity bits is odd; `stopOk` whether the stop bit was 1. A frame with either wrong is still reported.
- `ack` is set for frames the host sent: whether the device pulled the data line low on its last clock pulse. It is `null` for the device's frames, which have no acknowledge.
- `start`: the device clocked a frame whose start bit was 1. `aborted`: the frame stopped before its last clock — the host took the clock, the clock paused, the device did not answer a request, or a gap or the end of the samples cut it off.

```json
{"kind":"frame","direction":"device-to-host","value":28,"parityOk":true,"stopOk":true,"ack":null}
{"kind":"frame","direction":"host-to-device","value":237,"parityOk":true,"stopOk":true,"ack":true}
{"kind":"error","reason":"aborted"}
```

| Event                    | Span                                                    | Settled                                                             |
| ------------------------ | ------------------------------------------------------- | ------------------------------------------------------------------- |
| `frame` (device to host) | the first clock fall through the eleventh               | the eleventh fall, where the stop bit is read                       |
| `frame` (host to device) | the inhibit's fall through the acknowledge pulse's fall | that fall; without an acknowledge pulse, the stop bit's rise + 2 ms |
| `error` (`start`)        | the first clock pulse                                   | its rise                                                            |
| `error` (`aborted`)      | the frame from its start through its last clock edge    | where the abort was decided (below)                                 |

Progress never passes the start of a frame still being read, nor the fall of a clock low that may turn out to be the host's inhibit: after progress at tick _t_, no event begins before _t_.

## Decoding rules

- Every level is read at a clock edge, from the samples of that tick. Time limits are turned into ticks: a bound of "at least _t_ µs" is ⌈_t_ · rate / 10⁶⌉ ticks, one of "at most _t_ µs" ⌊_t_ · rate / 10⁶⌋ ticks.
- A clock low of at least 100 µs is the host inhibiting the bus, wherever it comes. Released with the data line low, it is a request to send and begins a host-to-device frame spanning from the inhibit's fall; released with the data line high, it was an inhibit alone and nothing is reported.
- A device-to-host frame begins at the first clock fall on an idle bus that the clock rises from within 100 µs. Its start bit and the ten bits after it are read on the falling edges, and the frame is reported at the eleventh. A start bit of 1 is a `start` error at the first rise.
- In a host-to-device frame the device's first ten clock pulses carry the data bits, parity and stop bit, read on their rising edges. The acknowledge is the data line's level on the eleventh pulse's falling edge: the host reads what the device drives as it reads a device's bits. When no eleventh pulse comes within 2 ms of the stop bit, the frame is reported unacknowledged (`ack: false`) once the 2 ms have passed.
- A frame is `aborted` when 2 ms pass after one of its clock edges without the next — a device sends a whole packet within 2 ms —, settled 2 ms after the last edge; when the host holds a clock low for 100 µs before the frame's last pulse, settled at the inhibit's last tick of those 100 µs (the inhibit can itself be a request to send, which then begins the host's frame); when the device does not begin clocking within 15 ms of the host releasing the clock after a request to send, settled there; and when a gap or the end of the samples cuts it off, settled at the gap's start or the end. The gap or the end is passed on after it.
- The decoder knows nothing of what came before the first sample, or before the first sample after a gap: a frame may be under way there. It begins reading frames once both lines have been high for 100 µs — twice the longest half period of the clock — and skips the clock pulses before that, a host's inhibit aside; it does the same after a `start` error or a frame whose stop bit was 0, whose bits may be misaligned. A capture should therefore begin with the bus idle for 100 µs.

| Time limit                                     | Value  |
| ---------------------------------------------- | ------ |
| A clock low that is the host's inhibit         | 100 µs |
| A pause between two clock edges of a frame     | 2 ms   |
| The device's first clock after a request       | 15 ms  |
| Both lines high before the bus is trusted idle | 100 µs |

## Limits

- The decoder reports bytes: scan codes, mouse packets and commands are the next layer's business.
- Clock frequencies, setup and hold times are not checked; a frame is read as long as its edges come within the limits above.
- A host frame is read as ten clock pulses and an acknowledge whatever its stop bit: after a stop bit of 0, the eleventh pulse is taken as the acknowledge.
