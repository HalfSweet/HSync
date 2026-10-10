// @ts-check
/**
 * Sets the version everywhere it lives (AGENTS.md「发布」): package.json, which tauri.conf.json
 * reads, the Cargo workspace, and the workspace's crates in Cargo.lock. The release workflow runs
 * it for the version asked for and refuses one that does not come after the last release.
 *
 *   node scripts/version.mjs <version>
 */
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/

/**
 * Orders versions: by number, a pre-release before its release, pre-releases by their text.
 * @param {string} a
 * @param {string} b
 */
export function compareVersions(a, b) {
  const [, ...left] = SEMVER.exec(a) ?? []
  const [, ...right] = SEMVER.exec(b) ?? []
  for (let i = 0; i < 3; i++) {
    const difference = Number(left[i]) - Number(right[i])
    if (difference !== 0) return Math.sign(difference)
  }
  const [preLeft, preRight] = [left[3], right[3]]
  if (preLeft === preRight) return 0
  if (preLeft === undefined) return 1
  if (preRight === undefined) return -1
  return preLeft.localeCompare(preRight, 'en', { numeric: true })
}

/** @param {string} message @returns {never} */
function fail(message) {
  console.error(message)
  process.exit(1)
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('..', import.meta.url))
  const version = process.argv[2] ?? ''
  if (!SEMVER.test(version)) fail(`not a version: "${version}" (e.g. 0.2.0, 0.2.0-beta.1)`)

  const released = execFileSync('git', ['tag', '--list', 'v*'], { cwd: root, encoding: 'utf8' })
    .split('\n')
    .map((tag) => tag.slice(1))
    .filter((tag) => SEMVER.test(tag))
    .sort(compareVersions)
  const last = released.at(-1)
  if (released.includes(version)) fail(`${version} is released already`)
  if (last && compareVersions(version, last) <= 0) fail(`${version} does not come after ${last}`)

  const packagePath = `${root}/package.json`
  const packageText = readFileSync(packagePath, 'utf8')
  const current = /** @type {{ version: string }} */ (JSON.parse(packageText)).version
  if (compareVersions(version, current) < 0) fail(`${version} comes before ${current}`)
  writeFileSync(packagePath, packageText.replace(/^(\s*"version": ")[^"]*"/m, `$1${version}"`))

  const cargoPath = `${root}/Cargo.toml`
  const cargoText = readFileSync(cargoPath, 'utf8')
  const workspaceVersion = /(\[workspace\.package\][^[]*?\nversion = ")[^"]*"/
  if (!workspaceVersion.test(cargoText)) fail('Cargo.toml has no [workspace.package] version')
  writeFileSync(cargoPath, cargoText.replace(workspaceVersion, `$1${version}"`))
  // Only the workspace's own entries change.
  execFileSync('cargo', ['update', '--workspace'], { cwd: root, stdio: 'inherit' })
  console.warn(`version ${version}`)
}
