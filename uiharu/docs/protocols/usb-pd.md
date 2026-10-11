# USB Power Delivery

`uiharu_protocols::usb_pd::UsbPd` decodes the USB Power Delivery packets on the CC line from logic samples and emits them as `usbpd.events`, version 1. It follows the _Universal Serial Bus Power Delivery Specification_, revision 3.1 (usb.org): chapter 5 for the physical layer — BMC signalling, the preamble, the 4b5b symbols and their K-codes, the ordered sets, the CRC and the end of packet — and chapter 6 for the protocol layer — the message header and the extended message header, the control, data and extended message types, and the power and request data objects.

The CC line carries BMC at 300 kbit/s with a swing of about 1.1 V; a comparator, or a logic analyzer input whose threshold sits inside the swing, turns it into the logic level the decoder reads. BMC carries everything in its transitions, so either level may be the idle one.

```rust
let pd = graph.add(UsbPd::new(UsbPdConfig { cc: 0, ..UsbPdConfig::default() })?);
graph.connect(logic.samples, UsbPd::input(pd))?; // checks channels and rateHz
let events = UsbPd::events(pd);
```

## Configuration

| Field       | Default | Meaning                                                                                                                                 |
| ----------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `cc`        | 0       | The channel of the CC line.                                                                                                             |
| `bit_rate`  | 300 000 | The nominal bit rate in bit/s.                                                                                                          |
| `tolerance` | 10      | How far, in percent, the bit rate a preamble shows and each bit may be off, 0 to 25. USB PD transmitters keep 270 to 330 kbit/s, ±10 %. |

The input `logic` requires `channels` ≥ `cc` + 1 and `rateHz` ≥ 6 × `bit_rate`, 1.8 MHz at 300 kbit/s: with fewer samples per bit, a bit's middle and its end can no longer be told apart within the tolerance. The graph refuses a source that does not meet them, naming the property, for example `usb-pd#0.logic requires rateHz >= 1800000 of logic#0.samples, which declares 1000000`.

The output `events` declares:

| Property    | Value                       |
| ----------- | --------------------------- |
| `bitRate`   | The nominal bit rate, bit/s |
| `tolerance` | The tolerance, percent      |

## The stream: `usbpd.events` v1

The stream is framed. Every packet has a `begin` and an `end` with the same `pkt`, numbered from 1 in the order packets begin; between them come the packet's fields in the order they are on the line. Hard Reset and Cable Reset signalling are packets too.

