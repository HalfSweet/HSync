# AVR in-system programming

The AVR ISP decoder reads the serial programming interface of AVR microcontrollers over an SPI bus: it turns `spi.events` into `avrisp.events`, one event per four-byte instruction. It follows the serial programming sections of Microchip (formerly Atmel) AVR datasheets — the "Serial Downloading" description and the "Serial Programming Instruction Set" table of the ATmega48A/PA/88A/PA/168A/PA/328/P datasheet, with the Load Extended Address byte of the larger devices such as the ATmega640/1280/2560.

| Node      | `AvrIsp` (`avr-isp`)                                                                           |
| --------- | ---------------------------------------------------------------------------------------------- |
| Input     | `bus`: `spi.events` version 1                                                                  |
| Output    | `events`: `avrisp.events` version 1                                                            |
| Requires  | `wordBits` = 8, `bitOrder` = `"msb-first"`, `mosi` = true, `miso` = true; chip select optional |
| Declares  | nothing                                                                                        |
| Configure | nothing                                                                                        |

The SPI decoder reads SCK as its clock in mode 0. Chip select, if connected, is the target's RESET (active low): it frames a whole programming session, not an instruction.

## Events

| `kind`        | Fields                                                                   | Meaning                                                                                                                                        |
| ------------- | ------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `instruction` | `op`, `tx`, `command`, `address`, `dataIn`, `dataOut`, `inSync`, `bytes` | A whole instruction the set names; `op` numbers them from 1, `tx` is the SPI transaction (0 on a bus without chip select)                      |
| `unknown`     | `tx`, `bytes`                                                            | Four bytes whose first, or first two, name no instruction of the set                                                                           |
| `error`       | `tx`, `reason`, `bytes`                                                  | An instruction cut short after one to three bytes: `incomplete` when RESET was released, `aborted` when a gap or the end of the samples cut it |

The fields of an instruction:

| Field     | Meaning                                                                                                                          |
| --------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `command` | Which instruction, named after the datasheet (below)                                                                             |
| `address` | Bytes two and three, the second most significant, for the instructions that carry an address; `null` otherwise                   |
| `dataIn`  | The fourth byte, for the instructions that load or write it; `null` otherwise                                                    |
| `dataOut` | What the device sent on MISO during the fourth byte, for the reads and Poll RDY/BSY; `null` otherwise                            |
| `inSync`  | For Programming Enable, whether the device echoed `53` on MISO during the third byte, as a device in sync does; `null` otherwise |
| `bytes`   | The four bytes on MOSI                                                                                                           |

```json
{"kind":"instruction","op":1,"tx":1,"command":"programming-enable","address":null,"dataIn":null,"dataOut":null,"inSync":true,"bytes":[172,83,0,0]}
{"kind":"instruction","op":2,"tx":1,"command":"read-signature-byte","address":1,"dataIn":null,"dataOut":149,"inSync":null,"bytes":[48,0,1,0]}
{"kind":"error","tx":1,"reason":"incomplete","bytes":[48,0]}
```

Values are bus data: fuse and lock bits, signature bytes and memory contents are not interpreted.

## Instructions

`aa` stands for an address byte, `xx` for a byte whose value does not matter, `in` for the programmer's data and `out` for the device's answer.

