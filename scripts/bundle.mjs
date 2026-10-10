// @ts-check
/**
 * Builds the installers for this machine as releases are built, with the bundled Python.
 * Arguments go on to `tauri build`:
 *
 *   pnpm bundle [--bundles app]
 */
import { spawnSync } from 'node:child_process'

import { hostTriple, prepare } from './python-runtime.mjs'

const runtime = await prepare(hostTriple())
const result = spawnSync(
  'pnpm',
  ['tauri', 'build', '--config', runtime.tauriConfig, ...process.argv.slice(2)],
  {
    stdio: 'inherit',
    // pnpm is a batch file on Windows, which only a shell runs.
    shell: process.platform === 'win32',
    env: { ...process.env, PYO3_CONFIG_FILE: runtime.pyo3Config },
  },
)
process.exit(result.status ?? 1)
