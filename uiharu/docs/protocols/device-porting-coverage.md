# P4 device overlay coverage

This ledger records actual supported profiles rather than a claim of annotation
or option parity with the reference application. Protocol pages link the primary
specifications and attribution. New implementations are TypeScript DSL nodes;
existing `eeprom24xx` is reused without replacing its native engine.

| Entry             | Actual node           | Semantic profile and options                                                                                                                                  | Remaining scope                                                                                                                                                                |
| ----------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| DS1307            | `ds1307`              | Address, BCD/range-checked clock/calendar, 12/24-hour, CH/PM, control, RAM                                                                                    | No inferred oscillator state or calendar progression                                                                                                                           |
| RTC8564           | `rtc8564`             | Address, BCD clock/calendar, VL/century, alarm/AE, timer/CLKOUT registers                                                                                     | No timebase/alarm occurrence inference                                                                                                                                         |
| LM75              | `lm75`                | Address, resolution 9/10/11/12, signed integer microdegrees Celsius and thresholds/config                                                                     | Resolution is configured; variant identity is not detected                                                                                                                     |
| MLX90614          | `mlx90614`            | Address, little-endian words, SMBus PEC, raw/centidegree temperature/error flag                                                                               | No EEPROM programming timing, sleep pulse or calibration                                                                                                                       |
| ADXL345 I²C       | `adxl345`             | Address, named registers, raw signed 16-bit axis pairs                                                                                                        | Additional I²C profile; baseline SPI row uses separate `adxl345-spi`; no scale/justify inference                                                                               |
| MCP230xx          | `mcp230xx`            | Address, MCP23008/17 model, initial BANK/SEQOP and observed IOCON changes                                                                                     | Gap makes bank/sequence unknown; raw bytes remain available                                                                                                                    |
| PCA9571           | `pca9571`             | Address, every byte is output latch (no pointer byte)                                                                                                         | No general-call/device-ID command profile                                                                                                                                      |
| TCA6408A          | `tca6408a`            | Address, input/output/polarity/config register masks                                                                                                          | No electrical pin-state inference                                                                                                                                              |
| LTC26x7           | `ltc26x7`             | Address/global call, 2607/2617/2627 resolution, command/channel, padding, reference_microvolts and ideal code voltage                                         | No inferred latched output after separate update; extra bytes raw                                                                                                              |
| EDID              | `edid`                | Address/segment assumptions, base identity/chromaticity/established/standard/full detailed timings, range/text descriptors, E-DDC and CTA collection/checksum | No DisplayID, vendor-specific CTA payload semantics, GTF/CVT expansion or fragmented checksum assembly                                                                         |
| ATSHA204A         | `atsha204a`           | Address, Reset/Sleep/Idle/Command flags, count/opcode/parameters/status, packet CRC16                                                                         | Contiguous addressed packet profile; no split FIFO/wake pulse/authentication state                                                                                             |
| MXC6225XU         | `mxc6225xu`           | Address, raw signed X/Y, orientation/shake/detection reference fields                                                                                         | Limited published/reference register profile; no calibrated physical units                                                                                                     |
| Nunchuk           | `nunchuk`             | Address, plain/legacy-xor encoding, observed initialization writes and six-byte stick/accelerometer/button report                                             | No extension-ID/calibration/MotionPlus negotiation                                                                                                                             |
| XFP               | `xfp`                 | Address/initial table, thresholds/control/diagnostics, named page-one capability/link/technology/identity fields and table tracking                           | Optional PEC disabled, no vendor pages/management command engine                                                                                                               |
| Amulet ASCII      | `amulet-ascii`        | Default scalar/string/array/RPC/color, graphics coordinates/RGB/weight and echoed replies, ACK/NACK/page checksum; max_bytes                                  | No undocumented ASCII pixel/custom opcode/CRC protocol/software flow control                                                                                                   |
| PAN1321           | `pan1321`             | AT command identifiers, JAAC/JPRO mode, JSDA counted binary payload, OK/ROK/ERR; max_bytes                                                                    | Limited attributed reference profile; JSEC/JSLN parameters raw, no link/security state                                                                                         |
| EEPROM24xx native | existing `eeprom24xx` | 1/2 address bytes, optional device address; writes/reads/polls and bounded data chunks                                                                        | No chip model/initial address-counter option, page/capacity wrap or small-device block-select reconstruction; no 34AA04 bank commands; ACKs after control byte not interpreted |
| EEPROM24xx memory | `eeprom24xx-memory`   | Chip/generic geometry, optional counter/bank, page/capacity/block addresses, write ACK validity, 34AA04 SBA/RBA                                               | No retained memory image or software write-protect command execution                                                                                                           |
| DS2408            | `ds2408`              | F0/CC registers, F5 samples and CRC every32 bytes, 5A complemented writes/readback, reset latch and disable-test sequence                                     | No electrical PIO state or programming-effect inference; exact profile in [ds2408.md](ds2408.md)                                                                               |
| DS243x            | `ds243x`              | Observed ROM-family selection, DS2432/DS2433 scratchpad/memory, CRC16, authenticated-page and secret command transport                                        | Skip-ROM captures use configured fallback model; cryptographic MAC validity and internal programming remain unverified                                                         |
| DS28EA00          | `ds28ea00`            | Signed temperature, scratchpad CRC8, PIO complement/readback, conditional ROM and chain/status commands                                                       | Observed serial order is not a physical topology proof; conversion/power state remains an observation                                                                          |
| CFP               | `cfp`                 | Clause45 DEVAD1 address tracking, post-increment, register regions and module identifier                                                                      | Configured PHY, vendor registers stay raw; no optical measurements inferred                                                                                                    |

