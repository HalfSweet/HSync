/**
 * Channel groups of the timeline (DESIGN.md §6.1): one group per decoder with the channels it
 * reads, then the channels no decoder uses under "unnamed channels".
 */
import type { ChannelSummary } from '@/ipc/bindings/ChannelSummary'

export type BusKind = 'i2c' | 'uart' | 'spi' | 'can' | 'other'

export interface ChannelRef {
  index: number
  /** Text on the row: the decoder role (SDA, RX…) or the channel's own name. */
  label: string
  /** The label is just the channel number, shown as `D<n>` in mono. */
  numeric: boolean
  /** Probe wire colour, 0..7 (black, brown, red, orange, yellow, green, blue, violet). */
  wire: number
  /** Role in the decoder, e.g. `sda`. */
  role?: string
  /** The role as shown, e.g. `SDA`, when the label is the channel's own name instead. */
  roleLabel?: string
}

export interface ChannelGroup {
  /** Stable key: the decoder id, or `unnamed`. */
  key: string
  kind: BusKind
  /** Group title, e.g. `I²C`; empty for the unnamed group, whose title is translated. */
  title: string
  unnamed: boolean
  channels: ChannelRef[]
  /** Decoder options as stored with the capture. */
  options: Record<string, unknown>
  /** Position of the decoder in the capture's decoder list, which names its results. */
  decoder: number | null
}

export const UNNAMED_GROUP = 'unnamed'

/** Order of decoder roles on the timeline, by bus. */
const ROLE_ORDER: Record<BusKind, string[]> = {
  i2c: ['sda', 'scl'],
  uart: ['rxtx', 'rx', 'tx'],
  spi: ['miso', 'mosi', 'clk', 'cs'],
  can: ['can_rx', 'rx'],
  other: [],
}

const ROLE_LABELS: Record<string, string> = {
  sda: 'SDA',
  scl: 'SCL',
  rxtx: 'RX',
  rx: 'RX',
  tx: 'TX',
  miso: 'MISO',
  mosi: 'MOSI',
  clk: 'CLK',
  cs: 'CS#',
  can_rx: 'CAN',
}

/** Decoder id without DSView's version prefix (`1:i2c` → `i2c`). */
export function baseDecoderId(id: string): string {
  return id.replace(/^\d+:/, '')
}

export function busKind(decoderId: string): BusKind {
  const id = baseDecoderId(decoderId)
  if (id === 'i2c') return 'i2c'
  if (id === 'uart') return 'uart'
  if (id === 'spi') return 'spi'
  if (id === 'can' || id === 'can-fd') return 'can'
  return 'other'
}

export function wireOf(index: number): number {
  return index % 8
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Roles of a DSView decoder entry: `"channel": [{"sda": 0}, {"scl": 1}]`. */
function decoderRoles(entry: Record<string, unknown>): [string, number][] {
  const mapping = Array.isArray(entry.channel) ? entry.channel : []
  return mapping.flatMap((item) =>
    isRecord(item)
      ? Object.entries(item).flatMap(([role, index]) =>
          typeof index === 'number' ? [[role, index] as [string, number]] : [],
        )
      : [],
  )
}

/**
 * Groups `channels` by the decoders that read them. `wires` gives a channel's colour when the
 * user chose one; otherwise its number picks one.
 */
export function groupChannels(
  channels: ChannelSummary[],
  decoders: unknown,
  wires: Partial<Record<number, number>> = {},
): ChannelGroup[] {
  const wire = (index: number) => wires[index] ?? wireOf(index)
  const present = new Map(channels.map((channel) => [channel.index, channel]))
  const used = new Set<number>()
  const groups: ChannelGroup[] = []

  for (const [position, entry] of (Array.isArray(decoders) ? decoders : []).entries()) {
    if (!isRecord(entry) || typeof entry.id !== 'string') continue
    const kind = busKind(entry.id)
    const order = ROLE_ORDER[kind]
    const rank = (role: string) => {
      const at = order.indexOf(role)
      return at < 0 ? order.length : at
    }
    const roles = decoderRoles(entry)
      .filter(([, index]) => present.has(index))
      .sort(([a], [b]) => rank(a) - rank(b))
    const label = typeof entry.label === 'string' ? entry.label : entry.id
    groups.push({
      key: entry.id,
      kind,
      title: baseDecoderId(label),
      unnamed: false,
      channels: roles.map(([role, index]) => {
        used.add(index)
        // A channel named other than its number shows its name, with the role beside it.
        const roleLabel = ROLE_LABELS[role] ?? role.toUpperCase()
        const name = present.get(index)?.name ?? String(index)
        const named = name !== String(index) && name.toUpperCase() !== roleLabel.toUpperCase()
        return {
          index,
          label: named ? name : roleLabel,
          numeric: false,
          wire: wire(index),
          role,
          ...(named ? { roleLabel } : {}),
        }
      }),
      options: isRecord(entry.options) ? entry.options : {},
      decoder: position,
    })
  }

  const rest = channels.filter((channel) => !used.has(channel.index))
  if (rest.length > 0) {
    groups.push({
      key: UNNAMED_GROUP,
      kind: 'other',
      title: '',
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

/** The UART line format the design shows next to the group title, e.g. `115200 8N1`. */
export function uartFormat(options: Record<string, unknown>): string | null {
  const baud = options.baudrate
  if (typeof baud !== 'number') return null
  const bits = typeof options.num_data_bits === 'number' ? options.num_data_bits : 8
  const parity = typeof options.parity_type === 'string' ? options.parity_type : 'none'
  const stop = typeof options.num_stop_bits === 'number' ? options.num_stop_bits : 1
  const letter = { none: 'N', even: 'E', odd: 'O', zero: '0', one: '1' }[parity] ?? 'N'
  return `${baud} ${bits}${letter}${stop}`
}
