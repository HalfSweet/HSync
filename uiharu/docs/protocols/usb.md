# USB low and full speed

Two nodes decode USB 2.0 at 1.5 Mbit/s (low speed) and 12 Mbit/s (full speed), after the _Universal Serial Bus Specification Revision 2.0_ (usb.org): §7.1 for the signalling, chapter 8 for packets and transactions.

| Node              | Reads                              | Writes                |
| ----------------- | ---------------------------------- | --------------------- |
| `UsbPackets`      | logic samples: D+ and D−           | `usb.packets` v1      |
| `UsbTransactions` | `usb.packets` v1 (`UsbBus` cursor) | `usb.transactions` v1 |

```rust
let usb = graph.add(UsbPackets::new(UsbConfig { dp: 0, dm: 1, speed: Speed::Full })?);
graph.connect(logic.samples, UsbPackets::input(usb))?; // checks channels and rateHz
let xact = graph.add(UsbTransactions::new());
graph.connect(UsbPackets::packets(usb), UsbTransactions::input(xact))?;
```

## Configuration and ports

| Option  | Values          | Default | Meaning                                                                          |
| ------- | --------------- | ------- | -------------------------------------------------------------------------------- |
| `dp`    | channel 0–63    | 0       | D+                                                                               |
| `dm`    | channel 0–63    | 1       | D−; not the channel of D+                                                        |
| `speed` | `low` \| `full` | `full`  | the bit rate, and the polarity: J is D+ high at full speed, D− high at low speed |

| Port                               | Declares or requires                                                                       |
| ---------------------------------- | ------------------------------------------------------------------------------------------ |
| `usb-packets.logic` (input)        | requires `channels` ≥ max(`dp`, `dm`) + 1 and `rateHz` ≥ 4 × the bit rate: 6 MHz or 48 MHz |
| `usb-packets.packets` (output)     | declares `speed` (`"low"`, `"full"`) and `bitRateHz` (1 500 000, 12 000 000)               |
| `usb-transactions.packets` (input) | requires nothing beyond the `usb.packets` schema: transactions are the same at both speeds |
| `usb-transactions.transactions`    | declares nothing                                                                           |

A source slower than four samples per bit is refused when the graph is built, with a diagnostic naming `rateHz`.

## `usb.packets`

Each packet is framed by `begin` and `end` and numbered (`pkt`) from 1; the bus's own signalling between packets has events of its own.

| Kind         | Fields                                     | On the bus                                                                                                |
| ------------ | ------------------------------------------ | --------------------------------------------------------------------------------------------------------- |
| `begin`      | `pkt`                                      | a SYNC pattern                                                                                            |
| `pid`        | `pkt`, `pid`, `ok`                         | the PID byte: its type, and whether its high nibble is the type's complement                              |
| `token`      | `pkt`, `address`, `endpoint`, `crc5`, `ok` | OUT, IN, SETUP and PING: ADDR (7 bits), ENDP (4 bits), the CRC5 field, whether the CRC5 matches           |
| `sof`        | `pkt`, `frame`, `crc5`, `ok`               | the 11-bit frame number, the CRC5 field, whether it matches                                               |
| `data`       | `pkt`, `offset`, `bytes`                   | payload bytes of DATA0, DATA1, DATA2 or MDATA, at most 32, `offset` counted from the payload's first byte |
| `data-end`   | `pkt`, `crc16`, `ok`, `length`             | the CRC16 field (`null` when fewer than two whole bytes came), whether it matches, the payload's length   |
| `split`      | `pkt`, `raw`                               | the 24 bits after a SPLIT PID, the first in bit 0                                                         |
| `end`        | `pkt`, `reason`                            | how the packet ended: `eop`, `preamble`, `bit-stuff-error`, `pid-error`, `se1`, `aborted`                 |
| `reset`      | `ticks`                                    | SE0 for 2.5 µs or more, and how long it lasted                                                            |
| `suspend`    | —                                          | the bus has been idle for 3 ms                                                                            |
| `resume`     | —                                          | K for 20 ms or more, ended by an SE0 and J                                                                |
| `keep-alive` | —                                          | low speed: an end of packet on the idle bus                                                               |

`pid` is one of `out`, `in`, `sof`, `setup`, `data0`, `data1`, `data2`, `mdata`, `ack`, `nak`, `stall`, `nyet`, `pre`, `err`, `split`, `ping` and `reserved` (type `0000`). Type `1100` is reported as `pre`: ERR shares it and is a high-speed handshake. `crc5` and `crc16` are the fields as they appear on the wire, read least significant bit first like every field: SETUP to address 0, endpoint 0 is `2D 00 10` and carries `crc5` 2; a data packet's `crc16` is its last two bytes, little-endian.

