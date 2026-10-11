# SWD

The `swd` decoder (`uiharu_protocols::swd::Swd`) reads Arm Serial Wire Debug from logic samples: the packets a debug host sends and a target answers, line resets, and the sequences that switch a Serial Wire/JTAG debug port between SWD and JTAG. It is written from the _Arm Debug Interface Architecture Specification, ADIv5.0 to ADIv5.2_ (ARM IHI 0031): its Serial Wire Debug protocol and the switching sequences of the SWJ-DP.

## Configuration

| Field   | Meaning           | Default |
| ------- | ----------------- | ------- |
| `swclk` | The clock channel | 0       |
| `swdio` | The data channel  | 1       |

The two channels must differ. The input requires a source with `channels` of at least the higher channel plus one.

## Output: `swd.events`, version 1

| Property     | Value                                              |
| ------------ | -------------------------------------------------- |
| `turnaround` | `1`: the turnaround the decoder assumes, in clocks |

Each event is tagged with its `kind`. "A bit" below is the tick at which SWCLK rose with that bit on SWDIO.

| Kind         | Fields                                                                     | What happened on the wire                                                                                                                                                                     | Span and settled tick                                                                     |
| ------------ | -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `begin`      | `tx`                                                                       | A packet begins with its Start bit.                                                                                                                                                           | The Start bit; settled at the first low bit after it, which shows it is no line reset     |
| `request`    | `tx`, `apndp`, `rnw`, `addr`, `parityOk`                                   | The host's request: an access port (`apndp`) or debug port request, a read (`rnw`) or write, A\[3:2\] as the byte address `0x0`, `0x4`, `0x8` or `0xC`, and whether the parity bit was right. | Start through Park; settled at Park                                                       |
| `ack`        | `tx`, `value`: `ok`, `wait`, `fault` or `{"other": n}`                     | The target's acknowledge, read least significant bit first (OK is 1, 0, 0 on the wire); `other` carries any other value, such as 7 when no target drove the line.                             | The three acknowledge bits; settled at the last                                           |
| `data`       | `tx`, `value`, `parityOk`, `fromTarget`                                    | The 32-bit data word, least significant bit first, whether its parity bit was right, and whether the target sent it (a read) or the host (a write).                                           | The data bits through the parity bit; settled at the parity bit                           |
| `end`        | `tx`, `reason`: `complete`, `wait`, `fault`, `protocol-error` or `aborted` | How the packet ended (see below).                                                                                                                                                             | The bit that ended it; empty at the gap's start or the end of the samples for `aborted`   |
| `line-reset` | —                                                                          | SWDIO was high for at least 50 clocks in a row.                                                                                                                                               | The run of high bits; settled at the first low bit after it, or where the samples stopped |
| `switch`     | `to`: `swd` or `jtag`                                                      | A select sequence followed a line reset: JTAG-to-SWD (`0xE79E`) or SWD-to-JTAG (`0xE73C`), sent least significant bit first.                                                                  | The sixteen bits; settled at the last                                                     |

Bits the decoder had to hold back — after a line reset, while they could still be a select sequence, or in a run of 8 to 49 high bits, which could have been a line reset — are decoded once they are resolved, and the events they decide are settled at the bit that resolved them, never earlier.

Packets are numbered from 1. A packet ends:

| Reason           | On the bit                                                                                                                                                                                                                                                 |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `complete`       | OK, and the data phase was whole: the turnaround after a read's parity bit, or a write's parity bit                                                                                                                                                        |
| `wait`, `fault`  | The turnaround after the WAIT or FAULT acknowledge; no data follows                                                                                                                                                                                        |
| `protocol-error` | The bit that broke the protocol: Park, for a request whose parity is wrong, whose Stop bit is not 0 or whose Park bit is not 1; the last acknowledge bit, for an acknowledge other than OK, WAIT and FAULT; the parity bit, for data whose parity is wrong |
| `aborted`        | None: a gap or the end of the samples cut the packet off                                                                                                                                                                                                   |

```json
{"kind":"begin","tx":1}
{"kind":"request","tx":1,"apndp":false,"rnw":true,"addr":0,"parityOk":true}
{"kind":"ack","tx":1,"value":"ok"}
{"kind":"data","tx":1,"value":731911287,"parityOk":true,"fromTarget":true}
{"kind":"end","tx":1,"reason":"complete"}
```

