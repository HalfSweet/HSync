// @ts-check
/**
 * Independent uiharu versions, package verification and crates.io publishing.
 * Source generation is a maintainer step; consumers build the packaged Rust alone.
 */
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { compareVersions } from './version.mjs'

export const packages = [
  'uiharu-core',
  'uiharu-ir',
  'uiharu-codegen',
  'uiharu-dsl-protocols',
  'uiharu-protocols',
  'uiharu-interp',
  'uiharu-registry',
  'uiharu-cli',
  'uiharu',
]
const root = fileURLToPath(new URL('..', import.meta.url))
const selection = packages.flatMap((name) => ['-p', name])
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/

/** @param {string} version */
export function validateVersion(version) {
  const match = semver.exec(version)
  if (!match || match[4]?.split('.').some((part) => /^0\d+$/.test(part))) {
    throw new Error(`Invalid release version: ${version}`)
  }
}

/** Update package and uiharu dependency versions without touching unrelated packages.
 * @param {string} manifest
 * @param {string} version
 */
export function versionManifest(manifest, version) {
  validateVersion(version)
  if (!/^version = "[^"]+"$/m.test(manifest)) {
    throw new Error('A uiharu package must have its own explicit version')
  }
  return manifest
    .replace(/^version = "[^"]+"$/m, `version = "${version}"`)
    .replace(/^(uiharu(?:-[a-z-]+)?\s*=\s*\{[^\n}]*\bversion = ")[^"]+/gm, `$1${version}`)
}

/** @param {string} command @param {string[]} args @param {string} [cwd] */
function run(command, args, cwd = root) {
  execFileSync(command, args, { cwd, stdio: 'inherit' })
}

/** @param {string} command @param {string[]} args @param {string} [cwd] */
function capture(command, args, cwd = root) {
  return execFileSync(command, args, { cwd, encoding: 'utf8' }).trim()
}

/** Cargo regards explicitly included, ignored generated files as dirty. Check the
 * authored tree separately before allowing those generated files into an archive.
 */
function requireCleanSource() {
  if (capture('git', ['status', '--porcelain', '--untracked-files=normal'])) {
    throw new Error('Commit authored changes before packaging or publishing a release')
  }
}

/** Read and validate the release family from Cargo's resolved metadata. */
function metadata() {
  const result =
    /** @type {{ target_directory: string, packages: {
     * name: string, version: string, license: string, rust_version: string,
     * publish: string[] | null, manifest_path: string,
     * dependencies: { name: string, path?: string, req: string, kind: string | null }[]
     * }[] }} */ (
      JSON.parse(capture('cargo', ['metadata', '--no-deps', '--format-version', '1', '--locked']))
    )
  const crates = packages.map((name) => {
    const crate = result.packages.find((candidate) => candidate.name === name)
    if (!crate) throw new Error(`Missing package: ${name}`)
    if (crate.license !== 'Apache-2.0' || !crate.publish?.includes('crates-io')) {
      throw new Error(`${name} must be Apache-2.0 and publishable to crates.io`)
    }
    if (crate.dependencies.some((dependency) => dependency.name.startsWith('hsync-'))) {
      throw new Error(`${name} depends on HSync`)
    }
    for (const dependency of crate.dependencies.filter((dependency) => dependency.path)) {
      const sourceTest = dependency.kind === 'dev' && dependency.req === '*'
      if (
        !packages.includes(dependency.name) ||
        (!sourceTest && dependency.req !== `^${crate.version}`)
      ) {
        throw new Error(`${name}: ${dependency.name} must be a versioned uiharu dependency`)
      }
    }
    return crate
  })
  if (new Set(crates.map((crate) => crate.version)).size !== 1) {
    throw new Error('All uiharu packages must share one release version')
  }
  const version = crates[0]?.version
  if (!version) throw new Error('No uiharu release version')
  validateVersion(version)
  return { crates, version, target: result.target_directory }
}

/** Set a release version using authored manifests, independently of the app version.
 * @param {string} version
 */
