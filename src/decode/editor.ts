/** Lossless editing of stored graphs; structural controls are limited to verified chains. */
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import {
  compatiblePorts,
  LOGIC_SCHEMA,
  typeOf,
  type DecoderEntry,
  type EntryLayer,
} from './catalog'
import type { NumberFormat } from './numberFormat'

function endpoint(value: string): { id: string; port: string } | null {
  const split = value.lastIndexOf('.')
  return split < 1 ? null : { id: value.slice(0, split), port: value.slice(split + 1) }
}

/** Returns chain order only when every stored connection is accounted for and compatible. */
export function linearLayers(
  entry: DecoderEntry,
  catalog: readonly CatalogEntry[],
): EntryLayer[] | null {
  const connections = entry.connections ?? []
  const source = connections.filter((edge) => edge.from === 'source')
  if (source.length !== 1 || connections.length !== Object.keys(entry.nodes).length) return null
  const first = endpoint(source[0]!.to)
  if (!first) return null
  const layers: EntryLayer[] = []
  const seen = new Set<string>()
  let current: { id: string; port: string } | null = first
  let fromPort: string | undefined
  while (current) {
    const id: string = current.id
    const port: string = current.port
    if (seen.has(id)) return null
    const node = entry.nodes[id]
    const type = node && typeOf(node.type, catalog)
    if (!node || !type) return null
    if (
      layers.length === 0 &&
      !type.inputs.some((input) => input.port === port && input.schema === LOGIC_SCHEMA)
    )
      return null
    if (type.inputs.length !== 1) return null
    const previous = layers.at(-1)
    if (
      previous &&
      !compatiblePorts(previous.type, type).some(
        (pair) => pair.from === fromPort && pair.to === port,
      )
    )
      return null
    layers.push({ id, type, config: { ...node.config }, fromPort, toPort: port })
    seen.add(id)
    const outgoing: { from: string; to: string }[] = connections.filter(
      (edge) => endpoint(edge.from)?.id === id,
    )
    if (outgoing.length > 1) return null
    const next: { from: string; to: string } | undefined = outgoing[0]
    fromPort = next ? endpoint(next.from)?.port : undefined
    current = next ? endpoint(next.to) : null
    if (next && !current) return null
  }
  return seen.size === Object.keys(entry.nodes).length ? layers : null
}

/** Patch node fields and display without replacing topology or unknown metadata. */
export function patchEntry(
  original: DecoderEntry,
  patches: Record<string, Record<string, unknown>>,
  label: string,
  numbers: NumberFormat,
): DecoderEntry {
  const nodes = { ...original.nodes }
  for (const [id, patch] of Object.entries(patches)) {
    const node = nodes[id]
    if (!node || Object.keys(patch).length === 0) continue
    const config = { ...node.config, ...patch }
    for (const [key, value] of Object.entries(config)) if (value === undefined) delete config[key]
    nodes[id] = { ...node, config }
  }
  const entry: DecoderEntry = { ...original, nodes }
  if (label !== (original.label ?? '')) {
    if (label.trim()) entry.label = label.trim()
    else delete entry.label
  }
  if (numbers === (original.show?.numbers ?? 'hex')) return entry
  const show = { ...original.show }
  if (numbers === 'hex') delete show.numbers
  else show.numbers = numbers
  if (Object.keys(show).length) entry.show = show
  else delete entry.show
  return entry
}

/** Reconcile an explicitly edited linear chain while retaining metadata on surviving items. */
export function replaceLinearLayers(
  original: DecoderEntry,
  layers: readonly EntryLayer[],
): DecoderEntry {
  const ids = new Set(layers.map((layer) => layer.id!))
  const nodes = Object.fromEntries(
    layers.map((layer) => [
      layer.id!,
      original.nodes[layer.id!] ?? { type: layer.type.name, config: layer.config },
    ]),
  )
  const connections = layers.map((layer, index) => {
    const from = index === 0 ? 'source' : `${layers[index - 1]!.id}.${layer.fromPort}`
    const to = `${layer.id}.${layer.toPort}`
    return (
      original.connections?.find((edge) => edge.from === from && edge.to === to) ?? { from, to }
    )
  })
  const expose = Object.fromEntries(
    Object.entries(original.expose ?? {}).filter(([, value]) => {
      const target = endpoint(value)
      return target === null || ids.has(target.id)
    }),
  )
  for (const layer of layers) {
    if (original.nodes[layer.id!]) continue
    const output = layer.type.outputs[0]
    if (output) expose[layer.id!] = `${layer.id}.${output.port}`
  }
  return { ...original, nodes, connections, expose }
}
