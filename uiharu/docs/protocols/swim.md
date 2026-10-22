# STM8 SWIM

Implemented independently from [STMicroelectronics UM0470, Rev 4](https://www.st.com/resource/en/user_manual/cd00173911-stm8-swim-communication-protocol-and-debug-module-stmicroelectronics.pdf), §§3.2–3.6, figures 5–9 and table 1. One open-drain data line carries arbitration, payload, XOR parity and the receiver's acknowledgement.

## Configuration

| Field          | Default | Meaning                                                                              |
| -------------- | ------- | ------------------------------------------------------------------------------------ |
| `data`         | 0       | SWIM channel, 0–63                                                                   |
| `clock_hz`     | 8000000 | Target SWIM clock, usually HSI/2; 1–100000000 Hz                                     |
| `speed`        | `low`   | Initial bit coding: `low` or `high`                                                  |
| `start_active` | true    | Treat the initial high line as a command boundary; false waits for a reset condition |

The input requires `rateHz >= clock_hz`, providing two samples in the shortest nominal low pulse, and the configured channel. Output properties are `clockHz` and `bitOrder = msb-first`.

## Stream: `swim.events` v1

| Event     | Fields                                                      | Observation                                                                                                            |
| --------- | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `command` | `value`, `parityOk`, `ack`                                  | Host packet with three command bits: 0 SRST, 1 ROTF, 2 WOTF; others reserved                                           |
| `byte`    | `direction`, `field`, `value`, `parityOk`, `ack`, `address` | Eight-bit physical packet; command position is inferred separately                                                     |
| `reset`   | —                                                           | Low pulse exceeding 64 configured SWIM clocks; host reset and target synchronization are indistinguishable on one wire |
| `entry`   | —                                                           | Four nominal 1 kHz pulses followed by four 2 kHz pulses after a low prelude of at least 16 µs                          |
| `error`   | `reason`, `partial`                                         | Packet or pulse interrupted by lost samples, capture end (`aborted`) or a reset (`reset`)                              |

`direction` is `host-to-target` for header 0 and `target-to-host` for header 1. `field` is `count`, `address-extended`, `address-high`, `address-low`, `data` or `unclassified`. `address` is only present for payload bytes after an accepted count and 24-bit address. A NACK or incorrect parity leaves the same command position and payload address for a retry. ACK is 1; NACK is 0 and may mean the receiver is busy. A NACK alone is not marked an error.

## Decoding rules

- Every falling edge starts a bit; its low width determines the bit at the rising edge. Low speed uses the documented receiver threshold of eight clock periods, high speed four: widths at or below the threshold are 1, greater widths are 0. A width exceeding 64 clocks takes priority as a reset condition. Integer cross-products compare widths without rounding individual clock periods.
- Header, payload MSB first, XOR parity and ACK form one packet. A packet event spans the header's falling edge through the ACK's rising edge inclusive and settles at that rising edge. At most eleven bits are retained. There is no invented inter-bit timeout: the manual gives no maximum high interval.
- Accepted ROTF/WOTF commands establish count and extended/high/low address fields. Accepted data increments the 24-bit address, wrapping at its boundary; read/write retries do not increment it. A zero count clears context instead of guessing 256 bytes. A host command can replace a pending read between returned bytes. Reserved commands are reported raw and clear context.
- An accepted WOTF write to SWIM_CSR (`0x007f80`) tracks HS bit 4, applying the resulting speed once the accepted transfer count completes. Reset and entry select low speed. No other register semantics are interpreted.
- Entry pulses accept low and intervening high half-periods within 25% of nominal. Recognition reports the waveform, not proof of the target's mode. The prelude also meets the reset condition and produces `reset`; the subsequent target synchronization pulse also produces `reset`.
- A low present at the first sample is partial: short partial pulses produce an error and decoding waits for reset. A sufficiently long observed low proves the reset condition even if its beginning was missing. After every gap the decoder discards command context and awaits reset. Incomplete packets emit an error and never a fabricated byte.
- Progress never passes the start of a pending packet, pulse or entry sequence. Errors settle at the interruption; reset and entry events settle at the last observed rising edge. Gaps and end of input follow any error.

## Limits

The target clock must be supplied; synchronization pulses are observed but do not recalibrate it. A capture starting on a high line cannot reveal whether the next host packet is a command or a command parameter. With `start_active` enabled, begin at a known command boundary; otherwise capture a reset. After damaged or lost framing, capture a reset to regain trustworthy alignment. The decoder does not infer a missing packet or distinguish electrically colliding transmitters.

Physical packet observations remain separate from inferred command positions. An unexpected target packet during host parameters is reported unclassified and does not advance them. Parity/ACK cannot prove that a memory access succeeded. CPU/debug registers, flash programming algorithms, NRST and target internal state are not decoded. Only nominal entry timing with the documented frequency ratio is recognized; arbitrary entry frequencies outside the stated windows are not recognized.
