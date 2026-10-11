# AGENTS.md

This file is for the AI agents and engineers working in this repository: what the project sets out to do, its hard requirements, how to build and verify, and how to commit. Read all of it before you start; before changing the interface, also read [DESIGN.md](./DESIGN.md) (the interface design spec).

## What the project is

HSync is a general-purpose logic analyzer application on a modern stack, built around no single brand of hardware. It began as a rewrite of [DSView](https://github.com/DreamSourceLab/DSView): **the first release (v1) matches DSView's logic analyzer features**: every DSLogic model and the Demo device, Buffer and Stream capture, simple, advanced and serial triggers, protocol decoding, measurement, search, reading and writing `.dsl` files, and export. The oscilloscope (DSO) and data acquisition (DAQ) modes are out of scope for v1. Each family of devices is ported from its maker's own application: DreamSourceLab's DSLogic from DSView, ALIENTEK's ATK-Logic analyzers (DL16, DL16 Plus) from the [ATK-Logic](https://github.com/alientek-openedv/atk-logic) application, and the PXLogic analyzers (PX-Logic 32, 16 Pro, 16 Plus, 16 Base) from [PXView](https://github.com/PXLogic/PXView)'s `pxlogic` driver.

**uiharu**, in `uiharu/`, is the streaming decode engine that replaces the sigrok decoders: decoders run as resumable state machines over blocks of samples, exchange typed events through bounded streams with backpressure, and are wired into a checked decode graph. It is built on its own first — its architecture is in [uiharu/docs/design.md](uiharu/docs/design.md), how to write a decoder in [uiharu/docs/writing-decoders.md](uiharu/docs/writing-decoders.md) — and wired into the app later; until then the app keeps decoding with `hsync-decode` and `hsync-srd`.

## Hard requirements

These come from the project owner; no change may break them:

1. **Stack**: the desktop shell is **Tauri 2**; the frontend is **Vue 3 + TypeScript + Tailwind CSS v4**; waveforms render with Canvas2D as the baseline, **accelerated with WebGPU where it is available**, and the fallback always stays.
2. **DESIGN.md governs the UI and UX; the `.hsync/` mockups are only a reference**: `.hsync/handoff.md` and the HTML mockup beside it (local files, not in the repository) are a starting point, not a spec to follow. The interface may improve on them for real-world use; such changes go into DESIGN.md.
3. **Internationalization**: the first release supports Simplified Chinese (the default) and English. Every user-visible string in the interface goes through vue-i18n, never hard-coded; the Rust side outputs no natural language, only i18n keys and parameters.
4. **All interaction with real hardware is written in Rust**, and USB is reached only through **nusb**. Never bring in libusb, rusb or FFI to any C/C++ driver code; DSView's and PXView's C code and ATK-Logic's C++ are only porting references.
5. **Code style**: the frontend follows **ESLint + Prettier**; Rust follows rustfmt + clippy. **lefthook** makes sure code is formatted when it is committed, and commitlint checks the commit messages.
6. **Commits**: use **fine-grained Conventional Commits**; **write pull requests and issues in English**. See [Commit conventions](#commit-conventions) below.
7. **Language**: everything in the repository is written in English (documentation, DESIGN.md, this file, code comments, commit messages), except the Chinese README `README.zh-CN.md` and the Chinese interface strings `src/i18n/locales/zh-CN.json`. Chinese that is data stays as it is (such as the values DSView writes into `.dsc` sessions, which `src/session/session.ts` reads). `README.md` and `README.zh-CN.md` say the same thing; change them together.
8. **Tests at the end**: run the tests only when you wrap up a task, not after each step on the way; [Common commands](#common-commands) says which ones.
9. **Don't watch CI**: after a push, don't wait for the CI runs or keep checking their results (`gh run watch`, `gh run list` …).
10. **uiharu is Apache-2.0 and free of sigrok**: nothing under `uiharu/` may be ported from or modelled on DSView, libsigrok or libsigrokdecode code, depend on the hsync crates or on any GPL-licensed crate, or copy DSView's decoders. Its dependencies are permissively licensed; every source file starts with `// SPDX-License-Identifier: Apache-2.0`. The rest of the repository may use uiharu, not the other way round.

## Repository map

| Path                           | Contents                                                                                                                                                                                                                                                                                                                                                                       |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `README.md`, `README.zh-CN.md` | The project page in English and Chinese: features, downloads, getting started, building                                                                                                                                                                                                                                                                                        |
| `DESIGN.md`                    | Interface design spec: layout, design tokens, each area's look and behaviour, accessibility, motion                                                                                                                                                                                                                                                                            |
| `docs/`                        | The user guide (`user-guide.md`), updated when the interface or how it works changes, and the maintainer guide on CI, releases and packaging (`maintainer-guide.md`)                                                                                                                                                                                                           |
| `assets/`                      | The brand mark and social preview (`brand/`), the README screenshots (`screenshots/`)                                                                                                                                                                                                                                                                                          |
| `src/`                         | Vue frontend (components, Pinia stores, renderers, IPC wrappers, i18n)                                                                                                                                                                                                                                                                                                         |
| `src-tauri/`                   | Tauri app crate (commands, events, resources: firmware and demo data)                                                                                                                                                                                                                                                                                                          |
| `crates/hsync-*`               | Rust libraries: core, capture, driver, dslogic, atklogic, pxlogic, demo, decode, srd, fileio, session                                                                                                                                                                                                                                                                          |
| `uiharu/`                      | The uiharu decode engine (Apache-2.0): `crates/uiharu-core` (the runtime), `crates/uiharu-protocols` (the decoders: SPI, SPI NOR, SD card, I²C, 24xx EEPROM, UART, Modbus RTU, CAN, 1-Wire, DS18B20, USB, I²S, PWM, timing, parallel, JTAG, SWD), `crates/uiharu-cli` (the `uiharu` command), `docs/` (architecture, how to write a decoder, one page per protocol), `LICENSE` |
| `scripts/`                     | Lays out the bundled CPython; builds this machine's installers the way releases are built                                                                                                                                                                                                                                                                                      |
| `.github/`                     | CI checks, installer builds and releases                                                                                                                                                                                                                                                                                                                                       |
| `.hsync/`                      | Design mockups (a local reference, listed in `.gitignore`; never commit them)                                                                                                                                                                                                                                                                                                  |
| `../DSView`                    | The reference implementation (DSView's source), read-only                                                                                                                                                                                                                                                                                                                      |

Dependencies between the Rust crates run `src-tauri → hsync-session → {driver, capture, decode, srd, fileio} → hsync-core`; `hsync-srd` (the Python decoder host) builds on `hsync-decode`, and `hsync-dslogic`, `hsync-atklogic`, `hsync-pxlogic` and `hsync-demo` implement the traits of `hsync-driver`. Library crates must not depend on Tauri.

## Common commands

```sh
pnpm install              # install dependencies and the lefthook hooks
pnpm dev                  # the frontend alone (Vite, http://localhost:1420)
pnpm tauri dev            # the desktop app (frontend + Rust)
pnpm build                # type-check and build the frontend
pnpm tauri build          # package the desktop app
pnpm bundle               # build this machine's installers as releases are built (bundled CPython)
pnpm release 0.2.0        # ask for a release: opens a pull request setting this version; merging it releases (see Releasing)

pnpm lint                 # ESLint
pnpm lint:fix             # ESLint, applying fixes
pnpm format               # Prettier, writing
pnpm format:check         # Prettier, checking
pnpm typecheck            # vue-tsc
pnpm test                 # Vitest (single run)

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cargo test -p uiharu-core -p uiharu-protocols -p uiharu-cli   # the uiharu crates alone
cargo run -p uiharu-cli -- gen flash.bin                      # a synthetic SPI NOR capture
cargo run -p uiharu-cli -- decode flash.bin --stats           # NDJSON events on stdout
cargo run -p uiharu-cli -- bench                              # throughput, latency, memory
```

When you wrap up a change, run at least the ones that concern it: `pnpm lint`, `pnpm typecheck`, `pnpm test`, `cargo clippy`, `cargo test`.

Building `crates/hsync-srd` (the Python decoder host, which embeds CPython through pyo3) needs a Python 3 with a shared library on the PATH; release builds and CI use the bundled CPython that `scripts/python-runtime.mjs` lays out instead. `src-tauri/resources/decoders/` holds the decoders copied from DSView; do not change their contents.

## Coding conventions

### Frontend

- Components use `<script setup lang="ts">`; file names are PascalCase; composables start with `use`; Pinia stores are named `useXxxStore`.
- Types shared with Rust come from `src/ipc/bindings/` (generated by ts-rs; never edit them by hand). The backend is reached only through the wrappers in `src/ipc/`; components never call `invoke` themselves.
- Style with design tokens only: Tailwind theme classes (`bg-canvas`, `text-ink-2`, `border-line` …) or `var(--token)`. **Never** hard-code colours, gradients, shadows or glows; red and amber are for errors and warnings alone.
- New icons come from [Lucide](https://lucide.dev/) (`@lucide/vue`), not hand-drawn SVG (`AppIcon.vue` holds the older ones); `App.vue` sets their defaults: 16 px, 1.5 px stroke.
- Numbers always use tabular figures (`tabular-nums`); times, frequencies and sample counts are formatted through `src/utils/units.ts`.
- Long lists (the data stream, decoded results) must be virtualized. Waveform drawing never loops over samples; it uses only the bins and edges the backend returns.
- Accessibility: icon buttons need an `aria-label`; menus, tabs and segmented controls follow DESIGN.md §12.

### Rust

- Edition 2024 on the stable toolchain; `clippy -D warnings` must pass; public APIs get rustdoc.
- Errors are enums defined with `thiserror`; errors that cross IPC turn into `{ code, params }`, where `code` is an i18n key.
- The hardware, storage and decoding crates stay free of `unsafe` as a rule; where it is really needed, explain it in a `// SAFETY:` comment.
- Long tasks (capture, decoding, saving, export) must be cancellable and report progress through events or a `Channel`.

### Hardware layer (`hsync-dslogic`, `hsync-atklogic`, `hsync-pxlogic`, `hsync-driver`)

- Behave **byte for byte like DSView**: commands, registers, the `DSL_setting` layout, trigger encoding, transfer sizes and the stop sequence all follow `libsigrok4DSL/hardware/DSL/`. `hsync-atklogic` likewise follows the ATK-Logic application (`pv/usb/`, `pv/thread/`, `pv/data/analysis.cpp`, `SessionController::start()`): command frames, the lane order of the USB data, the connect, start and stop sequences. `hsync-pxlogic` follows PXView's `pxlogic` driver (`libsigrok/src/hardware/pxlogic/`): register requests, the open sequence with its firmware and bitstream uploads, and `start_transfers()`; its `tools/golden` harness runs that C code to record the reference sequences. Where the port departs from its reference, say why in a code comment.
- Control transfers always go through a claimed `nusb::Interface` (Windows does not support `Device::control_*`).
- Polling that has no limit in the reference gets a timeout in the port.
- Every protocol detail has unit tests: struct byte layouts are asserted against golden data; command sequences are replayed with the `UsbTransport` mock.
- Do not modify the firmware and bitstreams in `src-tauri/resources/firmware/`: the DSLogic files are MIT licensed, from `DSView/res` (keep their license file); the PXLogic files come from PXView's `PXView/res`.

### uiharu (`uiharu/`)

- A decoder is an `async fn` over the runtime's handles (`SignalCursor`, `Inbox`, `Outbox`, or a contract's cursor such as `spi::Transaction`); every read and write is an `await` point and nothing is consumed by an operation that did not complete. Decoders never buffer a whole transaction and every buffer they keep has a declared bound.
- Every event names its provenance (`Provenance`: span and settled tick) following the table in `uiharu/docs/design.md` §2; a stream's events settle in order.
- Nodes declare their ports with `InputSpec`/`OutputSpec`: the properties an output promises (`Props`) and what an input requires of it (`Requires`), so that the graph can refuse a connection with a diagnostic that names the port, the property and both values.
- Contracts stay generic: the cursor over a contract (such as `spi::Transaction`) offers only the contract's primitives — the next word, the normal end, draining. Which line a protocol reads, byte order, field widths and integer assembly are the consuming decoder's own code; never add a protocol-specific operation to a shared contract.
- Behaviour must not depend on how samples are cut into blocks, on the reader's pauses or on queue sizes and budgets; a change to a decoder or to the runtime comes with a case in the acceptance suite (`uiharu/crates/uiharu-protocols/tests/acceptance.rs`) when it could affect this. Rust decoders of a contract are written the way the DSL will express them (see `uiharu/docs/design.md` §7).
- New decoders follow `uiharu/docs/writing-decoders.md`: a module per protocol with its contract, decoder, synthesizer and tests, tested through `uiharu_core::testing` (feature `testing`), implemented from the protocol's public specification and documented in `uiharu/docs/protocols/<name>.md`.
- The runtime's rules live in `uiharu/docs/design.md`; a change to them changes the document in the same pull request. `uiharu/README.md` describes the CLI and the embedding API for users of the engine.

### Implementing the design

- Sizes, colours, type sizes, spacing, interaction rules and copy follow DESIGN.md. The mockups are only a reference: being pleasant to use matters more than looking like them. Whatever the mockups mark as illustrative (hard-coded data, buttons that only look the part) gets wired to real functionality.
- Ideas the mockups rejected stay out by default: a details panel on the right, small waveforms in the bottom bar, gradient shading under high levels, data streams squeezed side by side onto one page, global trigger settings in the top bar. If one is really needed, ask the owner first.

### Internationalization

- Strings live in `src/i18n/locales/zh-CN.json` and `en-US.json`; both files must have exactly the same keys (a test checks this).
- Keys are hierarchical, `<area>.<element>[.<state>]`; sentences are messages with parameters, never strings concatenated in code.
- The Chinese copy in the mockups is a reference and may change where it is unclear; the English must mean the same as the Chinese and be about as long.

## Commit conventions

Format:

```
<type>(<scope>): <subject>

<body: why it changed, what changed and what it affects>
```

- **type**: `feat`, `fix`, `perf`, `refactor`, `style`, `test`, `docs`, `build`, `ci`, `chore`, `revert`.
- **scope** (optional; when given it must be on the allow-list in `commitlint.config.js`): `core`, `capture`, `driver`, `dslogic`, `atklogic`, `pxlogic`, `demo`, `decode`, `fileio`, `session`, `app`, `uiharu`, `ui`, `timeline`, `stream`, `trigger`, `render`, `i18n`, `theme`, `ipc`, `store`, `deps`, `tooling`, `repo`.
- **subject**: an imperative sentence in English, starting in lower case, without a full stop, at most 72 characters.
- **Fine-grained**: a commit does one thing that can be reverted and reviewed on its own. Configuration, scaffolding, features, tests and documentation go in separate commits; formatting and logic changes are never mixed; a new dependency may share a commit with the code that uses it.
- Breaking changes add `!` after the type and a `BREAKING CHANGE:` line in the body.
- git-cliff writes the release notes and `CHANGELOG.md` from the commit messages: the subjects of `feat`, `fix` and `perf` commits go into them as they are, so write them for users to read. Never edit the version or `CHANGELOG.md` by hand; see [Releasing](#releasing).
- Never skip the hooks with `--no-verify`; when a hook fails, fix the cause and commit again. Do not commit `.hsync/`, build output or local configuration.
- **Pull requests and issues**: titles, descriptions and comments are always written in English; Chinese interface copy may be quoted as it is.

Examples:

```
feat(dslogic): encode simple trigger into fpga setting stages
fix(timeline): keep trigger menu inside the viewport near the bottom edge
build(tooling): run prettier on staged files in lefthook pre-commit
docs: describe the data stream's tabs in DESIGN.md
```

## Releasing

The owner decides when to release and which version it is; the version does not have to follow from the commit types:

1. Run `pnpm release <version>` (it needs the GitHub CLI), or click **Run workflow** on the Release workflow in GitHub Actions and enter the version. The workflow branches `release/v<version>` off main, sets the version of `package.json`, the Cargo workspace and `Cargo.lock` with `scripts/version.mjs`, rewrites `CHANGELOG.md` with git-cliff (`cliff.toml`), takes the `feat`, `fix`, `perf` and `revert` commits since the last tag as a preview of the notes, and opens the pull request `chore: release <version>`.
2. Merging that pull request is the release: the merge commit is tagged `v<version>`, the notes are written again from the commits the tag really contains, and a draft release is created; the Bundle workflow builds the installers on macOS, Windows and Linux (with the bundled CPython and update signatures) into the draft, which is published once all of them succeed.
3. Main may move on while the pull request is open; the release is main as it is at the merge. To see fresh notes in the pull request, run the release again with the same version. A version with a `-` (such as `0.2.0-beta.1`) is published as a pre-release, which the in-app updater does not offer.

`pnpm bundle` builds the installers on this machine the same way (without update signatures); the Bundle workflow can also be run by hand, keeping the installers as artifacts of that run. Releasing needs the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the update signatures; the public key is in `src-tauri/tauri.conf.json`). The [maintainer guide](docs/maintainer-guide.md) goes into the details: CI, failed releases, the installers, the bundled Python, updates and signing.

## Testing

- New features come with tests; a bug fix starts with a test that reproduces the bug.
- Rust: storage and queries are checked with proptest against a naive implementation; `.dsl` reading and writing get round-trip tests, and compatibility tests with the demo files that ship with DSView.
- Frontend: pure logic (unit formatting, trigger summaries, data stream rows, viewport maths) gets Vitest unit tests; interactive components are tested with Vue Test Utils.
- uiharu: the runtime's data structures get unit tests beside them; decoders get integration tests on synthesized waveforms (`uiharu-protocols/tests/spi.rs`, `nor.rs`) and the acceptance suite (`acceptance.rs`, proptest) must keep passing: random scenarios cut into random blocks and read with random pauses through tiny queues and budgets yield exactly the events of one uninterrupted run. Tests use `expect("…")`, never `unwrap()`.
- For changes to how the interface looks, open the Demo device's protocol demo and check the four combinations of Light/Dark × Comfortable/Compact at 1440×900; the mockup artboards help as a comparison.

## License

HSync's own code — the hsync crates, the app, the frontend, the scripts and the documentation — is dual-licensed: Apache-2.0 or GPL-3.0-or-later, at the user's option (`LICENSE-APACHE`, `LICENSE-GPL`). Files ported from another project (DSView's libsigrok4DSL and libsigrokdecode, ATK-Logic, PXView) name their source in the file header. The protocol decoders copied from DSView (`src-tauri/resources/decoders/`) keep their GPL license as they are, so the installers, which bundle them, are distributed under the GPL-3.0-or-later (`src-tauri/tauri.conf.json` says so). The uiharu engine (`uiharu/`) is Apache-2.0 alone (`uiharu/LICENSE`); see hard requirement 10. The DSLogic firmware and FPGA bitstreams are MIT licensed, the PXLogic ones come with PXView, and the fonts are SIL OFL 1.1.

## Way of working

- Before changing the interface, read the relevant sections of DESIGN.md; changes to the interface or its interaction update DESIGN.md as well.
- Changes users notice update `docs/user-guide.md`. When they change what the READMEs show (features, supported devices, downloads, screenshots), update `README.md` and `README.zh-CN.md` together.
- The README screenshots in `assets/screenshots/<locale>/` come from `pnpm dev` in Chrome, put into state with the URL parameters of `src/dev/devParams.ts`: 1440×900 for `hero-*`, 1240×770 for the rest, at a device pixel ratio of 2, framed as a macOS window and saved as lossless WebP. When the interface they show changes, retake them in both languages. `assets/brand/social-preview.png` is the repository's social preview on GitHub.
- Ask the project owner about product questions you are unsure of (interaction, copy, scope) rather than guessing.
- Do not modify `../DSView`; it is only the reference implementation.
