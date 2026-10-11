# 1-Wire

Two stacked decoders in `crates/uiharu-protocols/src/onewire/`:

- the **link layer** (`OneWireLink`) reads the data line and reports each low pulse as a reset, a bit or a bad slot on `onewire.link`;
- the **network layer** (`OneWireNetwork`) reads `onewire.link` and reports transactions — the ROM command, the ROM code or search result, the bytes after them — on `onewire.events`.

Both are implemented from Maxim Integrated (Analog Devices) application note 126, _1-Wire Communication Through Software_, application note 937, _Book of iButton Standards_, and the DS2401, DS2431 and DS18B20 datasheets. Device decoders, such as the [DS18B20](ds18b20.md), read `onewire.events`.

## Link layer: `onewire.link` v1

### Configuration

| Field   | Meaning                                       | Default    |
| ------- | --------------------------------------------- | ---------- |
| `data`  | the channel of the data line (idle high)      | 0          |
| `speed` | `standard` or `overdrive`: which windows hold | `standard` |

The speed is the user's to say: the link layer cannot see the Overdrive Skip ROM or Overdrive Match ROM command that switches the slaves to overdrive, nor the standard-speed reset that switches them back. A capture that changes speed decodes correctly only where it runs at the configured speed.

The input requires `channels >= data + 1` and a sample rate (`rateHz`) of at least **1 MHz at standard speed** — one sample per microsecond, the shortest write-1 the standard allows — and **10 MHz at overdrive**, whose windows are a few microseconds wide. The output declares `speed` (`"standard"` or `"overdrive"`).

### Events

| Kind       | Fields           | Meaning                                                    | Span                                                              | Settled             |
| ---------- | ---------------- | ---------------------------------------------------------- | ----------------------------------------------------------------- | ------------------- |
| `reset`    | `presence: bool` | a reset pulse, and whether a slave answered it             | the fall through the end of the presence pulse or presence window | the end of the span |
| `bit`      | `value: bool`    | a time slot and the bit the line carried                   | the fall through the later of the release and the sampling point  | the end of the span |
| `bad-slot` | `lowTicks: u64`  | a low pulse that fits no window; how long it was, in ticks | the low pulse and its release                                     | the release         |

Read and write slots look alike on the line — a read-1 is a short pulse like a write-1, a read-0 a long one like a write-0 — so `bit` says what the line carried, not who drove it.

### Windows

A pulse is measured from the tick the line falls to the tick it rises.

| Pulse                       | Standard speed                     | Overdrive                        |
| --------------------------- | ---------------------------------- | -------------------------------- |
| 1                           | released by 15 µs (sampling point) | released by 2 µs                 |
| 0                           | over 15 µs, released by 120 µs     | over 2 µs, released by 16 µs     |
| reset                       | at least 480 µs                    | 48 to 80 µs, or at least 480 µs  |
| presence pulse: fall        | 15 to 60 µs after the release      | 2 to 6 µs after the release      |
| presence pulse: low         | at least 60 µs                     | at least 8 µs                    |
| presence pulse: released by | 300 µs after the master's release  | 30 µs after the master's release |

A bound of "at most _t_ µs" is ⌊_t_ · rate / 10⁶⌋ ticks, one of "at least _t_ µs" ⌈_t_ · rate / 10⁶⌉ ticks, in integer arithmetic.

### Rules

- A 1 must also find the line still high at its sampling point; when the next pulse falls before it, the slot is a `bad-slot`.
- A write-0 lasts 60 to 120 µs (6 to 16 µs), but in a read slot the slave holds a 0 from the master's fall until anywhere between the sampling point and the end of the slot: the window for a 0 therefore starts at the sampling point.
- At overdrive, a low of at least 480 µs is the standard-speed reset, which the slaves answer at standard speed; the presence window after it is the standard one.
- A presence pulse may last up to 240 µs (24 µs) for one slave; the pulses of several slaves merge on the line, hence the latest release of 60 + 240 µs (6 + 24 µs). The first fall in the presence window decides: when that pulse is no presence pulse (too early, too short or too long), the reset is reported without presence and the pulse as a `bad-slot` — or as a `reset`, if it is one — never as a bit. A pulse that falls after the window is a slot.
- Any other low is a `bad-slot`: longer than a slot without being a reset (at overdrive, 16 to 48 µs or 80 to 480 µs), or a pulse in a presence window that is no presence pulse.
- A low found at the first sample or after a gap began unseen and is dropped, unless what was seen of it already lasts 480 µs: it is then reported as a reset whose span starts at the first sample seen.
- A pulse, or a reset's presence window, cut off by a gap or the end of the samples is dropped: a reset whose presence could not be seen is not reported. The gap or the end is passed on.

