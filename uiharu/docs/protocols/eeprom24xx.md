# 24xx EEPROMs

The `eeprom24xx` decoder (`uiharu_protocols::eeprom24xx::Eeprom24xx`) reads [`i2c.events`](i2c.md) and emits `eeprom24xx.events` version 1: the commands a 24xx-series I²C EEPROM (24AA, 24LC, 24FC, AT24C and compatible parts) was sent — writes, reads and acknowledge polls — as they stream. It is implemented from the Microchip 24AA256/24LC256/24FC256 and AT24C32 datasheets. `eeprom24xx::synth` lays out the I²C traffic of each command for the I²C waveform synthesizer.

## Configuration

| Field            | Meaning                                                                         | Default |
| ---------------- | ------------------------------------------------------------------------------- | ------- |
| `address_bytes`  | Bytes in a word address: `1` for parts up to 16 Kbit, `2` for 32 Kbit and above | `2`     |
| `device_address` | The device's 7-bit address; none follows any of `0x50`–`0x57` (`1010 xxx`)      | none    |

`device_address` is the 7-bit address: the 8-bit form with the R/W̄ bit (`0xa0`) is refused.

## Requirements and props

The input requires of the I²C stream `bus = "i2c"` and `framing = "start-stop"`, which the `i2c` decoder declares. A stream of another schema, such as the SPI decoder's `spi.events`, is refused when it is connected (`GraphError::SchemaMismatch`); an `i2c.events` stream that does not declare both is refused with `GraphError::Requirement`, naming the property.

The output declares `addressBytes`: `1` or `2`.

## Events

| Kind       | Fields                          | Meaning                                                      |
| ---------- | ------------------------------- | ------------------------------------------------------------ |
| `begin`    | `op`, `tx`, `device`, `command` | A command was told                                           |
| `address`  | `op`, `wordAddress`             | The word address the command gave                            |
| `data`     | `op`, `offset`, `bytes`         | Bytes written or read, in order, at most 32 per event        |
| `end`      | `op`, `reason`, `length`        | The operation ended                                          |
| `ack-poll` | `tx`, `device`, `acked`         | A control byte with nothing after it: the controller polling |
| `unknown`  | `tx`, `device`                  | Traffic to the device that fits no command                   |

- `op` numbers the operations from 1; `tx` is the I²C transaction the event rides on; `device` is the 7-bit address in the control byte.
- `command` is one of:

  | Command                | The controller sent                                                                      |
  | ---------------------- | ---------------------------------------------------------------------------------------- |
  | `set-address`          | a write of the word address alone, which sets where the next current-address read starts |
  | `byte-write`           | a write of one data byte                                                                 |
  | `page-write`           | a write of several data bytes                                                            |
  | `current-address-read` | a read from where the device's address counter stands                                    |
  | `random-read`          | a set-address, a repeated START and a read from the address it set                       |
  | `write`                | a write cut off by a gap or the end of the samples before it could be told which         |

- `wordAddress` is the address as sent, one or two bytes, most significant first.
- `offset` is the position of the event's first byte, counted from the operation's first data byte; it is not a memory address.
- `reason` is `complete` when the command ran to its end; `incomplete` when the controller ended it before it was whole — a read of no byte, or a byte or page write ended by a repeated START, which the device does not program; `aborted` when a gap or the end of the samples cut it off.
- `length` counts the data bytes of the operation; a sequential read is a read whose `length` is more than 1.
- `acked` is whether the device acknowledged the control byte: a device busy with a write cycle, or absent, does not.

### Provenance

