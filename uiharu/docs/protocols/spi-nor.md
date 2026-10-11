# SPI NOR flash

The SPI NOR decoder reads the commands a host sends to a serial NOR flash over a single-line SPI bus: it turns `spi.events` into `nor.events`, one operation per chip select period. It follows the command sets of the Winbond W25Q64JV, Macronix MX25L6433F and Micron N25Q/MT25Q datasheets (with the four-byte address mode of their larger relatives) and JEDEC JESD216 for the SFDP read.

| Node      | `SpiNor` (`spi-nor`)                                                                                            |
| --------- | --------------------------------------------------------------------------------------------------------------- |
| Input     | `bus`: `spi.events` version 1                                                                                   |
| Output    | `events`: `nor.events` version 2                                                                                |
| Requires  | `wordBits` = 8, `bitOrder` = `"msb-first"`, `cs` = true, `mosi` = true, `miso` = true                           |
| Declares  | `addressBytes`: the configured width, `"three"`, `"four"` or `"auto"`                                           |
| Configure | `address_bytes`: `three` (default), `four`, or `auto` — the width of the addresses that follow the flash's mode |

## Events

Every event but `unknown-command` names its operation, `op`, numbered from 1. An operation begins with `begin`, ends with `end`, and carries between them the events its command calls for, in bus order.

| `kind`            | Fields                               | Meaning                                                                                                                                                            |
| ----------------- | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `begin`           | `op`, `tx`, `command`                | The opcode named a known command; `tx` is the SPI transaction it rides on                                                                                          |
| `address`         | `op`, `address`, `bytes`             | The command's address, sent most significant byte first, and its width: 3 or 4                                                                                     |
| `dummy`           | `op`, `bytes`                        | Dummy bytes were clocked; their values mean nothing                                                                                                                |
| `data`            | `op`, `direction`, `offset`, `bytes` | Data in order: `in` from the host on MOSI (programs), `out` from the flash on MISO (reads); `offset` counts from the operation's first data byte; at most 32 bytes |
| `status`          | `op`, `register`, `value`, `written` | A value of status register 1, 2 or 3, read from MISO (`written` false) or written on MOSI (`written` true)                                                         |
| `id`              | `op`, `scheme`, `bytes`              | Identification bytes from the flash, as read: `jedec`, `manufacturer-device`, `unique` or `electronic-signature`; at most 32 bytes                                 |
| `end`             | `op`, `reason`                       | `complete`: chip select was released after the command was whole; `incomplete`: before; `aborted`: the samples ended or were lost under it                         |
| `unknown-command` | `tx`, `opcode`                       | The transaction's first byte is not an opcode the decoder knows; the transaction is skipped                                                                        |

Values are bus data. Status register bits, manufacturer codes and SFDP tables are not interpreted; for orientation only, bit 0 of status register 1 is commonly WIP (busy) and bit 1 WEL (write enabled).

## Commands

`command` names the command in kebab-case. The address width "mode" is three or four bytes as configured (see below).

| Opcode     | `command`                     | After the opcode                                                                    |
| ---------- | ----------------------------- | ----------------------------------------------------------------------------------- |
| `06`       | `write-enable`                | nothing                                                                             |
| `04`       | `write-disable`               | nothing                                                                             |
| `50`       | `volatile-sr-write-enable`    | nothing                                                                             |
| `05`       | `read-status`                 | `status` (register 1) per byte read                                                 |
| `35`       | `read-status-2`               | `status` (register 2) per byte read                                                 |
| `15`       | `read-status-3`               | `status` (register 3) per byte read                                                 |
| `01`       | `write-status`                | `status` (register 1), then register 2 if a second byte follows                     |
| `31`       | `write-status-2`              | `status` (register 2)                                                               |
| `11`       | `write-status-3`              | `status` (register 3)                                                               |
| `03`       | `read`                        | `address` (mode), `data` out                                                        |
| `0B`       | `fast-read`                   | `address` (mode), `dummy` 1, `data` out                                             |
| `13`       | `read-4b`                     | `address` (4), `data` out                                                           |
| `0C`       | `fast-read-4b`                | `address` (4), `dummy` 1, `data` out                                                |
| `02`       | `page-program`                | `address` (mode), `data` in, at least one byte                                      |
| `12`       | `page-program-4b`             | `address` (4), `data` in, at least one byte                                         |
| `20`       | `sector-erase`                | `address` (mode)                                                                    |
| `21`       | `sector-erase-4b`             | `address` (4)                                                                       |
| `52`       | `block-erase-32k`             | `address` (mode)                                                                    |
| `D8`       | `block-erase-64k`             | `address` (mode)                                                                    |
| `DC`       | `block-erase-64k-4b`          | `address` (4)                                                                       |
| `C7`, `60` | `chip-erase`                  | nothing                                                                             |
| `B9`       | `deep-power-down`             | nothing                                                                             |
| `AB`       | `release-power-down`          | nothing; or, when bytes follow, `dummy` 3 and `id` (`electronic-signature`, 1 byte) |
| `90`       | `read-manufacturer-device-id` | `address` (3), `id` (`manufacturer-device`, 2 bytes)                                |
| `9F`       | `read-jedec-id`               | `id` (`jedec`), as many bytes as read                                               |
| `5A`       | `read-sfdp`                   | `address` (3), `dummy` 1, `data` out                                                |
| `4B`       | `read-unique-id`              | `dummy` 4, `id` (`unique`, 8 bytes)                                                 |
| `B7`       | `enter-4b-mode`               | nothing                                                                             |
| `E9`       | `exit-4b-mode`                | nothing                                                                             |
| `66`       | `reset-enable`                | nothing                                                                             |
| `99`       | `reset`                       | nothing                                                                             |
| `75`       | `program-erase-suspend`       | nothing                                                                             |
| `7A`       | `program-erase-resume`        | nothing                                                                             |
| `48`       | `read-security-register`      | `address` (mode), `dummy` 1, `data` out                                             |
| `42`       | `program-security-register`   | `address` (mode), `data` in, at least one byte                                      |
| `44`       | `erase-security-register`     | `address` (mode)                                                                    |
| `36`       | `block-lock`                  | `address` (mode)                                                                    |
| `39`       | `block-unlock`                | `address` (mode)                                                                    |
| `7E`       | `global-block-lock`           | nothing                                                                             |
| `98`       | `global-block-unlock`         | nothing                                                                             |

