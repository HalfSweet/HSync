// @ts-check
/** Prepare decoder sources before Tauri starts Cargo or its development server. */
import { spawnSync } from 'node:child_process'

import { run } from '@tauri-apps/cli'

const args = process.argv.slice(2)
const command = args.find((argument) => !argument.startsWith('-'))
const help = args.includes('--help') || args.includes('-h')
if (!help && (command === 'dev' || command === 'build')) {
  const result = spawnSync('pnpm', ['dsl:generate'], {
    stdio: 'inherit',
    // pnpm is a batch file on Windows, which only a shell runs.
    shell: process.platform === 'win32',
  })
  if (result.error) console.error(result.error.message)
  if (result.status !== 0) process.exit(result.status ?? 1)
}

try {
  await run(args, 'pnpm tauri')
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error))
  process.exit(1)
}
