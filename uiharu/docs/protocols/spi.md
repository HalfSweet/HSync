# SPI

Single-line SPI decoded from logic samples: words shifted on the sampling edge of the clock while chip select is asserted, each chip select period a transaction. Node type `spi`; module `crates/uiharu-protocols/src/spi/`.

## Contract `spi.events` v1

| Kind    | Fields                                       | Meaning                                                                                                                                                           |
| ------- | -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `begin` | `tx`, `partial: bool`                        | Chip select was asserted. `partial` when it was found asserted at the first sample or after a gap: earlier words were not seen and the word alignment is unknown. |
| `word`  | `tx`, `mosi: u8 \| null`, `miso: u8 \| null` | One word on both lines; a line that is not connected reads `null`.                                                                                                |
| `end`   | `tx`, `reason: "cs-release" \| "aborted"`    | Chip select was released, or the samples ended or were lost while it was asserted.                                                                                |

`tx` numbers transactions from 1; a bus without chip select is one unframed transaction `0` with words only, no `begin` or `end`.

Provenance: `begin` and `end` at the tick chip select changed (an aborted end at the gap's start or the end of the samples, with an empty span); a `word` spans its first bit's sampling edge to its last bit's sampling edge + 1, settled at the last.

Props: `wordBits` (1–8), `bitOrder` (`msb-first` / `lsb-first`), `mode` (0–3), `cs`, `mosi`, `miso` (whether each line is connected). Consumers such as SPI NOR require `wordBits = 8`, `bitOrder = "msb-first"`, `cs = true`, `mosi = true`, `miso = true`.

## Configuration

| Field           | Default     | Meaning                                              |
| --------------- | ----------- | ---------------------------------------------------- |
| `clk`           | 0           | Clock channel                                        |
| `cs`            | 1           | Chip select channel, or `null` for a bus without one |
| `mosi`          | 2           | Master-out channel, or `null` when not connected     |
| `miso`          | 3           | Master-in channel, or `null` when not connected      |
| `mode`          | 0           | SPI mode: bit 1 is CPOL, bit 0 is CPHA               |
| `word_bits`     | 8           | Bits per word, 1 to 8                                |
| `bit_order`     | `msb-first` | Which bit is shifted first                           |
| `cs_active_low` | true        | Whether chip select is asserted low                  |

At least one data line must be connected; the source must have more channels than the highest one used.

## Rules

- A bit is taken on the sampling edge of the clock: rising when CPOL equals CPHA (modes 0 and 3), falling otherwise (modes 1 and 2).
- Bits are taken only while chip select is asserted; after `word_bits` bits a `word` goes out.
- A clock edge at the very tick chip select changes belongs to neither transaction.
- Bits left over when chip select is released, the samples end or a gap starts do not make a word and are dropped.
- Chip select found asserted at the first sample, or at the first sample after a gap, begins a `partial` transaction; a gap or the end of the samples while chip select is asserted ends the transaction `aborted`.
- The clock's idle level is not checked: the mode only picks the sampling edge.

## Reading the stream

Decoders built on SPI read it through `spi::SpiBus` and `spi::Transaction`: `next()` waits for the next transaction, gap, progress or end; a transaction offers `word()` (the next word with both lines; the transaction's end is an error here), `word_or_end()` (the only read that returns the normal end), `drain()` and `consumed()` for the provenance of what was read. Byte order, field widths and which line a protocol reads are the consuming decoder's own code.

## Limits

Single-line SPI only: dual and quad I/O are not decoded. Words have at most 8 bits.
