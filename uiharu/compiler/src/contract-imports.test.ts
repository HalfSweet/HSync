// SPDX-License-Identifier: Apache-2.0
// @vitest-environment node

import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, test } from 'vitest'

import { lowerModule } from './module.ts'
import { loadProject } from './program.ts'

const dsl = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../dsl')
const consumerFile = path.join(dsl, 'decoders/version-probe.ts')

function owner(event: string, version: number): string {
  return `
import type { u8 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type ${event} = { kind: 'data'; value: u8 }
export const events = eventSchema<${event}>('versioned.events', ${version}).roles({ data: 'data' })
`
}

const consumer = `
import { StreamDecoder, defineStreamDecoder } from 'uiharu'
import type { EventV1 } from '../contracts/version-one'
import type { EventV2 } from '../contracts/version-two'
import { events as input } from '../contracts/version-one'
import { events as output } from '../contracts/version-two'
interface Config {}
class VersionProbe extends StreamDecoder<Config, EventV1, EventV2> {
  async run(): Promise<void> { await this.input.next() }
}
export const probe = defineStreamDecoder({
  name: 'version-probe', category: 'device', config: { defaults: {} },
  input, output, decoder: VersionProbe,
})
`

function lower(input: string, output: string) {
  const project = loadProject(path.join(dsl, 'tsconfig.json'), {
    [consumerFile]: consumer,
    [path.join(dsl, 'contracts/version-one.ts')]: input,
    [path.join(dsl, 'contracts/version-two.ts')]: output,
  })
  const file = project.program.getSourceFile(consumerFile)
  if (!file) throw new Error('the virtual consumer is loaded')
  return lowerModule(project, file)
}

describe('imported schema identity', () => {
  test('retains different versions and their canonical event types', () => {
    const result = lower(owner('EventV1', 1), owner('EventV2', 2))
    expect(result.diagnostics).toEqual([])
    expect(
      result.module?.imports?.map((entry) => ({
        owner: entry.module,
        schema: entry.contract.schema,
        event: entry.contract.event,
        types: entry.types?.map((def) => def.name),
      })),
    ).toEqual([
      {
        owner: 'version-two',
        schema: { name: 'versioned.events', version: 2 },
        event: 'EventV2',
        types: ['EventV2'],
      },
      {
        owner: 'version-one',
        schema: { name: 'versioned.events', version: 1 },
        event: 'EventV1',
        types: ['EventV1'],
      },
    ])
  })

  test('rejects different canonical declarations sharing one schema identity', () => {
    const result = lower(owner('EventV1', 1), owner('EventV2', 1))
    expect(result.module).toBeUndefined()
    expect(result.diagnostics.map((diagnostic) => diagnostic.message)).toEqual([
      'versioned.events@1 has conflicting contract declarations or owners',
    ])
  })
})
