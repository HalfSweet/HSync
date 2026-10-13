/**
 * Channel groups of the timeline (DESIGN.md §6.1): one group per decoder with the channels it
 * reads, then the channels no decoder uses under "unnamed channels".
 */
import { DEFAULT_CATALOG, entryChannels, entryKinds, isEntry, nodeConfig } from '@/decode/catalog'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { ChannelSummary } from '@/ipc/bindings/ChannelSummary'

/** The buses with colours of their own (DESIGN.md §3.2); every other decoder is `other`. */
export type BusKind = 'i2c' | 'uart' | 'spi' | 'can' | 'other'

export interface ChannelRef {
  index: number
  /** Text on the row: the decoder role (SDA, RX…) or the channel's own name. */
  label: string
  /** The label is just the channel number, shown as `D<n>` in mono. */
  numeric: boolean
  /** Probe wire colour, 0..7 (black, brown, red, orange, yellow, green, blue, violet). */
  wire: number
  /** Role in the decoder: the configuration field, e.g. `sda`, or `data0` for a line of a list. */
  role?: string
  /** The role as shown, e.g. `SDA`, when the label is the channel's own name instead. */
  roleLabel?: string
}

export interface ChannelGroup {
  /** Stable key: `decoder:<position>`, or `unnamed`. */
  key: string
  kind: BusKind
  /** The user's label for the decoder; empty when it has none (its type names it) and for the
   * unnamed group. */
  title: string
  /** The node type of the decoder's first node, e.g. `i2c`; `null` for the unnamed group. */
  type: string | null
  unnamed: boolean
  channels: ChannelRef[]
  /** The first node's configuration, defaults filled in. */
  options: Record<string, unknown>
  /** Position of the decoder in the capture's decoder list, which names its results. */
  decoder: number | null
}

export const UNNAMED_GROUP = 'unnamed'

/** Order of decoder roles on the timeline, by bus. */
const ROLE_ORDER: Record<BusKind, string[]> = {
  i2c: ['sda', 'scl'],
  uart: ['line'],
  spi: ['miso', 'mosi', 'clk', 'cs'],
  can: ['rx'],
  other: [],
}

const ROLE_LABELS: Record<string, string> = {
  sda: 'SDA',
  scl: 'SCL',
  line: 'RX',
  rx: 'RX',
  tx: 'TX',
  miso: 'MISO',
  mosi: 'MOSI',
  clk: 'CLK',
  cs: 'CS#',
}

/** The bus a node type is, for its colour. */
export function busKind(type: string | undefined): BusKind {
  switch (type) {
    case 'i2c':
    case 'uart':
    case 'spi':
    case 'can':
      return type
    default:
      return 'other'
  }
}

export function wireOf(index: number): number {
  return index % 8
}

/** The role as the timeline labels it: `SDA`, `CS#`, or the field name in capitals. */
export function roleLabel(role: string): string {
  return ROLE_LABELS[role] ?? role.toUpperCase()
}

/**
 * Groups `channels` by the decoders that read them. `wires` gives a channel's colour when the
 * user chose one; otherwise its number picks one. `catalog` tells which configuration fields
 * name channels.
 */
export function groupChannels(
  channels: ChannelSummary[],
  decoders: unknown,
  wires: Partial<Record<number, number>> = {},
  catalog: readonly CatalogEntry[] = DEFAULT_CATALOG,
): ChannelGroup[] {
  const wire = (index: number) => wires[index] ?? wireOf(index)
  const present = new Map(channels.map((channel) => [channel.index, channel]))
  const used = new Set<number>()
  const groups: ChannelGroup[] = []

  for (const [position, entry] of (Array.isArray(decoders) ? decoders : []).entries()) {
    if (!isEntry(entry)) continue
    const kinds = entryKinds(entry)
    const type = kinds[0] ?? null
    const kind = busKind(type ?? undefined)
    const order = ROLE_ORDER[kind]
    const rank = (role: string) => {
      const at = order.indexOf(role)
      return at < 0 ? order.length : at
    }
    const seen = new Set<number>()
    const roles = entryChannels(entry, catalog)
      .filter((use) => present.has(use.index) && !seen.has(use.index) && seen.add(use.index))
      .sort((a, b) => rank(a.role) - rank(b.role))
    const first = Object.values(entry.nodes)[0]
    groups.push({
      key: `decoder:${position}`,
      kind,
      title: entry.label?.trim() ?? '',
      type,
      unnamed: false,
      channels: roles.map(({ role, index }) => {
        used.add(index)
        // A channel named other than its number shows its name, with the role beside it.
        const shown = roleLabel(role)
        const name = present.get(index)?.name ?? String(index)
        const named = name !== String(index) && name.toUpperCase() !== shown.toUpperCase()
        return {
          index,
          label: named ? name : shown,
          numeric: false,
          wire: wire(index),
          role,
          ...(named ? { roleLabel: shown } : {}),
        }
      }),
      options: first ? nodeConfig(first, catalog) : {},
      decoder: position,
    })
  }

  const rest = channels.filter((channel) => !used.has(channel.index))
  if (rest.length > 0) {
    groups.push({
      key: UNNAMED_GROUP,
      kind: 'other',
      title: '',
      type: null,
      unnamed: true,
      channels: rest.map((channel) => {
        const numeric = channel.name === String(channel.index)
        return {
          index: channel.index,
          label: numeric ? `D${channel.index}` : channel.name,
          numeric,
          wire: wire(channel.index),
        }
      }),
      options: {},
      decoder: null,
    })
  }
  return groups
}

const PARITY_LETTERS: Record<string, string> = {
  none: 'N',
  even: 'E',
  odd: 'O',
  mark: 'M',
  space: 'S',
}
const STOP_BITS: Record<string, string> = { one: '1', 'one-and-a-half': '1.5', two: '2' }

/** The UART line format the design shows next to the group title, e.g. `115200 8N1`. */
export function uartFormat(config: Record<string, unknown>): string | null {
  const baud = config.baud
  if (typeof baud !== 'number') return null
  const bits = typeof config.data_bits === 'number' ? config.data_bits : 8
  const parity = typeof config.parity === 'string' ? (PARITY_LETTERS[config.parity] ?? 'N') : 'N'
  const stop = typeof config.stop_bits === 'string' ? (STOP_BITS[config.stop_bits] ?? '1') : '1'
  return `${baud} ${bits}${parity}${stop}`
}
