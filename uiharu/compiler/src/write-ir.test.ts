// SPDX-License-Identifier: Apache-2.0
// @vitest-environment node

import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { expect, test } from 'vitest'

import type { Module } from './ir.ts'
import { writeIrModules } from './write-ir.ts'

test('a clean checkout can generate IR and discard modules removed from TypeScript', async () => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'uiharu-ir-'))
  const directory = path.join(temporary, 'ir')
  const module = JSON.parse(
    fs.readFileSync(
      new URL('../../crates/uiharu-ir/tests/fixtures/pwm.json', import.meta.url),
      'utf8',
    ),
  ) as Module
  try {
    await writeIrModules(directory, [module])
    expect(JSON.parse(fs.readFileSync(path.join(directory, 'pwm.json'), 'utf8'))).toEqual(module)
    fs.writeFileSync(path.join(directory, 'README.md'), 'Local generation notes')

    const renamed = { ...module, name: 'renamed-pwm' }
    await writeIrModules(directory, [renamed])
    expect(fs.existsSync(path.join(directory, 'pwm.json'))).toBe(false)
    expect(JSON.parse(fs.readFileSync(path.join(directory, 'renamed-pwm.json'), 'utf8'))).toEqual(
      renamed,
    )
    expect(fs.readFileSync(path.join(directory, 'README.md'), 'utf8')).toBe(
      'Local generation notes',
    )
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true })
  }
})