| Event                          | Span                                                            | Settled                                                        |
| ------------------------------ | --------------------------------------------------------------- | -------------------------------------------------------------- |
| `begin`                        | the SYNC: its first cell's start through its last cell's centre | the last cell's centre                                         |
| `pid`, `token`, `sof`, `split` | the field's bits, first cell's start through last cell's centre | the last cell's centre                                         |
| `data` (full batch)            | the batch's bytes                                               | the last bit of the byte two after its last                    |
| `data` (flushed)               | the batch's bytes                                               | the first SE0 cell's centre, the gap or the end of the samples |
| `data-end`                     | the CRC16's bytes                                               | the first SE0 cell's centre                                    |
| `end` (`eop`)                  | from the SE0's start through the tick the bus left it           | that tick                                                      |
| `end` (errors)                 | the PID, or the cell where SE1 or the seventh 1 was read        | that cell's centre                                             |
| `end` (`preamble`)             | the PID                                                         | its last cell's centre                                         |
| `end` (`aborted`)              | empty, at the gap's start or the end of the samples             | that tick                                                      |
| `reset`                        | the SE0                                                         | the tick it ended                                              |
| `suspend`                      | the 3 ms of idle                                                | 3 ms after the bus went idle                                   |
| `resume`, `keep-alive`         | from the K or SE0's start through the J that ended it           | that J's first tick                                            |

### Decoding rules

- The two lines make the bus state: J and K are the differential states, SE0 is both lines low, SE1 both high; the idle bus is J.
- Bit times come from the speed and the source's time base in integer arithmetic: cell _n_ of a run starts ⌊_n_ · rate / bit rate⌋ ticks after the edge that began the run and is read at its centre, ⌊(2_n_ + 1) · rate / (2 · bit rate)⌋ ticks after it.
- The bit clock is re-aligned to the first edge after each cell centre, on either line; a cell's state is the state at its centre, so the brief single-ended state of a J/K transition whose lines change a few ticks apart is no bit. Tested are transmitters 5 % slow to 3 % fast at four samples per bit and 5 % either way at eight (the specification allows 0.25 % at full speed and 1.5 % at low speed), and lines changing up to half a bit apart.
- NRZI: a change of state is a 0, no change a 1. After six 1s the next bit is a stuffed 0 and is dropped; a seventh 1 ends the packet `bit-stuff-error`.
- SYNC: from the idle bus, K, then J and K alternating, ended by a second K in a row. Three alternating cells before the final K are enough (`KJKK`), for a SYNC shortened by repeaters; activity that does not alternate like that, such as low-speed traffic on a full-speed bus, starts no packet.
- The PID's check nibble must be the complement of its type, or the packet ends `pid-error`. Tokens, SOF and SPLIT then carry their fields; data packets their bytes and a CRC16; handshakes and the reserved type nothing; PRE ends after its PID, `preamble`, as a preamble has no EOP.
- A data packet's last two bytes are its CRC16, which only its end tells: a byte is reported once two more have followed it, in batches of at most 32 that go out as soon as they are full.
- EOP: SE0 for at least one cell. The packet ends when the SE0 does; bits after the last whole byte, or after the fields a packet's type has, are dropped. Payload lengths are reported, not checked against the speeds' limits (8 bytes at low speed, 1023 at full speed).
- Between packets: an SE0 of 2.5 µs or more is a reset, reported when it ends; J for 3 ms since the last activity is a suspend, reported as soon as the samples reach that tick; K for 20 ms or more, ended by an SE0 that is no reset and then J, is a resume; at low speed, an SE0 of the idle bus ended by J that is no reset is a keep-alive. A state that lasts less than half a bit outside a packet is a glitch.
- The first sample, and the first after a gap, may fall inside a packet, which cannot be aligned: the decoder waits for the bus to be idle — J for eight bit times, longer than bit stuffing lets a packet hold a level, or the J that ends an SE0 — before it looks for a SYNC. A reset, suspend or resume whose beginning was not seen is not reported.
- A gap or the end of the samples during a packet ends it `aborted`; the bytes not yet known to be payload, which may be its CRC16, are not reported.

## `usb.transactions`

| Kind        | Fields                                                             | On the bus                                                                    |
| ----------- | ------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `begin`     | `xact`, `token`, `address`, `endpoint`, `frame`                    | a token: `setup`, `in`, `out`, `ping` or `sof`; ADDR and ENDP, or SOF's frame |
| `setup`     | `xact`, `bmRequestType`, `bRequest`, `wValue`, `wIndex`, `wLength` | a SETUP transaction's 8 data bytes, the 16-bit fields little-endian           |
| `data`      | `xact`, `pid`, `offset`, `bytes`                                   | the data packet's payload in batches of at most 32, with its data PID         |
| `handshake` | `xact`, `pid`                                                      | ACK, NAK, STALL or NYET within the transaction                                |
| `end`       | `xact`, `reason`                                                   | `complete`, `nak`, `stall`, `no-response`, `error` or `aborted`               |

