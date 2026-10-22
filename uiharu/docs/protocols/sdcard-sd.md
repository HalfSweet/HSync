# Native SD (SDR)

`sdcard-sd` decodes the native SD CMD transport and optionally the DAT read blocks of
CMD17 and CMD18. It is implemented from the SD Association's public
[Physical Layer Simplified Specification 7.10](https://www.sdcard.org/cms/wp-content/themes/sdcard-org/dl.php?f=Part1_Physical_Layer_Simplified_Specification_Ver7.10.pdf),
sections 3.6 (bus protocol), 4.5 (CRC), and 4.9 (responses), available through the
[official simplified specification archive](https://www.sdcard.org/downloads/pls/archives/).
No third-party decoder implementation was used.

## Configuration

| Field         | Default | Meaning                                               |
| ------------- | ------- | ----------------------------------------------------- |
| `clk`         | 0       | CLK channel; rising edges sample CMD and DAT          |
| `cmd`         | 1       | Bidirectional CMD channel                             |
| `data`        | `[]`    | No DAT decoding, `[DAT0]`, or `[DAT0,DAT1,DAT2,DAT3]` |
| `block_bytes` | 512     | Fixed read block length, 1–4096 bytes                 |

Channels must be distinct and in 0–63. The logic source must provide every configured
channel. Output properties are `dataWidth`, `blockBytes`, `sampling: "rising"`, and
`readOnly: true`. Schema: `sdcard-sd.events`, version 1.

## Events

Token and block IDs share a monotonically increasing sequence starting at one.

| Kind         | Fields                                     | Meaning                                                                 |
| ------------ | ------------------------------------------ | ----------------------------------------------------------------------- |
| `begin`      | `token`, `partial`                         | CMD start bit, or initial low CMD of unknown alignment                  |
| `header`     | `token`, `host`, `index`, `bits`           | Direction, raw six-bit index/reserved field, 48/136-bit length          |
| `payload`    | `token`, `offset`, `value`, `bits`         | Next 1–32 argument/register bits, MSB first; offset counts payload bits |
| `crc`        | `token`, `value`, `expected`, `ok`         | CRC7; expected and ok are null for R3                                   |
| `end`        | `token`, `reason`                          | CMD token end                                                           |
| `data-begin` | `block`, `partial`                         | Simultaneous low start bits on all configured DAT lines                 |
| `data`       | `block`, `offset`, `value`                 | One byte, byte offset from block start                                  |
| `data-crc`   | `block`, `lane`, `value`, `expected`, `ok` | Per-lane CRC16, lane zero is DAT0                                       |
| `data-end`   | `block`, `reason`                          | Read data block end                                                     |

`complete` means the high end bit(s) arrived; a bad checksum is reported by its check
event and does not turn a complete-length token into a truncated one. `invalid-end`
means the end bit(s) were low. `aborted` means samples were missing or input ended.
`resynchronized` closes an unknown-alignment partial CMD after eight consecutive
sampled high CMD bits; this is a recovery heuristic, not validation of the skipped bits.

## Decoding rules

- CMD start is a sampled low while no token is active; the next sampled bit is the
  transmission direction. CMD is processed before DAT when events share a CLK edge.
- Commands contain 48 bits. Responses following valid CMD2, CMD9 or CMD10 contain
  136 bits (R2). Other responses contain 48 bits. The most recent valid command is
  consumed by the next response and is cleared by gaps and invalid commands.
- R2 payload is the 120 register bits preceding the register's CRC7. Its eight-bit
  response header is excluded from the CRC calculation. Other CRC7 checks cover
  the first 40 bits, with polynomial x^7+x^3+1 and initial zero. Responses following
  ACMD41 (following an accepted CMD55 with APP_CMD set) have reserved trailing bits, reported as an unavailable CRC check (R3).
- Read data starts only after a valid, matching non-error R1 to CMD17 or CMD18. CMD17 permits one block;
  CMD18 permits successive blocks until the next host command. Block length is configured, never
  inferred from CMD16. Every byte is emitted as soon as its bits arrive. In 4-bit
  mode DAT3 is the nibble's most significant bit; the high nibble comes first.
- Read CRC16 is calculated independently on each connected lane, initial zero,
  polynomial x^16+x^12+x^5+1. Start/end bits are excluded. CRCs arrive over sixteen
  clocks. The end clock must show every lane high.
- A gap aborts active CMD/data frames before the gap item is forwarded and clears
  read arming. After a gap, eight consecutive sampled high CMD bits are required to recover
  alignment, even if CMD resumes high. A low before recovery emits a partial
  begin and is drained through eight high samples. Low CMD at the first sample
  also emits a partial begin;
  the unknown-alignment token is drained without inventing payload bits. DAT is
  not armed on capture startup, so an already-active read block is ignored.
- Bytes/chunks have spans from their first to last sampled bit (inclusive, encoded
  as a half-open range); header and CRC spans cover their own bits. Their settled
  tick is the final bit. Boundary events span their deciding tick; aborted ends have an empty span at the gap/input end. Progress is
  held at the earliest unfinished header/chunk/byte/CRC start. Simultaneous events
  settle in CMD-before-DAT order. All input and output operations await backpressure.

## Limits

This is an SDR memory-card transport subset. There is no DAT write decoding, write
status, DAT0 busy interpretation, SDIO, DDR, UHS-II, timing validation, command-specific
argument/status interpretation, or automatic bus-width/block-length negotiation.
Enable DAT only with the read width and length used by the capture. Read arming requires a CRC-valid response with matching command index and no R1 error
flags; a new host token, error, or gap cancels it. A capture beginning with an orphan R2 response lacks the command context
needed to identify its length and is decoded as a short response. Unknown or vendor
commands are preserved as raw indices and payload. R1/R6/R7 share the short transport;
R1b's separate DAT0 busy interval is outside this decoder.

Memory is independent of block length: one 32-bit CMD accumulator, one byte accumulator,
and four calculated/received CRC16 pairs, plus counters. No transaction or data block is
buffered. Synthesizer: `sdcard_sd::synth::SdcardSdWaveform`.
