<!-- SPDX-License-Identifier: Apache-2.0 -->

# ST25R3916/3917 SPI

`st25r39xx-spi` is the explicit ST25R3916/3917 profile from
[ST DS12484 Rev8 §§4.3–4.5, Tables 11/13/15/17/18](https://www.st.com/resource/en/datasheet/st25r3916.pdf).
It consumes eight-bit MSB-first SPI with select and both data lines; physical SPI
mode 1 is configured in the preceding SPI node. Its authoritative source is
`dsl/decoders/st25r39xx-spi.ts`. There are no decoder-specific options.

Register traffic adds a `register` event with space, address, manufacturer-derived
identifier, raw value and direction. All 64 Space-A registers and all 16 published
Space-B registers are named, as is test-space 01 analog test/observation. Other
test/unpublished addresses remain unknown. Prefix FB selects Space B and FC selects
test access until select is released; the next transaction starts in Space A.
Automatic increment is tracked only within the published six-bit address range;
an extra byte outside that range remains raw and terminates the transaction as
unsupported. No fictitious wrap to a named register is inferred.

FIFO 80/9F traffic stays raw FIFO data and never receives register names. Direct
commands are named, and existing operation, IRQ/FIFO status and selected control
bitfields remain exposed. Names distinguish identical numeric addresses across
spaces, such as Space-A protocol configuration versus Space-B EMD/subcarrier
settings. Commands and register writes are observations, not confirmation of RF
state transitions or completion.

This profile does not claim other ST25R39xx family maps, passive-target-memory
commands, every analog/test bitfield, NFC/RF frame reconstruction or inferred
unobserved state. Unknown direct commands terminate as unsupported. Missing data,
partial select, gaps and capture end preserve completed observations but close
incomplete work without fabricated bytes. State and buffering remain bounded.

`tests/radio_registers.rs` verifies the full Space-A range, selected Space-B/test
names, space reset, FIFO discrimination and full generated/interpreted parity
with gaps and backpressure. Existing SPI tests cover Space-B discrimination,
FIFO flags and Clear-FIFO. The [SPI ledger](spi-porting-coverage.md) retains the
broader profile limits.