Transactions are numbered (`xact`) from 1. `address` and `endpoint` are `null` for SOF, `frame` for the other tokens. The field holding the token is named `token` because `kind` is the event's tag.

| Event                        | Span                                                       | Settled                                             |
| ---------------------------- | ---------------------------------------------------------- | --------------------------------------------------- |
| `begin`                      | the token packet from its SYNC through its fields          | the fields' last bit                                |
| `data`                       | the packet stream's batch; with no payload, its `data-end` | as the batch, or the `data-end`                     |
| `setup`                      | the 8 bytes and the CRC16                                  | the data packet's `data-end`                        |
| `handshake`                  | the handshake packet's SYNC and PID                        | the PID's last bit                                  |
| `end` (complete, nak, stall) | the end of the packet that ended it                        | as that end                                         |
| `end` (no-response)          | empty                                                      | where the next token's PID or the bus event settled |
| `end` (error)                | what was read of the packet that showed the error          | where it was read                                   |
| `end` (aborted)              | what was read of the packet cut off, if any                | the gap's start or the end of the stream            |

### Transaction rules

- A token begins a transaction once its address and endpoint are read. SETUP and OUT expect the host's data packet, then the device's handshake; IN expects the device's data packet, or NAK or STALL instead, then the host's handshake; PING expects the device's handshake; SOF is a transaction of its own, `complete` when its packet ends.
- ACK, or NYET after data, ends the transaction `complete`; NAK `nak`; STALL `stall`.
- A token, SOF, PRE or SPLIT before then, or a reset, suspend, resume or keep-alive, ends it `no-response`. Nothing ends a transaction on time alone, so the last one before a quiet bus stays open until the next packet. Isochronous transactions, which have no handshake, end `no-response` too: the endpoint's type is in its descriptors.
- A packet the transaction does not expect next, and an expected packet that is damaged — a CRC that does not match, a failed PID check, a bit stuff error, SE1, missing fields — end it `error`. A token whose CRC5 does not match begins a transaction that ends `error` at once; the device ignores it.
- Data passes through as the packet stream batched it; a data packet with no payload is one `data` event with no bytes, which keeps its PID. A SETUP transaction's data packet with 8 bytes and a matching CRC16 is also reported as `setup`.
- Packets no transaction expects — data or handshakes with none open, ERR, the reserved PID — start none; nor do PRE and SPLIT, which announce traffic this decoder cannot follow.
- A gap or the end of the stream ends the open transaction `aborted`.

The setup packet's numbers are reported as they are. For reading them, USB 2.0 §9.3 and §9.4: `bmRequestType` holds the direction in bit 7 (1: device to host), the type in bits 6–5 (0 standard, 1 class, 2 vendor) and the recipient in bits 4–0 (0 device, 1 interface, 2 endpoint, 3 other); the standard requests (`bRequest`) are GET_STATUS (0), CLEAR_FEATURE (1), SET_FEATURE (3), SET_ADDRESS (5), GET_DESCRIPTOR (6), SET_DESCRIPTOR (7), GET_CONFIGURATION (8), SET_CONFIGURATION (9), GET_INTERFACE (10), SET_INTERFACE (11) and SYNCH_FRAME (12).

## Reading packets from another decoder

`UsbBus` reads a `usb.packets` stream for a decoder built on it. `next()` hands out a `Packet`, a `BusEvent` (reset, suspend, resume, keep-alive), a gap, progress or the end. A `Packet` offers the packet's fields one at a time — `field()`, `field_or_end()`, which alone returns the normal end (`eop` or `preamble`), and `drain()` — and `consumed()` for the provenance of what was read. When a packet ends otherwise, reads fail with `Damaged(reason)` (`bit-stuff-error`, `pid-error`, `se1`), `Truncated` (`aborted`), `Gap` or `EndOfInput`; `Incomplete` when it ended normally where a field was expected.

## Limits

- High speed (480 Mbit/s) is not decoded; nor are the low-speed packets that follow PRE on a full-speed bus.
- Descriptors and what requests mean are out of scope: the streams carry the bus's numbers.
- Transactions close on packets only, not on the bus turnaround time.
- Grouping transactions into control transfers — the setup, data and status stages — is the next step: a third node reading `usb.transactions`.
- The synthesizer, `usb::synth::UsbWaveform`, builds both speeds' packets and bus states for tests and tools, with knobs for CRC, PID and stuffing errors, clock offsets and skewed lines.
