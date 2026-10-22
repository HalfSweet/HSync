import { describe, expect, it } from 'vitest'
import {
  buildEntry,
  compatiblePorts,
  DEFAULT_CATALOG,
  typeOf,
  uniqueNodeId,
  type DecoderEntry,
} from './catalog'
import { linearLayers, patchEntry, replaceLinearLayers } from './editor'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'

const kind = (name: string) => typeOf(name)!
const custom = (name: string, input: string, output: string): CatalogEntry => ({
  name,
  category: 'device',
  inputs: [
    { port: 'unused', schema: 'other' },
    { port: 'actual-in', schema: input },
  ],
  outputs: [
    { port: 'unused', schema: 'other-out' },
    { port: 'actual-out', schema: output },
  ],
  config: [],
  events: [],
  module: null,
})

describe('lossless decoder editing', () => {
  it('adds and discovers all three 1-Wire layers', () => {
    const entry = buildEntry(kind('onewire-link'), { data: 2 }, [
      { type: kind('onewire'), config: {} },
      { type: kind('ds18b20'), config: {} },
    ])
    expect(entry.connections).toEqual([
      { from: 'source', to: 'onewire-link.logic' },
      { from: 'onewire-link.events', to: 'onewire.link' },
      { from: 'onewire.events', to: 'ds18b20.bus' },
    ])
    const layers = linearLayers(entry, DEFAULT_CATALOG)!
    expect(layers.map((layer) => layer.id)).toEqual(['onewire-link', 'onewire', 'ds18b20'])
    expect(
      patchEntry(entry, { 'onewire-link': { data: 3 } }, '', 'hex').nodes['onewire-link']?.config,
    ).toEqual({ data: 3 })
    expect(patchEntry(entry, {}, '', 'hex')).toEqual(entry)
  })
  it('uses explicit compatible ports beyond the first and allows repeated types', () => {
    const base = custom('root', 'logic.samples', 'chain')
    const repeat = custom('repeat', 'chain', 'chain')
    expect(compatiblePorts(base, repeat)).toEqual([
      { from: 'actual-out', to: 'actual-in', schema: 'chain' },
    ])
    const entry = buildEntry(base, {}, [
      { type: repeat, config: {} },
      { type: repeat, config: {}, fromPort: 'actual-out', toPort: 'actual-in' },
    ])
    expect(Object.keys(entry.nodes)).toEqual(['root', 'repeat', 'repeat-2'])
    expect(entry.connections).toEqual([
      { from: 'source', to: 'root.actual-in' },
      { from: 'root.actual-out', to: 'repeat.actual-in' },
      { from: 'repeat.actual-out', to: 'repeat-2.actual-in' },
    ])
    expect(uniqueNodeId('repeat', ['repeat', 'repeat-2', 'repeat-4'])).toBe('repeat-3')
  })
  it('preserves arbitrary graph metadata, branches, unknown types and untouched configuration', () => {
    const entry: DecoderEntry = {
      nodes: {
        custom: { type: 'uart', config: { line: 2, future: { value: 9 } }, futureNode: 42 },
        unknown: { type: 'future', config: { raw: [1, 2] } },
        branch: { type: 'modbus-rtu', config: {} },
      },
      connections: [
        { from: 'source', to: 'custom.logic' },
        { from: 'custom.events', to: 'unknown.bus' },
        { from: 'custom.events', to: 'branch.bus' },
      ],
      expose: { saved: 'unknown.raw', other: 'custom.events' },
      label: ' original ',
      show: { numbers: 'hex', futureDisplay: true },
      import: { format: 'dsview', raw: { decoder: 'unknown' }, runnable: true, diagnostics: [] },
      future: { keep: ['all'] },
    }
    expect(linearLayers(entry, DEFAULT_CATALOG)).toBeNull()
    expect(patchEntry(entry, {}, ' original ', 'hex')).toEqual(entry)
    const saved = patchEntry(entry, { custom: { line: 3 } }, 'Changed', 'dec')
    expect(saved).toEqual({
      ...entry,
      label: 'Changed',
      show: { numbers: 'dec', futureDisplay: true },
      nodes: {
        ...entry.nodes,
        custom: { ...entry.nodes.custom, config: { line: 3, future: { value: 9 } } },
      },
    })
    expect(entry.nodes.custom?.config?.line).toBe(2)
  })
  it('keeps original instance IDs and custom exposure names when extending a chain', () => {
    const original: DecoderEntry = {
      nodes: {
        wire7: { type: 'onewire-link', config: { data: 2 }, custom: true },
        packets9: { type: 'onewire', config: {} },
      },
      connections: [
        { from: 'source', to: 'wire7.logic' },
        { from: 'wire7.events', to: 'packets9.link' },
      ],
      expose: { named: 'packets9.events' },
      future: true,
    }
    const layers = linearLayers(original, DEFAULT_CATALOG)!
    layers.push({
      id: 'temperature5',
      type: kind('ds18b20'),
      config: {},
      fromPort: 'events',
      toPort: 'bus',
    })
    const extended = replaceLinearLayers(original, layers)
    expect(Object.keys(extended.nodes)).toEqual(['wire7', 'packets9', 'temperature5'])
    expect(extended.nodes.wire7).toEqual(original.nodes.wire7)
    expect(extended.expose).toEqual({
      named: 'packets9.events',
      temperature5: 'temperature5.events',
    })
    expect(extended.future).toBe(true)
    expect(original.nodes.temperature5).toBeUndefined()
  })
})
