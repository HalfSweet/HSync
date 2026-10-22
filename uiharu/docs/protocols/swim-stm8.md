<!-- SPDX-License-Identifier: Apache-2.0 -->

# STM8 SWIM streaming transport

`swim-stm8` implements the SWIM physical and command-position state machine in
TypeScript, from [ST UM0470 Rev4 §§3.2–3.6](https://www.st.com/resource/en/user_manual/um0470-stm8-swim-communication-protocol-and-debug-module-stmicroelectronics.pdf).
It owns `swim.events@1`; the original native `swim` decoder mechanically reuses
these generated event types while retaining its original algorithm as a baseline.

| Option           | Default | Meaning                                                                                                |
| ---------------- | ------- | ------------------------------------------------------------------------------------------------------ |
| `data`           | 0       | SWIM channel                                                                                           |
| `clock_hz`       | 8000000 | Initial SWIM clock, commonly HSI/2; 1–100 MHz                                                          |
| `speed`          | low     | Initial receiver threshold, low or high                                                                |
| `start_active`   | true    | Trust an initial high line as a command boundary; otherwise await reset/entry                          |
| `calibrate_sync` | false   | Trust the next low pulse after observed reset/entry as the target's 128-clock synchronization response |

Packets report arbitration direction, three-bit command or eight-bit byte,
MSB-first value, XOR parity and observed receiver ACK. The receiver uses the
specified eight-clock low-speed or four-clock high-speed threshold. SRST, ROTF
and WOTF command values remain observed commands. Count/address/data position is
inferred only from accepted parity-valid command and parameter packets; NACK or
bad parity retains the same field/address for a retry. A zero count is illegal
(UM0470 permits 1–255), reports `invalid-count` and clears command context.
Unmatched target traffic remains `unclassified`.

An acknowledged WOTF byte at SWIM_CSR `0x7f80` retains its requested HS bit and
changes the receiver's speed profile only after the complete command reaches
idle. This is interpretation of observed transport, not proof that a target
completed a register or memory effect. Reset returns the receiver to low speed.
No maximum inter-bit high interval is invented.

Entry recognition requires the prelude followed by four nominal 1 kHz and four
2 kHz pulses, with matching high/low half-periods within 25%. A low pulse longer
than 64 configured clocks reports reset/synchronization and clears partial
traffic. Seeing the entry pattern does not prove the target entered active mode.

With `calibrate_sync` enabled, the next complete low pulse following a recognized
entry or communication reset is interpreted as the 128-clock response specified
in UM0470. The decoder emits `calibration` with measured `lowTicks`, estimated
`clockHz` and the preceding `entry`/`reset` condition, then applies that estimate
to subsequent pulse thresholds. The estimate must be 1–100 MHz and no higher
than sample rate. It is quantized by capture resolution. The option is an explicit
assumption about capture phase: one wire cannot distinguish target response from
an unrelated host pulse or prove that a damaged entry succeeded. Leave it off
when that response is missing or ambiguous. A partial initial pulse never
calibrates. No capture timestamp or target execution clock is inferred.

The implementation retains only scalar fields, one packet, one entry candidate
and provenance. It buffers no transaction. Packet output settles at ACK release;
entry/calibration output covers observed pulses. Gaps/end report interrupted
traffic only through the known captured boundary, clear all command and
calibration phase context and await a new reset/entry. A capture beginning low is
explicitly partial. Malformed parity/ACK does not fabricate a completed command.

Tests reuse independently authored UM0470 literal bit vectors and nominal
waveforms against both native and TS physical decoders. Additional vectors cover
calibration at a different initial clock, entry/reset recovery, illegal count,
all options, damage, gaps, truncation and block/budget/queue invariance. Generated
and interpreted execution compare complete event/provenance streams. No GPL
source was used for the SWIM algorithm.
