/**
 * The decoders HSync decodes natively, with DSView's ids, channel roles and options, so the
 * entries they produce are the ones DSView stores in `.dsl` files (DESIGN.md §7.2).
 */
import type { BusKind } from '@/timeline/groups'

export interface RoleSpec {
  id: string
  /** Shown on the timeline, e.g. `SCL`. */
  name: string
  required: boolean
}

export type OptionSpec =
  | { id: string; kind: 'choice'; values: readonly (string | number)[]; default: string | number }
  | { id: string; kind: 'number'; default: number; min: number; max: number; step: number }

export interface DecoderSpec {
  /** DSView's decoder id. */
  id: string
  /** DSView's label, which the timeline shows without the version prefix. */
  label: string
  bus: BusKind
  roles: readonly RoleSpec[]
  options: readonly OptionSpec[]
}

export const DECODERS: readonly DecoderSpec[] = [
  {
    id: '1:i2c',
    label: '1:I²C',
    bus: 'i2c',
    roles: [
      { id: 'scl', name: 'SCL', required: true },
      { id: 'sda', name: 'SDA', required: true },
    ],
    options: [
      {
        id: 'address_format',
        kind: 'choice',
        values: ['shifted', 'unshifted'],
        default: 'shifted',
      },
    ],
  },
  {
    id: '1:uart',
    label: '1:UART',
    bus: 'uart',
    roles: [{ id: 'rxtx', name: 'RX/TX', required: true }],
    options: [
      { id: 'baudrate', kind: 'number', default: 115200, min: 1, max: 100_000_000, step: 1 },
      { id: 'num_data_bits', kind: 'choice', values: [5, 6, 7, 8, 9], default: 8 },
      {
        id: 'parity_type',
        kind: 'choice',
        values: ['none', 'odd', 'even', 'zero', 'one'],
        default: 'none',
      },
      { id: 'num_stop_bits', kind: 'choice', values: [0.5, 1, 1.5, 2], default: 1 },
      { id: 'bit_order', kind: 'choice', values: ['lsb-first', 'msb-first'], default: 'lsb-first' },
      { id: 'invert', kind: 'choice', values: ['no', 'yes'], default: 'no' },
    ],
  },
  {
    id: '1:spi',
    label: '1:SPI',
    bus: 'spi',
    roles: [
      { id: 'clk', name: 'CLK', required: true },
      { id: 'miso', name: 'MISO', required: false },
      { id: 'mosi', name: 'MOSI', required: false },
      { id: 'cs', name: 'CS#', required: false },
    ],
    options: [
      { id: 'cpol', kind: 'choice', values: [0, 1], default: 0 },
      { id: 'cpha', kind: 'choice', values: [0, 1], default: 0 },
      { id: 'bitorder', kind: 'choice', values: ['msb-first', 'lsb-first'], default: 'msb-first' },
      { id: 'wordsize', kind: 'number', default: 8, min: 1, max: 32, step: 1 },
      {
        id: 'cs_polarity',
        kind: 'choice',
        values: ['active-low', 'active-high'],
        default: 'active-low',
      },
    ],
  },
  {
    id: 'can',
    label: 'CAN',
    bus: 'can',
    roles: [{ id: 'can_rx', name: 'CAN', required: true }],
    options: [
      { id: 'bitrate', kind: 'number', default: 1_000_000, min: 1000, max: 10_000_000, step: 1000 },
      { id: 'sample_point', kind: 'number', default: 70, min: 10, max: 95, step: 0.5 },
    ],
  },
  {
    id: 'can-fd',
    label: 'CAN-FD',
    bus: 'can',
    roles: [{ id: 'can_rx', name: 'CAN', required: true }],
    options: [
      {
        id: 'nominal_bitrate',
        kind: 'number',
        default: 1_000_000,
        min: 1000,
        max: 10_000_000,
        step: 1000,
      },
      {
        id: 'fast_bitrate',
        kind: 'number',
        default: 2_000_000,
        min: 1000,
        max: 20_000_000,
        step: 1000,
      },
      { id: 'sample_point', kind: 'number', default: 70, min: 10, max: 95, step: 0.5 },
    ],
  },
]

/** The catalog entry of a DSView decoder id. */
export function decoderSpec(id: string): DecoderSpec | undefined {
  return DECODERS.find((spec) => spec.id === id)
}

/** A decoder as DSView stores it in a capture's `decoders` list. */
export interface DecoderEntry {
  id: string
  label: string
  channel: Record<string, number>[]
  options: Record<string, string | number>
  'stacked decoders': unknown[]
  version: number
}

/** Builds the entry for `spec` with channels by role and options by id. */
export function buildEntry(
  spec: DecoderSpec,
  channels: Record<string, number | null>,
  options: Record<string, string | number>,
): DecoderEntry {
  return {
    id: spec.id,
    label: spec.label,
    channel: spec.roles.flatMap((role) => {
      const index = channels[role.id]
      return index === null || index === undefined ? [] : [{ [role.id]: index }]
    }),
    options: Object.fromEntries(
      spec.options.map((option) => [option.id, options[option.id] ?? option.default]),
    ),
    'stacked decoders': [],
    version: 2,
  }
}

/** Channels by role of a stored entry. */
export function entryChannels(entry: unknown): Record<string, number> {
  const mapping = (entry as { channel?: unknown }).channel
  const out: Record<string, number> = {}
  for (const item of Array.isArray(mapping) ? mapping : []) {
    if (typeof item !== 'object' || item === null) continue
    for (const [role, index] of Object.entries(item as Record<string, unknown>)) {
      if (typeof index === 'number') out[role] = index
    }
  }
  return out
}
