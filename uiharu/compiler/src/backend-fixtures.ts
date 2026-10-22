// SPDX-License-Identifier: Apache-2.0

/** Load the backend acceptance sources virtually, without production decoder registration.
 * Regenerate their local IR together with production modules using `pnpm dsl:compile`. */
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import type { Module } from './ir.ts'
import { lowerModule } from './module.ts'
import { formatDiagnostic, loadProject } from './program.ts'
import { writeIrModules } from './write-ir.ts'

const here = path.dirname(fileURLToPath(import.meta.url))
const dsl = path.resolve(here, '../../dsl')
export const backendFixtureIr = path.resolve(
  here,
  '../../crates/uiharu-interp/tests/fixtures/backend/ir',
)

/** Compile the TS fixtures using the same project and lowering as production decoders. */
export function backendFixtures(): Module[] {
  const sources = ['backend-contracts', 'backend-probe', 'backend-forward']
  const virtual = Object.fromEntries(
    sources.map((name) => [
      path.join(dsl, 'decoders', `${name}.ts`),
      fs.readFileSync(path.resolve(here, '../fixtures', `${name}.ts`), 'utf8'),
    ]),
  )
  const project = loadProject(path.join(dsl, 'tsconfig.json'), virtual)
  return Object.keys(virtual).map((name) => {
    const file = project.program.getSourceFile(name)
    if (!file) throw new Error(`fixture source ${name} is in the project`)
    const { module, diagnostics } = lowerModule(project, file)
    if (!module) throw new Error(diagnostics.map(formatDiagnostic).join('\n'))
    return module
  })
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await writeIrModules(backendFixtureIr, backendFixtures())
}
