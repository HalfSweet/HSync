<!-- SPDX-License-Identifier: Apache-2.0 -->

# qspi-flash

The authoritative implementation is `dsl/decoders/qspi-flash.ts`. Its schema, configuration defaults and localized labels are generated from that source.

Existing byte SPI NOR plus explicit 3/4-byte address and lane-aware flash transport.

The remaining scope is: Manufacturer-wide opcode database, protection policy and SFDP interpretation.

Reference: [Winbond JV rev F](https://www.winbond.com/resource-files/w25q128jv%20revf%2003272018%20plus.pdf).

Verification: Existing NOR suite; flash phase vectors. Test execution is recorded in the [SPI coverage ledger](spi-porting-coverage.md); authored vectors are not a passing-test claim.

Events carry sample provenance. Gaps discard incomplete fields; complete previously emitted fields remain observations. Buffers are bounded and no whole transaction is retained. See the coverage ledger for input word widths, capture wiring and shared limitations.

The explicit `command_mode` option selects SPI or QPI framing; it does not assume QE was set. `qpi_dummy_clocks` includes the two EBh mode clocks, while `dtr_quad_dummy_clocks` excludes the EDh mode clock. The original Winbond DTR diagrams require the SDR opcode phase to end before both-edge address/mode/data sampling.

Erase events are requests observed on the bus, not confirmation of a completed memory operation. Status fields use the Winbond register map. SFDP header and parameter-header fields stream without buffering a table; an observed FF00 basic-table pointer permits DWORD2 density decoding. Unsupported exponent encodings report an unknown density rather than overflowing. A gap forgets the discovered table pointer. SFDP format follows the manufacturer-authored [Microchip table8-1](https://onlinedocs.microchip.com/oxy/GUID-EF5727DA-42CC-4107-8984-4D8EAF42FF5D-en-US-1/GUID-0394DD94-818C-41EF-BD6C-0AB8FB863C2A.html).

The Winbond flash profile accepts clock modes 0 and 3. Independent mode-3 SDR and quad-DTR vectors pass in compiled and interpreted execution, including the SDR opcode-to-DTR boundary. Modes 1 and 2 are rejected.

The Winbond profile now follows complete observed commands at CS release:
B7h/E9h select four/three address bytes; SPI-specific13h/0Ch/3Ch/6Ch/BCh/ECh/
12h/34h layouts always carry32-bit addresses. QPI0Ch keeps its separate burst
read meaning and uses the current address mode. An exact66h followed directly
by99h returns the candidate command parser to SPI and its configured initial
`address_bytes` assumption. An intervening, unknown, partial or overlong command
disarms the reset sequence. Trailing clocks on commands with no payload are
reported as unsupported and never commit mode changes.

A completed Status Register2 read establishes observed QE. A completed38h
then emits `mode-request` and follows QPI only when observed QE is true. QE
false leaves SPI active; QE unknown emits the request and suspends subsequent
command interpretation (partial frames) because either width could now be in
use. Status writes remain requests and invalidate observed QE in SPI; they do
not prove a successful flash write. An observed QPI FFh exits to SPI. A gap
forgets QE and suspends phase interpretation until the decoder is restarted
with a deliberate `command_mode` assumption. The initial mode option remains
an explicit capture assumption, not a detected flash power-up state.

`address-mode` and `mode-request` report parser context and command requests,
not verified effects inside the device. The profile adds manufacturer/device
ID92h/94h lane phases, unique-ID4Bh dummy clocks, security42h/44h/48h,
extended-address C5h/C8h, lock36h/39h/3Dh, wrap77h and common suspend/power/
volatile-status-enable opcodes.30h and5Ch from broader command databases are
not assigned Winbond semantics without an applicable part specification.
Unique/security/extended/lock payloads remain observed raw bytes; no physical
memory contents or protection policy is reconstructed.

Primary additions: Winbond-authored [W25Q128FV §§6.1.4,7.1.10,8.2.36–37](https://www.pjrc.com/teensy/W25Q128FV.pdf)
for QPI and QE, and [W25Q256JV Rev.B §§6.1.4,8.1/8.2](https://e2e.ti.com/cfs-file/__key/communityserver-discussions-components-files/151/FLASH-datasheet.pdf)
for address-mode and extended-address operations. The latter is an unmodified
manufacturer datasheet hosted by TI. The GPL DSView Smart QSPI source was
consulted only for its implemented capability checklist; no code was copied.
Its `invalidlevel`/`twolinesmode` per-byte heuristics are not equivalent to this
phase-aware decoder's explicit `command_mode`, so such imported configurations
remain preserved with a diagnostic rather than being relabeled as QPI.
