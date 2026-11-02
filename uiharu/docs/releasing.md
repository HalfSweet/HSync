# Publishing uiharu

uiharu is an Apache-2.0 Rust library and command line, released independently of the
HSync desktop application. Its source remains in this repository. Applications can
use the crates without depending on HSync, Tauri, a USB driver, Node or pnpm.

## Packages and compatibility

The `uiharu` facade re-exports the core runtime. Its default `protocols` feature
provides the built-in protocol implementations; the optional `registry` feature
adds decoder discovery, manifests, JSON pipeline construction and runtime IR
loading. `default-features = false` selects the core runtime alone.

Advanced applications may depend directly on `uiharu-core`, `uiharu-protocols`,
`uiharu-registry`, `uiharu-ir` or `uiharu-interp`. `uiharu-dsl-protocols` holds
generated Rust used by the protocol library. `uiharu-cli` installs the `uiharu`
command, and `uiharu-codegen` is the separate developer tool for Rust generation.
Publishing these packages individually preserves Cargo's dependency resolution;
users adding the facade do not need to add each implementation crate themselves.

All nine packages share an explicit release version, starting at `0.1.0`. HSync's
workspace/app version does not select their version. Before 1.0, incompatible API
changes increment the minor version; patch releases preserve API compatibility.
The declared minimum supported Rust version is 1.90 and is checked using the
unpacked release archives. An MSRV increase is called out in release notes.

The decoder IR has a separate format version: a module's `ir` field must equal
`uiharu_ir::IR_VERSION` (currently 1). Validation rejects unsupported versions.
Changes to serialized events and pipeline descriptions must be considered along
with Rust API compatibility; persisted IR modules are not promised compatibility
across an IR format-version change. Compiler, generator and interpreter changes
must remain in the same release family.

## What is published

Authored TypeScript stays in Git; generated IR and Rust stay ignored. Maintainers
run `pnpm dsl:generate` before packaging. Explicit Cargo `include` lists put the
generated `uiharu-dsl-protocols/src/generated/` files into the `.crate`, along with
the runtime source, registry manifests, each package's README and Apache license.
Generated Rust is never edited or committed by hand. Source-checkout acceptance
tests and compiler fixtures remain in the repository rather than the library
packages.

The protocol/interpreter equivalence tests use unversioned path-only
dev-dependencies. Cargo omits these repository-only edges from published manifests,
so they cannot create a first-publish dependency cycle. Runtime dependencies always
have both a local path and a registry version.

The code generator is not a build dependency of the protocol libraries. Published
libraries and the CLI build with Cargo alone; no build script invokes Node/pnpm,
reads a sibling checkout or downloads a decoder implementation. Using the
`uiharu-codegen` developer tool itself requires rustfmt when generating Rust.

Before a release, run the repository checks that cover the change, plus:

```sh
pnpm dsl:generate
cargo clippy --locked -p 'uiharu*' --all-targets --all-features -- -D warnings
cargo test --locked -p 'uiharu*' --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked -p 'uiharu*' --all-features --no-deps --lib
pnpm uiharu:package
rustup toolchain install 1.90.0 --profile minimal
pnpm uiharu:smoke 1.90.0
```

`uiharu:package` generates sources, validates the release metadata and packages all
nine crates together. Cargo verifies their builds and resolves the unpublished
family through its temporary packaging registry. The script then unpacks the
archives outside the repository, checks required files and runs a new consumer
with core-only, default-protocol and registry configurations. Finally it installs
the CLI from the archives and decodes a synthetic capture. Local path patches in
this temporary consumer point only at the unpacked archives. `uiharu:smoke` repeats
that consumer verification with an optional Rust toolchain argument. The source
tree must be committed for a release; `uiharu:package --allow-dirty` is available
for reviewing an unfinished change.

## First release

The owner chooses the release version. Check the names are available on crates.io,
then prepare the release with `pnpm uiharu:version <version>`. This updates only
the uiharu package/dependency versions and the Cargo lockfile. Use git-cliff to
generate the independent changelog, and commit the authored changes:

```sh
pnpm exec git-cliff --config uiharu/cliff.toml --include-path 'uiharu/**' \
  --tag uiharu-v0.1.0 --output uiharu/CHANGELOG.md
pnpm exec prettier --write uiharu/CHANGELOG.md
```

After all checks pass, use the owner's existing local Cargo login for the initial
publish:

```sh
pnpm uiharu:publish
git tag uiharu-v0.1.0
git push origin uiharu-v0.1.0
```

The publisher selects the nine uiharu packages explicitly, leaving every HSync
crate private. Cargo handles dependency ordering, archive rebuild verification
and index availability. Publishing is not atomic: if it stops after publishing
some packages, inspect crates.io and publish only the missing packages at that
same version. A published version cannot be replaced.

Create a GitHub release named `uiharu <version>` from the engine tag, using the
engine changelog. Set `--latest=false` so the desktop app's download link keeps
pointing at a HSync installer release. The bootstrap tag does not trigger the
PR-based publishing workflow.

## OIDC for subsequent releases

For each of the nine crates, configure crates.io Trusted Publishing with:

| Setting            | Value                |
| ------------------ | -------------------- |
| Repository owner   | `HalfSweet`          |
| Repository name    | `HSync`              |
| Workflow filename  | `uiharu-release.yml` |
| GitHub environment | `uiharu-release`     |

Use the crates.io settings UI, or its authenticated Trusted Publishing API with a
local credential authorized for that scope. Do not copy the local Cargo token
into GitHub secrets. The workflow uses `id-token: write` and
`rust-lang/crates-io-auth-action` to obtain a short-lived token; the action revokes
it when the job completes. See the [crates.io documentation](https://crates.io/docs/trusted-publishing).

Once the release workflow is on main, request a release with:

```sh
pnpm uiharu:release 0.1.1
```

The workflow opens `release/uiharu-v0.1.1`, updates the uiharu versions and
`uiharu/CHANGELOG.md`, and leaves the HSync version untouched. Review and merge
that pull request to release. The merged source passes the reusable uiharu checks,
including independent archive and MSRV verification, before it is tagged and
published with OIDC. Release notes use only engine tags and commits touching
`uiharu/`. HSync's release process remains separate.

Neither local release commands nor this workflow watch CI runs. Actions runs can
be reviewed when needed; no Cargo API token is used as a CI fallback.
