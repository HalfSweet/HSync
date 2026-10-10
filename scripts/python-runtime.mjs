// @ts-check
/**
 * Lays out the CPython that release builds embed: python-build-standalone for the
 * target, checked against its published digest and trimmed of what no decoder uses.
 *
 *   node scripts/python-runtime.mjs [--target <rust triple>] [--github-env]
 *
 * Under `target/python/<triple>/` it leaves
 * - `dist/python/`: the distribution as published; builds link against its libpython,
 * - `home/`: what the app ships as its `python` resource and starts Python with,
 * - `pyo3.cfg`: the `PYO3_CONFIG_FILE` linking against `dist/` (the distribution's sysconfig still
 *   names the machine it was built on, so pyo3 cannot ask the interpreter),
 * - `tauri.conf.json`: the bundle additions, for `tauri build --config`.
 *
 * `--github-env` also hands the paths to the later steps of a GitHub Actions job.
 */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  appendFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { basename, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

/** The python-build-standalone release and the CPython in it; update `DIGESTS` with them. */
const RELEASE = '20261009'
const VERSION = '3.14.8'

/** SHA-256 of each target's `install_only_stripped` archive, from the release's `SHA256SUMS`. */
const DIGESTS = {
  'aarch64-apple-darwin': '936217c0e6893f51af939474bee18ecf5e6fe47b986d2c163f19b57c682e4a83',
  'x86_64-apple-darwin': '0fe0c68a7f90458abd13b50a3d8f5f0cb5b2f0a1f33b6ef1445751cf2dc8b36e',
  'aarch64-pc-windows-msvc': '82ef531d9d23cfc0cf36d00d27cb4652dae370a1317c0b34b7eb19893b7366e0',
  'x86_64-pc-windows-msvc': '5b6fa7ddeabe23ae83d7b54e5043eb4fa1ccb10a73fa4c6b0f541dea8160d505',
  'aarch64-unknown-linux-gnu': '54f9cd13b97add6bbd077175b6d89f974cc582ece1c40ace1893067582edaf0e',
  'x86_64-unknown-linux-gnu': 'ddc902fd26460f728758c67d80417c1031edf97589f1f162c4ba3aa158a35998',
}

/** Standard library parts no decoder reaches for: tooling, Tk, tests, pip and stale bytecode. */
const UNUSED = new Set([
  '__phello__',
  '__pycache__',
  '_pyrepl',
  'ensurepip',
  'idlelib',
  'pydoc_data',
  'site-packages',
  'test',
  'tkinter',
  'turtle.py',
  'turtledemo',
  'venv',
])

/** Extension modules and libraries only those parts load: Tk, SQLite, OpenSSL, CPython's tests. */
const UNUSED_NATIVE =
  /^(_ctypes_test|_dbm|_hashlib|_sqlite3|_ssl|_test|_tkinter|lib(crypto|ssl|tommath)|sqlite3|tcl)/

const ROOT = fileURLToPath(new URL('..', import.meta.url))

/** @typedef {keyof typeof DIGESTS} Triple */

/**
 * Where a prepared runtime is.
 * @typedef {object} Runtime
 * @property {string} home The Python home the app ships.
 * @property {string} libDir Where the loader finds libpython when running builds in place.
 * @property {string} pyo3Config The `PYO3_CONFIG_FILE`.
 * @property {string} tauriConfig The `tauri build --config` file.
 */

/** @returns {Triple} The target of this machine. */
export function hostTriple() {
  /** @type {Record<string, string>} */
  const arch = { arm64: 'aarch64', x64: 'x86_64' }
  /** @type {Record<string, string>} */
  const os = { darwin: 'apple-darwin', linux: 'unknown-linux-gnu', win32: 'pc-windows-msvc' }
  const triple = `${arch[process.arch]}-${os[process.platform]}`
  if (!(triple in DIGESTS))
    throw new Error(`no Python runtime for ${process.platform}/${process.arch}`)
  return /** @type {Triple} */ (triple)
}

/** Paths as the Tauri configuration and pyo3 take them on every platform. */
function slashes(/** @type {string} */ path) {
  return path.replaceAll('\\', '/')
}

/** Writes `text` unless the file holds it already, so builds keyed on its time stay fresh. */
function writeIfChanged(/** @type {string} */ path, /** @type {string} */ text) {
  if (existsSync(path) && readFileSync(path, 'utf8') === text) return
  writeFileSync(path, text)
}

/** Downloads the archive once and checks it on every use. */
async function archive(/** @type {Triple} */ triple) {
  const name = `cpython-${VERSION}+${RELEASE}-${triple}-install_only_stripped.tar.gz`
  const downloads = join(ROOT, 'target', 'python', 'downloads')
  const path = join(downloads, name)
  if (!existsSync(path)) {
    const url = `https://github.com/astral-sh/python-build-standalone/releases/download/${RELEASE}/${encodeURIComponent(name)}`
    console.warn(`downloading ${url}`)
    const response = await fetch(url)
    if (!response.ok) throw new Error(`${url}: ${response.status} ${response.statusText}`)
    mkdirSync(downloads, { recursive: true })
    writeFileSync(path, Buffer.from(await response.arrayBuffer()))
  }
  const digest = createHash('sha256').update(readFileSync(path)).digest('hex')
  if (digest !== DIGESTS[triple]) {
    rmSync(path)
    throw new Error(`${name}: SHA-256 ${digest} is not the published ${DIGESTS[triple]}`)
  }
  return path
}

/** Unpacks with the system's tar; on Windows the bundled bsdtar, which takes drive letters. */
function untar(/** @type {string} */ file, /** @type {string} */ into) {
  const tar =
    process.platform === 'win32'
      ? join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe')
      : 'tar'
  const result = spawnSync(tar, ['-xzf', file, '-C', into], { stdio: 'inherit' })
  if (result.status !== 0) throw new Error(`tar failed on ${file}`)
}

/** Copies the parts of a standard library or extension directory the runtime keeps. */
function copyKept(/** @type {string} */ from, /** @type {string} */ to) {
  cpSync(from, to, {
    recursive: true,
    filter: (source) => {
      const name = basename(source)
      // Hidden files are placeholders such as lib-dynload/.empty; artifact uploads drop them,
      // which breaks the signature of a macOS bundle that sealed them.
      return (
        !name.startsWith('.') &&
        !UNUSED.has(name) &&
        !name.startsWith('config-') &&
        !UNUSED_NATIVE.test(name)
      )
    },
  })
}

/**
 * Prepares the runtime for `triple` under `target/python/<triple>/`.
 * @param {Triple} triple
 * @returns {Promise<Runtime>}
 */
export async function prepare(triple) {
  const [major, minor] = VERSION.split('.')
  const work = join(ROOT, 'target', 'python', triple)
  const dist = join(work, 'dist')
  const python = join(dist, 'python')
  const home = join(work, 'home')
  const windows = triple.endsWith('-windows-msvc')
  const mac = triple.endsWith('-apple-darwin')

  rmSync(dist, { recursive: true, force: true })
  rmSync(home, { recursive: true, force: true })
  mkdirSync(dist, { recursive: true })
  untar(await archive(triple), dist)

  /** @type {Record<string, string>} */
  const resources = { [`${slashes(home)}/`]: 'python/' }
  /** @type {Record<string, unknown>} */
  const bundle = { resources }
  let libDir
  if (windows) {
    copyKept(join(python, 'Lib'), join(home, 'Lib'))
    copyKept(join(python, 'DLLs'), join(home, 'DLLs'))
    // Beside the executable, where Windows looks for the DLLs it imports.
    for (const dll of [`python${major}${minor}.dll`, 'vcruntime140.dll', 'vcruntime140_1.dll']) {
      resources[slashes(join(python, dll))] = dll
    }
    libDir = python
  } else {
    const stdlib = join('lib', `python${major}.${minor}`)
    copyKept(join(python, stdlib), join(home, stdlib))
    if (mac) {
      // Contents/Frameworks; its install name is @rpath/…, and Tauri adds the rpath.
      bundle.macOS = {
        frameworks: [slashes(join(python, 'lib', `libpython${major}.${minor}.dylib`))],
      }
      libDir = join(python, 'lib')
    } else {
      // Beside the standard library; src-tauri/build.rs points the executable's RUNPATH here.
      const library = `libpython${major}.${minor}.so.1.0`
      cpSync(join(python, 'lib', library), join(home, 'lib', library))
      libDir = join(home, 'lib')
    }
  }

  const pyo3Config = join(work, 'pyo3.cfg')
  writeIfChanged(
    pyo3Config,
    [
      'implementation=CPython',
      `version=${major}.${minor}`,
      'shared=true',
      `target_abi=CPython-gil_enabled-${major}.${minor}`,
      `lib_name=${windows ? `python${major}${minor}` : `python${major}.${minor}`}`,
      `lib_dir=${slashes(join(python, windows ? 'libs' : 'lib'))}`,
      `executable=${slashes(windows ? join(python, 'python.exe') : join(python, 'bin', 'python3'))}`,
      'pointer_width=64',
      'build_flags=',
      'python_framework_prefix=',
      'suppress_build_script_link_lines=false',
      '',
    ].join('\n'),
  )
  const tauriConfig = join(work, 'tauri.conf.json')
  writeIfChanged(tauriConfig, `${JSON.stringify({ bundle }, null, 2)}\n`)
  return { home, libDir, pyo3Config, tauriConfig }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({
    options: { target: { type: 'string' }, 'github-env': { type: 'boolean', default: false } },
  })
  const triple = /** @type {Triple} */ (values.target ?? hostTriple())
  if (!(triple in DIGESTS)) throw new Error(`no Python runtime for ${triple}`)
  const runtime = await prepare(triple)
  console.warn(`CPython ${VERSION} for ${triple} in ${runtime.home}`)
  if (values['github-env']) {
    const env = process.env.GITHUB_ENV
    if (!env) throw new Error('--github-env needs GITHUB_ENV')
    appendFileSync(
      env,
      [
        `PYO3_CONFIG_FILE=${runtime.pyo3Config}`,
        `HSYNC_PYTHON_HOME=${runtime.home}`,
        `HSYNC_PYTHON_LIB=${runtime.libDir}`,
        `HSYNC_PYTHON_TAURI_CONFIG=${runtime.tauriConfig}`,
        '',
      ].join('\n'),
    )
  }
}