| Bytes          | `command`                       | Fields                               |
| -------------- | ------------------------------- | ------------------------------------ |
| `AC 53 xx xx`  | `programming-enable`            | `inSync`                             |
| `AC 80 xx xx`  | `chip-erase`                    | —                                    |
| `F0 00 xx out` | `poll-rdy-bsy`                  | `dataOut` (bit 0: busy)              |
| `4D 00 aa xx`  | `load-extended-address`         | `address`: the extended address byte |
| `48 00 aa in`  | `load-program-memory-page-high` | `address`, `dataIn`                  |
| `40 00 aa in`  | `load-program-memory-page-low`  | `address`, `dataIn`                  |
| `C1 00 aa in`  | `load-eeprom-memory-page`       | `address`, `dataIn`                  |
| `28 aa aa out` | `read-program-memory-high`      | `address`, `dataOut`                 |
| `20 aa aa out` | `read-program-memory-low`       | `address`, `dataOut`                 |
| `A0 aa aa out` | `read-eeprom-memory`            | `address`, `dataOut`                 |
| `58 00 xx out` | `read-lock-bits`                | `dataOut`                            |
| `30 00 aa out` | `read-signature-byte`           | `address` (0 to 2), `dataOut`        |
| `50 00 xx out` | `read-fuse-bits`                | `dataOut`                            |
| `58 08 xx out` | `read-fuse-high-bits`           | `dataOut`                            |
| `50 08 xx out` | `read-extended-fuse-bits`       | `dataOut`                            |
| `38 00 xx out` | `read-calibration-byte`         | `dataOut`                            |
| `4C aa aa xx`  | `write-program-memory-page`     | `address`                            |
| `C0 aa aa in`  | `write-eeprom-memory`           | `address`, `dataIn`                  |
| `C2 aa aa xx`  | `write-eeprom-memory-page`      | `address`                            |
| `AC E0 xx in`  | `write-lock-bits`               | `dataIn`                             |
| `AC A0 xx in`  | `write-fuse-bits`               | `dataIn`                             |
| `AC A8 xx in`  | `write-fuse-high-bits`          | `dataIn`                             |
| `AC A4 xx in`  | `write-extended-fuse-bits`      | `dataIn`                             |

The first byte names the instruction, and the second where the table fixes it; where the second byte is an address byte, any value will do.

## Decoding rules

- **Instructions.** An instruction is four bytes on MOSI, four SPI words in a row. The device answers on MISO a byte late, so the decoder reads MISO in two places only: during the third byte of Programming Enable, where a device in sync echoes the second byte, `53`, and during the fourth byte of the reads and Poll RDY/BSY, which carries their answer.
- **Grouping.** In a transaction seen to begin — RESET asserted within the samples — the words are grouped in fours from the first. Anywhere else the grouping is not known: in a transaction already under way at the first sample or after a gap, whose words may not even be aligned on bytes, and on a bus without chip select, at the first sample and after every gap. There words are skipped until a Programming Enable answered in sync — `AC 53` on MOSI and the echo `53` on MISO during the next byte — and grouped from its first byte on. That is how a programmer itself gets in step with a device, and the echo keeps data that happens to hold `AC 53` from passing for it.
- **Unknown bytes** are reported with the four bytes, and the grouping goes on.
- **Output.** An instruction goes out once its fourth byte is in. The assertion of RESET belongs to no instruction.
- **Cut short.** RESET released before an instruction's fourth byte gives an `error` with the reason `incomplete` and the bytes sent so far; a gap, the end of the samples or an aborted SPI transaction gives `aborted`. Words that only may begin Programming Enable, while the grouping is not known, are dropped without an event.
- **Progress** is passed on, held back to the first word of the instruction under way.

## Placement

| Event                    | Span                                    | Settled                                   |
| ------------------------ | --------------------------------------- | ----------------------------------------- |
| `instruction`, `unknown` | the four words                          | the fourth word's last bit                |
| `error` (`incomplete`)   | the words sent, through RESET's release | the release                               |
| `error` (`aborted`)      | the words sent, up to the cut           | the gap's start or the end of the samples |

The events of the stream settle in order, and they are the same however the samples are cut into blocks and however the reader pauses.

## Limits

- The decoder does not follow the session's state — whether programming was enabled, which extended address or page is loaded — and checks no echo but Programming Enable's.
- Without RESET connected, a capture that starts after the session's Programming Enable, or a gap inside it, decodes nothing until the next Programming Enable; a gap inside a word also leaves the SPI words unaligned until a gap ends between words.
- The instruction sets of the older AT90S devices, and the TPI, PDI and UPDI programming interfaces, are out of scope.

The optional TypeScript [`avr-isp-device`](avr-isp-device.md) layer combines
observed signature addresses0–2 and labels a small manufacturer-verified device
table. The transport DTO is declared in `dsl/contracts/avr-isp.ts`, preserving
exact four-byte arrays and wire JSON; its source API aliases are documented
with the overlay. Extra MISO reply-echo warnings require bytes not exposed by
the current transport contract and remain a documented information gap.
