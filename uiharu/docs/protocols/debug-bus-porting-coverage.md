<!-- SPDX-License-Identifier: Apache-2.0 -->

> Backend follow-up: the previously recorded shared-output, fixed-array,
> hexadecimal-u64, numeric-enum and mixed-enum limitations have been implemented
> in the [DSL compiler and both execution paths](../dsl.md#21-types). The failure
> probes below are historical evidence from the initial P0–P4 checkpoint.
> The continuation now supplies cJTAG, SWD/AVR semantics, 1-Wire devices, CFP
> and AM230x values using canonical native/DSL event types. See the
> [current execution record](../../../docs/decoder-coverage.md#p0p4-continuation-after-the-backend-follow-up).

# Debug and common bus porting coverage

This ledger supplements the frozen inventory in `docs/decoder-coverage.md`.
A selectable node, an observed transport and application semantics are separate
claims. Rows marked pending remain work; a profile is not full upstream parity.
All new decoder logic is authored in TypeScript from public specifications.
Generated Rust is not a handwritten fallback.

## P2 debug and trace

| Inventory row  | Current node/profile                       | Implemented behavior and evidence                                                                                                                                                                                        | Remaining scope                                                                                                                                                        |
| -------------- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `jtag`         | `jtag` → `jtag-chain`                      | TAP state tracking, IR/DR scans, reset and bounded bit batches; explicit chain windows with exact complete-length validation; existing JTAG/JTAG-DP/MEMAP tests and four chain tests passed                              | Fixed DR-width profile; per-instruction DR layouts and topology/BSDL discovery remain                                                                                  |
| `jtag_stm32`   | `jtag-stm32`; generic chain/DP/MEM-AP path | STM32F1 two-TAP IR9/reset ID64, dynamic MCU DR widths with BSC BYPASS, BSC density IDs, MCU ID and DP/AP/ABORT fields; four profile tests and interpreted equivalence passed                                             | F1-specific, unsupported BSC instructions rejected; MEM-AP candidate addressing remains generic path, no verified memory effects                                       |
| `swd`          | `swd` → `swd-dp`                           | Existing physical packets plus SELECT banks, posted AP results/RDBUFF/RESEND, WAIT retention, sticky faults and candidate MEM-AP addresses; strict-start selectable                                                      | One configured MEM-AP, ADIv5 32-bit address profile; no proof of target memory effects                                                                                 |
| `swim`         | `swim` baseline; TS `swim-stm8`            | UM0470 framing, parity/ACK retries, reset/entry, command/count/address/data and delayed CSR speed observation; optional explicitly trusted response calibration; 8 native, 10 TS, 2 serde and 1 interpreted tests passed | Calibration defaults off because one wire cannot prove response direction; no inferred target memory/debug success or unavailable initial clock; scope in swim-stm8.md |
| `arm_itm`      | `arm-itm`                                  | Software/hardware, DWT event/exception/PC, local timestamps, 48/64-bit global timestamp fragments, extension/sync/overflow; five packet/error/invariance tests and interpreted equivalence passed                        | Global fragments remain observed components, no absolute-clock reconstruction or source attribution; additional DWT formats raw                                        |
| `arm_tpiu`     | `arm-trace-transport` (`format: tpiu`)     | Fixed formatter frames, delayed source-ID selection, auxiliary-bit reconstruction and full/halfword synchronization; five tests plus compiled/interpreted equivalence passed                                             | Serial UART byte transport only; selected-source filtering, no parallel pin or Manchester reconstruction                                                               |
| `arm_etmv3`    | `arm-etmv3`                                | Original/alternative branches and exception fields, cycle/noncycle atoms, I-sync/LSiP/context/VMID, data packets and timestamp fragments; four protocol tests, interpreted equivalence and focused clippy passed         | No ELF/objdump code-image disassembly, source/symbol/function mapping or fabricated instruction PCs; raw timestamps retain unknown high bits                           |
| `jtag_ejtag`   | `jtag-ejtag`                               | MIPS EJTAG IR capture, IDCODE/IMPCODE, ADDRESS/DATA/CONTROL/ALL96, FASTDATA33 and processor-access completion requests; six profile tests and interpreted equivalence passed                                             | Single TAP, configured IR width; no verified target memory effects, TCB/performance-register semantics or implementation-specific DMA                                  |
| `cjtag`        | `cjtag`                                    | OScan1 nTDI/TMS/TDO, short/standard activation, escape and canonical JTAG stream                                                                                                                                         | Explicit aligned-start assertion for captures without activation; unsupported control/scan formats remain outside the profile                                          |
| `cjtag-oscan0` | `cjtag` (`format: oscan0`)                 | Primary TI/NXP OScan0 ready/stall rules, RDYC1–4, fixed delay0–2, bounded waits and shared JTAG output                                                                                                                   | Variable-delay directives and wider control-level sequences are outside the configured profile                                                                         |
| `avr_isp`      | `avr-isp` → `avr-isp-device`               | Existing23 instruction classes plus complete signature assembly, named primary-documented devices and observed programming-enable echo                                                                                   | Additional MISO echo bytes absent from transport stay unknown; fuse/lock data remain raw                                                                               |
| `avr_pdi`      | `avr-pdi`                                  | All eight instructions, streamed REPEAT including 2^32, pointer context and BREAK recovery; two waveform/interpreted/invariance tests passed                                                                             | No NVM success inference, electrical collision detection or JTAG mode; Microchip XMEGA8077I/AVR1612 sources in avr-pdi.md                                              |
| `c2`           | `c2`                                       | All four instructions, variable WAIT bounded by configuration, reset detection; two waveform/interpreted/invariance tests passed                                                                                         | Conservative transport/profile limits documented in c2.md; Silicon Labs AN127 source                                                                                   |
| `fsi`          | `fsi`                                      | POWER9 basic transport ABS/REL/SAME, polls/TERM, CRC4, bounded four-slave address context and ACK/BUSY handling; two waveform/interpreted/invariance tests passed                                                        | Profile limits and OpenPOWER primary source in fsi.md; no unobserved retry success or unknown address inference                                                        |
| `aud`          | `aud`                                      | Renesas branch trace: sync-high8/9/A/B selects1/2/4/8 low-first nibbles; partial addresses retain unknown high bits; two waveform/interpreted/invariance tests passed                                                    | No inferred unknown initial high address bits; short/overlong/gaps lose context; primary references in aud.md                                                          |

## P3 common buses

These are concrete existing capabilities, not a declaration of upstream option
or annotation parity. The corresponding protocol documents and integration test
files describe their vectors and timing choices.

| Inventory row        | Actual node/profile                       | Current semantic depth                                                                                                                                                                                                                                                       | Remaining work                                                                                                                                                                                                                                                                                                                                                                                                |
| -------------------- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `modbus`             | UART → `modbus-rtu` → `modbus-pdu`        | RTU CRC/framing plus explicit request/response fields for FC1–8,11,12,15,16,17,20,21,22,23,24,MEI43/14 and exceptions; streamed file records, FIFO counts, diagnostic words and device-identification objects; generated/interpreted and scheduling vectors in modbus_pdu.rs | No request/response pairing, vendor register map, full diagnostic/event-bit interpretation or CANopen MEI13. Unknown MEI types remain raw and unsupported; structural/transport validity is distinct from device execution.                                                                                                                                                                                   |
| `ps2`                | `ps2` → `ps2-device`                      | Canonical host/device frames; explicit keyboard scan sets1/2/3, Pause/PrintScreen, standard/8-bit-wheel/Explorer4-bit-wheel mouse profiles, signed movement/overflow, button masks and bounded host ACK/parameter/query/reset reply contexts; vectors in ps2_device.rs       | Configured profiles do not auto-switch from device IDs. No language layout, pressed-key map, unsupported side effects or shortened modifier-dependent PrintScreen forms. Framing/gaps clear context; see ps2-device.md.                                                                                                                                                                                       |
| `i2c`                | `i2c` → `smbus`                           | Physical I2C addressing/ACK/repeated START/STOP plus TS 17 explicit SMBus transaction profiles, ACK/count/direction validation and CRC8 PEC; five overlay tests passed                                                                                                       | Physical SMBus clock timeout requires a timing-aware profile; combined-format/device-specific application payloads remain raw, details smbus.md                                                                                                                                                                                                                                                               |
| `can`                | `can`                                     | Classic standard/extended frames, ISO CAN FD, CRC15/17/21, stuff count/parity, form/stuff errors and BRS configuration                                                                                                                                                       | Fractional sample-point and raw bit annotations; non-ISO FD/CAN XL outside profile; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                             |
| `can-fd`             | Same `can` node                           | FD-enabled profile shares actual CAN transport, not a second independent implementation                                                                                                                                                                                      | Shared CAN profile, same limits; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                                                                                |
| `dmx512`             | UART → `dmx512` or `dmx512-analysis`      | Native slot stream plus direct-UART TS BREAK/MAB/MARK, BREAK-to-BREAK/RESET-to-RESET measurements and bounds, dimmer zero filter, alternate start-code identification and excess-slot diagnostics; p3_semantics.rs vectors                                                   | Alternate-code application payloads remain raw; no individual-bit annotation stream. Timing requires per-character UART placement rather than native batched-slot output; [current audit](existing-bus-audit.md).                                                                                                                                                                                             |
| `lin`                | UART → `lin` or `lin-profile`             | Native checks plus TS enforced classic/enhanced version, diagnostic classic exception, optional LIN1.x identifier lengths and silence-deadline completion using timestamps/progress/EOF; p3_semantics.rs vectors                                                             | No LDF signal map, diagnostic application payload or maximum frame-time analysis. Idle duration is a configured analysis threshold; no UART idle-event dependency; [current audit](existing-bus-audit.md).                                                                                                                                                                                                    |
| `mdio`               | `mdio` → optional `cfp`                   | Clause22/45 framing, preamble suppression, selected rising/falling edge, turnaround diagnostics and canonical validated numeric clauses; CFP adds configured-port DEVAD1 address/data pairing, increment, regions and ID                                                     | Native MDIO still exposes generic frames rather than all-device address-pair state. CFP pairing is its explicit device profile. Raw bit/idle debug rows remain absent; [current audit](existing-bus-audit.md).                                                                                                                                                                                                |
| `microwire`          | `microwire`                               | Clock/chip-select transport and configurable command/data words                                                                                                                                                                                                              | No start-bit/selection-clock warning or every ready/busy transition; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                                            |
| `midi`               | UART → `midi` → optional `midi-semantics` | Native message/running-status/realtime/SysEx transport plus note/GM percussion/controller names, pitch bend, song-position clocks, MTC quarter-frame fields and bounded manufacturer-ID subset; p3_semantics.rs vectors                                                      | No complete vendor catalog, full time-code reconstruction, retained instrument state, tuning or vendor SysEx application interpretation; [current audit](existing-bus-audit.md).                                                                                                                                                                                                                              |
| `onewire_link`       | `onewire-link`                            | Reset/presence and standard/overdrive slots with recovery                                                                                                                                                                                                                    | No observed command-driven speed switching or DSView pulse/order options; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                                       |
| `onewire_network`    | `onewire`                                 | ROM commands, ROM CRC, search triplets and selection context                                                                                                                                                                                                                 | Resume/other ROM commands raw; no retained selection across resets; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                                             |
| `uart`               | `uart`                                    | 5–9 bits, inversion, bit order, stop lengths, parity, break, sample point; generated/interpreted existing equivalence                                                                                                                                                        | No wide/4-bit characters, unusual stop lengths, ignore parity, IDLE or packet summaries; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                        |
| `usb_packet`         | `usb`                                     | Low/full-speed NRZI, stuffing, token/SOF/data/handshake fields and CRC, reset/suspend/signalling observations                                                                                                                                                                | No auto speed/PRE-followed low-speed, raw symbols or payload-limit enforcement; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                                 |
| `usb_signalling`     | Integrated `usb` signalling               | Physical line/signalling events share packet node                                                                                                                                                                                                                            | Signalling integrated; raw-symbol stream and auto/PRE profiles absent; [completed comparison](existing-bus-audit.md)                                                                                                                                                                                                                                                                                          |
| `usb_power_delivery` | `usb-pd`                                  | BMC/4b5b/ordered sets, headers, CRC32, messages, fixed/battery/variable/PPS PDO fields, raw RDO quantities and bounded extended bytes; native physical/packet engine retained                                                                                                | Exact TS semantic input is blocked by flattened records plus flattened PowerObject union using discriminator supply rather than kind. No schema-changing adapter/native fallback. RDO/PPS/AVS context, VDM/BIST and chunks await that contract capability; [minimal probe](usb-pd.md#typescript-semantic-consumer-boundary-2026-10-22). Requests/control transfers/descriptors are separately scope-deferred. |
| `usb_request`        | Deferred by explicit user scope           | No USB request/descriptor implementation in this phase                                                                                                                                                                                                                       | Preserve imported configuration with diagnostics; do not count as implemented                                                                                                                                                                                                                                                                                                                                 |

## Historical backend probes and supported source workarounds

The contract failures in this section were observed at the initial P0–P4
checkpoint and are resolved by the authorized backend follow-up. Their original
failure diagnostics are retained as historical evidence. The source workarounds
remain valid where stated; the current protocol rows above govern implemented
capabilities. USB PD's different, still-current flatten/discriminator blocker is
recorded in its own protocol page.

- **Exact SWD ACK declaration:** the existing JSON ACK is a union of strings
  (`ok`, `wait`, `fault`) and an externally tagged object (`{"other":7}`).
  A minimal TS declaration `type Ack='ok'|'wait'|'fault'|{other:u8}` used by
  `eventSchema<{kind:'ack';tx:u64;value:Ack}>` fails lowering with
  `the type Ack has no IR type; name a record or give an integer its width`.
  This blocks an exact typed consumer of the current SWD contract, not the
  existing physical decoder or JTAG-based MEM-AP interpretation.
- **Imported output contracts:** using an imported schema as a decoder output
  fails generation with `the output's contract is not declared here`; a local
  schema over an imported event alias fails validation because the alias is not
  a local union. A coherent common raw/TPIU trace-transport producer owns the
  generic byte/error contract instead. No duplicate native payload type is
  connected by matching JSON shape alone.
- **Switch break scope:** a `break` inside a TypeScript switch nested in a loop
  currently emits a Rust loop break. Affected I²C consumers use explicit
  continues for item dispatch; the Modbus switch helper returns directly and
  ARM trace/MEM-AP dispatch use if guards. This is recorded for compiler design
  follow-up; no compiler/interpreter semantics were changed here.

## Historical cJTAG shared-output reproduction

The following diagnostic probe is **not** a physical cJTAG implementation. It
isolates the ability a physical decoder needs: emitting the existing generic
JTAG contract for typed JTAG-DP/chain consumers. It was lowered virtually as
`cjtag-output-probe.ts`; no decoder or compiler source was changed for the probe.

```ts
import type { Channel } from 'uiharu'
import { LogicDecoder, defineLogicDecoder, mask, at } from 'uiharu'
import { jtagEvents, type JtagEvent } from './jtag-chain'
interface Config {
  clk: Channel
  data: Channel
}
class Probe extends LogicDecoder<Config, JtagEvent> {
  readonly watch = mask(this.config.clk, this.config.data)
  override async run(): Promise<void> {
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      await this.out.emit({ kind: 'reset', by: 'tms' }, at(edge.tick))
    }
  }
}
export const probe = defineLogicDecoder({
  name: 'cjtag-output-probe',
  category: 'bus',
  config: { defaults: { clk: 0, data: 1 } },
  output: jtagEvents,
  decoder: Probe,
})
```

The frontend returned a module with no diagnostics. Codegen over its IR plus
`jtag-chain.json` failed with:

```text
cjtag-output-probe: the output's contract is not declared here
```

The shared contract already exists; declaring an additional private payload with
the same JSON fields does not make its native type interchangeable. A
handwritten adapter or compiler/runtime change is outside the authorized
TS-only workaround policy. Consequently neither cJTAG row is counted as a
working node. This is independent of the separate task of establishing all
OScan0 stall and phase rules from a primary source.

Primary wire evidence consulted:

- [Arm, Low Pin-count Debug Interfaces for Multi-device Systems](https://developer.arm.com/-/media/Arm%20Developer%20Community/PDF/Low_Pin-Count_Debug_Interfaces_for_Multi-device_Systems.pdf?hash=EB46B18CAE090A1EB7458F382936AB7C1216A242&la=en&revision=895bec7a-7373-45d6-9f40-2c63bed5422d),
  describing the T4 two-wire OScan1 format.
- [SEGGER J-Link cJTAG specifics](https://kb.segger.com/J-Link_cJTAG_specifics),
  describing OScan1 activation and TMSC release/escape behavior.
- [TI CC2538 SWRU319C §§6.2–6.3](https://www.ti.com/lit/ug/swru319c/swru319c.pdf),
  describing supported scan formats, stalls and STFMT command-window selection;
  it does not establish a complete OScan0 data-phase decoder here.

## Primary references

- Arm IHI0031A §§4.4–4.5, Tables 4-5/4-6, §§6/8/11:
  <https://documentation-service.arm.com/static/5f900a61f86e16515cdc0610?token=>.
  JTAG AP accesses have one accepted-scan response pipeline; DP RDBUFF shifts
  the final prior result. This is distinct from SW-DP posted AP reads.
- Arm DDI0553B.y Part F1, ITM/DWT packet tables:
  <https://community.arm.com/cfs-file/__key/communityserver-discussions-components-files/471/DDI0553B_5F00_y_5F00_armv8m_5F00_arm.pdf>.
- Arm IHI0029D Chapter D4, CoreSight formatter:
  <https://documentation-service.arm.com/static/5f9009d5f86e16515cdc0417?token=>.
- MODBUS Application Protocol V1.1b3 §§6–7:
  <https://modbus.org/docs/Modbus_Application_Protocol_V1_1b3.pdf>.
- ST UM0470 Rev4 §§3.2–3.6, SWIM:
  <https://www.st.com/resource/en/user_manual/cd00173911-stm8-swim-communication-protocol-and-debug-module-stmicroelectronics.pdf>.

The per-protocol pages retain their own primary references. An unverified option
or pending row remains a scope gap until its implementation and evidence exist.

## Historical AVR ISP capability audit

The continuation now adds [avr-isp-device](avr-isp-device.md) with exact tuple
contract ownership. The original audit below records why it was deferred; the
remaining unavailable echo bytes are still not inferred.

The existing `avr-isp` has no options of its own and consumes eight-bit MSB-first
SPI with both lines. Its 23 named instructions cover programming enable, erase,
ready/busy polling, extended addressing, flash/EEPROM page and byte reads/writes,
signature, calibration, fuse and lock accesses. This was checked against
[Microchip ATmega328PB Table 32-17](https://onlinedocs.microchip.com/oxy/GUID-0EC909F9-8FB7-46B2-BF4B-05290662B5C3-en-US-12.1.1/GUID-737FF6A5-3611-41DC-AC95-55D5B0568D27.html).
Flash addresses are observed word addresses; the decoder does not invent a
byte address or combine an unobserved extended-address setting.

The [pinned upstream AVR ISP decoder](https://github.com/sigrokproject/libsigrokdecode/blob/71f451443029322d57376214c330b518efd84f88/decoders/avr_isp/pd.py)
was consulted only for its capability checklist, not as an algorithm source.
Its recognized command categories are covered by the existing native node;
it additionally labels vendor/device names from signature sequences and emits
several reply-echo warnings. Those names and warnings are absent here. Both
leave fuse/lock bit interpretation largely as raw data. Upstream has no
configurable option requiring a separate import translation. HSync additionally
reports bounded partial-command errors and explicitly reacquires grouping from
Programming Enable after capture loss. There is no claim of annotation parity,
programming success, page-state tracking, old AT90S variants, TPI or UPDI.
The missing signature naming and additional reply warnings are feasible semantic
work, not an IR limitation. Existing `tests/avr_isp.rs` covers commands, recovery,
truncation and scheduling; no new AVR tests were run for this read-only audit.

The attempted exact AVR ISP contract declaration has an actual fixed-array
blocker. The native instruction event carries `bytes: [u8; 4]`; its TS declaration
was probed as follows, without changing any producer or compiler:

```ts
import type { u8, u64 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type InstructionBytes = [u8, u8, u8, u8]
export type Event = { kind: 'instruction'; op: u64; bytes: InstructionBytes }
export const events = eventSchema<Event>('avrisp.events', 1).roles({
  instruction: 'command',
})
```

Lowering returned no diagnostics but omitted the tuple declaration. Codegen
rejected the resulting IR with
`types[0].variants[0].fields[1]: type InstructionBytes is not declared`.
Replacing it with a bounded vector changes the native payload and loses the
exact four-byte contract; that is not a mechanical declaration of the current
producer. The native decoder is retained. A typed signature-name consumer is
therefore blocked on exact fixed-array declarations. Additional reply-echo
warnings also require MISO bytes that the current event does not expose; no
successful programming or missing echo is inferred from absent bytes.
