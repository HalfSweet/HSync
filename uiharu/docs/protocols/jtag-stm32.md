<!-- SPDX-License-Identifier: Apache-2.0 -->

# STM32F1 JTAG chain

`jtag-stm32` consumes `jtag.events@1` with both TDI and TDO. Its empty configuration
selects the STM32F1 geometry documented by [ST RM0008 Rev21 §§31.5–31.7](https://www.st.com/resource/en/reference_manual/rm0008-stm32f101xx-stm32f102xx-stm32f103xx-stm32f105xx-and-stm32f107xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).
The physical chain is JTDI → five-bit boundary-scan TAP → four-bit MCU TAP → JTDO;
captured shift order therefore contains MCU bits first. This is a specific
STM32F1 profile, not a topology detector for every STM32 family.

A completed nine-bit instruction scan commits both instructions only when both
IR capture fields contain `01`. Observed reset selects MCU IDCODE and BSC IDCODE,
so the initial data scan contains two 32-bit identifiers. MCU IDCODE uses 32 bits,
BYPASS one, and DPACC/APACC/ABORT 35. BSC IDCODE uses 32 bits and BSC BYPASS one.
The decoder checks the complete total width, including the unused TAP. Unsupported
BSC instructions, unknown MCU instructions and simultaneous 35-bit MCU access
with 32-bit BSC IDCODE are explicitly rejected. Ordinary debug access uses BSC
BYPASS as required by RM0008 §31.5.

Identifiers retain raw value, version, part and manufacturer fields. Known BSC
parts identify low, medium, high, XL and connectivity-line density using the
RM0008 §31.6.2 table. Version remains the observed numeric nibble; no fabricated
revision-letter mapping is attached. MCU DP ID is distinct from memory-mapped
DBGMCU_IDCODE and its DEV_ID field.

DP/AP access events expose captured ACK bits, submitted request fields and the
previous accepted scan identity, following [Arm IHI0031A §4.4](https://documentation-service.arm.com/static/5f900a61f86e16515cdc0610?token=).
WAIT retains the pending accepted request and discards the current request;
OK/FAULT records acceptance without asserting a successful memory operation.
ABORT retains the 32-bit submitted word and three low scan bits. This node does
not interpret every AP register or infer memory effects. Compose the generic
`jtag-chain → jtag-dp → arm-memap` path when MEM-AP candidate addressing is needed.

The implementation stores two 64-bit bit accumulators and bounded scalar state;
it never retains a complete transaction. No output is committed from a partial
scan. Gaps, malformed offsets, missing lines and interrupted scans invalidate
instruction and response context. Source provenance covers the completed scan;
gap/end aborts stop at the known boundary.

Independent tests use RM0008 IDs and geometry, dynamic DR widths, WAIT pipelining,
ABORT, invalid IR capture/length/profile and block/gap/budget invariance. An
interpreted test compares complete events and provenance with generated Rust.

The [pinned sigrok `jtag_stm32` decoder](https://github.com/sigrokproject/libsigrokdecode/blob/71f451443029322d57376214c330b518efd84f88/decoders/jtag_stm32/pd.py) was consulted only for a capability checklist
(two TAPs, device IDs, DP/AP registers). Its GPL implementation was not copied or
used as the implementation model; the geometry and algorithms above come from
ST and Arm specifications.
