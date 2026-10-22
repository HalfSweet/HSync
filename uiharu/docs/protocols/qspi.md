# QSPI SDR transport

The `qspi` node reads one, two or four parallel data lanes on an SPI clock.
It is a **fixed-lane transport view**, with no flash opcode interpretation or
automatic command/address/dummy/data phase switching. Use a capture region whose
lane width and word alignment are known; decoding an entire mixed-width flash
transaction with one configuration does not reconstruct its phases correctly.

Implemented independently from [Winbond W25Q128JV revision F, March 27, 2018](https://www.winbond.com/resource-files/w25q128jv%20revf%2003272018%20plus.pdf),
§§4.2 and 6.1–6.3 and figures 18 (dual output) and 20 (quad output).
Those diagrams place IO0 at the low bit of each lane group; the quad byte's
upper nibble precedes its lower nibble. Modes 1/2, active-high CS, configurable
word widths and low-group-first order are explicit generic transport options,
not claims about supported operating modes of that flash device.

## Configuration

| Field           | Default     | Meaning                                                      |
| --------------- | ----------- | ------------------------------------------------------------ |
| `cs`            | `0`         | CS channel; `null` means an unframed stream                  |
| `clk`           | `1`         | Clock channel                                                |
| `data`          | `[2,3,4,5]` | Channels in IO0-first order; exactly 1, 2 or 4 lanes         |
| `mode`          | `0`         | SPI mode 0–3; leading edge sampled in 0/2, trailing in 1/3   |
| `word_bits`     | `8`         | 1–32 bits, divisible by lane count                           |
| `cs_active_low` | `true`      | CS assertion polarity                                        |
| `lsb_first`     | `false`     | First lane group fills low bits if true, high bits otherwise |

All channels must be distinct and in 0–63. Every clock phase needs a sample;
there is no analog timing validation or clock frequency estimate. The decoder
samples the stable data on the configured receive edge, rather than the flash's
output transition edge.

## Contract

Schema `qspi.events`, version 1. `QspiBus` is the generic framed cursor;
its transaction items are `Word { value }` and it supports normal end, drain,
and consumed provenance through the runtime's `FramedBus` API.

| Event   | Fields                        | Meaning                                                   |
| ------- | ----------------------------- | --------------------------------------------------------- |
| `begin` | `tx`, `partial`               | CS asserted; partial if assertion was not observed        |
| `word`  | `tx`, `value`                 | A complete configured-width unsigned word                 |
| `end`   | `tx`, `reason`, `droppedBits` | CS release or sample loss; incomplete word bits discarded |

Transaction ids start at 1. Without CS there are only words, transaction 0,
and alignment restarts at the first observed sample edge after every gap.
Framed captures beginning or resuming while selected have `partial: true`;
word alignment is unknown and reported words start at the first observed clock
edge. Consumers needing confirmed alignment must skip partial frames.

A CS transition suppresses a simultaneous clock edge. A word spans its first
sampled lane group through the last sampled group's tick plus one, settled at
that last tick. Begin and normal release cover their deciding tick. A gap or
end aborts an open frame with an empty end span at that boundary before forwarding
the gap/end. Incomplete words never emit data. Progress is held at the first
sampled group of an incomplete word. At most one 32-bit word is retained, so data
is emitted incrementally under bounded output backpressure. Runtime cancellation
drops the suspended decoder without flushing invented trailing events.

Output properties are `cs`, `lanes`, `wordBits`, `mode`, `bitOrder` (`msb-first`
or `lsb-first`) and `sdr: true`. The input requires enough source channels for
all configured lines. There is no direction inference, tri-state detection,
DDR, octal SPI, flash command decoding, automatic lane switching or dummy-cycle
suppression. Dummy clocks and command phases appear as raw configured-width data
if they are included in the input.