| Event                            | Span                                                                                               | Settled                                                                     |
| -------------------------------- | -------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `begin`                          | the START or repeated START and the control byte; for a random read, through its read control byte | where the command was told (below)                                          |
| `address`                        | the word address bytes                                                                             | its `begin`'s tick                                                          |
| `data` (full batch)              | the bytes of the batch                                                                             | the last byte's acknowledge edge                                            |
| `data` (flushed by an end)       | the bytes of the batch                                                                             | the end that decided it                                                     |
| `end` (`complete`, `incomplete`) | the STOP or repeated START that ended the command                                                  | that tick, or where it was told for a set-address ended by a repeated START |
| `end` (`aborted`)                | the bytes read but not reported, if any; else empty, at the gap's start or the end of the samples  | the gap's start or the end of the samples                                   |
| `ack-poll` (`acked: false`)      | the control byte                                                                                   | its acknowledge edge                                                        |
| `ack-poll` (`acked: true`)       | the control byte through the STOP or repeated START after it                                       | that tick                                                                   |
| `unknown`                        | the control byte through the STOP or repeated START that cut the word address short                | that tick                                                                   |

A command is told as soon as it is certain:

| Command                                 | Told at                                                                                      |
| --------------------------------------- | -------------------------------------------------------------------------------------------- |
| `current-address-read`                  | its control byte                                                                             |
| `random-read`                           | the read control byte after the set-address and the repeated START                           |
| `page-write`                            | its second data byte                                                                         |
| `byte-write`, `set-address`             | the STOP or repeated START that ends the write                                               |
| `set-address` ended by a repeated START | the address after the repeated START when it is not a read from the same device, or the STOP |
| `write`                                 | the gap or the end of the samples                                                            |

The word address, and the data byte held while a write's command was not known, go out right after the `begin`, settled with it. A byte write of `0xaa` to word address `0x0123`, as NDJSON lines (see `design.md` §8) on a stream named `eeprom`, sampled at eight samples per SCL period — all four lines are settled at the STOP, which tells the command:

```json
{"stream":"eeprom","seq":0,"span":[10,81],"settled":306,"event":{"kind":"begin","op":1,"tx":1,"device":80,"command":"byte-write"}}
{"stream":"eeprom","seq":1,"span":[88,225],"settled":306,"event":{"kind":"address","op":1,"wordAddress":291}}
{"stream":"eeprom","seq":2,"span":[232,297],"settled":306,"event":{"kind":"data","op":1,"offset":0,"bytes":[170]}}
{"stream":"eeprom","seq":3,"span":[306,307],"settled":306,"event":{"kind":"end","op":1,"reason":"complete","length":1}}
```

## Decoding rules

- The control byte is the device's 7-bit address and R/W̄. The decoder follows the configured device, or any of `0x50`–`0x57`. A segment of a transaction — an address and what follows it up to the next repeated START or the STOP — addressed to another device or with a 10-bit address is skipped, and so is a partial I²C transaction, whose bytes cannot be aligned.
- A write (R/W̄ = 0) carries the word address, then data: no data byte makes a set-address, one a byte write, more a page write. A STOP ends a write complete. A repeated START ends a set-address complete as well — it may be the first half of a random read — but a byte or page write incomplete: the device programs nothing that a STOP did not end.
- A read (R/W̄ = 1) is the second half of a random read when a set-address to the same device and a repeated START came right before it, and a current-address read otherwise. It ends at the STOP or repeated START after it: complete if it read a byte, incomplete if not.
- A control byte that is not acknowledged is a poll that found the device busy, `ack-poll { acked: false }`; the device takes no part in the rest of the segment. A write control byte that is acknowledged and followed at once by a STOP or repeated START is the poll that found it ready, `ack-poll { acked: true }`. One followed by a word address begins a command and is reported as that command, not as a poll.
- A write whose word address a STOP or repeated START cuts short fits no command: `unknown`.
- A gap or the end of the samples while a command is under way ends it `aborted`. A write cut off before its command could be told is reported as `write`, with the word address and data seen; a set-address that a repeated START had already ended stays a complete `set-address`.
- Data goes out in batches of at most 32 bytes (`DATA_BATCH`), each as soon as it is full, so a long read or page write shows up while it is still under way.

## Limits

- Word addresses are reported as sent: the wrap-around within a page, the roll-over of the address counter and the block-select bits that small parts take from the control byte are not modelled; the control byte's address is in each `begin`.
- Acknowledge bits after the control byte are not interpreted: data and word address bytes are reported whether the device acknowledged them or not.
- One word address width per decoder: devices with different widths on one bus need a decoder each, with `device_address` set.
