// @ts-check
/**
 * Asks for a release (AGENTS.md "Releasing"): the Release workflow sets the version on a branch
 * from main and opens its pull request; merging that pull request releases it. Needs the GitHub
 * CLI.
 *
 *   pnpm release <version>
 */
import { spawnSync } from 'node:child_process'

const version = process.argv[2] ?? ''
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error('usage: pnpm release <version>, e.g. pnpm release 0.2.0')
  process.exit(1)
}
const result = spawnSync(
  'gh',
  ['workflow', 'run', 'release.yml', '--ref', 'main', '--field', `version=${version}`],
  { stdio: 'inherit' },
)
if (result.status === 0) {
  console.warn(`The pull request for ${version} opens in a minute: gh pr view release/v${version}`)
}
process.exit(result.status ?? 1)
