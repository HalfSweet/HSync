// SPDX-License-Identifier: Apache-2.0

import fs from 'node:fs'
import path from 'node:path'

import * as prettier from 'prettier'

import type { Module } from './ir.ts'

/** Write disposable IR, dropping JSON for modules no longer present in the source set. */
export async function writeIrModules(directory: string, modules: readonly Module[]): Promise<void> {
  const outputs = new Map<string, string>()
  for (const module of modules) {
    const filename = `${module.name}.json`
    const target = path.join(directory, filename)
    if (outputs.has(filename)) throw new Error(`duplicate IR module: ${module.name}`)
    outputs.set(
      filename,
      await prettier.format(JSON.stringify(module, null, 2), {
        ...((await prettier.resolveConfig(target)) ?? {}),
        filepath: target,
      }),
    )
  }

  fs.mkdirSync(directory, { recursive: true })
  for (const [filename, text] of outputs) {
    fs.writeFileSync(path.join(directory, filename), text)
  }
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith('.json') && !outputs.has(entry.name)) {
      fs.unlinkSync(path.join(directory, entry.name))
    }
  }
  console.error(`wrote ${outputs.size} IR modules to ${path.relative(process.cwd(), directory)}`)
}
