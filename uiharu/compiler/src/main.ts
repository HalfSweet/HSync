// SPDX-License-Identifier: Apache-2.0

/**
 * The compiler's command line: lowers the production modules and backend acceptance
 * fixtures to disposable, ignored IR. All sources must compile before outputs are written.
 *
 *   pnpm dsl:compile    write the IR files, removing obsolete JSON
 *   pnpm dsl:generate   compile IR and generate local Rust sources
 *   pnpm dsl:check      generate sources and type-check the generated Rust
 */

import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { backendFixtureIr, backendFixtures } from './backend-fixtures.ts'
import type { Module } from './ir.ts'
import { lowerModule } from './module.ts'
import { formatDiagnostic, loadProject, moduleFiles } from './program.ts'
import { writeIrModules } from './write-ir.ts'

const here = path.dirname(fileURLToPath(import.meta.url))
const dsl = path.resolve(here, '..', '..', 'dsl')

const project = loadProject(path.join(dsl, 'tsconfig.json'))
const outDir = path.join(dsl, 'ir')
let failed = false
const modules: Module[] = []
for (const file of moduleFiles(project)) {
  const { module, diagnostics } = lowerModule(project, file)
  for (const diagnostic of diagnostics) console.error(formatDiagnostic(diagnostic))
  if (!module) {
    failed = true
    continue
  }
  modules.push(module)
}
if (failed) process.exit(1)
const fixtures = backendFixtures()
await writeIrModules(outDir, modules)
await writeIrModules(backendFixtureIr, fixtures)
