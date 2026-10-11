# SD card, SPI mode

The SD card decoder reads the traffic between a host and an SD memory card on an SPI bus: it turns `spi.events` into `sdcard.events` — command frames, the card's responses, control tokens and data blocks. It follows the SD Association's Physical Layer Simplified Specification (version 9.10, Part 1): chapter 7, SPI Mode, for the formats of commands, responses and tokens and their timing, with the CRCs of §4.5 and the command lists of §4.7.

| Node      | `SdCardSpi` (`sdcard-spi`)                                                                                                        |
| --------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Input     | `bus`: `spi.events` version 1                                                                                                     |
| Output    | `events`: `sdcard.events` version 1                                                                                               |
| Requires  | `wordBits` = 8, `bitOrder` = `"msb-first"`, `cs` = true, `mosi` = true, `miso` = true                                             |
| Declares  | `blockLength`: the configured block length                                                                                        |
| Configure | `block_length`: the length of read and write blocks until a CMD16 changes it, 1 to 2048 bytes (default 512, a card's at power-up) |

## Events

An operation is a command and what it brings about: the response, and the data the command moves. Every event but `unknown-command` names its operation, `op`, numbered from 1; an operation begins with `command`, ends with `end`, and carries between them its events in bus order. A chip select period may hold several operations.

| `kind`            | Fields                                                    | Meaning                                                                                                                                                                                       |
| ----------------- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `command`         | `op`, `tx`, `index`, `acmd`, `argument`, `crc7`, `crc_ok` | A command frame: index 0–63, the 32-bit argument, the CRC7 field as sent and whether it matches; `acmd` when the card takes it as an application command; `tx` is the SPI transaction         |
| `response`        | `op`, `format`, `bytes`, `busy_bytes`                     | The card's response: `r1`, `r1b`, `r2`, `r3` or `r7`, its 1 to 5 bytes, R1 first; `busy_bytes` counts the busy bytes after an R1b                                                             |
| `token`           | `op`, `token`, `value`                                    | A control token and its byte: `start-block` (`FE`), `start-multi-block` (`FC`), `stop-tran` (`FD`), `data-response` (`xxx0sss1`), `error` (`0000xxxx`)                                        |
| `data`            | `op`, `offset`, `bytes`                                   | Bytes of a data block, at most 32; `offset` counts from the operation's first data byte, across its blocks                                                                                    |
| `block-end`       | `op`, `crc16`, `crc_ok`, `length`                         | The end of a block: the CRC16 field as sent, whether it matches the block's bytes, and the block's length                                                                                     |
| `end`             | `op`, `reason`                                            | `complete`: the exchange the command calls for was whole; `incomplete`: chip select was released, or the host began another command, before it was; `aborted`: the samples ended or were lost |
| `unknown-command` | `tx`, `byte`                                              | The transaction's first byte on MOSI that is not fill does not begin a command frame; the transaction is skipped                                                                              |

Values are bus data: the response flags, the OCR, CSD, CID and SCR are not interpreted. For orientation only, R1's bits are, from bit 0: in idle state, erase reset, illegal command, command CRC error, erase sequence error, address error, parameter error; bit 7 is 0.

## Commands

| Command                           | Response | Data                                                                       |
| --------------------------------- | -------- | -------------------------------------------------------------------------- |
| CMD8                              | R7       | —                                                                          |
| CMD12, CMD20, CMD28, CMD29, CMD38 | R1b      | —                                                                          |
| CMD13                             | R2       | —                                                                          |
| CMD58                             | R3       | —                                                                          |
| CMD6                              | R1       | a 64-byte block from the card                                              |
| CMD9, CMD10                       | R1       | a 16-byte block from the card (CSD, CID)                                   |
| CMD17                             | R1       | a block from the card                                                      |
| CMD18                             | R1       | blocks from the card until the host's next command                         |
| CMD24, CMD42                      | R1       | a block from the host                                                      |
| CMD25                             | R1       | blocks from the host until the stop token                                  |
| CMD27                             | R1       | a 16-byte block from the host (CSD)                                        |
| CMD30                             | R1       | a 4-byte block from the card                                               |
| CMD48                             | R1       | a 512-byte block from the card                                             |
| CMD49                             | R1       | a 512-byte block from the host                                             |
| CMD56                             | R1       | a block from the card when argument bit 0 is 1, from the host when it is 0 |
| ACMD13                            | R2       | a 64-byte block from the card (SD status)                                  |
| ACMD22                            | R1       | a 4-byte block from the card                                               |
| ACMD51                            | R1       | an 8-byte block from the card (SCR)                                        |
| every other command               | R1       | —                                                                          |

"A block" is the block length long: the configured one until a CMD16 sets another.

## Decoding rules

- **Commands.** Between operations MOSI carries fill (`FF`). A byte `01xxxxxx` begins a six-byte command frame: the index, the argument most significant byte first, the CRC7 (x⁷ + x³ + 1) and an end bit, which is not checked. `crc_ok` only says whether the CRC7 field matches; in SPI mode a card checks it for CMD0 and CMD8 alone unless CMD59 turned checking on, and hosts often send `FF` instead. A frame cut short by chip select or a gap is no command and is dropped.
- **Unknown traffic.** A transaction whose first byte on MOSI that is not fill does not begin a command frame is reported as `unknown-command` and skipped; stray bytes between the operations of a transaction are skipped.
- **Transactions without a command** — every MOSI byte fill — are skipped, whatever MISO carries. Busy polling looks like that, and so does a multiple-block read that a host goes on clocking in a new chip select period: the operation that asked for the data ended `incomplete` when chip select was released, and data without the command that asked for it is not reported.
- **Responses.** The response is the first byte from the card with bit 7 clear; the fill bytes before it are not reported. After CMD12 the first byte is a stuff byte, which may still carry bits of the stopped block, and is skipped. A card that refuses a command — illegal command or CRC error in R1 — answers with R1 alone, which is reported with the format `r1` whatever the command's format.
- **Busy.** After an R1b the busy bytes (`00`) are counted until the card turns ready (any other byte), the host begins its next command or releases chip select; the response is reported then, and the operation is `complete` in all three cases (the specification lets the host release chip select while the card is busy). The busy period after a data response or a stop token belongs to no operation and is skipped.
- **Reads.** A command whose R1 has an error flag (bits 2 to 6) moves no data. Otherwise the decoder waits for a token on MISO, skipping fill: a start block token is followed by the block and its CRC16 (x¹⁶ + x¹² + x⁵ + 1); an error token takes the block's place. A single-block read ends with its block or its error token; CMD18 goes on to the next token until the host begins its next command — normally CMD12, possibly inside a block — which ends it `complete`.
- **Writes.** The decoder waits for the host's token on MOSI, skipping fill: the block follows, every byte of it data whatever it looks like, then its CRC16, then the card's data response token on MISO. A single-block write ends with the data response; CMD25 goes on until the stop token, or a command (the host stops it with CMD12 after an error), either of which ends it `complete`.
- **Cut short.** A single-block operation that a command interrupts ends `incomplete`, and so does any operation whose chip select is released before its response or data is whole; the data read so far is reported first, without a `block-end`.
- **Batches.** Data goes out in batches of at most 32 bytes, each as soon as it is full and the last of a block with the block's last byte, followed by the block's `block-end`.
- **Block length.** The configured length until a CMD16 whose R1 has no error flag sets another of 1 to 2048 bytes; CMD0 returns it to 512 bytes. A gap leaves it as it was: the card keeps its block length, and a CMD16 hidden in a gap is rarer than the transfers that make up most of the traffic.
- **Application commands.** The command after a CMD55 the card did not refuse is an application command when its index names one (ACMD6, 13, 18, 22, 23, 25, 26, 38, 41, 42, 43 to 49, 51) and a regular command otherwise; the CMD55 holds across chip select periods, but a gap forgets it.
- **Gaps.** A transaction already under way when the samples begin or after a gap is skipped, since its words may not be aligned on bytes. A gap or the end of the samples ends the open operation `aborted`, after what was read; the gap or the end is then passed on.

## Placement

Each event covers the SPI words it is made of (`span`) and is settled at the tick it became certain:

| Event                            | Span                                                                    | Settled                                                                                                    |
| -------------------------------- | ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `command`                        | the six frame words                                                     | the last one's last bit                                                                                    |
| `response`                       | R1 through the response's last byte; for R1b through the last busy byte | the last byte's last bit; for R1b the byte that ended the busy period, the release, or the end that cut it |
| `token`, `unknown-command`       | the byte's word                                                         | its last bit                                                                                               |
| `data` (full, or a block's last) | the words of the batch                                                  | the last byte's last bit                                                                                   |
| `data` (flushed by an end)       | the words of the batch                                                  | the end that decided it                                                                                    |
| `block-end`                      | the two CRC words                                                       | the second one's last bit                                                                                  |
| `end` (`complete`)               | none: an instant                                                        | the last bit of the byte that completed the operation                                                      |
| `end` (`incomplete`)             | the words read toward an event that never came, through the release     | the release, or the last bit of the next command's first byte                                              |
| `end` (`aborted`)                | as for `incomplete`, placed at the cut                                  | the gap's start or the end of the samples                                                                  |

Fill bytes belong to no event. The events of the stream settle in order, and they are the same however the samples are cut into blocks and however the reader pauses.

## Limits

- The host must send fill (`FF`) on MOSI while it reads: a byte `01xxxxxx` there is taken for a command.
- High-capacity cards (SDHC, SDXC) keep 512-byte blocks for reads and writes whatever CMD16 says; hosts do not send them CMD16 for that, but the decoder follows CMD16 whatever the card.
- `busy_bytes` saturates at 65535 and `offset` at 2³² − 1.
- SDIO (CMD5, CMD52, CMD53 and their R4 and R5 responses), the security commands' data and MultiMediaCards are out of scope; the SD-mode bus (four data lines, CMD line) is a different decoder.
