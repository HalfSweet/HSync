# SWD

The `swd` decoder (`uiharu_protocols::swd::Swd`) reads Arm Serial Wire Debug from logic samples: the packets a debug host sends and a target answers, line resets, the sequences that switch a Serial Wire/JTAG debug port between SWD, JTAG and the dormant state, and the Selection Alert that wakes a dormant port. It is written from the _Arm Debug Interface Architecture Specification, ADIv6.0_ (ARM IHI 0074): the Serial Wire Debug protocol of its chapter B4, the switching and dormant sequences of chapter B5, and the debug port registers of chapter B2.

## Configuration

| Field            | Meaning                                                                                                                                                    | Default |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ------- |
| `swclk`          | The clock channel                                                                                                                                          | 0       |
| `swdio`          | The data channel                                                                                                                                           | 1       |
| `turnaround`     | The turnaround period in clocks, 1 to 4: what the debugger wrote to DLCR.TURNROUND, one after a reset                                                      | 1       |
| `overrun_detect` | Whether overrun detection (CTRL/STAT.ORUNDETECT) is on: `auto` follows the writes to CTRL/STAT on the wire, `on` and `off` say so outright (see the rules) | `auto`  |

The two channels must differ. The input requires a source with `channels` of at least the higher channel plus one.

## Output: `swd.events`, version 1

| Property     | Value                                                     |
| ------------ | --------------------------------------------------------- |
| `turnaround` | The turnaround the decoder assumes, in clocks: `1` to `4` |

Each event is tagged with its `kind`. Every bit belongs to a rising edge of SWCLK, where the target samples what the host drives: "a bit" below is that edge's tick. The target changes SWDIO on the rising edge, so its bits — the acknowledge, a read's data and parity — are read at the falling edge before the edge they belong to.

| Kind         | Fields                                                                     | What happened on the wire                                                                                                                                                                                                                                           | Span and settled tick                                                                     |
| ------------ | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `begin`      | `tx`                                                                       | A packet begins with its Start bit.                                                                                                                                                                                                                                 | The Start bit; settled at the first low bit after it, which shows it is no line reset     |
| `request`    | `tx`, `apndp`, `rnw`, `addr`, `parityOk`                                   | The host's request: an access port (`apndp`) or debug port request, a read (`rnw`) or write, A\[3:2\] as the byte address `0x0`, `0x4`, `0x8` or `0xC`, and whether the parity bit was right.                                                                       | Start through Park; settled at Park                                                       |
| `ack`        | `tx`, `value`: `ok`, `wait`, `fault` or `{"other": n}`                     | The target's acknowledge, read least significant bit first (OK is 1, 0, 0 on the wire); `other` carries any other value, such as 7 when no target drove the line — as after a TARGETSEL write, which no target acknowledges.                                        | The three acknowledge bits; settled at the last                                           |
| `data`       | `tx`, `value`, `parityOk`, `fromTarget`                                    | The 32-bit data word, least significant bit first, whether its parity bit was right, and whether the target sent it (a read) or the host (a write).                                                                                                                 | The data bits through the parity bit; settled at the parity bit                           |
| `end`        | `tx`, `reason`: `complete`, `wait`, `fault`, `protocol-error` or `aborted` | How the packet ended (see below).                                                                                                                                                                                                                                   | The bit that ended it; empty at the gap's start or the end of the samples for `aborted`   |
| `line-reset` | —                                                                          | SWDIO was high for at least 50 clocks in a row.                                                                                                                                                                                                                     | The run of high bits; settled at the first low bit after it, or where the samples stopped |
| `switch`     | `to`: `swd`, `jtag` or `dormant`                                           | A select sequence followed a line reset — JTAG-to-SWD (`0xE79E`), SWD-to-JTAG (`0xE73C`) or SWD-to-dormant (`0xE3BC`), sixteen bits sent least significant first — or a Selection Alert and an activation code woke the port from the dormant state to SWD or JTAG. | The sixteen bits, or the alert through the code; settled at the last                      |

Bits the decoder had to hold back — after a line reset, while they could still be a select sequence, or in a run of 8 to 49 high bits, which could have been a line reset — are decoded once they are resolved, and the events they decide are settled at the bit that resolved them, never earlier.

