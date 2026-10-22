// SPDX-License-Identifier: Apache-2.0
// @vitest-environment node

import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, test } from 'vitest'

import { lowerModule } from './module.ts'
import { loadProject } from './program.ts'

const dsl = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../dsl')
const virtualFile = path.join(dsl, 'contracts/wire-type-probe.ts')

const contract = `
import type { HexU64, NumericEnum, bool, u8 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type Bytes = readonly [u8, u8, u8, u8]
export type Clause = NumericEnum<{ c22: 22; c45: 45 }>
export type Ack = 'ok' | 'wait' | 'fault' | { other: u8 }
export type Event = { kind: 'reading'; id: HexU64; bytes: Bytes; clause: Clause; ack: Ack }
export const events = eventSchema<Event>('probe.events', 1).roles({ reading: 'data' })
export function isClause22(clause: Clause): bool { return clause === 22 }
`

function lower(source: string) {
  const project = loadProject(path.join(dsl, 'tsconfig.json'), { [virtualFile]: source })
  const file = project.program.getSourceFile(virtualFile)
  if (!file) throw new Error('the virtual source is loaded')
  return lowerModule(project, file)
}

describe('native contract wire types', () => {
  test('keeps fixed arrays, hexadecimal fields and nominal enum declarations', () => {
    const result = lower(contract)
    expect(result.diagnostics).toEqual([])
    expect(result.module?.types).toEqual([
      {
        name: 'Clause',
        type: 'numeric-enum',
        variants: [
          { name: 'c22', value: 22 },
          { name: 'c45', value: 45 },
        ],
      },
      {
        name: 'Ack',
        type: 'external-enum',
        variants: [
          { name: 'ok' },
          { name: 'wait' },
          { name: 'fault' },
          { name: 'other', payload: 'u8' },
        ],
      },
      {
        name: 'Event',
        type: 'union',
        variants: [
          {
            name: 'reading',
            fields: [
              { name: 'id', type: 'u64', json: 'hex-u64' },
              { name: 'bytes', type: { fixedArray: 'u8', length: 4 } },
              { name: 'clause', type: 'Clause' },
              { name: 'ack', type: 'Ack' },
            ],
          },
        ],
      },
    ])
    expect(result.module?.functions?.[0]?.body).toEqual([
      {
        stmt: 'return',
        value: {
          expr: 'binary',
          op: 'eq',
          left: { expr: 'local', name: 'clause' },
          right: { expr: 'enum', type: 'Clause', value: 'c22' },
        },
      },
    ])
  })

  test('lowers enum construction, payload access, comparison and switches', () => {
    const result = lower(`${contract}
export function bytes(): Bytes { return [1, 2, 3, 4] }
export function other(value: u8): Ack { return { other: value } }
export function ok(): Ack { return 'ok' }
export function clause(clause: Clause): Clause {
  switch (clause) { case 22: return 45; case 45: return 22 }
}
export function ackValue(ack: Ack): u8 {
  if (typeof ack === 'object' && 'other' in ack) return ack.other
  if (ack === 'ok') return 1
  switch (ack) { case 'wait': return 2; default: return 4 }
}
`)
    expect(result.diagnostics).toEqual([])
    const functions = result.module?.functions ?? []
    expect(functions.find((fn) => fn.name === 'bytes')?.ret).toEqual({
      fixedArray: 'u8',
      length: 4,
    })
    expect(functions.find((fn) => fn.name === 'other')?.body).toEqual([
      {
        stmt: 'return',
        value: {
          expr: 'variant',
          type: 'Ack',
          kind: 'other',
          fields: { value: { expr: 'local', name: 'value' } },
        },
      },
    ])
    expect(functions.find((fn) => fn.name === 'ok')?.body).toEqual([
      { stmt: 'return', value: { expr: 'variant', type: 'Ack', kind: 'ok' } },
    ])
    const ack = JSON.stringify(functions.find((fn) => fn.name === 'ackValue'))
    expect(ack).toContain('"pattern":"variant","name":"other"')
    expect(ack).toContain('"binds":{"value":')
    expect(ack).toContain('"pattern":"variant","name":"wait"')
    const numeric = JSON.stringify(functions.find((fn) => fn.name === 'clause'))
    expect(numeric).toContain('"values":["c22"]')
    expect(numeric).toContain('"value":"c45"')
  })

  test('supports bare numeric unions with stable generated names', () => {
    const result = lower(`${contract}
export type Numbered = 22 | 45
export function numbered(): Numbered { return 22 }
`)
    expect(result.diagnostics).toEqual([])
    expect(result.module?.types?.find((def) => def.name === 'Numbered')).toEqual({
      name: 'Numbered',
      type: 'numeric-enum',
      variants: [
        { name: 'value22', value: 22 },
        { name: 'value45', value: 45 },
      ],
    })
  })

  test('narrows multiple payload variants only after checking their property', () => {
    const result = lower(`${contract}
export type Reply = 'done' | { other: u8 } | { retry: u8 }
export function reply(reply: Reply): u8 {
  if (typeof reply !== 'string' && 'other' in reply) return reply.other
  if (typeof reply === 'object' && 'retry' in reply) return reply.retry
  return 0
}
`)
    expect(result.diagnostics).toEqual([])
    const source = JSON.stringify(result.module?.functions?.find((fn) => fn.name === 'reply'))
    expect(source).toContain('"op":"or"')
    expect(source).toContain('"pattern":"variant","name":"other"')
    expect(source).toContain('"pattern":"variant","name":"retry"')
  })

  test('formats scalar enums after optional values are explicitly coalesced', () => {
    const result = lower(`${contract}
export function clauseText(clause: Clause | undefined): string { return \`clause \${clause ?? 22}\` }
export function optionalText(text: string | undefined): string { return \`text \${text ?? 'empty'}\` }
`)
    expect(result.diagnostics).toEqual([])
    const source = JSON.stringify(result.module?.functions?.find((fn) => fn.name === 'clauseText'))
    expect(source).toContain('"expr":"enum","type":"Clause","value":"c22"')
  })

  test.each([
    ['optional hexadecimal field', `export interface Bad { id: HexU64 | undefined }`, 'HexU64'],
    ['hexadecimal function result', `export function bad(): HexU64 { return 1 }`, 'HexU64'],
    ['duplicate numeric values', `export type Bad = NumericEnum<{ a: 22; b: 22 }>`, 'unique'],
    [
      'enum arithmetic',
      `export function bad(clause: Clause): u8 { return clause + 1 }`,
      'arithmetic',
    ],
    ['enum cast', `export function bad(clause: Clause): u8 { return clause as u8 }`, 'casts'],
    ['enum increment', `export function bad(clause: Clause): void { clause++ }`, 'arithmetic'],
    ['heterogeneous tuple', `export interface Bad { bytes: readonly [u8, bool] }`, 'no IR type'],
    ['optional tuple item', `export interface Bad { bytes: readonly [u8, u8?] }`, 'no IR type'],
    ['rest tuple item', `export interface Bad { bytes: readonly [u8, ...u8[]] }`, 'no IR type'],
    ['empty tuple', `export interface Bad { bytes: readonly [] }`, 'no IR type'],
    [
      'oversized tuple',
      `export interface Bad { bytes: readonly [${Array(33).fill('u8').join(', ')}] }`,
      'native serde array limit',
    ],
    [
      'unsafe function literal',
      `export function bad(): u8 { return 9007199254740993 }`,
      'safe integer',
    ],
    ['unsafe top-level literal', `export const bad: u8 = 9007199254740993`, 'safe integer'],
    [
      'optional enum template',
      'export function bad(clause: Clause | undefined): string { return `${clause}` }',
      'coalesce optional',
    ],
    [
      'optional text template',
      'export function bad(text: string | undefined): string { return `${text}` }',
      'coalesce optional',
    ],
    [
      'record template',
      'export function bad(value: { value: u8 }): string { return `${value}` }',
      'coalesce optional',
    ],
  ])('rejects %s at the source', (_name, declaration, message) => {
    expect(
      lower(`${contract}\n${declaration}`)
        .diagnostics.map((diagnostic) => diagnostic.message)
        .join('\n'),
    ).toContain(message)
  })
})
