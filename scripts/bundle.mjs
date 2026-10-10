// @ts-check
/**
 * Builds the installers for this machine as releases are built, with the bundled Python: on macOS
 * a universal build for Apple silicon and Intel, which needs both Rust targets installed.
 * Arguments go on to `tauri build`:
 *
 *   pnpm bundle [--bundles app]
 */
import { spawnSync } from 'node:child_process'

import { UNIVERSAL, hostTriple, prepare } from './python-runtime.mjs'

const mac = process.platform === 'darwin'
const runtime = await prepare(mac ? UNIVERSAL : hostTriple())
const result = spawnSync(
  'pnpm',
  [
    'tauri',
    'build',
    '--config',
    runtime.tauriConfig,
    ...(mac ? ['--target', UNIVERSAL] : []),
    ...process.argv.slice(2),
  ],
  {
    stdio: 'inherit',
    // pnpm is a batch file on Windows, which only a shell runs.
    shell: process.platform === 'win32',
    env: { ...process.env, PYO3_CONFIG_FILE: runtime.pyo3Config },
  },
)
process.exit(result.status ?? 1)
