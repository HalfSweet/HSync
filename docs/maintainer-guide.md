# Maintainer guide

How HSync is checked, released and packaged: continuous integration, the release process, the installers, the Python runtime they bundle, in-app updates and signing. Rules for everyday work (commits, code style, tests) are in [AGENTS.md](../AGENTS.md); the interface is specified in [DESIGN.md](../DESIGN.md).

## Where things are

| What                   | Files                                                                                                         |
| ---------------------- | ------------------------------------------------------------------------------------------------------------- |
| Checks on every change | `.github/workflows/ci.yml`                                                                                    |
| Releases               | `.github/workflows/release.yml`, `scripts/release.mjs`, `scripts/version.mjs`, `cliff.toml`                   |
| Installers             | `.github/workflows/bundle.yml`, `scripts/bundle.mjs`                                                          |
| Bundled Python         | `scripts/python-runtime.mjs`, `crates/hsync-srd/src/python.rs`, `src-tauri/build.rs`                          |
| In-app updates         | `plugins.updater` in `src-tauri/tauri.conf.json`, `src-tauri/tauri.updater.conf.json`, `src/stores/update.ts` |

## Continuous integration

`ci.yml` runs on every pull request and every push to `main`, and can be run by hand. It runs on Linux only, to save the Actions minutes of a private repository; macOS and Windows problems show up when the installers are built (see [Installers](#installers)).

| Job             | What it checks                                                                                                                                                                 |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Commit messages | commitlint over the new commits: base..head of a pull request, before..after of a push; only the last commit when there is nothing to compare with (new branch, force push)    |
| Frontend        | `pnpm format:check`, `lint`, `typecheck`, `test`, `build`                                                                                                                      |
| Rust            | `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -D warnings`, `cargo test --workspace --locked`, and that the tests left `src/ipc/bindings` as committed |

The Rust job builds and tests against the Python the installers ship (see [Bundled Python](#bundled-python)), so a decoder that needs a module the trimmed standard library lacks fails here. When the bindings check fails, run `pnpm bindings` and commit the result.

With a warm cache the Rust job takes about 4 minutes; after `Cargo.lock` changes, or on a cold cache, it can take 15–30. Jobs stop after 10–60 minutes.

Pull requests opened by the Release workflow come from `github-actions[bot]`, so GitHub holds their CI until someone approves it ("Approve and run"). Approving is optional: those pull requests only change version numbers and the changelog.

## Releasing

The owner decides when to release and which version it is. Versions are not derived from commit types.

### 1. Ask for a release

```sh
pnpm release 0.2.0
```

This needs the [GitHub CLI](https://cli.github.com/), signed in, run inside the repository. Instead, you can open **Actions → Release → Run workflow** and enter the version.

The Release workflow then, from the current `main`:

1. Checks the version (`scripts/version.mjs`): it must be a semantic version, come after the latest `v*` tag, and not come before the version in `package.json`.
2. Sets the version in `package.json` (which `tauri.conf.json` reads), in `[workspace.package]` of `Cargo.toml`, and for the workspace's crates in `Cargo.lock` (`cargo update --workspace`).
3. Writes `CHANGELOG.md` from the whole history with git-cliff (`cliff.toml`).
4. Pushes branch `release/v0.2.0` and opens the pull request **`chore: release 0.2.0`**. Its description previews the release notes.

### 2. Review and merge

- The pull request changes the version files and `CHANGELOG.md`; don't edit them by hand.
- If `main` moves on while the pull request is open, nothing breaks: what gets released is `main` at the moment of the merge. To refresh the preview, run the same command again with the same version. The branch is rebuilt from `main` and the pull request updated.
- Merge with **Squash and merge** or **Rebase and merge**, which keep the history linear.

### 3. What happens after the merge

Merging a `release/v*` pull request runs the rest of the Release workflow:

1. **Tag and draft** (about a minute): tags the merge commit `v0.2.0`, writes the notes again from what that tag actually holds, and creates a draft release.
2. **Installers**: the Bundle workflow builds on three platforms and uploads into the draft. Expect about 5 minutes for macOS, 10 for Linux and 20 for Windows.
3. **Publish**: once all three succeed, the draft is published and becomes the latest release.

The published release carries:

| Platform              | Installers                  | Used by in-app updates      |
| --------------------- | --------------------------- | --------------------------- |
| macOS (Apple silicon) | `.dmg`                      | `.app.tar.gz` + `.sig`      |
| Windows (x64)         | NSIS `-setup.exe`, `.msi`   | the NSIS installer + `.sig` |
| Linux (x64)           | `.AppImage`, `.deb`, `.rpm` | the AppImage + `.sig`       |

It also carries `latest.json`, which the installed apps read.

A version with a pre-release part, such as `0.2.0-beta.1`, is published as a pre-release. In-app updates never offer it.

### When something fails

- **The pull request doesn't open.** Read the log of the "Release pull request" job. Most often the version was refused (already released, or not above the last release).
- **An installer build fails.** The release stays a draft that only maintainers see. If the failure was transient, use **Re-run failed jobs** on that run; publishing follows once every platform succeeds.
- **It needs a code fix.** Push the fix to `main`. Then remove the draft and its tag (`gh release delete v0.2.0 --cleanup-tag`) and ask for the same version again. Or release the next patch version instead.

### Release notes and the changelog

Notes come from the conventional commits since the previous tag:

- `feat`, `fix`, `perf` and `revert` commits are listed, grouped, with their scope.
- Breaking changes are marked.
- Everything else (`docs`, `build`, `ci`, `chore`, `refactor`, `test`, `style`) stays out.

Subjects appear as written, so write them for users.

`CHANGELOG.md` is written in full by every release pull request. A section that missed commits merged while its pull request was open is therefore complete again by the next release. Never edit it by hand.

## Installers

`bundle.yml` builds the installers on three runners:

| Runner         | Target                     | Why this runner                                      |
| -------------- | -------------------------- | ---------------------------------------------------- |
| `macos-15`     | `aarch64-apple-darwin`     | Apple silicon; the bundle needs macOS 13.3 or later  |
| `windows-2025` | `x86_64-pc-windows-msvc`   |                                                      |
| `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | the oldest supported Ubuntu, so an older glibc works |

On each runner it:

1. Lays out the bundled Python.
2. Runs the `hsync-srd` tests against it.
3. Builds with tauri-action, passing `--config` with the runtime's Tauri config. When the `TAURI_SIGNING_PRIVATE_KEY` secret exists, it also passes `--config src-tauri/tauri.updater.conf.json`, which produces the signed update artifacts.

The Release workflow calls it with a draft release to fill. You can also run it by hand (**Actions → Bundle → Run workflow**) to check packaging without releasing. The installers then stay with the run as artifacts. Artifact zips drop hidden files and file modes: to try a macOS build, use the `.dmg` artifact rather than the zipped `.app`.

To build the installers for your own machine the same way (without update signatures):

```sh
pnpm bundle                  # all bundle types
pnpm bundle --bundles app    # arguments go on to `tauri build`
```

## Bundled Python

The sigrok-compatible decoders run in CPython, embedded through pyo3 (`crates/hsync-srd`). Installers must not depend on a Python on the user's machine, so they bundle a trimmed [python-build-standalone](https://github.com/astral-sh/python-build-standalone) CPython.

**Development builds** don't bundle anything. pyo3 links and starts whatever Python 3 the build finds; it needs a shared library, which system and Homebrew Pythons have.

**`scripts/python-runtime.mjs`** pins the python-build-standalone release (`RELEASE`), the CPython version (`VERSION`) and the SHA-256 of each target's `install_only_stripped` archive (`DIGESTS`). It downloads into `target/python/downloads/`, checks the digest, and writes `target/python/<target>/`:

- `dist/`: the distribution as published. Builds link against its libpython.
- `home/`: the trimmed standard library, shipped as the app's `python` resource. Left out are pip and `site-packages`, `ensurepip`, `idlelib`, Tk, `turtle`, tests, `venv`, `_pyrepl`, `pydoc_data`, `config-*`, the SQLite/OpenSSL/Tk native modules, stale bytecode and hidden files. Everything else stays, so any import a decoder makes keeps working.
- `pyo3.cfg`: a `PYO3_CONFIG_FILE` for linking. The distribution's own sysconfig still points at the machine it was built on (`/install`), so pyo3 cannot ask the interpreter.
- `tauri.conf.json`: the additions to the bundle, passed with `tauri build --config`.

**Where libpython goes:**

| Platform | Location                                                     | How the app finds it                                                                           |
| -------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| macOS    | `Contents/Frameworks/libpython3.14.dylib`                    | its install name is `@rpath/…`; tauri-build adds `@executable_path/../Frameworks`              |
| Windows  | `python314.dll` beside `hsync.exe`, with `vcruntime140*.dll` | Windows looks beside the executable                                                            |
| Linux    | `/usr/lib/HSync/python/lib/libpython3.14.so.1.0`             | RUNPATH `$ORIGIN/../lib/HSync/python/lib`, set by `src-tauri/build.rs` (follows `productName`) |

**At startup**, if the app's resources contain `python/`, `DecoderHost::with_python_home` starts Python through `PyConfig` before anything else uses it:

- isolated from `PYTHONHOME`, `PYTHONPATH` and the user's site-packages;
- with that directory as its home;
- with bytecode writing off, because the install is read-only and signed.

If that start fails, only the Python decoders become unavailable; it never falls back to pyo3's default start, which would abort the process.

**Upgrading CPython:**

1. Pick a python-build-standalone release whose CPython version pyo3 supports.
2. Update `RELEASE`, `VERSION` and `DIGESTS` from that release's `SHA256SUMS`. If the minor version changes, the library names change with it (`python3.15`, `python315`).
3. Push, let CI pass, then run the Bundle workflow by hand to check all three platforms.

**Testing locally against the bundled runtime** (macOS shown; use `LD_LIBRARY_PATH` on Linux, `PATH` on Windows):

```sh
node scripts/python-runtime.mjs
R=$PWD/target/python/aarch64-apple-darwin
PYO3_CONFIG_FILE=$R/pyo3.cfg CARGO_TARGET_DIR=target/bundled \
  DYLD_LIBRARY_PATH=$R/dist/python/lib PYTHONHOME=$R/home HSYNC_TEST_PYTHON_HOME=$R/home \
  cargo test -p hsync-srd
```

`HSYNC_TEST_PYTHON_HOME` enables `crates/hsync-srd/tests/python_home.rs`, which starts Python the way the app does. Use a separate target directory: switching `PYO3_CONFIG_FILE` rebuilds pyo3 and everything above it.

## In-app updates

- **Where the app looks.** tauri-plugin-updater reads `https://github.com/HalfSweet/HSync/releases/latest/download/latest.json`. tauri-action assembles that file from all three platforms while building a release.
- **Which releases count.** Only the latest published release that isn't a pre-release.
- **How updates are verified.** Against the public key in `plugins.updater.pubkey`. On Windows the installer runs in passive mode.
- **The interface.** "Check for updates at startup" in the settings panel and the About dialog; see [DESIGN.md](../DESIGN.md).
- **The repository must be public**, or the releases hosted elsewhere, for users to download them. While it is private, update checks fail. The check at startup only logs the failure.

**The signing key.**

- The private key and its password are the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep an offline backup of both.
- Installed apps accept only updates signed with this key. If it is lost or replaced, those installs can't update themselves and must be reinstalled by hand.
- To replace it:
  1. Run `tauri signer generate -w <file>`. Call `node_modules/.bin/tauri` directly: `pnpm tauri` echoes the command line, password included.
  2. Put the new public key in `tauri.conf.json`.
  3. Update both secrets.

## Signing

**macOS:** bundles are signed ad hoc (`signingIdentity: "-"`), which seals the bundle.

- Users open a downloaded copy once through **System Settings → Privacy & Security → Open Anyway**.
- The hardened runtime is off. Under an ad hoc signature, its library validation refuses the bundled libpython ("different Team IDs").
- To notarize:
  1. Get a Developer ID certificate.
  2. Pass it to the build through the `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` and `APPLE_SIGNING_IDENTITY` environment variables in `bundle.yml`, plus `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` (or an App Store Connect API key) for notarization.
  3. Turn `hardenedRuntime` back on. libpython is then signed with the same Team ID and loads.

**Windows:** installers are unsigned, so SmartScreen warns on first run. Code signing would be configured in `bundle.yml`.

## Repository settings

| Setting                                                                                   | Why                                                 |
| ----------------------------------------------------------------------------------------- | --------------------------------------------------- |
| Secret `TAURI_SIGNING_PRIVATE_KEY`                                                        | signs updates; releases fail without it             |
| Secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`                                               | the key's password                                  |
| Settings → Actions → General → "Allow GitHub Actions to create and approve pull requests" | the Release workflow opens the release pull request |
| Settings → General → Pull Requests: allow squash and rebase merging only                  | keeps `main` linear                                 |

## Troubleshooting

- **A macOS build stops at launch with "unknown path".** It was started through a symlinked path; on macOS `/tmp` is a link to `/private/tmp`. Tauri refuses executables reached through symlinks. Start it from its real location (Applications, or `/private/tmp/…`).
- **A zipped `.app` from a Bundle run fails `codesign --verify`.** The artifact zip dropped hidden files. The `.dmg` artifact and the release assets are complete.
- **The Release workflow shows skipped runs.** It is triggered by every merged pull request, but works only for `release/v*` branches. Skipped runs are expected.
- **CI on a release pull request says "action required".** See [Continuous integration](#continuous-integration). Approving is optional.