Where vendors give an opcode different meanings, the decoder takes the Winbond W25Q one.

## Decoding rules

- Every SPI transaction — one chip select period — carries one command; its first byte on MOSI is the opcode. An opcode outside the table is reported as `unknown-command` and the rest of the transaction is skipped.
- Dual and quad I/O commands (`3B`, `6B`, `BB`, `EB`, `32`, `38`, `92`, `94` …) and QPI move their address or data over two or four lines, which a single-line bus does not show; they are unknown commands.
- **Address width.** The four-byte commands (`13`, `0C`, `12`, `21`, `DC`) always take four bytes, SFDP (`5A`, as JESD216 defines it) and REMS (`90`) always three. Every other addressed command takes the configured width: `three`, `four`, or with `auto` three until `enter-4b-mode` completes, four until `exit-4b-mode` or a software reset — `reset-enable` directly followed by `reset` — completes. Another command between `reset-enable` and `reset` disarms the reset. After a gap `auto` goes back to three bytes, since the gap may have hidden a mode change or a reset; three is also what it assumes at the start.
- **What the host sends must be whole**: the opcode, the address, the dummy bytes, at least one byte for a program or a status register write, and the three dummy bytes of `release-power-down` when anything follows its opcode. Chip select released earlier ends the operation `incomplete`.
- **What the flash sends lasts as long as the host clocks it.** A read, a status read or an identification read may stop after any byte, or before the first, and is `complete`. A read of no bytes has no `data` event.
- `data` and JEDEC `id` bytes go out in batches of at most 32 bytes, each as soon as it is full and the last when the operation ends, so a long read shows up while it is under way; a JEDEC ID longer than 32 bytes continues in the next `id` event. `status` values go out one by one. The fixed-length ids — two bytes of REMS, eight of RUID, one of RES — go out once whole, or, if the host stopped earlier, with the bytes read when the operation ends.
- Bytes after a command that takes no more — past an erase's address, a fixed-length id, the last register of a status register write — are skipped; the operation still ends `complete` when chip select is released, and its `end` covers them.
- A transaction already under way when the samples begin or after a gap is skipped: its first word is not the opcode, and its words may not be aligned on bytes.
- A gap or the end of the samples during a command ends it `aborted`, after the data read so far; the gap or the end is then passed on.

## Placement

Each event covers the SPI words it is made of (`span`) and is settled at the tick it became certain:

| Event                              | Span                                                           | Settled                                            |
| ---------------------------------- | -------------------------------------------------------------- | -------------------------------------------------- |
| `begin`                            | chip select's assertion through the opcode word                | the opcode's last bit                              |
| `address`, `dummy`, `status`       | their words                                                    | the last word's last bit                           |
| `data`, `id` (full, or a whole id) | the words of the batch                                         | the last byte's last bit                           |
| `data`, `id` (flushed by an end)   | the words of the batch                                         | the end that decided it                            |
| `end` (`complete`)                 | the words skipped after the command, and chip select's release | the release                                        |
| `end` (`incomplete`, `aborted`)    | the words read but not reported, through the transaction's end | the release, the gap's start or the end of samples |
| `unknown-command`                  | chip select's assertion through the opcode word                | the opcode's last bit                              |

The events of the stream settle in order, and they are the same however the samples are cut into blocks and however the reader pauses.

## Limits

- Single-line SPI only: no dual, quad or QPI transfers, no DTR.
- One command per chip select period, as the flash requires; the decoder does not judge whether the flash accepted a command (whether writes were enabled, whether chip select rose on the byte boundary it requires).
- `auto` assumes the flash powers up in three-byte mode; a flash configured to power up in four-byte mode is read with `four`.
- Vendor-specific commands outside the table — configuration registers beyond the three status registers, extended address registers, OTP areas reached by other opcodes — are unknown commands.
