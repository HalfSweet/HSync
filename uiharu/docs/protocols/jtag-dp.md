# Arm JTAG-DP

The `jtag-dp` node reads one Arm ADIv5 Debug Port TAP from `jtag.events`.
Its source is [Arm IHI0031C, ADIv5.0–ADIv5.2, chapter 3](https://e2e.ti.com/cfs-file/__key/communityserver-discussions-components-files/538/IHI0031C_5F00_debug_5F00_interface_5F00_as.pdf),
especially §§3.3–3.4, table 3-2, table 3-5 and figures 3-7/3-8.
This is transport decoding, with no CPU register, memory-access or device-family interpretation.

## Input and configuration

Connect a `jtag` node's `events` output to `jtag-dp.bus`. Both `tdi: true`
and `tdo: true` are required. Configuration is `{}`; unknown fields are rejected.
Only a single TAP with a four-bit instruction register is supported. A chain
containing additional TAPs, including MCU configurations exposing extra TAPs,
must be separated before this decoder can interpret its scans. No offsets,
IR-length discovery or device-family assumptions are made.

## Contract

Schema `jtag-dp.events`, version 1; output properties `singleTap: true`, `irBits: 4`.
Each scan produces one event after its normal or abnormal end, with provenance
covering consumed JTAG scan events through the deciding end. No complete scan
is buffered: only the first 35 bits of each line, a saturating bit count,
the current instruction and a previous scan id are retained. Overlong scans
are consumed and rejected by length. Backpressure and cancellation use the
runtime's awaited reads and writes. Progress during scans is held by `JtagBus`
until the scan event is emitted.

| Event         | Fields                         | Meaning                                                                       |
| ------------- | ------------------------------ | ----------------------------------------------------------------------------- |
| `reset`       | none                           | Observed TAP reset selects IDCODE, loses access association                   |
| `instruction` | `scan`, `value`                | A complete four-bit Update-IR committed this instruction                      |
| `idcode`      | `scan`, `value`                | Captured 32-bit IDCODE; no manufacturer lookup                                |
| `bypass`      | `scan`, `tdi`, `tdo`           | A complete one-bit BYPASS scan                                                |
| `abort`       | `scan`, `value`, `lowBits`     | 35-bit ABORT: high 32 bits and raw low three bits                             |
| `access`      | see below                      | Current shifted-in request plus separate captured response                    |
| `scan`        | `scan`, `ir`, `bits`, `status` | Short WAIT, aborted, invalid-length, missing-line or unknown-instruction scan |

Instruction values are ABORT `8`, DPACC `10`, APACC `11`, IDCODE `14`, BYPASS `15`.
The initial instruction is unknown until a four-bit Update-IR or an observed
reset. Reset selects IDCODE according to the architecture. Gaps lose instruction
and pipeline knowledge. Unsupported instructions remain unknown; they are not
asserted to be protocol errors. An incomplete IR scan does not select the value
partly observed on TDI. The decoder conservatively loses instruction knowledge.

## Access fields and pipelining

For a 35-bit DPACC/APACC scan, TDI bit 0 is `read`; bits 2:1 become `addr`
0, 4, 8 or 12; bits 34:3 are raw `dataIn`. `instruction` distinguishes DPACC
and APACC; no SELECT/bank resolution is attempted. `dataIn` is ignored by the
device for an ordinary read, but is retained as a wire fact.

TDO bits 2:0 are `ackBits`, interpreted as `ack: wait` for 1, `ok-fault` for 2,
or `invalid` otherwise. TDO bits 34:3 are raw `responseData`. The response belongs
to a previous request, **not the request on TDI in this scan**. `previousScan`
identifies the last observed request admitted at Update-DR, or is null when
association is unknown. WAIT preserves that association and discards the
current shifted-in request (`accepted: false`). OK/FAULT admits the current
request (`accepted: true`) and replaces the association for the next scan.
Reserved ACK loses association and gives `accepted: null`.

`accepted` describes transport admission only: OK/FAULT does not distinguish
successful operations from faults or sticky-error-discarded AP operations.
`responseData` may be unknown on WAIT, after writes, faults, or before a known
preceding request. It is never advertised as a successful read value. ABORT,
reset, malformed scans, gaps and selection of IDCODE/unsupported instructions
clear association; BYPASS and DPACC/APACC instruction changes preserve it.
No sticky-state, pushed-verify/compare or AP memory transaction model is kept.

A normally completed DPACC/APACC scan with at least three bits and nonstandard length whose captured ACK is WAIT
reports `scan` with `status: wait`, preserving the previous request association.
It emits no invented full request or response data. This permits early WAIT
polling as described in [Arm Cortex-M1 TRM DDI0413C §9.3, pages 9-17 and 9-20](https://documentation-service.arm.com/static/5e8e180688295d1e18d357ad).
Nonstandard-length scans with other ACKs remain invalid-length.