| Kind              | Fields                                                                                                                                           | Meaning                                                                                                                                                                                                                                                                  |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `begin`           | `pkt`                                                                                                                                            | A packet begins: its preamble.                                                                                                                                                                                                                                           |
| `sop`             | `pkt`, `set`, `corrupt`                                                                                                                          | The ordered set of a message: `sop`, `sop-prime`, `sop-double-prime`, `sop-prime-debug` or `sop-double-prime-debug`. `corrupt` is the position, 1 to 4, of a K-code that did not match the set, which was recognised from the other three; `null` when all four matched. |
| `hard-reset`      | `pkt`, `corrupt`                                                                                                                                 | Hard Reset signalling: RST-1 RST-1 RST-1 RST-2.                                                                                                                                                                                                                          |
| `cable-reset`     | `pkt`, `corrupt`                                                                                                                                 | Cable Reset signalling: RST-1 Sync-1 RST-1 Sync-3.                                                                                                                                                                                                                       |
| `header`          | `pkt`, `messageType`, `control`, `data`, `extendedMessage`, `dataRole`, `powerRole`, `cablePlug`, `revision`, `messageId`, `objects`, `extended` | The message header; see below.                                                                                                                                                                                                                                           |
| `extended-header` | `pkt`, `dataSize`, `requestChunk`, `chunkNumber`, `chunked`                                                                                      | The extended message header of an extended message: Data Size (bits 8–0, the bytes of the whole message's data), Request Chunk (bit 10), Chunk Number (bits 14–11), Chunked (bit 15).                                                                                    |
| `object`          | `pkt`, `index`, `raw`                                                                                                                            | A data object of a data message, its 32 bits as sent; `index` counts from 1, as Object Position does.                                                                                                                                                                    |
| `pdo`             | `pkt`, `index`, `supply`, …                                                                                                                      | The object before it, read as a power data object: in Source_Capabilities and Sink_Capabilities messages.                                                                                                                                                                |
| `rdo`             | `pkt`, `position`, `giveBack`, `capabilityMismatch`, `usbCommunicationsCapable`, `noUsbSuspend`, `unchunkedExtended`, `operating`, `maximum`     | The first object of a Request, read as a request data object.                                                                                                                                                                                                            |
| `data`            | `pkt`, `offset`, `bytes`                                                                                                                         | Bytes of an extended message after its extended header, at most 32 per event; `offset` counts from the first of them.                                                                                                                                                    |
| `crc`             | `pkt`, `value`, `ok`                                                                                                                             | The CRC-32 field, its first bit in bit 0, and whether it matches the header and data before it.                                                                                                                                                                          |
| `error`           | `pkt`, `reason`                                                                                                                                  | What went wrong; `end` with `error` follows.                                                                                                                                                                                                                             |
| `end`             | `pkt`, `reason`                                                                                                                                  | The packet ended.                                                                                                                                                                                                                                                        |

A packet that ended early lacks the events of the fields it did not reach.

| `end` reason                | The packet ended because                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------ |
| `eop`                       | Its EOP and final transition came: it is whole. Receivers accept it when `crc.ok` is true. |
| `hard-reset`, `cable-reset` | It was reset signalling, which ends after its ordered set.                                 |
| `error`                     | An error ended it; the `error` event before it says which.                                 |
| `aborted`                   | The samples ended or were lost while it was under way.                                     |

| `error` reason | What happened                                                                                                                     |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `timing`       | A bit outside the tolerance — a transition too early, too late or missing — or a preamble whose bit rate is outside it.           |
| `symbol`       | A five-bit code the 4b5b table does not have, or a K-code other than EOP among the data.                                          |
| `ordered-set`  | The first four symbols are no ordered set, not even with one K-code corrupted, or they are one K-code away from two sets at once. |
| `early-eop`    | An EOP before the fields the header announces, or in the middle of a byte.                                                        |
| `missing-eop`  | Another symbol where the EOP belongs, after the CRC: the packet is longer than its header says.                                   |

A Source_Capabilities with one fixed supply of 5 V at 3 A from a DFP source, and the sink's GoodCRC, as serde writes the events:

```json
{"kind":"begin","pkt":1}
{"kind":"sop","pkt":1,"set":"sop","corrupt":null}
{"kind":"header","pkt":1,"messageType":1,"control":null,"data":"source-capabilities","extendedMessage":null,"dataRole":"dfp","powerRole":"source","cablePlug":null,"revision":2,"messageId":0,"objects":1,"extended":false}
{"kind":"object","pkt":1,"index":1,"raw":102700}
{"kind":"pdo","pkt":1,"index":1,"supply":"fixed","millivolts":5000,"milliamps":3000,"flags":0}
{"kind":"crc","pkt":1,"value":824979162,"ok":true}
{"kind":"end","pkt":1,"reason":"eop"}
{"kind":"begin","pkt":2}
{"kind":"sop","pkt":2,"set":"sop","corrupt":null}
{"kind":"header","pkt":2,"messageType":1,"control":"good-crc","data":null,"extendedMessage":null,"dataRole":"ufp","powerRole":"sink","cablePlug":null,"revision":2,"messageId":0,"objects":0,"extended":false}
{"kind":"crc","pkt":2,"value":1665252341,"ok":true}
{"kind":"end","pkt":2,"reason":"eop"}
```

### The message header

| Field             | Bits  | Meaning                                                                                                                             |
| ----------------- | ----- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `messageType`     | 4–0   | Message Type, as sent.                                                                                                              |
| `control`         | —     | The name of a control message — no data objects, not extended — from the table below; `null` for other messages and reserved types. |
| `data`            | —     | The name of a data message — data objects, not extended; `null` otherwise.                                                          |
| `extendedMessage` | —     | The name of an extended message; `null` otherwise.                                                                                  |
| `dataRole`        | 5     | Port Data Role in SOP messages, `ufp` or `dfp`; `null` in the others, where the bit is reserved.                                    |
| `powerRole`       | 8     | Port Power Role in SOP messages, `sink` or `source`; `null` in the others.                                                          |
| `cablePlug`       | 8     | Cable Plug in the messages of SOP', SOP'' and their debug sets: `true` when a cable plug or VPD sent it; `null` in SOP messages.    |
| `revision`        | 7–6   | Specification Revision as sent: 0 for revision 1.0, 1 for 2.0, 2 for 3.x.                                                           |
| `messageId`       | 11–9  | MessageID.                                                                                                                          |
| `objects`         | 14–12 | Number of Data Objects.                                                                                                             |
| `extended`        | 15    | Extended.                                                                                                                           |

Message types are named as the specification names them, in kebab-case:

| Messages        | Types                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Control (§6.3)  | 1 `good-crc`, 2 `goto-min`, 3 `accept`, 4 `reject`, 5 `ping`, 6 `ps-rdy`, 7 `get-source-cap`, 8 `get-sink-cap`, 9 `dr-swap`, 10 `pr-swap`, 11 `vconn-swap`, 12 `wait`, 13 `soft-reset`, 14 `data-reset`, 15 `data-reset-complete`, 16 `not-supported`, 17 `get-source-cap-extended`, 18 `get-status`, 19 `fr-swap`, 20 `get-pps-status`, 21 `get-country-codes`, 22 `get-sink-cap-extended`, 23 `get-source-info`, 24 `get-revision`                                                    |
| Data (§6.4)     | 1 `source-capabilities`, 2 `request`, 3 `bist`, 4 `sink-capabilities`, 5 `battery-status`, 6 `alert`, 7 `get-country-info`, 8 `enter-usb`, 9 `epr-request`, 10 `epr-mode`, 11 `source-info`, 12 `revision`, 15 `vendor-defined`                                                                                                                                                                                                                                                         |
| Extended (§6.5) | 1 `source-capabilities-extended`, 2 `status`, 3 `get-battery-cap`, 4 `get-battery-status`, 5 `battery-capabilities`, 6 `get-manufacturer-info`, 7 `manufacturer-info`, 8 `security-request`, 9 `security-response`, 10 `firmware-update-request`, 11 `firmware-update-response`, 12 `pps-status`, 13 `country-info`, 14 `country-codes`, 15 `sink-capabilities-extended`, 16 `extended-control`, 17 `epr-source-capabilities`, 18 `epr-sink-capabilities`, 30 `vendor-defined-extended` |

The other types are reserved and reported by number only.

### Power data objects

`pdo` reads each object of a Source_Capabilities or Sink_Capabilities message by its type in bits 31–30, with its fields in physical units: the field times its unit, exactly.

| `supply`         | Fields                                                                                                                                                                                                                                       |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fixed` (00)     | `millivolts` (bits 19–10, 50 mV units); `milliamps` (bits 9–0, 10 mA units: the maximum current of a source, the operational current of a sink); `flags`, bits 29–20 as they are, bit 29 in bit 9.                                           |
| `battery` (01)   | `maxMillivolts` (bits 29–20, 50 mV); `minMillivolts` (bits 19–10, 50 mV); `milliwatts` (bits 9–0, 250 mW: the maximum allowable power of a source, the operational power of a sink).                                                         |
| `variable` (10)  | `maxMillivolts` (bits 29–20, 50 mV); `minMillivolts` (bits 19–10, 50 mV); `milliamps` (bits 9–0, 10 mA).                                                                                                                                     |
| `augmented` (11) | `apdoType` (bits 29–28). For type 0, a programmable power supply (PPS): `maxMillivolts` (bits 24–17, 100 mV), `minMillivolts` (bits 15–8, 100 mV), `milliamps` (bits 6–0, 50 mA). The other APDO types leave them `null`: read `object.raw`. |

The flags of a fixed supply are reported as they are because their meanings differ between source and sink objects and between revisions of the specification. For reading them — `flags` bit 9 is bit 29 of the object, bit 0 is bit 20 — revision 3.1 gives a source's: 29 Dual-Role Power, 28 USB Suspend Supported, 27 Unconstrained Power, 26 USB Communications Capable, 25 Dual-Role Data, 24 Unchunked Extended Messages Supported, 23 EPR Mode Capable, 21–20 Peak Current; and a sink's: 29 Dual-Role Power, 28 Higher Capability, 27 Unconstrained Power, 26 USB Communications Capable, 25 Dual-Role Data, 24–23 Fast Role Swap required USB Type-C Current.

### Request data objects

`rdo` reads the first object of a Request as a request for a fixed supply, a variable supply or a battery lays it out:

| Field                      | Bits  | Meaning                                                                                                                 |
| -------------------------- | ----- | ----------------------------------------------------------------------------------------------------------------------- |
| `position`                 | 31–28 | Object Position: the power data object requested, from 1.                                                               |
| `giveBack`                 | 27    | GiveBack flag.                                                                                                          |
| `capabilityMismatch`       | 26    | Capability Mismatch.                                                                                                    |
| `usbCommunicationsCapable` | 25    | USB Communications Capable.                                                                                             |
| `noUsbSuspend`             | 24    | No USB Suspend.                                                                                                         |
| `unchunkedExtended`        | 23    | Unchunked Extended Messages Supported.                                                                                  |
| `operating`                | 19–10 | The operating current in 10 mA units for a fixed or variable supply, the operating power in 250 mW units for a battery. |
| `maximum`                  | 9–0   | The maximum operating current or power in the same units, or the minimum when `giveBack` is set.                        |

Which units apply depends on the object requested, which the Request does not say, so `operating` and `maximum` are the fields' numbers. A request for a programmable power supply lays out bits 22–0 differently (an output voltage and an operating current): read it from `object.raw`. The objects of every other message — BIST, Alert, Battery_Status, Vendor_Defined, EPR_Request and the rest — are reported raw only.

### Extended messages

An extended message carries an extended header, then data bytes, all reported as the packet carries them:

- A chunked message carries `4 × objects − 2` bytes after its extended header, its data and then the padding up to its last object, all reported as `data`. A chunk request carries only padding.
- An unchunked message (Chunked clear) is read up to its EOP: its last four bytes are the CRC, the bytes before them its data. Each byte is reported once four more have followed it; its `objects` and `dataSize` are not relied on.

Reassembling the chunks of a message — `chunkNumber`, `dataSize` and the 26 bytes a chunk carries at most — is the next layer's business.

### Provenance

A bit spans from the transition that begins it to the one that ends it, the next bit's first, and is settled at that transition, where it is known to be whole. A field spans the bits of its symbols.

| Event                                        | Span                                                                                                                                            | Settled                                                          |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `begin`                                      | the preamble: its first transition through the first transition of the ordered set; when the packet ends inside it, as far as it was read       | the span's end                                                   |
| `sop`, `hard-reset`, `cable-reset`           | the four K-codes                                                                                                                                | the last one's end                                               |
| `header`, `extended-header`, `object`, `crc` | the field's symbols                                                                                                                             | the last symbol's end                                            |
| `pdo`, `rdo`                                 | as their object                                                                                                                                 | as their object                                                  |
| `data` (chunked)                             | the batch's bytes                                                                                                                               | the last byte's end                                              |
| `data` (unchunked)                           | the batch's bytes                                                                                                                               | the end of the fourth byte after the batch's last, or of the EOP |
| `data` (flushed by an error or a cut)        | the batch's bytes                                                                                                                               | where the packet ended                                           |
| `crc` (unchunked)                            | the four bytes before the EOP                                                                                                                   | the EOP's end                                                    |
| `end` (`eop`)                                | the EOP                                                                                                                                         | its end, the final transition                                    |
| `end` (`hard-reset`, `cable-reset`)          | empty, at the final transition                                                                                                                  | that tick                                                        |
| `error`, `end` (`error`)                     | the symbol at fault; for `timing`, the bit under way from its first transition to the tick the timing failed at; for the bit rate, the preamble | the span's end                                                   |
| `end` (`aborted`)                            | empty, where the samples stopped                                                                                                                | that tick                                                        |

## Decoding rules

- **BMC.** Every bit begins with a transition; a 1 has a second one in its middle, a 0 none. A transition before three quarters of the bit period after a bit's first is its middle, one at or after it the next bit's first.
- **Idle.** The line is idle once it has held its level for longer than any bit the decoder accepts: ⌈rate × (100 + tolerance) / (bit rate × (100 − tolerance))⌉ + 2 ticks, about 1.2 bit periods at the default tolerance (51 ticks at 12 MHz). After the first sample, after a gap and after every packet the decoder waits for the line to be idle; the next transition may begin a preamble. A packet already under way at the first sample or after a gap is therefore skipped, and so are the edges a transmitter makes after a packet's final transition, such as releasing the line after holding it low.
- **The preamble.** It starts with a 0 and a 1: a first transition whose bit 0 is shorter than ⌊rate × (100 − tolerance) / (bit rate × (100 + tolerance))⌋ − 1 ticks or lasts until the line is idle, or whose bit 1 is no 1, starts no packet and is not reported. Once these two bits are read the packet is open and reported, whatever comes next. Bit 1 is timed by bit 0, every later bit by the period the bits before it show — the time from the first transition to the start of the bit, divided by the bits since. The 64 bits of the preamble give the period for the rest of the packet; the bit rate they show must lie within the tolerance of `bit_rate`, allowing one tick of sampling, or the packet ends `timing`. A preamble outside the tolerance but within about twice it is therefore reported, with a `timing` error; one further off is not recognised.
- **Resynchronisation.** Each bit is timed from the transition that begins it: its middle transition must come half a period later, the next bit's first a period later, each within the tolerance of the period, plus a tick of sampling and the period's own uncertainty, in integer arithmetic. No bit lasts as long as the line takes to be idle. A transition elsewhere, or none in time, ends the packet `timing`.
- **Symbols.** After the preamble come five-bit symbols, sent least significant bit first: the specification's table prints each with its first bit on the right, so Sync-1, `11000`, is sent 0, 0, 0, 1, 1.
- **Ordered sets.** The first four symbols must be an ordered set with all four K-codes in place, or three of them, the other corrupted, as receivers are to accept (§5.4). The sets differ in two K-codes at least, so one corrupted K-code can leave a sequence one away from two sets, such as Sync-1 Sync-1 Sync-1 Sync-3 between SOP and SOP': it ends the packet `ordered-set`, as do symbols that make no set.
- **Messages.** After the ordered set of a message, the data symbols carry nibbles, the low nibble of each byte first and the bytes of each field least significant first: the 16-bit header; then as many 32-bit data objects as it says, or, in an extended message, the extended header and the data; then the 32-bit CRC; then the EOP, whose last bit the final transition ends.
- **CRC.** The CRC-32 (polynomial 0x04C11DB7, preset to all ones, inverted, sent least significant bit first) is checked by running the register over the header, the data and the CRC field in the order they are sent, which leaves the residual 0xC704DD7B when nothing was damaged. A damaged CRC does not end the packet: `crc` says so and the EOP still ends it.
- **Gaps and the end of the samples.** A packet under way ends `aborted` after the data bytes already known; the bits of a field not read to its end are dropped, and so are the last four bytes of an unchunked message, which might be its CRC. The gap or the end is passed on.
- **Progress** is passed on, held back to the start of whatever is under way — the preamble, the field being read, or data not yet reported — so that no event follows that begins before it.
- A source slower than six samples per bit is refused when it is connected. Should a decoder start on one all the same, it reports no packets and only passes gaps, progress and the end on.
- Events do not depend on how the samples are cut into blocks, on how often the reader pauses, or on queue sizes and budgets.

## Reading the stream

Decoders built on USB PD read the stream through `usb_pd::UsbPdBus`. `next()` waits for the next packet, gap, progress or end; a packet comes as a `Packet` cursor whose reads each consume one field and can wait across blocks:

| Read             | Returns                                                                                                                                                             |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `field()`        | the next `Field`: `Sop`, `HardReset`, `CableReset`, `Header`, `ExtendedHeader`, `Object`, `Pdo`, `Rdo`, `Data` or `Crc`, as the events of the same names carry them |
| `field_or_end()` | the next field, or `Read::End` at the packet's normal end: its EOP, or the end of reset signalling                                                                  |
| `drain()`        | consumes the rest of the packet up to its normal end                                                                                                                |

Only `field_or_end` returns the normal end; everywhere else, running out of fields is a `ReadError`: `Incomplete` where the packet ended normally, `Damaged` with the reason of its `error` event, `Truncated` for a packet cut off, `Gap` or `EndOfInput` when the samples stopped. `consumed()` gives the provenance of what was read since it was last called, and `UsbPdBus::finish()` drains a packet its reader left open.

## Limits

- One CC line. No glitch filter: a comparator that rings at the edges, making several transitions in a few samples, shows as `timing` errors.
- A preamble is counted from its first transition after the line was idle: one whose first transition was not captured cannot be aligned.
- The stream carries the bus's numbers: what messages mean together — GoodCRC answering a message, retries by MessageID, the contract negotiated — and what the objects of other messages and extended data hold is the next layer's business.
- Tested: transmitters 10 % slow and 10 % fast at 40, 8 and 6 samples per bit, with the line idling either high or low; a bit 8 % long decodes, one 30 % long or short does not.

### Uncertain points

The decoder was written from the specification's public structure; these points rest on less than a verbatim reading of it:

- The 3-of-4 rule for ordered sets does not say what a sequence one K-code away from two sets is; the decoder refuses it.
- Bits 5 and 8 of the header in `SOP'_Debug` and `SOP''_Debug` messages are read as in SOP' and SOP'' messages.
- Object Position is read from bits 31–28, the width revision 3.1 gives it for EPR; earlier revisions reserve bit 31, which is then 0.
- How Number of Data Objects and padding work in unchunked extended messages: the decoder relies on neither and reads such messages to their EOP.
- The meanings of the fixed supply flags differ by revision; they are reported as they are.
