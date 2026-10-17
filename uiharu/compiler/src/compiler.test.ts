// SPDX-License-Identifier: Apache-2.0
// @vitest-environment node

/**
 * The compiler on the reference decoders and on snippets outside the subset: the PWM
 * decoder lowers to the IR written by hand in `uiharu-ir`'s fixtures, every reference
 * decoder lowers to the IR checked in under `uiharu/dsl/ir/`, and what the subset leaves out
 * is reported where it is.
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, test } from 'vitest'

import { lowerModule } from './module.ts'
import { loadProject, moduleFiles, type Diagnostic } from './program.ts'

const here = path.dirname(fileURLToPath(import.meta.url))
const dsl = path.resolve(here, '..', '..', 'dsl')
const config = path.join(dsl, 'tsconfig.json')
const fixture = path.resolve(
  here,
  '..',
  '..',
  'crates',
  'uiharu-ir',
  'tests',
  'fixtures',
  'pwm.json',
)

function read(file: string): unknown {
  return JSON.parse(fs.readFileSync(file, 'utf8')) as unknown
}

describe('the modules', () => {
  const project = loadProject(config)
  const files = moduleFiles(project)

  test('are the contracts and the decoders', () => {
    expect(files.map((file) => path.basename(file.fileName))).toEqual([
      'i2c.ts',
      'pwm.ts',
      'spi-nor.ts',
      'spi.ts',
      'uart.ts',
    ])
  })

  test('pwm lowers to the IR written by hand', () => {
    const file = files.find((candidate) => candidate.fileName.endsWith('pwm.ts'))
    if (!file) throw new Error('pwm.ts is a reference decoder')
    const { module, diagnostics } = lowerModule(project, file)
    expect(diagnostics).toEqual([])
    expect(module).toEqual(read(fixture))
  })

  test('i2c declares a contract and no decoder', () => {
    const file = files.find((candidate) => candidate.fileName.endsWith('i2c.ts'))
    if (!file) throw new Error('i2c.ts is a contract module')
    const { module, diagnostics } = lowerModule(project, file)
    expect(diagnostics).toEqual([])
    expect(module?.decoder).toBeUndefined()
    expect(module?.contracts?.map((contract) => contract.framing)).toEqual([
      {
        begin: 'begin',
        items: ['address', 'data', 'restart'],
        end: 'end',
        id: 'tx',
        partial: 'partial',
        reason: 'reason',
        normal: ['stop', 'restart'],
      },
    ])
  })

  test.each(files.map((file) => [path.basename(file.fileName, '.ts'), file] as const))(
    '%s lowers to the IR checked in',
    (name, file) => {
      const { module, diagnostics } = lowerModule(project, file)
      expect(diagnostics).toEqual([])
      expect(module?.name).toBe(name)
      // Out of date? Run `pnpm dsl:compile` and commit the result.
      expect(module).toEqual(read(path.join(dsl, 'ir', `${name}.json`)))
    },
  )
})

/** A decoder module around `body`, the class body, and `extra` top-level statements. */
function snippet(body: string, extra = ''): string {
  return `
import type { Channel, Tick, u64 } from 'uiharu'
import { LogicDecoder, atLeast, defineLogicDecoder, eventSchema, mask, over } from 'uiharu'

export type Event = { kind: 'tick'; at: u64 }
export const events = eventSchema<Event>('test.events', 1)
export interface Config { channel: Channel }
${extra}
class Test extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.channel)
  last: Tick | undefined = undefined
  ${body}
}

export const test = defineLogicDecoder({
  name: 'test',
  category: 'line',
  config: { defaults: { channel: 0 } },
  requires: (config) => ({ channels: atLeast(config.channel + 1) }),
  output: events,
  decoder: Test,
})
`
}

const virtualFile = path.join(dsl, 'decoders', 'virtual-test.ts')

function compileSnippet(text: string): { ok: boolean; diagnostics: Diagnostic[] } {
  const project = loadProject(config, { [virtualFile]: text })
  const file = project.program.getSourceFile(virtualFile)
  if (!file) throw new Error('the virtual file is in the program')
  const { module, diagnostics } = lowerModule(project, file)
  return { ok: module !== undefined, diagnostics }
}

const RUN = `
  override async run(): Promise<void> {
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (this.last !== undefined) {
        await this.out.emit({ kind: 'tick', at: edge.tick - this.last }, over(this.last, edge.tick, edge.tick))
      }
      this.last = edge.tick
    }
  }`

describe('the subset', () => {
  test('accepts a well-formed snippet', () => {
    const { ok, diagnostics } = compileSnippet(snippet(RUN))
    expect(diagnostics).toEqual([])
    expect(ok).toBe(true)
  })

  test.each([
    [
      'a closure',
      snippet(`${RUN}
  helper(): u64 {
    const f = () => 1
    return f()
  }`),
      'ArrowFunction is not in the subset',
    ],
    [
      'an integer local without its width',
      snippet(`${RUN}
  helper(): u64 {
    let n = 0
    n += 1
    return n
  }`),
      'an integer local declared with a literal says its width',
    ],
    [
      'an operation that is not awaited',
      snippet(`${RUN}
  helper(): void {
    this.signal.sample()
  }`),
      'an operation is awaited',
    ],
    [
      'an async free function',
      snippet(RUN, 'export async function helper(): Promise<void> {}'),
      'a free function is synchronous',
    ],
    [
      'destructuring',
      snippet(`${RUN}
  helper(): u64 {
    const { channel } = this.config
    return channel
  }`),
      'destructuring is not in the subset',
    ],
    [
      'a narrowed option read after it was assigned',
      snippet(`${RUN}
  helper(): u64 {
    if (this.last === undefined) return 0
    this.last = 1
    return this.last
  }`),
      'was assigned after it was narrowed',
    ],
  ])('reports %s', (_what, text, message) => {
    const { ok, diagnostics } = compileSnippet(text)
    expect(ok).toBe(false)
    expect(diagnostics.map((diagnostic) => diagnostic.message).join('\n')).toContain(message)
    expect(diagnostics[0]?.file).toBe(virtualFile)
    expect(diagnostics[0]?.line).toBeGreaterThan(0)
  })

  test('reports a module with neither a decoder nor a contract', () => {
    const { ok, diagnostics } = compileSnippet(`
import type { u64 } from 'uiharu'

export type Event = { kind: 'tick'; at: u64 }
`)
    expect(ok).toBe(false)
    expect(diagnostics.map((diagnostic) => diagnostic.message).join('\n')).toContain(
      'a module declares a decoder or a contract',
    )
  })

  test('reports TypeScript errors first', () => {
    const { ok, diagnostics } = compileSnippet(
      snippet(RUN).replace('mask(this.config.channel)', 'mask(this.config.nowhere)'),
    )
    expect(ok).toBe(false)
    expect(diagnostics[0]?.message).toContain('nowhere')
  })
})
