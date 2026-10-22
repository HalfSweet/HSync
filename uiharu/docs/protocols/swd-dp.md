<!-- SPDX-License-Identifier: Apache-2.0 -->

# SW-DP and MEM-AP observations

`swd-dp` reads canonical `swd.events@1` packets from the existing SWD decoder.
The contract is now declared in `dsl/contracts/swd.ts`; generated and native
producers share the same Rust payload, including the externally tagged
`{"other":7}` ACK encoding. JSON and schema version remain unchanged.

| Event            | Meaning                                                                                                          |
| ---------------- | ---------------------------------------------------------------------------------------------------------------- |
| `access`         | Complete observed request/ACK/outcome, DP/AP register, optional APSEL and banked address, optional data          |
| `posted-result`  | A successful AP read or RDBUFF returned data for the earlier accepted AP read, linked by `tx`/`responseTx`       |
| `resend-result`  | DP RESEND's observed value, optional originating read identity and comparison with the last observed read result |
| `memory-request` | Configured MEM-AP's DRW request, candidate TAR, CSW size/packed fields and raw write data                        |
| `memory-result`  | Raw returned data and candidate address saved when the earlier DRW read was issued                               |
| `status`         | Observed CTRL/STAT and sticky fault bits                                                                         |
| `context-lost`   | Reset, switching, invalid packet, FAULT or abort made retained interpretation unreliable                         |

SELECT establishes APSEL/APBANKSEL/DPBANKSEL; before it is observed the bank
and selected AP are unknown. Writes to the configured `ap` (default 0) update
candidate CSW and TAR context. A DRW request advances candidate TAR according
to CSW SIZE and AddrInc: off preserves it, single increments by 1/2/4 bytes,
packed by four bytes, and reserved sizes/increment discard it. Increment wraps
within `tar_wrap_bits` (10–31, default 10). The configured AP is an assertion
that this AP uses the MEM-AP register layout; an IDR is not inferred.

SW-DP AP reads are posted. The first returned word has no known originating
read; a later accepted AP read returns the prior read, and RDBUFF returns the
last one without issuing another AP access. WAIT preserves the pending read
and candidate TAR. RESEND repeats the last AP/RDBUFF result without consuming
the pending read or advancing TAR. FAULT, request/data parity errors, aborted
packets, line reset and interface switches discard context. Sticky status
faults discard candidate CSW/TAR and pending results. Invalid request/data
fields are still visible in the underlying transport stream.

An OK ACK admits an AP request; AP operations can fail later. Candidate memory
addresses and raw data therefore do not assert a successful memory write or
verified memory contents. Packed data stays raw; byte-lane unpacking,
ADIv6 extended addresses, multiple cached MEM-APs and target-specific side
effects are outside this profile. Only one transport packet and one posted
read are retained. Output spans the packet's Begin through End and settles at
End; results link to earlier requests by identity rather than backdating events.

The native `swd::TxId` is now a `u64` alias rather than a tuple newtype. Native
`Ack::from_bits`/`bits` operations are supplied by reexported `AckExt`.
These source API adjustments allow canonical generated contract ownership;
wire values and physical behavior are preserved. The native `Field` still
implements Hash. Native `strict_start` defaults true; false enables its existing
request-header scanner at capture start and after input loss/protocol errors.

The decoder is independently written from [Arm IHI0031A §§5–6,8](https://documentation-service.arm.com/static/5f900a61f86e16515cdc0610?token=)
and [Arm IHI0074A B2.2.9/B2.2.10](https://documentation-service.arm.com/static/5f8ff91cf86e16515cdbfdf0),
which distinguish SW-DP posted reads/RDBUFF/RESEND from JTAG's response pipeline.
The GPL SWD reference was consulted only for its supported-feature checklist;
no source was copied or adapted. Tests independently exercise posting, WAIT,
FAULT, parity, reset, strict/non-strict start, RESEND, wrap behavior, interpreted
equivalence and streaming invariance.
