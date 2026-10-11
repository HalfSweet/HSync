# Microwire and 93Cxx EEPROMs

Two stacked decoders:

- the **Microwire bus** (`Microwire`, node type `microwire`, module `crates/uiharu-protocols/src/microwire/`) reads chip select, the clock (SK), DI and DO and reports every clock pulse as a bit on `microwire.events`;
- the **93Cxx EEPROM** decoder (`Eeprom93xx`, node type `eeprom93xx`, module `crates/uiharu-protocols/src/eeprom93xx/`) reads `microwire.events` and reports the instructions of the 93C46, 93C56, 93C66 and 93C86 families on `eeprom93xx.events`.

Both are implemented from the public datasheets of those EEPROMs: Microchip's 93AA46A/B/C, 93LC46A/B/C and 93C46A/B/C (DS21749), Atmel's AT93C46 and ST's M93C46, whose interface and instruction sets agree.

## Microwire bus: `microwire.events` v1

Chip select is active high; the device reads DI on the rising edge of SK, and drives DO, which changes a short delay (TPD) after the rising edge and holds until the next one.

### Configuration

| Field           | Default | Meaning                                                    |
| --------------- | ------- | ---------------------------------------------------------- |
| `cs`            | 0       | The chip select channel, or `null` for a bus without one   |
| `sk`            | 1       | The clock channel                                          |
| `di`            | 2       | The channel of DI, the data the controller sends           |
| `do`            | 3       | The channel of DO, the data the device sends, or `null`    |
| `cs_active_low` | false   | Whether chip select is asserted low; the 93Cxx assert high |

The input port `logic` requires of the source `channels >=` the highest channel used + 1; the decoder follows edges, so the sample rate does not enter. The output port `events` declares `cs`, `di` and `do`: whether each line is connected (`di` always is).

### Events

| Kind    | Fields                                                                  | Meaning                                                                                                                                                        |
| ------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `begin` | `tx`, `partial: bool`                                                   | Chip select was asserted. `partial` when it was found asserted at the first sample or after a gap: earlier bits were not seen                                  |
| `bit`   | `tx`, `di: bool`, `do: bool \| null`                                    | A clock pulse: DI at its rising edge, DO at the falling edge after it; `do` is `null` without a DO line                                                        |
| `end`   | `tx`, `bits: u32`, `reason`: `release` or `aborted`, `do: bool \| null` | Chip select was released, or the samples ended or were lost while it was asserted; `bits` counts the transaction's bits, `do` is DO's level on its last sample |

`tx` numbers transactions from 1; a bus without chip select is one unframed transaction `0` with bits only, no `begin` or `end`. The `end`'s `do` is where a device that shows its state on DO — a 93Cxx busy with a write, for one — can be read when the controller polls it without clocking.

```json
{"kind":"begin","tx":1,"partial":false}
{"kind":"bit","tx":1,"di":true,"do":null}
{"kind":"end","tx":1,"bits":25,"reason":"release","do":true}
```

| Event                      | Span                                                     | Settled          |
| -------------------------- | -------------------------------------------------------- | ---------------- |
| `begin`                    | the tick chip select asserted (or the first sample seen) | that tick        |
| `bit`                      | the rising edge through the falling edge DO was read on  | the falling edge |
| `bit` (without DO)         | the rising edge                                          | the rising edge  |
| `bit` (cut by the release) | the rising edge up to the release                        | the release      |
| `end` (`release`)          | the tick chip select released                            | that tick        |
| `end` (`aborted`)          | empty, at the gap's start or the end of the samples      | that tick        |

Progress never passes the rising edge of a bit whose falling edge is still to come.

### Rules

- DO is read on the falling edge after each rising edge, where the device's output has settled, rather than at the next rising edge, where a fast device may already have moved on. When the clock's high phase is shorter than the device's output delay, DO changes after the falling edge and is read one bit late.
- Chip select released before a bit's falling edge ends the bit at the release, DO read on the last sample before it.
- A clock edge at the very tick chip select changes belongs to neither transaction.
- SK found high at the first sample, or after a gap, is a bit whose rising edge was not seen and is not reported.
- A gap or the end of the samples while chip select is asserted ends the transaction `aborted`; a bit whose falling edge they cut off is dropped. The gap or the end is passed on after it.

### Reading the stream

Decoders built on Microwire read it through `MicrowireBus` and `Transaction`, shaped like the SPI cursor: `next()` waits for the next transaction, gap, progress or end; a transaction offers `bit()` (the next bit with both lines; the transaction's end is an error here), `bit_or_end()` (the only read that returns the normal end, with DO's level there), `drain()` and `consumed()` for the provenance of what was read. A read that cannot return a bit says why: `Incomplete` (chip select released), `Truncated` (an aborted end), `Gap`, `EndOfInput`, or `NoDataOut` for a DO level asked of a bus without DO. Start bits, opcodes, address and word widths are the consuming decoder's own code.

## 93Cxx EEPROM: `eeprom93xx.events` v1

### Configuration

| Field          | Default | Meaning                                                                                                                                               |
| -------------- | ------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `address_bits` | 6       | Address bits of an instruction, 2 to 16: 6 for a 93C46 of 16-bit words and 7 of 8-bit words, 8 and 9 for the 93C56 and 93C66, 10 and 11 for the 93C86 |
| `organisation` | `x16`   | `x16` or `x8`: the width of the words, which the ORG pin or the part number sets                                                                      |

