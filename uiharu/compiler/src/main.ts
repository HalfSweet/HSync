// SPDX-License-Identifier: Apache-2.0

/**
 * The compiler's command line: lowers every module of `uiharu/dsl/contracts/` and
 * `uiharu/dsl/decoders/` to the IR and writes `uiharu/dsl/ir/<name>.json`, formatted as the
 * repository formats JSON.
 *
 *   node uiharu/compiler/src/main.ts            write the IR files
 *   node uiharu/compiler/src/main.ts --check    exit 1 when a file is missing or out of date
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import * as prettier from 'prettier'

import { lowerModule } from './module.ts'
import { formatDiagnostic, loadProject, moduleFiles } from './program.ts'

const here = path.dirname(fileURLToPath(import.meta.url))
const dsl = path.resolve(here, '..', '..', 'dsl')
const check = process.argv.includes('--check')

const project = loadProject(path.join(dsl, 'tsconfig.json'))
const outDir = path.join(dsl, 'ir')
let failed = false
let stale = 0
for (const file of moduleFiles(project)) {
  const { module, diagnostics } = lowerModule(project, file)
  for (const diagnostic of diagnostics) console.error(formatDiagnostic(diagnostic))
  if (!module) {
    failed = true
    continue
  }
  const target = path.join(outDir, `${module.name}.json`)
  const options = (await prettier.resolveConfig(target)) ?? {}
  const text = await prettier.format(JSON.stringify(module, null, 2), {
    ...options,
    filepath: target,
  })
  if (check) {
    const current = fs.existsSync(target) ? fs.readFileSync(target, 'utf8') : undefined
    if (current !== text) {
      console.error(`${path.relative(process.cwd(), target)} is out of date; run pnpm dsl:compile`)
      stale += 1
    }
    continue
  }
  fs.mkdirSync(outDir, { recursive: true })
  fs.writeFileSync(target, text)
  console.error(`wrote ${path.relative(process.cwd(), target)}`)
}
if (failed || stale) process.exit(1)
