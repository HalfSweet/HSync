/**
 * Decoders as a capture stores them: a uiharu pipeline without its source — nodes, their
 * connections and the outputs shown as lanes — with a label and display settings; and what the
 * catalog of node types says about them.
 */
import { shallowRef } from 'vue'

import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { CatalogField } from '@/ipc/bindings/CatalogField'
import catalogFixture from '@/ipc/mock/fixtures/decoder-catalog.json'

import type { NumberFormat } from './numberFormat'

/** The schema of the samples a pipeline's source carries. */
export const LOGIC_SCHEMA = 'logic.samples'

export interface NodeEntry {
  type: string
  config?: Record<string, unknown>
}

/** How a decoder's results show; decoding does not read it. */
export interface DisplaySettings {
  numbers?: NumberFormat
}

/** A decoder entry, as the backend reads it (`hsync_session::DecoderEntry`). */
export interface DecoderEntry {
  /** The nodes by name, the bus first. */
  nodes: Record<string, NodeEntry>
  /** Connections between `source` or `node.port` and `node.port`. */
  connections?: { from: string; to: string }[]
  /** The outputs shown as lanes, by name, in lane order. */
  expose?: Record<string, string>
  label?: string
  show?: DisplaySettings
}

/** The catalog built into the app; the backend's replaces it once read. */
export const DEFAULT_CATALOG: readonly CatalogEntry[] = catalogFixture as CatalogEntry[]

/**
 * The catalog in use: the built-in one until the decode store reads the backend's. Kept here,
 * outside the stores, so that the capture store can group channels by it without depending
 * on the decode store, which depends on the capture store.
 */
export const activeCatalog = shallowRef<readonly CatalogEntry[]>(DEFAULT_CATALOG)

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Whether `value` is a decoder entry (DSView's, which the backend translates, are not). */
export function isEntry(value: unknown): value is DecoderEntry {
  return isRecord(value) && isRecord(value.nodes)
}

/** The node types of an entry, in its order. */
export function entryKinds(entry: DecoderEntry): string[] {
  return Object.values(entry.nodes).map((node) => node.type)
}

/** What an entry is called: its label, else the name of its first node type. */
export function entryLabel(entry: DecoderEntry, nameOf: (type: string) => string): string {
  const label = entry.label?.trim()
  if (label) return label
  const first = entryKinds(entry)[0]
  return first === undefined ? '' : nameOf(first)
}

export function typeOf(
  name: string,
  catalog: readonly CatalogEntry[] = DEFAULT_CATALOG,
): CatalogEntry | undefined {
  return catalog.find((type) => type.name === name)
}

/** Node types reading samples: those a decoder starts with. */
export function baseTypes(catalog: readonly CatalogEntry[]): CatalogEntry[] {
  return catalog.filter((type) => type.inputs[0]?.schema === LOGIC_SCHEMA)
}

/** Node types that read what `base` puts out, to stack on it. */
export function stackableOn(base: CatalogEntry, catalog: readonly CatalogEntry[]): CatalogEntry[] {
  const schema = base.outputs[0]?.schema
  return schema === undefined
    ? []
    : catalog.filter((type) => type.inputs[0]?.schema === schema && type.name !== base.name)
}

/** The configuration fields of `type` naming channels. */
export function channelFields(type: CatalogEntry): CatalogField[] {
  return type.config.filter(
    (field) => field.kind.kind === 'channel' || field.kind.kind === 'channels',
  )
}

/** A node's configuration with the type's defaults filled in. */
export function nodeConfig(
  node: NodeEntry,
  catalog: readonly CatalogEntry[] = DEFAULT_CATALOG,
): Record<string, unknown> {
  const type = typeOf(node.type, catalog)
  const config: Record<string, unknown> = {}
  for (const field of type?.config ?? []) config[field.name] = field.default
  for (const [name, value] of Object.entries(node.config ?? {})) config[name] = value
  return config
}

/** A channel a decoder reads: through which node and field, under which role. */
export interface ChannelUse {
  node: string
  field: string
  /** The field name, or `field0`, `field1`… for the lines of a channel list. */
  role: string
  index: number
}

/** Every channel an entry's nodes read, in the entry's order. */
export function entryChannels(
  entry: DecoderEntry,
  catalog: readonly CatalogEntry[] = DEFAULT_CATALOG,
): ChannelUse[] {
  const uses: ChannelUse[] = []
  for (const [node, spec] of Object.entries(entry.nodes)) {
    const type = typeOf(spec.type, catalog)
    if (!type) continue
    const config = nodeConfig(spec, catalog)
    for (const field of channelFields(type)) {
      const value = config[field.name]
      if (field.kind.kind === 'channel') {
        if (typeof value === 'number')
          uses.push({ node, field: field.name, role: field.name, index: value })
      } else if (Array.isArray(value)) {
        value.forEach((index, at) => {
          if (typeof index === 'number')
            uses.push({ node, field: field.name, role: `${field.name}${at}`, index })
        })
      }
    }
  }
  return uses
}

/** A value of a field as the dialog holds it: `undefined` leaves the field out. */
export type FieldValue = unknown

function cleanConfig(config: Record<string, FieldValue>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(config).filter(([, value]) => value !== undefined))
}

/**
 * Builds the entry of `base` configured with `config`, and of the device decoders `stacked`
 * on its output, each with their own; the nodes are named after their types.
 */
export function buildEntry(
  base: CatalogEntry,
  config: Record<string, FieldValue>,
  stacked: readonly { type: CatalogEntry; config: Record<string, FieldValue> }[] = [],
  label?: string,
  show?: DisplaySettings,
): DecoderEntry {
  const nodes: Record<string, NodeEntry> = {
    [base.name]: { type: base.name, config: cleanConfig(config) },
  }
  const connections = [{ from: 'source', to: `${base.name}.${base.inputs[0]?.port ?? 'logic'}` }]
  const expose: Record<string, string> = {
    [base.name]: `${base.name}.${base.outputs[0]?.port ?? 'events'}`,
  }
  let below = base
  for (const layer of stacked) {
    nodes[layer.type.name] = { type: layer.type.name, config: cleanConfig(layer.config) }
    connections.push({
      from: `${below.name}.${below.outputs[0]?.port ?? 'events'}`,
      to: `${layer.type.name}.${layer.type.inputs[0]?.port ?? 'bus'}`,
    })
    expose[layer.type.name] = `${layer.type.name}.${layer.type.outputs[0]?.port ?? 'events'}`
    below = layer.type
  }
  const entry: DecoderEntry = { nodes, connections, expose }
  const trimmed = label?.trim()
  if (trimmed) entry.label = trimmed
  if (show && Object.keys(show).length > 0) entry.show = show
  return entry
}