function setVersion(version) {
  validateVersion(version)
  const tags = capture('git', ['tag', '--list', 'uiharu-v*'])
    .split('\n')
    .map((tag) => tag.slice('uiharu-v'.length))
    .filter((tag) => semver.test(tag))
    .sort(compareVersions)
  const last = tags.at(-1)
  if (last && compareVersions(version, last) <= 0) {
    throw new Error(`${version} must come after released version ${last}`)
  }
  // Validate every manifest before modifying any of them.
  const updates = packages.map((name) => {
    const file = path.join(root, 'uiharu/crates', name, 'Cargo.toml')
    const source = readFileSync(file, 'utf8')
    const current = /^version = "([^"]+)"$/m.exec(source)?.[1]
    if (!current || compareVersions(version, current) < 0) {
      throw new Error(`${name}: ${version} is older than ${current ?? 'an unknown version'}`)
    }
    return { file, contents: versionManifest(source, version) }
  })
  for (const { file, contents } of updates) writeFileSync(file, contents)
  run('cargo', ['update', '--workspace'])
}

/** Verify archives outside the source tree, including a Rust-only CLI installation.
 * @param {ReturnType<typeof metadata>} release
 * @param {string | undefined} toolchain
 */
function smoke(release, toolchain) {
  const temporary = mkdtempSync(path.join(tmpdir(), 'uiharu-packages-'))
  const cargo = toolchain ? [`+${toolchain}`] : []
  try {
    const patches = ['[patch.crates-io]']
    for (const crate of release.crates) {
      const archive = path.join(release.target, 'package', `${crate.name}-${crate.version}.crate`)
      run('tar', ['-xzf', archive, '-C', temporary])
      const directory = path.join(temporary, `${crate.name}-${crate.version}`)
      const files = capture('tar', ['-tf', archive]).split('\n')
      for (const required of ['Cargo.toml', 'README.md', 'LICENSE']) {
        if (!files.includes(`${crate.name}-${crate.version}/${required}`)) {
          throw new Error(`${crate.name} archive is missing ${required}`)
        }
      }
      if (
        !readFileSync(path.join(directory, 'LICENSE')).equals(
          readFileSync(path.join(root, 'uiharu/LICENSE')),
        )
      ) {
        throw new Error(`${crate.name} archive does not contain the engine's Apache-2.0 license`)
      }
      if (
        crate.name === 'uiharu-dsl-protocols' &&
        !files.some((file) => file.endsWith('/src/generated/spi.rs'))
      ) {
        throw new Error('The DSL archive is missing its generated Rust')
      }
      if (
        crate.name === 'uiharu-registry' &&
        !files.some((file) => file.endsWith('/manifests/spi.json'))
      ) {
        throw new Error('The registry archive is missing its built-in manifests')
      }
      patches.push(`${crate.name} = { path = ${JSON.stringify(directory)} }`)
    }
    const config = path.join(temporary, 'patches.toml')
    writeFileSync(config, `${patches.join('\n')}\n`)
    const consumer = path.join(temporary, 'consumer')
    mkdirSync(path.join(consumer, 'src'), { recursive: true })
    const facade = path.join(temporary, `uiharu-${release.version}`)
    const manifest = (features = '') =>
      `[package]\nname = "uiharu-consumer"\nversion = "0.0.0"\nedition = "2024"\nrust-version = "1.90"\n\n[dependencies]\nuiharu = { path = ${JSON.stringify(facade)}${features} }\nserde_json = "1"\n`
    writeFileSync(path.join(consumer, 'Cargo.toml'), manifest(', default-features = false'))
    writeFileSync(
      path.join(consumer, 'src/main.rs'),
      'fn main() { let _ = uiharu::Graph::new(); }\n',
    )
    run('cargo', [...cargo, 'check', '--config', config], consumer)
    const tree = capture(
      'cargo',
      [...cargo, 'tree', '--edges', 'normal', '--config', config],
      consumer,
    )
    if (/uiharu-(protocols|registry|interp|codegen)/.test(tree)) {
      throw new Error('The core-only facade pulled in optional protocol or registry dependencies')
    }
    for (const example of ['spi', 'registry']) {
      writeFileSync(
        path.join(consumer, 'Cargo.toml'),
        manifest(example === 'registry' ? ', features = ["registry", "testing"]' : ''),
      )
      writeFileSync(
        path.join(consumer, 'src/main.rs'),
        readFileSync(path.join(facade, 'examples', `${example}.rs`)),
      )
      run('cargo', [...cargo, 'run', '--config', config], consumer)
    }
    // The generator is not a runtime dependency, so check its own archive as well.
    run(
      'cargo',
      [
        ...cargo,
        'check',
        '--manifest-path',
        path.join(temporary, `uiharu-codegen-${release.version}`, 'Cargo.toml'),
        '--config',
        config,
      ],
      consumer,
    )
    run(
      'cargo',
      [
        ...cargo,
        'install',
        '--path',
        path.join(temporary, `uiharu-cli-${release.version}`),
        '--root',
        path.join(temporary, 'install'),
        '--config',
        config,
      ],
      consumer,
    )
    const cli = path.join(temporary, 'install/bin/uiharu')
    run(cli, ['gen', 'flash.bin'], temporary)
    const decoded = capture(cli, ['decode', 'flash.bin', '--stats'], temporary)
    if (!decoded.split('\n').some((line) => line.includes('"kind":"'))) {
      throw new Error('The installed CLI did not decode the synthetic capture')
    }
    console.warn(
      `Verified uiharu ${release.version} from archives${toolchain ? ` on Rust ${toolchain}` : ''}`,
    )
  } finally {
    rmSync(temporary, { recursive: true, force: true })
  }
}