## Network layer: `onewire.events` v1

The network layer has no configuration and requires nothing of `onewire.link` beyond its schema. Its output declares `wordBits = 8` and `bitOrder = "lsb-first"`.

### Events

Every event carries `tx`, the transaction, numbered from 1.

| Kind          | Fields                                                      | Meaning                                                                                                                                                                          | Span                            | Settled                     |
| ------------- | ----------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------- | --------------------------- |
| `begin`       | `presence: bool`                                            | a reset began a transaction                                                                                                                                                      | the reset                       | the reset                   |
| `rom-command` | `code: u8`                                                  | the transaction's first byte                                                                                                                                                     | its eight slots                 | the last slot               |
| `rom`         | `id: u64`, `family: u8`, `crcOk: bool`                      | the ROM code of Read ROM (sent by the slave) or of Match ROM and Overdrive Match ROM (sent by the master)                                                                        | its 64 slots                    | the last slot               |
| `search`      | `id: u64`, `crcOk: bool`, `conflicts: u8`, `unanswered: u8` | the 64 triplets of Search ROM or Alarm Search: the direction bits the master wrote, the triplets where devices disagreed (bit and complement 0) and where none answered (both 1) | its 192 slots                   | the last slot               |
| `byte`        | `value: u8`                                                 | a byte after the ROM level, raw                                                                                                                                                  | its eight slots                 | the last slot               |
| `bits`        | `value: u8`, `count: u8`                                    | the 1 to 7 bits after the last whole byte when the transaction ended, the first as bit 0                                                                                         | their slots                     | where the transaction ended |
| `end`         | `reason`: `reset`, `bad-slot` or `aborted`                  | the transaction ended: at the next reset, at a bad slot, or by a gap or the end of the samples                                                                                   | the slots read but not reported | where the transaction ended |

A ROM code is a `u64` whose bit 0 is the first bit on the line, written as a hexadecimal string in JSON (`"id": "0x5c000000a7b3d928"`) so that readers keep all 64 bits: the family code is its low byte, the 48-bit serial number the next six, the CRC its high byte. `crcOk` says whether the CRC is the CRC-8 (x⁸ + x⁵ + x⁴ + 1, least significant bit first, from zero) of the first seven bytes. An `end` without slots to cover is an empty span at the reset's fall, the gap's start or the end of the samples.

### Rules

- A transaction begins at a reset, answered or not, and ends at the next reset. Bits before the first reset — a transaction already under way at the first sample or after a gap — belong to no transaction and are skipped up to the next reset.
- Eight bits, least significant first, make a byte. The first byte is the ROM command: Read ROM (`0x33`), Match ROM (`0x55`) and Overdrive Match ROM (`0x69`) are followed by a ROM code; Search ROM (`0xF0`) and Alarm Search (`0xEC`) by 64 triplets; Skip ROM (`0xCC`), Overdrive Skip ROM (`0x3C`) and any other code by nothing the network layer knows.
- Every byte after the ROM level is a `byte`: the network layer does not know the device. The bits left when the transaction ends make a `bits` event, so that a single read slot (a device's status bit) is not lost; bits left in the ROM level make nothing, and the `end` covers them.
- A bad slot ends the transaction with `bad-slot`: the bits after it can no longer be told apart; those up to the next reset are skipped. A gap or the end of the samples ends it with `aborted`, then the gap or the end is passed on.

### Reading the stream

Decoders built on the network layer read it through `OneWireBus`: `next()` waits for a transaction and hands out a `Transaction` cursor with `rom_command()`, `rom()`, `byte()`, `byte_or_end()` (a byte, the trailing `bits`, or the normal end), `item_or_end()`, `drain()` and `consumed()` for provenance. Only `byte_or_end()` and `item_or_end()` return the normal end; elsewhere the end of a transaction is a `ReadError` naming how it ended. Decoders built on the link layer read it through `LinkBus`: `next()` waits for a reset, `bit()` reads the bits after it until a reset, a bad slot, a gap or the end stops them.

## Limits

- One speed per decoder; the switch to and from overdrive is not followed.
- Slot lengths and recovery times are not checked beyond the sampling point: a slot that follows the previous one too soon is classified by its low pulse alone.
- ROM commands other than the seven above (Resume, for instance) are not known: what follows them is reported as bytes.