Packets are numbered from 1. A packet ends:

| Reason           | On the bit                                                                                                                                                                                                                                                                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `complete`       | OK, and the data phase was whole: the last clock of the turnaround after a read's parity bit, or a write's parity bit; also a TARGETSEL write's parity bit, whatever its acknowledge                                                                                                        |
| `wait`, `fault`  | The last clock of the turnaround after the WAIT or FAULT acknowledge, when no data follows; under overrun detection, the last bit of the data phase that follows all the same                                                                                                               |
| `protocol-error` | The bit that broke the protocol: Park, for a request whose parity is wrong, whose Stop bit is not 0 or whose Park bit is not 1; the last acknowledge bit, for an acknowledge other than OK, WAIT and FAULT (except after a TARGETSEL write); the parity bit, for data whose parity is wrong |
| `aborted`        | None: a gap or the end of the samples cut the packet off                                                                                                                                                                                                                                    |

```json
{"kind":"begin","tx":1}
{"kind":"request","tx":1,"apndp":false,"rnw":true,"addr":0,"parityOk":true}
{"kind":"ack","tx":1,"value":"ok"}
{"kind":"data","tx":1,"value":731911287,"parityOk":true,"fromTarget":true}
{"kind":"end","tx":1,"reason":"complete"}
```

## Decoding rules

- The host's bits — the request, idle cycles, written data, line resets and select sequences — are read at the rising edge of SWCLK, where the target samples them. The target changes SWDIO on the rising edge (B4.3.1), so its bits — the acknowledge, and a read's data and parity — are read at the falling edge before the rising edge they belong to, where they have settled whatever the target's output delay; reading them at the rising edge would race that delay.
- A packet is a request — Start, APnDP, RnW, A\[2\], A\[3\], Parity, Stop, Park — a turnaround and a three-bit acknowledge. After OK, a read goes on with 32 data bits and a parity bit from the target, then a turnaround back to the host; a write goes on with a turnaround, then 32 data bits and a parity bit from the host. After WAIT or FAULT comes a turnaround and no data. Parity is even: the parity bit makes the count of ones it covers even. A turnaround lasts the clocks `turnaround` says; a packet ends on the last of them.
- A write to debug port address `0xC` is a TARGETSEL write (B4.3.4, SWD protocol version 2): the target does not drive the acknowledge, and the host sends the data phase whatever the line showed, then reads DPIDR with no line reset between. The decoder reports the acknowledge bits as read and decodes the write's data phase after them; a parity error there is a protocol error, as the specification treats it.
- With overrun detection on (CTRL/STAT.ORUNDETECT, B4.2.6), WAIT and FAULT carry a data phase all the same: undriven for a read, ignored by the target for a write. The decoder reads it through, emits no `data` for it and ends the packet after it. With `overrun_detect` at `auto` the decoder follows the wire: overrun detection is off, as after a powerup reset, until a debug port write to `0x4` acknowledged OK with a good parity sets or clears bit 0 while SELECT.DPBANKSEL is 0, where `0x4` is CTRL/STAT; writes to SELECT (`0x8`) choose the bank with their low four bits, and a line reset puts it back to 0 (B4.3.3) while CTRL/STAT keeps its value. `on` and `off` fix it for captures that begin after the write that set it.
- A line reset is at least 50 clocks in a row with SWDIO high. Right after one, the three select sequences are recognised; bits that may still be one are held back, and decoded as idle cycles and packets once they cannot be.
- The Selection Alert (B5.3.4) — 128 bits, then four clocks whose SWDIO the target ignores, then an activation code sent most significant bit first — is watched for on every bit of the wire, matched bit by bit. The Arm CoreSight SW-DP code (`0b0101_1000`) wakes the port to SWD, the Arm CoreSight JTAG-DP code (`0b0101_0000`) and the JTAG-Serial code (twelve zero bits) to JTAG; a code the decoder does not know leaves the port where it was. A packet under way when the alert completes was none: it ends with `protocol-error`.
- Packets are decoded only where it is known where they start: after a line reset, until a protocol error, a gap or a switch to JTAG or dormant. At the first sample, after a gap, after a protocol error and after a wake to SWD — which leaves the target in the protocol error state until the line reset the host must send — the decoder waits for the next line reset. It does not resynchronise on idle cycles, because the bits of a data phase can look just like idle cycles followed by a Start bit. After a switch to JTAG or dormant no packet is decoded until a line reset and a switch back to SWD, or a wake to it; a line reset without a select sequence leaves the port where it was, in SWD when that was not known.
- Between packets a low bit is an idle cycle and a high bit a Start bit, unless the run of high bits it starts grows into a line reset. A packet is therefore reported from the first low bit of its request, which a well-formed request has among its first six bits. A run of 8 to 49 high bits between packets is a request of all ones, which is malformed.
- High bits at the end of a packet that ended with a protocol error count toward the line reset that brings the decoder back, so its span may start inside that packet.
- A gap or the end of the samples ends the packet under way `aborted`; a run of at least 50 high bits that they cut short is still reported as a line reset.
- A bit decides at most one field of a packet and then the packet's end, in that order.