## Decoding rules

- Every bit is read at the tick SWCLK rises. The host drives SWDIO before that edge and the target after the falling edge before it, so the same edge reads both directions.
- A packet is a request — Start, APnDP, RnW, A\[2\], A\[3\], Parity, Stop, Park — one turnaround and a three-bit acknowledge. After OK, a read goes on with 32 data bits and a parity bit from the target, then a turnaround back to the host; a write goes on with a turnaround, then 32 data bits and a parity bit from the host. After WAIT or FAULT comes a turnaround and no data. Parity is even: the parity bit makes the count of ones it covers even.
- A line reset is at least 50 clocks in a row with SWDIO high. Right after one, the two select sequences are recognised; bits that may still be one are held back, and decoded as idle cycles and packets once they cannot be.
- Packets are decoded only where it is known where they start: after a line reset, until a protocol error, a gap or a switch to JTAG. At the first sample, after a gap and after a protocol error the decoder waits for the next line reset. It does not resynchronise on idle cycles, because the bits of a data phase can look just like idle cycles followed by a Start bit. After a switch to JTAG no packet is decoded until a line reset and a switch back to SWD; a line reset without a select sequence leaves the port where it was, in SWD when that was not known.
- Between packets a low bit is an idle cycle and a high bit a Start bit, unless the run of high bits it starts grows into a line reset. A packet is therefore reported from the first low bit of its request, which a well-formed request has among its first six bits. A run of 8 to 49 high bits between packets is a request of all ones, which is malformed.
- High bits at the end of a packet that ended with a protocol error count toward the line reset that brings the decoder back, so its span may start inside that packet.
- A gap or the end of the samples ends the packet under way `aborted`; a run of at least 50 high bits that they cut short is still reported as a line reset.
- A bit decides at most one field of a packet and then the packet's end, in that order.

## What the addresses select

The decoder reports `apndp`, `rnw` and `addr` as the wire carries them; naming the register is the next layer's business. For readers of the stream, ADIv5.2 assigns the debug port's addresses as follows (`apndp` false):

| `addr` | Read                                                                        | Write                                          |
| ------ | --------------------------------------------------------------------------- | ---------------------------------------------- |
| `0x0`  | DPIDR                                                                       | ABORT                                          |
| `0x4`  | CTRL/STAT, DLCR, TARGETID, DLPIDR or EVENTSTAT, as SELECT.DPBANKSEL chooses | CTRL/STAT or DLCR, as SELECT.DPBANKSEL chooses |
| `0x8`  | RESEND                                                                      | SELECT                                         |
| `0xC`  | RDBUFF                                                                      | TARGETSEL                                      |

An access port request (`apndp` true) addresses the register at A\[3:2\] in the bank SELECT.APBANKSEL chooses, of the access port SELECT.APSEL chooses. Access port reads are posted: the data a read returns is that of the access port read before it, and RDBUFF collects the last one.

## Reading the stream

Decoders built on SWD read the stream through `SwdBus`. `next()` waits for the next packet, line reset, switch, gap, progress or end; a packet comes as a `Packet` cursor whose `field()` reads the next field — `Field::Request`, `Field::Ack` or `Field::Data`, in wire order — and reports the packet's end as `ReadError::Ended(reason)` where a field was expected, `field_or_end()` returns the end as `Read::End(reason)` instead, and `drain()` skips to the end and says how the packet ended. A packet cut off by a gap or the end of the samples is `ReadError::Aborted`; `SwdBus::finish()` drains a packet a decoder left open, and `consumed()` hands over the ticks of what was read.

## Limits

- The turnaround lasts one clock, the reset value of DLCR.TURNROUND.
- Overrun detection (CTRL/STAT.ORUNDETECT) is not supported: with it, WAIT and FAULT are followed by a data phase the decoder does not expect.
- Multi-drop SWD: the TARGETSEL write, whose acknowledge no target drives, ends with `protocol-error`, and the decoder then waits for the next line reset.
- The dormant state of ADIv6 and the sequences that enter and leave it are out of scope.
- No register is named in the events (see above).