/** @param {string[]} args */
function main(args) {
  const [command, argument] = args
  if (command === 'version' && argument) return setVersion(argument)
  if (command === 'release' && argument) {
    validateVersion(argument)
    return run('gh', [
      'workflow',
      'run',
      'uiharu-release.yml',
      '--ref',
      'main',
      '--field',
      `version=${argument}`,
    ])
  }
  if (!['package', 'smoke', 'publish', 'verify'].includes(command ?? '')) {
    throw new Error(
      'Usage: pnpm uiharu:{version|release} <version> | pnpm uiharu:{package|smoke|publish|verify}',
    )
  }
  const releaseCommand = command === 'package' || command === 'publish'
  const review = command === 'package' && args.slice(1).includes('--allow-dirty')
  if (releaseCommand && !review) requireCleanSource()
  if (command === 'package' && args.slice(1).some((flag) => flag !== '--allow-dirty')) {
    throw new Error('Usage: pnpm uiharu:package [--allow-dirty]')
  }
  if (command !== 'smoke') run('pnpm', ['dsl:generate'])
  if (releaseCommand && !review) requireCleanSource()
  const release = metadata()
  if (command === 'verify') {
    if (argument && argument !== release.version && argument !== `uiharu-v${release.version}`) {
      throw new Error(`Expected uiharu-v${release.version}, got ${argument}`)
    }
    console.warn(`uiharu ${release.version}: ${release.crates.length} Apache-2.0 packages`)
    return
  }
  if (command === 'smoke') return smoke(release, argument)
  if (command === 'publish') {
    // Keep Cargo's archive rebuild verification and dependency ordering enabled.
    return run('cargo', ['publish', '--locked', '--allow-dirty', ...selection, '--all-features'])
  }
  run('cargo', ['package', '--locked', '--allow-dirty', ...selection, '--all-features'])
  for (const crate of release.crates) {
    const archive = path.join(release.target, 'package', `${crate.name}-${crate.version}.crate`)
    const checksum = createHash('sha256').update(readFileSync(archive)).digest('hex')
    console.warn(`${crate.name} ${crate.version}: ${checksum}`)
  }
  smoke(release, undefined)
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2))
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
  }
}