## What the addresses select

The decoder reports `apndp`, `rnw` and `addr` as the wire carries them; naming the register is the next layer's business. For readers of the stream, ADIv6 (DP architecture version 3, Table B2-3) assigns the debug port's addresses as follows (`apndp` false), SELECT.DPBANKSEL choosing the bank of `0x0` and `0x4`:

| `addr` | Read                                                                  | Write                                     |
| ------ | --------------------------------------------------------------------- | ----------------------------------------- |
| `0x0`  | DPIDR (bank 0), DPIDR1 (1), BASEPTR0 (2), BASEPTR1 (3)                | ABORT                                     |
| `0x4`  | CTRL/STAT (bank 0), DLCR (1), TARGETID (2), DLPIDR (3), EVENTSTAT (4) | CTRL/STAT (bank 0), DLCR (1), SELECT1 (5) |
| `0x8`  | RESEND                                                                | SELECT                                    |
| `0xC`  | RDBUFF                                                                | TARGETSEL (SWD protocol version 2)        |

An access port request (`apndp` true) addresses the register at A\[3:2\] in the bank SELECT.APBANKSEL chooses, of the access port SELECT.APSEL chooses. Access port reads are posted: the data a read returns is that of the access port read before it, and RDBUFF collects the last one.

## Reading the stream

Decoders built on SWD read the stream through `SwdBus`. `next()` waits for the next packet, line reset, switch, gap, progress or end; a packet comes as a `Packet` cursor whose `field()` reads the next field — `Field::Request`, `Field::Ack` or `Field::Data`, in wire order — and reports the packet's end as `ReadError::Ended(reason)` where a field was expected, `field_or_end()` returns the end as `Read::End(reason)` instead, and `drain()` skips to the end and says how the packet ended. A packet cut off by a gap or the end of the samples is `ReadError::Aborted`; `SwdBus::finish()` drains a packet a decoder left open, and `consumed()` hands over the ticks of what was read.

## Limits

- The turnaround is the configuration's: writes to DLCR are not followed, and a line reset, which resets DLCR, does not put it back to one.
- Overrun detection followed from the wire is only as right as the writes the decoder saw: a write to CTRL/STAT before the samples begin, in a gap or while the decoder was not aligned is missed. `on` and `off` are for those captures.
- The JTAG-to-dormant sequence is defined by the TAP controller's zero-bit scans (B5.3.2) and is not recognised on SWDIO; the Selection Alert that follows it is.
- A Selection Alert the decoder had already taken for idle cycles and packets — because it believed the port in SWD — is still reported as a switch, but the events decoded from its bits before it completed stand.
- No register is named in the events (see above).

The optional TypeScript [`swd-dp`](swd-dp.md) layer interprets observed DP/AP
registers, SW-DP posted AP reads/RDBUFF/RESEND and candidate MEM-AP addresses.
The physical decoder's `strict_start` defaults true (wait for an observed line
reset); false searches for request headers immediately and after input loss or
protocol errors. This does not infer an unseen SELECT, AP bank or memory state.