### Requirements and props

The input port `microwire` requires `cs = true` and `di = true`: transactions framed by chip select, with DI. A Microwire bus without chip select is refused when it is connected (`GraphError::Requirement`, naming `cs`), and a stream of another schema, such as `spi.events`, with `GraphError::SchemaMismatch`. DO is optional: without it reads report no words.

The output port `events` declares `addressBits` and `dataBits` (16 or 8).

### Events

| Kind          | Fields                                                           | Meaning                                                                             |
| ------------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `instruction` | `op`, `tx`, `command`, `address: u16 \| null`                    | An instruction: its start bit, opcode and address                                   |
| `data`        | `op`, `direction`: `in` or `out`, `offset: u32`, `values: [u16]` | Words written (on DI) or read (on DO), in order, at most 32 per event               |
| `status`      | `op`, `tx`, `ready: bool`                                        | DO showed the device busy (low) with the programming cycle of `op`, or ready (high) |
| `end`         | `op`, `reason`: `complete`, `incomplete` or `aborted`            | The operation ended                                                                 |

- `op` numbers the operations, one per instruction, from 1; `tx` is the Microwire transaction the event rides on.
- `command` is one of:

  | Command | Opcode | Address bits         | Then                      |
  | ------- | ------ | -------------------- | ------------------------- |
  | `read`  | `10`   | the address          | a dummy 0 and words on DO |
  | `write` | `01`   | the address          | a word on DI              |
  | `erase` | `11`   | the address          | —                         |
  | `ewen`  | `00`   | `11` and don't-cares | —                         |
  | `ewds`  | `00`   | `00` and don't-cares | —                         |
  | `eral`  | `00`   | `10` and don't-cares | —                         |
  | `wral`  | `00`   | `01` and don't-cares | a word on DI              |

- `address` is set for `read`, `write` and `erase`; it is `null` for the others, and for an instruction whose address chip select cut short.
- `offset` is the position of the event's first word, counted from the operation's first; it is not a memory address. Words are most significant bit first on the bus.
- `reason` is `complete` when the instruction ran to its end — for a read, when chip select was released after one or more whole words; `incomplete` when chip select was released before the instruction was whole, which the device does not execute, or a read before its first word or inside a word; `aborted` when a gap or the end of the samples cut it off.

```json
{"kind":"instruction","op":1,"tx":1,"command":"read","address":42}
{"kind":"data","op":1,"direction":"out","offset":0,"values":[43690,21845,4660]}
{"kind":"end","op":1,"reason":"complete"}
{"kind":"status","op":2,"tx":4,"ready":true}
```

| Event                             | Span                                                                                    | Settled                                   |
| --------------------------------- | --------------------------------------------------------------------------------------- | ----------------------------------------- |
| `instruction`                     | the transaction from chip select through the last address bit                           | that bit                                  |
| `instruction` (address cut short) | the transaction from chip select through what cut it short                              | the release, the gap's start or the end   |
| `data` (full batch, or a write)   | the bits of the words                                                                   | the last word's last bit                  |
| `data` (flushed by an end)        | the bits of the words                                                                   | the end that decided it                   |
| `status`                          | the bits or the transaction it was read from                                            | the bit's falling edge, or the release    |
| `end` (`complete`, `incomplete`)  | the bits read but not reported, through the release                                     | the release                               |
| `end` (`aborted`)                 | the bits read but not reported, else empty at the gap's start or the end of the samples | the gap's start or the end of the samples |

### Rules

- Every Microwire transaction carries at most one instruction. Clock pulses with DI low before the start bit, a 1 on DI, are no instruction and are skipped, as the device skips them; a transaction without a start bit carries none.
- After the start bit come two opcode bits and `address_bits` address bits, most significant first. The instruction goes out once its address is whole. One whose address chip select cut short is reported without it, with an `incomplete` end; one cut before it was known — in its opcode, or in the two address bits that select an instruction of opcode `00` — is not reported at all.
- READ: the device drives a dummy 0 on DO with the address's last bit, then a word on DO, and the next word after it for as long as chip select stays asserted: a sequential read. Words go out in batches of at most 32, each as soon as it is full, the last when the read ends, so that a long read shows up while it is under way. The dummy bit is not checked.
- WRITE and WRAL take one word on DI, which goes out once whole; ERASE, ERAL, EWEN and EWDS take nothing more. Clock pulses after what the instruction needs are skipped, and the operation ends `complete` when chip select is released, its end covering them.
- After a complete WRITE, ERASE, ERAL or WRAL the device runs a programming cycle and shows its state on DO whenever chip select is asserted again before a start bit: low while busy, high once ready. The decoder reads that state from the clock pulses before a start bit and from the `end`'s DO level of a transaction without one, and reports `status` each time it differs from the last, until the device is ready or a start bit clears it. A poll that neither clocks nor lasts until DO has settled shows whatever DO was.
- A transaction already under way when the samples begin or after a gap is skipped: its first bits are not the instruction's.
- A gap or the end of the samples during an instruction ends it `aborted`, after the words read so far; a gap also forgets a programming cycle under way, whose end it may have hidden.

## Limits

- One Microwire bus per decoder, single DI and DO lines.
- Whether erasing and writing are enabled, the address counter of a sequential read and the array's size are not modelled: events carry bus data only.
- The parts are not told apart: the 93C parts, which start programming on the last data bit's clock rather than at chip select's release, decode like the 93AA and 93LC parts.
