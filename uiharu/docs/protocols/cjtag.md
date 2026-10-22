<!-- SPDX-License-Identifier: Apache-2.0 -->

# Compact JTAG OScan0 and OScan1

The TypeScript `cjtag` decoder emits the canonical `jtag.events@1` contract,
including TAP states, resets, IR/DR scan identities, incremental one-bit shift
batches and complete/aborted scan ends. Its output connects directly to
`jtag-chain`, `jtag-dp`, `jtag-stm32` and `jtag-ejtag`; its payload type is owned
by the same DSL declaration as the native JTAG transport.

`clock` and `data` select TCKC/TMSC. Each rising TCKC samples a physical bit.
OScan1 carries nTDI, TMS and TDO, in that order. OScan0 inserts a variable RDY
phase before TDO; a zero stalls the packet. `ready_ones` (1–4, default 1)
selects the consecutive high RDY bits specified by RDYC + 1. TAP state and
shift offsets advance only when the packet completes. `delay_cycles` selects
zero, one or two fixed delay clocks after TDO. Variable delay directives and
other OScan formats are outside this profile.

By default, the decoder waits for observed activation. Six/seven TMSC
transitions while TCKC stays high start selection; eight or more return to
standard protocol and abort any open scan. The documented short activation
OAC `0xC`, EC `0x8`, four zero check bits selects OScan1. The all-zero long
selection and the documented level-2 command sequence (zero-bit scans, a
one-bit lock, counted STFMT opcode 3 and parameter 8/9 with zero-bit return
paths, minimum zero check packet) select OScan0/1. Other control command
sequences and nonzero CP preamble/postamble combinations are not inferred.
Control-level traffic is not proof of an instruction reaching a particular
internal device TAP.

For a capture already known to begin at nTDI, `start_aligned: true` selects the
configured initial `format` (`oscan1` by default). The initial TAP state is
Test-Logic-Reset; a known initial phase is a user assertion. A gap loses phase
and requires observed activation again. Five virtual clocks with TMS high
report reset. Pause/Exit2 retain the open scan until Update; reset, escape,
input loss or end abort it. `max_stall_cycles` (default 4096, at least 4) bounds
RDY waiting; exceeding it aborts the scan and requires activation.

Shift events span the nTDI through completion clocks and settle when the
packet is complete. At most one physical packet and counters are held;
complete scans are never buffered. Tests cover short/standard activation,
OScan0 ready stalls and repetitions, fixed delays, escape/loss recovery,
compiled/interpreted equivalence and arbitrary block/queue schedules.

Sources are wire specifications, not copied implementations:

- [Arm, Low Pin-count Debug Interfaces for Multi-device Systems, Fig.4](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Low_Pin-Count_Debug_Interfaces_for_Multi-device_Systems.pdf), OScan1 nTDI/TMS/TDO and rising-edge sampling.
- [SEGGER J-Link cJTAG specifics](https://kb.segger.com/J-Link_cJTAG_specifics), short/standard activation and high-clock escape behavior.
- [TI CC2538 SWRU319C §6.3](https://www.ti.com/lit/ug/swru319c/swru319c.pdf), explicit zero-bit scan, command lock and STFMT 8/9 selection.
- [TI CC13x0/CC26x0 SWCU117I §§5.2–5.2.1, Tables5-3/5-4](https://www.ti.com/lit/ug/swcu117c/swcu117c.pdf), OScan0 payload order, RDYC and fixed DLYC.
- [NXP MPC5777C Reference Manual Rev.8 §§56.7.2.3.6–7,56.8.8](https://community.nxp.com/pwmxy87654/attachments/pwmxy87654/mpc5xxx/19572/1/MPC5777CRM%2C%20MPC5777C%20Reference%20Manual.pdf), completed-packet state changes, ready/delay control and scan-format table.

The pinned DSView `cjtag/pd.py` was consulted for its feature checklist only.
No GPL code or tables were copied. The formerly recorded shared-output backend
blocker is resolved. OScan0's previously missing primary phase/stall evidence
is now supplied by the TI/NXP references above; it is no longer a blocker for
this explicitly configured profile.
