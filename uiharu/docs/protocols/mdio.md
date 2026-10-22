# MDIO

The `mdio` decoder (`uiharu_protocols::mdio::Mdio`) reads the management interface of Ethernet PHYs from logic samples: the frames a station sends on MDIO, clocked by MDC, to read and write the registers of PHYs and other MDIO manageable devices. It is written from IEEE Std 802.3: clause 22, the MII management interface — the MDC and MDIO signals and the management frame — and clause 45, whose frames start with `00` and reach a device's register in two steps, an address frame and a data frame.

| Node      | `Mdio` (`mdio`)                                                        |
| --------- | ---------------------------------------------------------------------- |
| Input     | `logic`: logic samples; requires `channels` ≥ the highest one used + 1 |
| Output    | `events`: `mdio.events` version 1                                      |
| Declares  | nothing: every frame names its clause                                  |
| Configure | `mdc`, `mdio`: the channels                                            |

## Configuration

| Field  | Meaning                              | Default |
| ------ | ------------------------------------ | ------- |
| `mdc`  | Management data clock channel        | 0       |
| `mdio` | Management data input/output channel | 1       |

The two lines must be on different channels.

## Events

Each event is tagged with its `kind`.

| `kind`  | Fields                                                             | Meaning                                                                                                                                                                     |
| ------- | ------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `frame` | `clause`, `op`, `phy`, `reg`, `turnaround`, `data`, `preambleBits` | A whole management frame                                                                                                                                                    |
| `error` | `reason`                                                           | `start`: a clause 22 start with an operation clause 22 does not define; `turnaround`: the turnaround broke its rule; `aborted`: a gap or the end of the samples cut a frame |

The fields of a frame, as on the bus:

| Field          | Meaning                                                                                                                                                                                            |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `clause`       | `22` (ST `01`) or `45` (ST `00`)                                                                                                                                                                   |
| `op`           | Clause 22: `read` (OP `10`), `write` (`01`). Clause 45: `address` (`00`), `write` (`01`), `read` (`11`), `post-read-increment` (`10`, a read after which the device advances its register address) |
| `phy`          | PHYAD in clause 22, PRTAD in clause 45: the port address, 0 to 31                                                                                                                                  |
| `reg`          | REGAD in clause 22, DEVAD in clause 45: the register, or the device within the port, 0 to 31                                                                                                       |
| `turnaround`   | The two TA bits as seen, the first in bit 1: `2` (`10`) as a station sends it                                                                                                                      |
| `data`         | The sixteen data bits, the first in bit 15: the register's value, or for a clause 45 `address` frame the register address                                                                          |
| `preambleBits` | The ones clocked in a row before ST, counted up to 32, a preamble's length; 0 when the station suppressed the preamble                                                                             |

```json
{"kind":"frame","clause":22,"op":"read","phy":1,"reg":2,"turnaround":2,"data":4660,"preambleBits":32}
{"kind":"error","reason":"turnaround"}
{"kind":"frame","clause":45,"op":"address","phy":3,"reg":1,"turnaround":1,"data":7,"preambleBits":0}
```

The fields are bus data. Register meanings, the device types DEVAD names, and the pairing of a clause 45 data frame with the address frame before it are the next layer's business.

## Decoding rules

- **Bits.** Every bit is MDIO's level at the tick MDC rises. MDC's falling edges and MDIO's own changes are not looked at, so MDC may run between frames or stop.
- **Frames.** After its preamble a frame is 32 bits: ST (2), OP (2), PHYAD or PRTAD (5), REGAD or DEVAD (5), TA (2) and the data (16), every field most significant bit first.
- **Start.** In sync, every one is idle or preamble, and the first zero is ST's first bit however many ones came before it — none for a station that suppresses the preamble and stops MDC between frames, or sends frames back to back. The ones in a row before ST are counted up to 32: ones before those are idle time, and the frame spans from the first one counted.
- **Sync.** The decoder is in sync after a whole frame, and after 32 ones in a row, which a PHY must see before it answers. At the first sample, after a gap and after a `start` error it is not: it skips bits until 32 ones in a row, so a frame already under way there is not decoded, nor are frames without a preamble until then.
- **Undefined starts.** ST `01` with OP `00` or `11` is no clause 22 frame: an `error` with the reason `start` covers ST and OP, and the decoder skips bits until it is in sync again, since it cannot tell where the bits it read end.
- **Turnaround.** On a write and an address frame the station keeps driving and sends `10`. On a read and a post-read-increment read it releases the line for the first bit, which is then nobody's and usually reads `1` from the pull-up, and the device drives the second bit `0`. A turnaround that breaks this — on a read, a second bit of `1` is what a port with no device answering shows — is reported as an `error` with the reason `turnaround` when its second bit is read; the frame goes on and is reported, with the turnaround bits as seen.
- **Cut short.** A gap or the end of the samples cuts the frame under way: an `error` with the reason `aborted`. Ones counted before a gap do not count after it.
- **Progress** is passed on, held back to the start of the frame under way, or to the first of the ones that may become the next frame's preamble.

## Placement

| Event                  | Span                                                                         | Settled                                   |
| ---------------------- | ---------------------------------------------------------------------------- | ----------------------------------------- |
| `frame`                | from the first preamble one counted, or ST's first bit, to the last data bit | the last data bit's rising edge           |
| `error` (`start`)      | ST and OP                                                                    | OP's second bit                           |
| `error` (`turnaround`) | the two turnaround bits                                                      | the second turnaround bit                 |
| `error` (`aborted`)    | the frame from its start through the last bit read                           | the gap's start or the end of the samples |

A bit's tick is the tick MDC rose; a span ends one tick after its last bit. A bit decides at most one event, so a `turnaround` error comes before the frame it belongs to, inside that frame's span. The events of the stream settle in order, and they are the same however the samples are cut into blocks and however the reader pauses.

## Limits

- Only the 32-bit frames of clauses 22 and 45 are decoded; vendor extensions of other lengths are not.
- A capture that begins, or resumes after a gap, in traffic without preambles stays out of sync until a preamble comes.
- The decoder judges the frame format only; it does not check that a device at `phy` exists, apart from what the turnaround shows.

## Sampling edge and canonical contract

`readEdge` is `rising` by default and can be `falling` to preserve an explicitly
selected capture interpretation (including the DSView import default). Both
options sample the chosen clock edge and keep identical frame validation,
preamble resynchronization and bounded streaming behavior. The waveform
synthesizer follows this choice. Independent manually assembled frame vectors
check both edges and fragmented scheduling in `tests/mdio.rs`.

The frame types now come from `dsl/contracts/mdio.ts`, retaining exactly numeric
Clause 22/45 JSON validation and the public `Clause::C22`/`C45` variants. Import
`ClauseExt` for `clause.number()` or `Clause::from_number(n)` (the latter replaces
`Clause::try_from(n)`); generated `clause.value()` is also available. Import
`OpExt` for `op.device_drives()`. This ownership change enables the TypeScript
`cfp` consumer without a parallel lower bus or a JSON adapter.