The [historical capability probes](device-dsl-blockers.md) are now resolved by
the authorized backend follow-up. The new consumers use canonical generated
contracts in the existing native producers; no registry adapter or replacement
bus was introduced. `uiharu-interp/tests/p4_devices.rs` supplies independent
wire/schema/value vectors, generated/interpreted comparisons and scheduling checks.

## Verification ledger

`uiharu-interp/tests/i2c_devices.rs` checks generated/interpreted event equality,
manufacturer-derived positive/negative/error vectors, register-state recovery,
small blocks, gaps, budgets and reader pauses. The current fifteen-device suite passes 24 focused tests, including all 9–12-bit
LM75 resolutions, EDID split fields/checksum/chromaticity/full timing/CTA/E-DDC/text, XFP threshold/capability/link sentinel/control/date pairs, EEPROM page/capacity/
block/bank wrapping and CAT24M01 17-bit addresses. Every profile also checks an
unrelated address followed by the selected repeated START in the same transaction.

`uiharu-interp/tests/uart_devices.rs` supplies independent ASCII/AT vectors,
embedded CR/LF binary data, invalid hexadecimal/checksum, truncated messages,
UART corruption, unknown commands, invalid lengths, bound recovery, gaps, scheduling/backpressure and generated/interpreted checks. All six focused tests pass, including all six Amulet graphics command/reply opcodes.

The existing EEPROM audit inspected its configuration, streaming decoder,
`tests/eeprom24xx.rs`, protocol documentation and the reference option inventory.
It established the native limits above; the supplementary `eeprom24xx-memory`
DSL node now implements the missing address geometry, wrapping, ACK validity and
34AA04 bank profile. This does not establish complete reference annotation parity.

## Pinned reference scope verification

The pinned DSView/libsigrokdecode method and annotation inventory was consulted
only to identify required scope. The implementations use original bounded DSL
logic and primary-authored protocol facts; no GPL implementation body was copied
or mechanically translated into Apache-2.0 code.

- EDID now covers chromaticity, all established timings, complete detailed timing
  fields, range limits and streamed monitor descriptor types, plus bounded CTA
  block collections and E-DDC segment selection/reset. Additional color-point and
  standard-timing descriptors stream named bytes: the reference labels these
  sections but explicitly lists their deeper interpretation as TODO. Full GTF/CVT,
  DisplayID, vendor CTA payloads and fragmented checksum assembly are stated
  limits, not claims of implemented semantics.
- Amulet now decodes line/rectangle/filled-rectangle commands and echoed replies,
  including coordinates, RGB and line weight from the manufacturer's 2018 guide.
  Its ASCII pixel handlers in the pinned reference explicitly report missing
  documentation. CRC mode and customized command maps are separate profiles;
  no undocumented pixel semantics are invented.
- Nunchuk's pinned reference covers six report registers and initialization.
  Those observed wire profiles are supported. Extension IDs, factory calibration
  and MotionPlus are beyond that reference and remain outside this profile.
- XFP now covers the complete named lower-memory and page-one handler inventory:
  thresholds, VPS/BER/wavelength/FEC/interrupt/general-control fields, diagnostics,
  extended ID, connector, transceiver/encoding/CDR capabilities, bit rates, five
  link media with unspecified/strict-lower-bound sentinels, technology, identity,
  wavelengths, case temperature, supply requirements, date, diagnostic mode,
  enhanced options and auxiliary monitor types. Fields that the reference reports
  only as raw register bytes remain named raw bytes. Vendor table execution,
  inferred analog state and optional PEC are explicitly outside this profile.

The former 1-Wire and MDIO contract gaps are resolved by the canonical generated
contracts and implemented consumers listed above. Remaining scope columns
identify explicit device-profile limits, not source implementations hidden behind
registry entries. Registration alone is not evidence of semantic or annotation
parity.
