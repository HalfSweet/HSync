// @ts-check
/**
 * Builds the installers for this machine as releases are built: on macOS a universal build for
 * Apple silicon and Intel, which needs both Rust targets installed. Arguments go on to
 * `tauri build`:
 *
 *   pnpm bundle [--bundles app]
 */
import { spawnSync } from 'node:child_process'

const UNIVERSAL = 'universal-apple-darwin'
const mac = process.platform === 'darwin'
const result = spawnSync(
  'pnpm',
  ['tauri', 'build', ...(mac ? ['--target', UNIVERSAL] : []), ...process.argv.slice(2)],
  {
    stdio: 'inherit',
    // pnpm is a batch file on Windows, which only a shell runs.
    shell: process.platform === 'win32',
  },
)
process.exit(result.status ?? 1)
