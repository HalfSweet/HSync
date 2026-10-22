# Decoder coverage and execution plan

Audit date: 2026-10-22. This is a capability inventory, not a declaration of
DSView or sigrok feature parity. The implementation baseline is HSync's 37 built-in
uiharu registry nodes before this batch, at commit
`81877fa066f50baeb57b180ca231700a3d7703e8`.

## Sources and method

The initial inventory used upstream directory metadata and public catalogue
information. During the subsequently authorized P0–P4 expansion, reference
implementations are also consulted for wire behavior and feature comparison.
New decoders follow [the source and implementation rules](../uiharu/docs/writing-decoders.md):
cite public specifications and the reference sources actually used, write the
implementation in the existing TypeScript DSL, and preserve the Apache-2.0 boundary.

- [DSView decoder directories](https://github.com/DreamSourceLab/DSView/tree/2e9e2c8e726df4ef5687d39b83d4f797cc44b574/libsigrokdecode4DSL/decoders),
  commit `2e9e2c8e726df4ef5687d39b83d4f797cc44b574`: 151 directories.
- [Upstream libsigrokdecode directories](https://github.com/sigrokproject/libsigrokdecode/tree/71f451443029322d57376214c330b518efd84f88/decoders),
  commit `71f451443029322d57376214c330b518efd84f88`: 132 directories.
- The [official sigrok protocol catalogue](https://sigrok.org/wiki/Protocol_decoders)
  also reports 131 supported decoders, consistent with excluding `common` from
  the directory count. Its work-in-progress entries are not treated as implemented.
- HSync: [registry](../uiharu/crates/uiharu-registry/src/kinds.rs),
  [protocol documentation](../uiharu/docs/protocols/),
  [application catalogue](../src/ipc/mock/fixtures/decoder-catalog.json).

DSView's `0-i2c`/`1-i2c`, `0-spi`/`1-spi` and
`0-uart`/`1-uart` are counted once per protocol family. Both `common`
directories and DSView's `example` are excluded. This leaves 146 DSView names,
131 upstream names and **148 distinct inventory names** in their union.
Overlapping families such as `max7219`/`max72xx` and
`gpib`/`ieee488` remain separate inventory entries; this is not a count
of independent protocol standards.

**37 inventory entries have related HSync implementations; 111 have no dedicated
built-in node at the baseline.** Related includes combined layers, narrower
device families and incomplete functionality. It does not mean equivalent:
`rgb_led_spi`, for example, is only related to the APA102 node.
Neither directory names nor matching names prove option, timing, command,
device-family or annotation parity. All rows below retain that distinction.

## Integration and depth gaps at the audit baseline

At the baseline, the engine registry, backend catalogue and frontend fixture each
contain 37 nodes. The application can configure 23 direct logic nodes and 13 immediate stacked
nodes: 36 in total. `ds18b20` requires
`onewire-link → onewire → ds18b20`, but the add/edit dialog currently exposes
only a base decoder and one upper layer. The backend and `buildEntry()`
already accept deeper graphs. Supporting arbitrary compatible stack depth is a
separate application task, especially before adding USB request/device layers.

DSView session import is another integration gap: the current
[`dsview::convert` adapter](../crates/hsync-session/src/dsview.rs) translates
I2C (with a 24xx EEPROM), UART, SPI and CAN/CAN-FD configurations only.
Other decoder configurations were omitted when opening `.dsl`/`.dsc` files,
even where a related uiharu node existed. P0 now retains original configurations
and diagnostics; translated device profiles remain narrower than the source and
are identified in the import notes. The historical baseline below is unchanged.

Existing nodes also need work beyond adding names:

- SPI NOR: dual/quad/QPI/DTR transfer phases and richer status/SFDP interpretation.
- SD SPI: native SD is a separate bus; CSD/CID/OCR/SCR values remain raw.
- USB: request/control-transfer/descriptor interpretation; high speed and PRE
  traffic are outside the current physical decoder.
- Modbus RTU: function-specific PDU decoding.
- I2C: SMBus-specific PEC and semantics.
- PS/2: keyboard and mouse interpretation.
- JTAG: device chain selection and higher-level debug access.
- CAN: fuller timing/resynchronization behavior.

`ds18b20` and `usb-transactions` are useful HSync nodes without a one-to-one
directory counterpart in this comparison. One HSync node can cover several
inventory entries, and one upstream entry can span several HSync layers.

## First implementation batch (historical)

The first batch added four registered nodes, bringing the catalogue from 37 to 41.
The baseline matrix below remains unchanged so that progress can be compared with
the original audit. These additions address parts of four missing inventory
entries; they do not close the full parity gap.

| New node                                             | Implemented scope                                                                        | Still outside this batch                                                                     |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| [`qspi`](../uiharu/docs/protocols/qspi.md)           | Fixed 1/2/4-lane SDR words, modes 0–3, CS framing and partial words                      | Automatic flash phases, dummy-cycle suppression, QPI/DTR and smart QSPI                      |
| [`sdcard-sd`](../uiharu/docs/protocols/sdcard-sd.md) | Native CMD commands/responses, CRC7, optional 1/4-bit SDR read blocks and per-lane CRC16 | DAT writes/busy/status, SDIO/DDR, automatic width/length negotiation                         |
| [`swim`](../uiharu/docs/protocols/swim.md)           | STM8 packets, parity/ACK, entry/reset, ROTF/WOTF count/address/data and speed state      | Target clock recovery, CPU/register semantics and flash programming algorithms               |
| [`jtag-dp`](../uiharu/docs/protocols/jtag-dp.md)     | Arm ADIv5 single-TAP instructions and pipelined DPACC/APACC scan fields                  | Multi-TAP chain selection, STM32-specific registers, MIPS EJTAG and AP memory reconstruction |

All four include synthesizers, protocol tests and block/queue/budget acceptance
cases, registry metadata, application catalogue entries and English/Chinese copy.
At that point the decoder editor still had a two-layer limit. P0 subsequently
removed that limit; current execution evidence and the family ledgers below
describe the later work.

## Execution plan

The owner selected foundational buses and debugging first and requested continuous
execution without asking for priorities after each small batch. This plan replaces
the earlier suggestions with a fixed queue. The current four additions remain
partial foundations; 41 registered nodes do not mean 41 completed inventory rows.

**Current execution scope:** P0 through P4, authorized on 2026-10-22. USB request,
control-transfer and descriptor layers are deferred; existing USB transport and
USB PD remain in scope. New decoders use the existing TypeScript DSL. If a protocol
requires changes to the IR, code-generation backend, interpreter or runtime,
record a reproducible language/backend blocker and continue another protocol.
Do not add a handwritten Rust fallback. Ordinary generated IR/Rust output and new
contract declarations supported by the existing toolchain are allowed. DSView and
sigrok implementations may be consulted with source attribution; copying code
must respect the retained Apache-2.0 boundary. These owner instructions supersede
the earlier clean-room and USB-request milestones below.

### Goal, scope and tracking

Work through all 148 entries at the pinned source revisions. DSView compatibility
has priority within a phase, with upstream-only entries included in the same queue.
An entry may share an implementation with another entry; adding one node per
directory is not a goal. The baseline mapping below is historical and must not be
rewritten to inflate progress. Its **Phase** column assigns every entry to exactly
one delivery phase, P1–P6; P0 and P7 apply across the inventory.

For each entry, record the following in its protocol page, or a coverage note here
until a protocol page exists: primary specification and revision, required channels,
modes/options, output fields and semantics, dependencies, actual node mapping,
remaining gaps, independent vectors and verification evidence. Public upstream
catalogues and reference implementations may help establish the feature checklist;
record consulted sources and keep copied/adapted code distinct from reference.

Freeze each capability checklist before implementation, including required variants
and the rationale for any exclusions. Discovering a larger requirement adds visible
work; it must not be silently excluded to match an already-written decoder. Missing
required inventory features keep the entry Partial or Blocked.

Use these states separately from the historical baseline:

- **Queued**: assigned a phase; detailed requirements still need research.
- **Specified**: primary sources, capability checklist and test vectors identified.
- **Partial**: some required functionality works; missing features remain explicit.
- **Integrated**: the specified capability is implemented and connected to the app.
- **Verified**: all applicable delivery gates below pass, with evidence recorded.
- **Blocked**: the existing TS/IR/backend cannot express a required operation, or a
  specific external prerequisite is missing; evidence and the next action are
  recorded. This state does not count as implemented or complete.

The first four nodes are Partial. Existing related entries require a capability
audit; none becomes Verified merely because a name or lower-level transport exists.
The coverage target concerns protocol analysis and session compatibility, not
hardware programming, oscilloscope/DAQ support or implementing every extension of
every protocol standard. Unobservable electrical behavior or missing capture
bandwidth must be reported as a limitation, never inferred as valid bus data.
Any feature-level difference from the pinned inventories remains visible.

### Ordered milestones

| Phase                                    | Work and delivery boundary                                                                                                                                                                                                                         | Dependency and exit evidence                                                                                                                                                               |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| P0 — Application foundations             | Depth-independent decoder configuration, stable node instance IDs, safe editing of existing graphs, explicit port compatibility, preservation and diagnostics for unsupported imported configurations; establish the per-entry capability records. | Begins first; protocol research can run alongside it. Three-layer graphs survive add/edit/save/reopen without losing nodes, connections, config or exposed outputs.                        |
| P1 — Finish SPI/QSPI and SD              | Phase-aware flash decoding, NOR semantics and native SD read/write/status/configuration; close the current foundation gaps against declared device/mode profiles.                                                                                  | Uses existing transport work. Complete flash and SD workflows, malformed/gap recovery, known vectors and application/import gates pass.                                                    |
| P2 — Debug and trace                     | JTAG chain selection, Arm DP/AP and memory-access interpretation, SWIM recovery, cJTAG, EJTAG, ITM/TPIU/ETMv3, AVR PDI, C2, FSI and AUD; audit existing JTAG/SWD/AVR ISP.                                                                          | JTAG transport precedes DP/AP layers; trace transport precedes packet semantics. Posted reads, WAIT, resets and missing context cannot produce invented successful operations.             |
| P3 — Common bus semantics                | Modbus PDUs, PS/2 device interpretation, SMBus PEC and semantics, plus capability audits of the other existing bus nodes; USB requests and descriptors are deferred.                                                                               | P0 permits deeper graphs. End-to-end transactions are decoded beyond raw bytes, including errors, retries, truncation and configuration context.                                           |
| P4 — Device overlays                     | RTCs, sensors, converters, GPIO expanders, displays, RF/network devices, EEPROM/1-Wire devices and SPI TPM; close every P4 row, including related existing devices.                                                                                | Reuse checked I2C/SPI/UART/1-Wire contracts and P1/P3 semantics where needed. Each supported chip family has explicit variants and independently checked register/command vectors.         |
| P5 — Additional physical buses           | Automotive, power/lighting, audio, memory/security and high-speed links listed as P5 below.                                                                                                                                                        | Build transport before upper layers; state electrical and sampling requirements. Every supported mode has timing/error vectors and bounded recovery; unsupported modes remain explicit.    |
| P6 — Signal tools and legacy protocols   | Measurement/transform nodes, OOK/RFID/infrared families, remote-control links, parallel/legacy buses and the remaining P6 entries.                                                                                                                 | Transport before dependent interpretation, for example OOK before Oregon/VIS. Measurement and transformation entries must have usable app paths, not be omitted from a decoder-only count. |
| P7 — Final compatibility and performance | Reconcile all 148 capability records, alias mappings, import configurations, saved graphs, localization, docs, live-decoding performance and the full regression suite.                                                                            | No unassigned entries or unexplained omissions. Full completion requires every applicable checklist to pass; Partial or Blocked entries prevent a claim of full parity.                    |

### Concrete work within each phase

**P0: make later decoders usable and preserve captures.**

1. Replace the base-plus-one-layer form with a list of compatible layers. Preserve
   existing node IDs and create unique IDs for repeated types; construct connections
   from declared ports rather than assuming every node's first port is suitable.
2. Preserve every node, connection and exposed stream when an existing graph is
   edited. A graph the current form cannot represent must remain intact; it must
   not be silently flattened into a two-node chain. Extend branch/multiple-input
   controls when a scheduled decoder requires them.
3. Verify `onewire-link → onewire → ds18b20` immediately. Verify
   additional deep chains when an in-scope protocol needs them; the USB request
   chain is deferred and is not a P0 acceptance requirement.
4. Keep unsupported DSView decoder metadata and unsupported options recoverable,
   and surface localized diagnostics. Preserve the original metadata alongside
   translated settings through unrelated edits, saving to `.hsync` and reopening.
   Add mappings beside each later decoder,
   covering both `.dsl` and `.dsc` entry paths and original channel assignments.
   HSync compatibility work stays outside uiharu.
5. Update DESIGN.md and the user guide with the final interaction. Check both
   locales and Light/Dark × Comfortable/Compact at 1440×900.

**P1: finish the initial bus work before counting it as coverage.**

1. Define explicit NOR command profiles from manufacturer specifications, then
   handle command/address/mode/dummy/data phases, lane changes, direction and
   3-/4-byte addressing. Keep the generic transport contract free of flash-specific
   operations. Connect phase output to NOR commands, status and SFDP interpretation.
2. Cover documented single/dual/quad SDR forms first, then QPI/DTR forms needed by
   the capability checklist. Mode entry/exit must be observed or configured;
   unrecognized opcodes or unknown state must retain raw evidence.
3. Extend native SD with CMD24/CMD25 writes, data response tokens, busy, stop and
   error handling; derive block length and bus width from successful observed
   configuration, with explicit overrides for captures starting mid-session.
4. Add CID/CSD/OCR/SCR interpretation shared where suitable with SD SPI. Audit
   SDIO/DDR requirements separately; implemented SDR must not close those modes.
5. Validate flash read/program/status and SD initialization/read/write workflows,
   including rejected commands, damaged CRCs and gaps during mode changes.

**P2: build debugging layers in dependency order.**

1. Add configured multi-TAP chain extraction and instruction widths without
   guessing chain topology. Preserve scan provenance and unknown state on gaps.
2. Interpret Arm DP/AP registers, SELECT banks and MEM-AP TAR/DRW accesses over
   JTAG-DP and compatible SWD data. Test posted reads, WAIT/retry, sticky faults,
   auto-increment boundaries and resets before reporting memory operations.
3. Add STM32-specific interpretation only from the relevant public device/debug
   documentation. JTAG-DP transport alone does not close `jtag_stm32`.
4. Improve SWIM synchronization, clock estimation where observed synchronization
   permits it, speed transitions and recovery. Keep a manual clock/configuration
   path; absence of evidence must remain unknown.
5. Implement cJTAG/OSCAN0 before consumers that need its reconstructed JTAG stream;
   complete EJTAG separately from Arm semantics. Add trace framing/demultiplexing
   and ITM packets, then ETMv3 packets. Instruction-level reconstruction requiring
   an unavailable program image must not be invented.
6. Complete AVR PDI, C2, FSI and AUD and audit AVR ISP using their independent
   specifications. Each remains a distinct checklist, even if framing is shared.

**P3: turn established transports into useful transaction interpretation.**

1. Defer USB control-transfer assembly, standard requests and descriptors. Audit
   the existing packet/transaction layers, PRE/high-speed requirements and USB PD
   object coverage without introducing the deferred request/descriptor layers.
2. Add Modbus function-specific request/response fields and exceptions; PS/2
   keyboard/mouse interpretation; SMBus PEC and command semantics.
3. Audit CAN/CAN-FD timing and resynchronization, LIN, DMX512, MIDI, MDIO,
   Microwire, UART and 1-Wire against their declared option/semantic checklists.

**P4: add device families on the completed contracts.**

Start with DS1307/RTC8564, LM75/MLX90614/ADXL345, MCP230xx/PCA9571/TCA6408A,
MAX7219/MAX72xx and ST7735/ST7789. Then cover the remaining converters/sensors,
RF/network parts, EEPROM and 1-Wire families, security devices and interface
modules assigned to P4 in the matrix. Existing EEPROM, nRF24L01, AM230x and APA102
implementations are audited too; APA102 alone must not close all `rgb_led_spi`
variants. Aliases may share code, with separately documented chip coverage.
Use the DSL when its current contracts and operations express the decoder;
otherwise record the exact language/backend blocker and skip the protocol.

**P5: add the remaining physical transports.**

Work in this order: DALI/CEC/UFCS/Qi; SAE J1850/FlexRay and SIPI/LFAST;
S/PDIF/TDM/AC97 and the I2S audit; LPC/SDQ/SLE44xx;
MIPI RFFE/DSI, DSI, Ethernet autonegotiation and HDCP transport fields.
The DSI names require distinct capability research before treating them as
aliases. Audit WS281x and Wiegand in the same phase. Require public framing,
clocking and error rules before implementation. Where the supported capture
hardware cannot resolve a mode, document the required sample stream and verify
the decoder on independent vectors; do not claim unperformed hardware validation.

**P6: close the long tail explicitly.**

1. Define measurement/transform contracts and app presentation for `counter`,
   `guess_bitrate`, `jitter`, `signature`, `numbers_and_state`, `i2cfilter`
   and `i2cdemux`; audit PWM/timing/parallel alongside them.
2. Implement OOK and other symbol transports before Oregon/VIS/RFID consumers.
   Cover EM4100/EM4305/T55xx, Miller, DCF77/Morse and the infrared families.
   `ir_irmp` needs an explicit family checklist; no GPL decoder/library is reused.
3. Cover GPIB/IEEE488, IEC, MCS48/Z80, Maple/NES/Nunchuk-related dependencies,
   caliper/graycode/seven-segment/stepper/XY2-100 and the remaining remote-control
   and serial links assigned here. Nunchuk's device decoder belongs to P4.
4. Validate shared names such as GPIB/IEEE488 against both inventory checklists;
   one implementation may satisfy both, but a name match is insufficient.

### Delivery gates

A decoder or capability becomes Verified only when all applicable gates pass:

1. **Evidence**: a cited public primary specification, explicit supported
   modes/options/variants, and expected-value vectors independent of the new
   synthesizer. Synthetic round trips alone are insufficient.
2. **Correctness**: exact events and provenance for normal, malformed, partial,
   truncated, retry/reset and gap cases. Unobserved state is unknown, not a
   fabricated successful transaction.
3. **Streaming**: declared buffer bounds, incremental output, cancellation and
   acceptance tests varying sample blocks, reader pauses, queues and budgets.
   Benchmark high-volume or expensive paths against a recorded baseline on the
   same machine. Record workloads and throughput/latency/memory acceptance budgets
   before implementation, based on the target sample stream and runtime bounds.
   A budget failure keeps the capability open; a revised budget needs a documented
   justification and must not conceal failure to support the declared workload.
4. **Integration**: manifests, registry/catalogue, usable application graph,
   both locales, saved-capture round trips, applicable DSView option/channel
   mappings, and user/protocol documentation. Unsupported import fields remain
   recoverable and diagnosed.
5. **Verification at task wrap-up**: relevant frontend lint/typecheck/tests,
   Rust fmt/clippy/tests, fixture generation and formatting checks. DSL changes
   also run DSL/IR/codegen freshness and generated/interpreted equivalence checks.
   Run the full workspace suite at integrated milestone boundaries and P7, not
   after every edit. UI changes get the required four theme/density checks.

DSL/compiler design work is deferred. Accumulate concrete requirements, minimal
desired TS examples, expected events/recovery, compiler/runtime evidence and other
affected protocols. Do not extend the IR or backend to unblock a protocol during
this expansion; ordinary generated output and supported contract declarations
remain part of decoder delivery.

### Execution and reporting rules

- Use GPT-6.1 sol implementation agents. Run up to three independent tracks:
  one foundational transport, one independent protocol family and one integration
  or verification task. Give modules clear ownership; coordinate shared registry,
  locale, fixture and runtime changes through one integrator.
- Start P0 together with P1 QSPI and SD work. Then take the lowest-numbered ready
  task, respecting per-protocol dependencies. Once a phase passes its gates,
  continue to the next without asking the owner to choose another batch.
  Research for later phases may proceed while an earlier phase is being integrated.
  If all remaining tasks in an earlier phase are externally blocked, implement
  later dependency-ready tasks and keep the earlier phase open; a phase number is
  a priority, not a barrier that idles otherwise useful implementation work.
- Maintain fine-grained Conventional Commits and reviewable capability changes.
  A milestone is a verification/reporting boundary, not an approval boundary.
  Do not wait for CI after a push.
- Report newly verified capabilities, remaining partial work, meaningful failures
  and exact blockers. Do not repeatedly ask for priorities or substitute increasing
  node counts for capability progress.
- If a specification cannot be obtained, the license boundary prevents an
  implementation route, or trustworthy vectors are missing, record the source
  attempts, exact missing prerequisite and affected capability. Continue another
  ready task. Do not guess protocol behavior or call the blocked item complete.
- Ask the owner only when a necessary decision actually changes the agreed scope
  or cannot be resolved from the requirements/evidence. A routine batch boundary,
  implementation detail or locally fixable failure is not such a decision.
- P7 must publish a capability-level result: Verified, Partial and Blocked rows,
  remaining feature differences and the evidence behind them. The project is not
  fully aligned while required capabilities remain Partial or Blocked. Do not
  silently reduce scope to make the final count look complete.

## Current execution evidence

P0 now preserves arbitrary stored graphs while supporting structural editing of
verified linear chains at any depth. Instances retain their IDs, repeated types
get distinct IDs, and connections use explicit compatible port pairs. Unknown
node/config/display metadata and named exposed outputs survive edits. Branches
and unavailable node types retain their topology while supported options remain
editable. DSView originals and unmapped options/layers remain in `import.raw`,
including inert entries that cannot run. Localized, paginated notes explain the
retained data and allow explicit removal.

P0 verification: 52 focused frontend tests, two import-note component tests and
all `hsync-session` tests passed. The new tests cover `.dsc` presets, capture
creation, unrelated edits, `.hsync` save/reopen, unknown entries, execution-state
changes and three-layer graph editing. The three-layer 1-Wire configuration was
visually checked at 1440×900 in Light/Dark × Comfortable/Compact, in English and
Simplified Chinese. Full integrated checks run again after protocol integration.

At the first P0–P4 checkpoint, the integrated catalogue contained **100 nodes**, including **59 new TypeScript
decoders** added after the four historical transport additions. The original
baseline contained 37 nodes. These are selectable nodes, not completed inventory
rows or a claim of full DSView/sigrok parity. During the P0–P4 decoder expansion, the IR, compiler implementation,
code generator, interpreter and runtime were not extended; the separately
authorized backend follow-up below addresses the recorded limitations. The compiler test
now discovers the decoder inventory instead of assuming five modules. Exact
shared JTAG, Modbus and PS/2 contract declarations reuse generated types in the
existing native producers without replacing their decoding algorithms.

| Phase | Delivered scope                                                                                                                                          | Explicit limits and skipped work                                                                                                                             |
| ----- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| P0    | Arbitrary-depth compatible chains, preserved complex graphs, recoverable DSView settings and localized import notes                                      | Structural editing is for linear chains; other graph topology remains intact                                                                                 |
| P1    | Manufacturer-scoped SPI/QSPI/QPI/DTR flash phases, status/SFDP; native SDR reads/writes, CRC/busy, accepted width negotiation and CID/CSD/OCR/SCR fields | SDIO/DDR/UHS/MMC and additional flash families remain outside the declared profiles                                                                          |
| P2    | JTAG chain/DP/MEM-AP, STM32F1, EJTAG, SWIM recovery/calibration, trace transport/ITM/ETMv3, PDI/C2/FSI/AUD; AVR ISP audit                                | SWD semantic overlay and cJTAG blocked by exact contracts; AVR ISP signature overlay blocked by fixed arrays; no invented target memory effects or trace PCs |
| P3    | Modbus PDU, SMBus and PS/2 semantics; completed audits of the existing common buses                                                                      | USB requests/control transfers/descriptors deferred; audited timing and semantic differences remain documented                                               |
| P4    | RTC/sensor/GPIO/converter/display/RF/network/memory/security/interface profiles in the family ledgers, including EDID/Amulet/XFP reference fields        | DS2408/DS243x/DS28EA00 and CFP blocked by exact contracts; AM230x engineering overlay blocked by fixed arrays; existing native decoders retained             |

Focused verification includes 24 I²C and six UART tests, nine SMBus/PS/2 overlay
tests plus thirteen native PS/2 tests, 38 SPI/native-SD/TLC tests, and seven
RGB/ADE stream tests. Debug/trace and transport checkpoints are recorded in the
family ledgers. The vectors compare generated/interpreted events and sample
provenance and vary blocks, queues, budgets, pauses, malformed input and gaps.
Independent regressions reject initial partial RGB/ADE frames, preserve lost
TLC5620 alignment after gaps, reject wrong-index SD responses and keep an
explicit fixed SD width across CMD0.

The final workspace run passed **1,636 Rust tests**, including registry and saved-capture integration, with no failures or ignored tests. Full workspace clippy passed with warnings denied. DSL/IR and generated-Rust freshness checks, Rust formatting and Prettier checks passed. The refreshed frontend passed ESLint, type checking and production build, plus **429 tests across 67 files**. The catalogue fixture was regenerated from the backend.

The ARM trace three-layer
form was checked at 1440×900 and saved successfully in the application preview.
The four-layer JTAG form was saved and reopened with all layers intact in
Chinese Dark/Compact mode. The latest SWIM form and defaults were checked in
English Light/Comfortable mode; P0 also passed the full theme/density matrix.

DSView translation now covers QSPI, native SD, JTAG, SWIM, C2, AVR PDI, FSI,
AUD, SDA2506, ST7735/ST7789, TLC5620, STM32/EJTAG layers,
byte-oriented SPI devices with explicit XN297L variant mapping, continuous RGB/ADE7758 without CS, PAN1321 and
additional I²C profiles. Non-generic 24xx
chip profiles preserve memory geometry and the initial address counter. Missing
SPI channels stay disconnected; unsupported UART framing stays inert rather
than being rounded to another mode. Import originals and profile diagnostics
remain saved. Configuration validation keys reach localized application messages
with the failing node's identity.

Family-level implementation ledgers track concrete profiles and remaining work:
[SPI and SD](../uiharu/docs/protocols/spi-porting-coverage.md),
[debug and common buses](../uiharu/docs/protocols/debug-bus-porting-coverage.md),
and [I2C/UART devices](../uiharu/docs/protocols/device-porting-coverage.md).
The [existing bus audit](../uiharu/docs/protocols/existing-bus-audit.md) records
option, timing and semantic differences for every audited P3 foundation.
An authored module is not counted as verified before its generated/interpreted
protocol vectors and application integration pass.

### Runtime smoke check

`cargo run -p hsync-bench -- --compare` found zero mismatches on the bundled
demo: SPI 288 items, I²C 45, UART 47 and CAN 3, all within the comparison's
sample-span tolerances.

A development-profile `uiharu bench --samples 5000000 --chunk 65536 --bytes 64
--half-period 2 --json` run processed 5,054,896 synthesized SPI-NOR samples in
77 complete-frame blocks (65,648 samples each): 64.1 million samples/s,
1.455 ms p95 block latency and 4,227,072 bytes ending RSS. On macOS this RSS
value is the process size after the run, not a measured peak. This short smoke
run exercises the existing SPI-NOR pipeline; it is not a real-device validation
or a throughput guarantee for every newly added profile.

### Backend follow-up

The owner subsequently authorized the compiler/backend changes recorded below.
This follow-up implements shared imported outputs, exact homogeneous arrays of
1–32 items, field-level hexadecimal `u64` serialization, validated numeric enums
and externally tagged unit/payload enums. The TS frontend, IR checks, Rust
emitter, interpreter and loaded catalog share those declarations. It also fixes
early switch exits, contextual integer widths and Rust variant identifier
collisions. Existing IR version 1 modules remain supported.

The supported source forms and limits are in
[the DSL specification](../uiharu/docs/dsl.md#21-types). Acceptance sources in
`uiharu/compiler/fixtures/backend-*.ts` and their generated/interpreted pipeline
exercise these capabilities outside the production catalog. Regenerate their
IR with `node uiharu/compiler/src/backend-fixtures.ts` and their Rust with
`cargo run -p uiharu-codegen -- uiharu/crates/uiharu-interp/tests/fixtures/backend/ir uiharu/crates/uiharu-interp/tests/fixtures/backend/generated`.

The acceptance pipeline covers all four compiled/interpreted producer-consumer
combinations, including a direct native connection using the imported Rust
payload. Independent assertions check wire values, raw configuration defaults,
custom fixed-array input, rejected lengths and enum values, numeric properties,
validation messages, early switch exits and integer casts. The same events and
provenance must survive changed sample blocks, gaps, queue capacity and budgets.

Final follow-up verification: all 1,670 Rust workspace tests and 454 frontend
tests pass. Workspace clippy with warnings denied, Rust/Prettier formatting,
ESLint, TypeScript checking, the frontend build, DSL IR freshness and generated
Rust freshness also pass. The fixture checks cover both source-to-IR and
IR-to-Rust freshness; generated files are not edited by hand.

These backend capabilities were delivered before the protocol continuation below;
at that checkpoint the catalog still contained 100 nodes. Matching serialized
fields alone did not make unrelated Rust payloads the same type. The continuation
migrates native contract ownership without changing the schemas or JSON values.
USB requests/control transfers/descriptors remain deferred.

### P0–P4 continuation after the backend follow-up

The continuation adds eleven selectable TS decoders to the existing catalog:
`cjtag`, `swd-dp`, `avr-isp-device`, `ds2408`, `ds243x`, `ds28ea00`, `cfp`,
`am230x-values`, `midi-semantics`, `dmx512-analysis` and `lin-profile`.
The catalog now contains 111 nodes, with 74 TypeScript decoder modules.
This count describes available graph nodes,
not one-to-one inventory parity. Native SWD, AVR ISP, 1-Wire, MDIO, AM230x,
MIDI and DMX payloads are owned by exact DSL contracts; helper methods move to
reexported extension traits and numeric identity wrappers become `u64` aliases.
Schema versions, hexadecimal ROM identity, numeric MDIO clauses, mixed SWD ACKs
and fixed-size arrays retain their wire representation.

| Phase | Additional delivered capability                                                                                                                                                                                   | Remaining delivery boundary                                                                                                                                                                |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| P0    | Deep imported 1-Wire/MIDI/AVR chains, dual-CC USB PD, MDIO edge and SWD strict-start options, UART LIN/DMX profiles, Microwire EEPROMs, SD/SPI flash and separate bit streams for non-byte SPI devices            | Unknown settings remain saved and diagnosed. Smart QSPI's heuristic lane detection and USB automatic-speed imports remain inert; their options are not silently treated as explicit modes. |
| P1    | Flash mode/state and SD register/status semantic follow-up, recorded in the SPI/SD ledger                                                                                                                         | Supported manufacturer and SDR profiles remain explicit; this does not add SDIO, DDR, UHS-II or every vendor's flash extensions.                                                           |
| P2    | OScan0/1 with bounded RDY stalls and fixed delays, observed activation, shared JTAG output; SWD SELECT/posted results/WAIT/RESEND/candidate MEM-AP addresses; AVR signatures and observed programming-enable echo | cJTAG variable-delay formats and wider control sequences are outside the configured profile. Unobserved AVR reply bytes and target-side effects are never invented.                        |
| P3    | Modbus diagnostic/file/FIFO/device-identification fields, PS/2 scan sets and wheel profiles, MIDI names/values, DMX timing/filtering, enforced LIN version and idle completion                                    | Existing-bus audits retain wire-option and annotation differences. USB request/descriptor layers remain deferred. USB PD semantic expansion has the concrete contract blocker below.       |
| P4    | DS2408 PIO and checks, DS2432/DS2433 memory/authentication transport, DS28EA00 sensor/PIO/chain, CFP register regions/identity, AM230x engineering values                                                         | Device pages distinguish observable transport from cryptographic verification, programming effects and unavailable family context.                                                         |

USB PD's existing `usbpd.events@1` uses flattened records and a nested union
with the discriminator `supply`, inside an outer `kind: "pdo"` event. The current
DSL supports `kind`-tagged unions, but cannot express that exact flattened,
alternately tagged shape. The minimal reproducer and required IR/emitter/
interpreter work are in [usb-pd.md](../uiharu/docs/protocols/usb-pd.md).
The upper layer is skipped under the owner's TS-first rule; the existing
native USB PD decoder and both CC import paths remain available. No new backend
capability was added in this continuation.

Final integrated verification for this continuation passed:

- `cargo test --workspace`: 1,713 tests passed with no failures or ignored tests.
  This includes independent protocol vectors, generated/interpreted equivalence,
  sample-block/queue/budget/gap invariance, native contract serialization and
  application import/save/reopen integration.
- `pnpm test`: 486 tests passed across 69 files, including DSL fixture freshness,
  compatible deep chains and shared JTAG state-lane localization.
- Full-workspace clippy with `-D warnings`, ESLint without warnings, standalone
  frontend type-check, the production frontend build, Prettier and rustfmt passed.
  Vite retains its advisory about the existing large application bundle.
- Production TS-to-IR and IR-to-Rust freshness checks passed; native/DSL contracts,
  registry manifests and the frontend mock fixtures were regenerated together.
  After the final equivalent iterator cleanup, all DSView import tests passed again.
- A browser check against this worktree's mock frontend saved the four-layer
  cJTAG → JTAG chain → JTAG-DP → MEM-AP configuration, displayed the cJTAG TAP
  state lane, and saved/reopened 1-Wire link → network → DS243x with DS2433 still
  selected. This checks editor integration, not decoding against physical hardware.

The phase scope is complete except for the explicitly deferred USB request layer
and the proven USB PD upper-contract blocker described above. Per-protocol mode,
device and annotation boundaries remain in the family ledgers; this is not a claim
of full DSView/sigrok parity. Earlier checkpoint totals apply only to their
corresponding snapshots.

### Recorded language/backend requirements

The following reproductions document the limitations at the P0–P4 checkpoint,
before the separately authorized backend follow-up. Their language features are
now implemented and used by the continuation above; these failures are historical.

**SWD mixed enum serialization — direct TS SWD consumers blocked.** Existing
`swd.events@1` serializes ordinary ACK values as `"ok"`, `"wait"` and `"fault"`,
but unknown acknowledgments as `{"other": 7}`. The following desired declaration
cannot describe that existing wire contract in the current DSL:

```ts
type Ack = 'ok' | 'wait' | 'fault' | { other: u8 }
type Event = { kind: 'ack'; tx: u64; value: Ack }
```

A virtual module declaring `eventSchema<Event>('swd-probe.events', 1)` fails to
lower with `the type Ack has no IR type; name a record or give an integer its
width`. An eventual design needs mixed scalar/object enum representations, or a
versioned contract adaptation, with identical generated/interpreted encoding.
The current expansion changes neither. This blocks a new TS overlay consuming
that SWD contract directly; the existing SWD decoder remains available, and the
flat JTAG-DP contract is investigated independently.

**1-Wire hexadecimal ROM identity — new TS device overlays blocked.** The native
`onewire.events@1` contract serializes `rom.id` and `search.id` with `hex_u64`, for
example `"id": "0x1122334455667728"`. A TS `id: u64` declaration generates a
numeric JSON field; the interpreter's numeric parser cannot read the native hex
string. Declaring `id: string` preserves JSON but changes the generated Rust
payload type and cannot replace the native producer's numeric API. The existing
registry connects two compiled nodes directly and does not insert JSON adapters
between different Rust payload types sharing a schema. Adding that special case
or changing the native encoding is outside this expansion.

The affected P4 additions are `ds2408`, `ds243x` and `ds28ea00`. They are skipped;
existing 1-Wire transport/network and DS18B20 decoding remain available. An
explicit hexadecimal-u64 field representation, understood by the compiler,
generator and interpreter together, would preserve both the typed contract and
lossless JSON. Opaque text with an explicitly declared adapter is a possible
future design alternative, not a workaround implemented here.

**CFP numeric MDIO clause enum — TS overlay blocked on the native contract.**
CFP uses MDIO rather than I2C. The native `MdioEvent` carries the validated Rust
`Clause::{C22,C45}` enum but serializes its field as the numbers `22` and `45`.
A virtual TS contract with `type Clause = 22 | 45` fails with `the type Clause has
no IR type; name a record or give an integer its width`. A string enum changes the
wire format; replacing the type with `u8` removes its validated enum semantics
and changes the native producer/consumer API. The requested design capability is
an enum with explicit numeric wire values, preserved by both execution paths.
No lower-contract weakening or registry adapter is introduced for this expansion.

**cJTAG shared output contract — compact transports blocked.** A minimal TS
`LogicDecoder<Config, JtagEvent>` can import `jtagEvents` from `jtag-chain` and
lower successfully, but generation rejects it with `the output's contract is
not declared here`. Both cJTAG inventory entries need to feed the same typed
JTAG consumers. A separate lookalike event type would not establish that
composition. They are skipped until shared output-contract ownership is supported;
the [debug ledger](../uiharu/docs/protocols/debug-bus-porting-coverage.md) includes
the complete reproducer and separately records the missing OScan0 stall-framing
reference. The specification gap is not treated as an IR limitation.

**Fixed-length array contracts — AM230x values and AVR ISP overlays blocked.**
The native AM230x reading contains `[u8; 5]`; an AVR ISP instruction contains
`[u8; 4]`. A bounded DSL array produces `Vec`, accepts shorter values and changes
the public Rust API, so it is not an exact replacement. A direct
`readonly [u8, u8, u8, u8, u8]` declaration fails frontend lowering with
`has no IR type`. The named AVR tuple probe lowers without a diagnostic but omits
its `InstructionBytes` type; codegen rejects it as `type InstructionBytes is not
declared`. The original decoders remain available and unchanged. Exact fixed-size
arrays require coordinated IR representation, generated `[T; N]` types and
interpreter length validation. The [SPI ledger](../uiharu/docs/protocols/spi-porting-coverage.md)
and [debug ledger](../uiharu/docs/protocols/debug-bus-porting-coverage.md) retain
the probes and affected semantic fields.

### TS source forms found during the decoder expansion

The backend follow-up fixes the source limitations below. Existing equivalent
TS decoder implementations remain valid and do not need stylistic rewrites.

These findings do not require abandoning a protocol when an equivalent bounded
TS formulation already works. No compiler, IR or runtime change is made here.

- **Nested switch exits:** an early `break` inside an `if` in a `switch` case,
  itself inside an item-reading loop, currently emits a Rust `break` that exits
  the loop. A trailing case `break` is handled differently. The desired TS
  semantics are to exit the switch and continue reading the transaction. Device
  decoders use explicit `continue` where the intended action is to advance the
  item loop. Regression vectors must include a non-target address followed by a
  target repeated START in the same transaction, and multi-byte EEPROM pointers.
  A future compiler needs a distinct switch-exit target, or must reject this form.
- **Output contract ownership:** declaring a stream decoder whose output is an
  imported schema fails with `the output's contract is not declared here`;
  redeclaring a schema over an imported event union is also rejected. A common
  trace transport node can support both raw UART and TPIU framing and own the
  generic trace-byte contract, allowing ITM/ETM consumers to share it. Independent
  producers should eventually be able to import one output contract without
  duplicating generated Rust payload types.
- **Integer expression widths:** a cast around a shifted expression can leave
  a Rust integer literal inferred as `i32`, and widening an outer expression can
  leave an inner multiplication at its smaller operand width. A narrow variable
  used as a shift count also needs an explicit supported count type. Decoders
  therefore give operands their required widths before arithmetic and exercise
  boundary values in independent vectors. Future lowering should propagate
  contextual widths consistently or issue a source-level diagnostic.
- **Enum variant spelling:** source enum values such as `"1.2"`/`"2.0"` generate
  invalid Rust identifiers. The TPM profile uses semantic identifiers `tpm12` and
  `tpm20` with localized display labels. Future identifier generation should
  preserve wire strings while producing valid, collision-free Rust variant names.

## Complete baseline matrix

“Missing” means no dedicated built-in node in the baseline, not necessarily that
the waveform cannot be inspected with a lower-level decoder. “Related” is only
a mapping for further comparison. The matrix is pinned to the source revisions
above; it does not assume that the projects stop evolving. Phase assignments are
forward-looking work allocation, not evidence of completed capabilities.

| Inventory name       | DSView directory     | Upstream | HSync baseline node | Scope or gap                                                                               | Phase |
| -------------------- | -------------------- | -------- | ------------------- | ------------------------------------------------------------------------------------------ | ----- |
| `a7105`              | `a7105`              | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `ac97`               | `ac97`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `ad5626`             | `ad5626`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `ad79x0`             | `ad79x0`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `ade77xx`            | `ade77xx`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `adf435x`            | `adf435x`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `adns5020`           | `adns5020`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `adxl345`            | `adxl345`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `am230x`             | `am230x`             | Yes      | `am230x`            | Related implementation; full option/annotation parity is not established.                  | P4    |
| `amulet_ascii`       | `amulet_ascii`       | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `arm_etmv3`          | `arm_etmv3`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `arm_itm`            | `arm_itm`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `arm_tpiu`           | `arm_tpiu`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `atsha204a`          | `atsha204a`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `aud`                | `aud`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `avr_isp`            | `avr_isp`            | Yes      | `avr-isp`           | Related implementation; full option/annotation parity is not established.                  | P2    |
| `avr_pdi`            | `avr_pdi`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `c2`                 | `c2`                 | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `caliper`            | `caliper`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `can`                | `can`                | Yes      | `can`               | Classic CAN and ISO CAN FD; simplified resynchronization.                                  | P3    |
| `can-fd`             | `can-fd`             | —        | `can`               | Covered by the same CAN node with FD enabled, not a separate node.                         | P3    |
| `cc1101`             | `cc1101`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `cec`                | `cec`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `cfp`                | `cfp`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `cjtag`              | `cjtag`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `cjtag-oscan0`       | `cjtag-oscan0`       | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `counter`            | `counter`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `dali`               | `dali`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `dcf77`              | `dcf77`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `dmx512`             | `dmx512`             | Yes      | `dmx512`            | Related implementation; full option/annotation parity is not established.                  | P3    |
| `ds1307`             | `ds1307`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `ds2408`             | `ds2408`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `ds243x`             | `ds243x`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `ds28ea00`           | `ds28ea00`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `dsi`                | `dsi`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `edid`               | `edid`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `eeprom24xx`         | `eeprom24xx`         | Yes      | `eeprom24xx`        | Related implementation; full option/annotation parity is not established.                  | P4    |
| `eeprom93xx`         | `eeprom93xx`         | Yes      | `eeprom93xx`        | Related implementation; full option/annotation parity is not established.                  | P4    |
| `em4100`             | `em4100`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `em4305`             | `em4305`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `enc28j60`           | `enc28j60`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `eth_an`             | `eth_an`             | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `flexray`            | `flexray`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `fsi`                | `fsi`                | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `gpib`               | `gpib`               | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `graycode`           | `graycode`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `guess_bitrate`      | `guess_bitrate`      | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `hdcp`               | `hdcp`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `i2c`                | `0-i2c`, `1-i2c`     | Yes      | `i2c`               | I2C transport; SMBus-specific semantics are not established.                               | P3    |
| `i2cdemux`           | `i2cdemux`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `i2cfilter`          | `i2cfilter`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `i2s`                | `i2s`                | Yes      | `i2s`               | Related implementation; full option/annotation parity is not established.                  | P5    |
| `iec`                | `iec`                | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `ieee488`            | `ieee488`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `ir_irmp`            | `ir_irmp`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `ir_nec`             | `ir_nec`             | Yes      | `ir-nec`            | Related implementation; full option/annotation parity is not established.                  | P6    |
| `ir_rc5`             | `ir_rc5`             | Yes      | `ir-rc5`            | Related implementation; full option/annotation parity is not established.                  | P6    |
| `ir_rc6`             | `ir_rc6`             | Yes      | `ir-rc6`            | Related implementation; full option/annotation parity is not established.                  | P6    |
| `ir_sirc`            | `ir_sirc`            | Yes      | `ir-sirc`           | Related implementation; full option/annotation parity is not established.                  | P6    |
| `jitter`             | `jitter`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `jtag`               | `jtag`               | Yes      | `jtag`              | TAP states and raw scan bits; no device-specific instruction/register interpretation.      | P2    |
| `jtag_ejtag`         | `jtag_ejtag`         | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `jtag_stm32`         | `jtag_stm32`         | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `lfast`              | `lfast`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `lin`                | `lin`                | Yes      | `lin`               | Related implementation; full option/annotation parity is not established.                  | P3    |
| `lm75`               | `lm75`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `lpc`                | `lpc`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `ltc242x`            | `ltc242x`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `ltc26x7`            | `ltc26x7`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `maple_bus`          | `maple_bus`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `max7219`            | `max7219`            | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `max72xx`            | —                    | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `mcp230xx`           | —                    | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `mcs48`              | `mcs48`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `mdio`               | `mdio`               | Yes      | `mdio`              | Related implementation; full option/annotation parity is not established.                  | P3    |
| `microwire`          | `microwire`          | Yes      | `microwire`         | Related implementation; full option/annotation parity is not established.                  | P3    |
| `midi`               | `midi`               | Yes      | `midi`              | Related implementation; full option/annotation parity is not established.                  | P3    |
| `miller`             | `miller`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `mipi_dsi`           | `mipi_dsi`           | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `mipi_rffe`          | `mipi_rffe`          | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `mlx90614`           | `mlx90614`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `modbus`             | `modbus`             | Yes      | `modbus-rtu`        | RTU framing, address/function and CRC; no general PDU semantics.                           | P3    |
| `morse`              | `morse`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `mrf24j40`           | `mrf24j40`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `mxc6225xu`          | `mxc6225xu`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `nes_gamepad`        | `nes_gamepad`        | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `nrf24l01`           | `nrf24l01`           | Yes      | `nrf24l01`          | Related implementation; full option/annotation parity is not established.                  | P4    |
| `nrf905`             | `nrf905`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `numbers_and_state`  | `numbers_and_state`  | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `nunchuk`            | `nunchuk`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `onewire_link`       | `onewire_link`       | Yes      | `onewire-link`      | Related implementation; full option/annotation parity is not established.                  | P3    |
| `onewire_network`    | `onewire_network`    | Yes      | `onewire`           | Related implementation; full option/annotation parity is not established.                  | P3    |
| `ook`                | `ook`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `ook_oregon`         | `ook_oregon`         | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `ook_vis`            | `ook_vis`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `pan1321`            | `pan1321`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `parallel`           | `parallel`           | Yes      | `parallel`          | Related implementation; full option/annotation parity is not established.                  | P6    |
| `pca9571`            | `pca9571`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `pjdl`               | `pjdl`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `pjon`               | `pjon`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `ps2`                | `ps2`                | Yes      | `ps2`               | Transport frames, not keyboard/mouse interpretation.                                       | P3    |
| `pwm`                | `pwm`                | Yes      | `pwm`               | Related implementation; full option/annotation parity is not established.                  | P6    |
| `pxx1`               | `pxx1`               | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `qi`                 | `qi`                 | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `qspi`               | `qspi`               | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P1    |
| `rc_encode`          | `rc_encode`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `rfm12`              | `rfm12`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `rgb_led_spi`        | `rgb_led_spi`        | Yes      | `apa102`            | Related APA102 implementation only; generic SPI RGB family equivalence is not established. | P4    |
| `rgb_led_ws281x`     | `rgb_led_ws281x`     | Yes      | `ws281x`            | Related implementation; full option/annotation parity is not established.                  | P5    |
| `rtc8564`            | `rtc8564`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `sae_j1850_vpw`      | `sae_j1850_vpw`      | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `sbus_futaba`        | `sbus_futaba`        | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `sda2506`            | `sda2506`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `sdcard_sd`          | `sdcard_sd`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P1    |
| `sdcard_spi`         | `sdcard_spi`         | Yes      | `sdcard-spi`        | SPI mode only; native mode absent at baseline; register contents remain raw.               | P1    |
| `sdq`                | `sdq`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `seven_segment`      | `seven_segment`      | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `signature`          | `signature`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `sipi`               | `sipi`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `sle44xx`            | `sle44xx`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `smart qspi`         | `smart qspi`         | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P1    |
| `spdif`              | `spdif`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `spi`                | `0-spi`, `1-spi`     | Yes      | `spi`               | Related implementation; full option/annotation parity is not established.                  | P1    |
| `spi_tpm`            | `spi_tpm`            | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `spiflash`           | `spiflash`           | Yes      | `spi-nor`           | Single-line SPI NOR subset; no dual/quad/QPI/DTR or SFDP interpretation.                   | P1    |
| `ssi32`              | `ssi32`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `st25r39xx_spi`      | `st25r39xx_spi`      | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `st7735`             | `st7735`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `st7789`             | `st7789`             | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `stepper_motor`      | `stepper_motor`      | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `swd`                | `swd`                | Yes      | `swd`               | Related implementation; full option/annotation parity is not established.                  | P2    |
| `swim`               | `swim`               | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P2    |
| `t55xx`              | `t55xx`              | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `tca6408a`           | `tca6408a`           | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `tdm_audio`          | `tdm_audio`          | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `timing`             | `timing`             | Yes      | `timing`            | Related implementation; full option/annotation parity is not established.                  | P6    |
| `tlc5620`            | `tlc5620`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `uart`               | `0-uart`, `1-uart`   | Yes      | `uart`              | Related implementation; full option/annotation parity is not established.                  | P3    |
| `ufcs`               | `ufcs`               | —        | Missing             | No dedicated built-in node at the audit baseline.                                          | P5    |
| `usb_packet`         | `usb_packet`         | Yes      | `usb`               | USB low/full-speed packets; no high-speed or PRE traffic.                                  | P3    |
| `usb_power_delivery` | `usb_power_delivery` | Yes      | `usb-pd`            | USB PD framing, headers, CRC and selected objects; some objects remain raw.                | P3    |
| `usb_request`        | `usb_request`        | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P3    |
| `usb_signalling`     | `usb_signalling`     | Yes      | `usb`               | USB low/full-speed signalling is integrated into usb; no separate signalling stream.       | P3    |
| `wiegand`            | `wiegand`            | Yes      | `wiegand`           | Related implementation; full option/annotation parity is not established.                  | P5    |
| `x2444m`             | `x2444m`             | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `xfp`                | `xfp`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P4    |
| `xy2-100`            | `xy2-100`            | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
| `z80`                | `z80`                | Yes      | Missing             | No dedicated built-in node at the audit baseline.                                          | P6    |
