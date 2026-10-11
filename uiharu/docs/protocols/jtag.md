# JTAG

The `jtag` decoder (`uiharu_protocols::jtag::Jtag`) follows an IEEE 1149.1 test access port from logic samples: the state of the TAP controller, its resets, and the scans that shift bits through its instruction and data registers. It is written from IEEE Std 1149.1-2013, _Test Access Port and Boundary-Scan Architecture_: the TAP signals and the TAP controller's state diagram.

## Configuration

| Field           | Meaning                                                                                     | Default   |
| --------------- | ------------------------------------------------------------------------------------------- | --------- |
| `tck`           | Test clock channel                                                                          | 0         |
| `tms`           | Test mode select channel                                                                    | 1         |
| `tdi`           | Test data input channel, or none                                                            | 2         |
| `tdo`           | Test data output channel, or none                                                           | 3         |
| `trst`          | Test reset channel, active low, or none                                                     | none      |
| `initial_state` | The state assumed where the samples begin: `unknown`, `test-logic-reset` or `run-test-idle` | `unknown` |

At least one of TDI and TDO must be connected, and no channel may serve two lines. The input requires a source with `channels` of at least the highest channel used plus one.

## Output: `jtag.events`, version 1

The output declares which lines are connected, so that a stacked decoder can require the ones it reads:

| Property | Value                     |
| -------- | ------------------------- |
| `tdi`    | Whether TDI is connected  |
| `tdo`    | Whether TDO is connected  |
| `trst`   | Whether TRST is connected |

Each event is tagged with its `kind`. "The clock" below is the tick at which TCK rose.

| Kind         | Fields                                            | What happened on the port                                                                                                                                                                                                                                            | Span and settled tick                                                                                                                  |
| ------------ | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `state`      | `state`                                           | The controller is in `state` from this clock on. Only changes are reported, not clocks that stay in a state; an assumed state is reported at the sample where it is assumed.                                                                                         | The clock (or that sample)                                                                                                             |
| `reset`      | `by`: `tms` or `trst`                             | The controller was reset to Test-Logic-Reset: by TMS through the state diagram, by five clocks with TMS high from an unknown state, or by TRST being asserted (or found asserted where the samples begin).                                                           | The clock, or the sample where TRST was asserted                                                                                       |
| `scan-begin` | `scan`, `register`: `ir` or `dr`                  | The controller entered Shift-IR or Shift-DR with no scan under way.                                                                                                                                                                                                  | The clock that entered the shift state                                                                                                 |
| `shift`      | `scan`, `offset`, `tdi`, `tdo`, `bits`            | `bits` bits (1 to 256) were shifted, the first at position `offset` of the scan. `tdi` and `tdo` hold them packed least significant bit first in shifting order: bit _i_ of the batch is bit _i_ mod 8 of byte ⌊_i_ / 8⌋. A line that is not connected reads `null`. | From the batch's first clock to its last; settled at its last clock, or where TRST, a gap or the end of the samples cut the scan short |
| `scan-end`   | `scan`, `bits`, `reason`: `complete` or `aborted` | The scan ended after `bits` bits in all: `complete` when the controller entered Update-IR or Update-DR, `aborted` when TRST, a gap or the end of the samples cut it off.                                                                                             | The clock that entered Update, or the sample where TRST was asserted; empty at the gap's start or the end of the samples               |

Scans are numbered from 1. The sixteen states are named as the standard names them, in kebab-case: `test-logic-reset`, `run-test-idle`, `select-dr-scan`, `capture-dr`, `shift-dr`, `exit1-dr`, `pause-dr`, `exit2-dr`, `update-dr`, `select-ir-scan`, `capture-ir`, `shift-ir`, `exit1-ir`, `pause-ir`, `exit2-ir`, `update-ir`.

```json
{"kind":"scan-begin","scan":2,"register":"ir"}
{"kind":"shift","scan":2,"offset":0,"tdi":[6],"tdo":[1],"bits":4}
{"kind":"scan-end","scan":2,"bits":4,"reason":"complete"}
{"kind":"state","state":"update-ir"}
```

## Decoding rules

- Every line is read at the tick TCK rises: TMS and TDI are sampled on the rising edge, and so is TDO, which the device drives on the falling edge before it.
- Each rising edge moves the controller as the state diagram says. The state is unknown until the controller is seen to reset — five rising edges in a row with TMS high, or TRST asserted — unless `initial_state` assumes one; while it is unknown, no state and no scan is reported.
- A scan is a passage through Shift-IR or Shift-DR. It begins on the clock that enters the shift state, takes one bit from each line on every rising edge in that state — including the edge that leaves it for Exit1 with TMS high, which shifts the last bit before the exit — and ends `complete` on the clock that enters Update-IR or Update-DR. Pause-IR and Pause-DR halt shifting without ending the scan: a scan that pauses goes on, with offsets that continue, when the controller returns to the shift state through Exit2. A path from Capture through Exit1 to Update that never shifts is no scan.
- Bits go out in `shift` batches of at most 256 bits: when a batch is full, and on the clock that leaves the shift state, so that a long scan shows up while it is still under way.
- TRST, when connected, is active low and asynchronous: asserting it aborts the scan under way, resets the controller and holds it in Test-Logic-Reset; TCK is ignored while TRST is low, and a TCK edge at the very tick TRST changes does not clock the controller.
- A gap or the end of the samples aborts the scan under way; the bits shifted so far go out first. After a gap the state is unknown again, or assumed again: `initial_state` holds wherever the samples begin, at the first sample and after every gap, so an assumption that may be wrong after a gap is better left `unknown`.
- Events of one clock come in this order: the batch its bit completed, the end of the scan it completed, the reset, the new state, the scan it began.

## Reading the stream

Decoders built on JTAG read the stream through `JtagBus`. `next()` waits for the next scan, state, reset, gap, progress or end; a scan comes as a `Scan` cursor whose `bit()` reads the next bit with both lines' values (`Bit { tdi, tdo }`, `None` for a line that is not connected), `bit_or_end()` also finds the normal end at Update, and `drain()` skips to it. A scan aborted by TRST, a gap or the end of the samples is an error (`ReadError::Aborted`), and `JtagBus::finish()` drains a scan a decoder left open. `consumed()` hands over the ticks of what was read, a whole `shift` batch at a time. The cursor knows nothing of register lengths or of the other devices in a chain: those are the reading decoder's own.

## Limits

- The decoder sees the chain as one register between TDI and TDO: it does not know instruction or register lengths, the devices in the chain or what an instruction means.
- TRST is taken as active low, as the standard defines it.
- Synchronizing to an unknown controller takes five clocks in a row with TMS high or TRST; other TMS sequences that would also pin the state down are not used.
