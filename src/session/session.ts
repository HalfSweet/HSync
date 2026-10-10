/**
 * Session files (DESIGN.md §8.3): HSync's own JSON with the device settings, channels, triggers
 * and decoders, and DSView's `.dsc` sessions, of which HSync reads what does not depend on a
 * driver's option codes (sample rate and count, channels, triggers, decoders).
 */
import type { OptionKey } from '@/ipc/bindings/OptionKey'
import type { OptionValue } from '@/ipc/bindings/OptionValue'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import type { SerialDraft, StageDraft, TriggerMode } from '@/trigger/advanced'

export const SESSION_FORMAT = 'hsync-session'
export const SESSION_VERSION = 1
/** File name ending of HSync sessions. */
export const SESSION_EXTENSION = 'hsync-session.json'

export interface TriggerSettings {
  mode: TriggerMode
  position: number
  stageCount: number
  stages: StageDraft[]
  serial: SerialDraft
}

export interface Session {
  format: typeof SESSION_FORMAT
  version: number
  /** The device the settings were made on; options only apply to the same driver. */
  device: { driver: string; model: string } | null
  /** Device options in the order they were set. */
  options: [OptionKey, OptionValue][]
  enabledChannels: number[] | null
  channelNames: Record<string, string>
  colours: Record<string, number>
  triggers: Record<string, TriggerCondition>
  trigger: TriggerSettings | null
  /** DSView's decoder entries. */
  decoders: unknown[] | null
}

const KEYS: readonly OptionKey[] = [
  'operationMode',
  'channelMode',
  'externalClock',
  'clockFallingEdge',
  'samplerate',
  'sampleLimit',
  'threshold',
  'vth',
  'filter',
  'rle',
  'bufferOption',
  'pattern',
]

/** Order to apply options in: a mode first, as it decides what the others may be. */
export function optionRank(key: OptionKey): number {
  const at = KEYS.indexOf(key)
  return at < 0 ? KEYS.length : at
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isOptionValue(value: unknown): value is OptionValue {
  if (!isRecord(value)) return false
  switch (value.type) {
    case 'bool':
      return typeof value.value === 'boolean'
    case 'integer':
    case 'number':
      return typeof value.value === 'number'
    case 'choice':
      return typeof value.value === 'string'
    default:
      return false
  }
}

const CONDITIONS: readonly TriggerCondition[] = ['none', 'rise', 'fall', 'both', 'high', 'low']

function conditions(value: unknown): Record<string, TriggerCondition> {
  if (!isRecord(value)) return {}
  return Object.fromEntries(
    Object.entries(value).filter((entry): entry is [string, TriggerCondition] =>
      CONDITIONS.includes(entry[1] as TriggerCondition),
    ),
  )
}

function numbers(value: unknown): Record<string, number> {
  if (!isRecord(value)) return {}
  return Object.fromEntries(
    Object.entries(value).filter(
      (entry): entry is [string, number] => typeof entry[1] === 'number',
    ),
  )
}

function strings(value: unknown): Record<string, string> {
  if (!isRecord(value)) return {}
  return Object.fromEntries(
    Object.entries(value).filter(
      (entry): entry is [string, string] => typeof entry[1] === 'string',
    ),
  )
}

/** Reads one of HSync's own sessions. */
function readHsync(data: Record<string, unknown>): Session {
  const device = isRecord(data.device) ? data.device : null
  const options = (Array.isArray(data.options) ? data.options : []).filter(
    (item): item is [OptionKey, OptionValue] =>
      Array.isArray(item) && KEYS.includes(item[0] as OptionKey) && isOptionValue(item[1]),
  )
  const trigger = isRecord(data.trigger) ? data.trigger : null
  return {
    format: SESSION_FORMAT,
    version: typeof data.version === 'number' ? data.version : SESSION_VERSION,
    device:
      device && typeof device.driver === 'string' && typeof device.model === 'string'
        ? { driver: device.driver, model: device.model }
        : null,
    options,
    enabledChannels: Array.isArray(data.enabledChannels)
      ? data.enabledChannels.filter((one): one is number => typeof one === 'number')
      : null,
    channelNames: strings(data.channelNames),
    colours: numbers(data.colours),
    triggers: conditions(data.triggers),
    trigger: trigger as TriggerSettings | null,
    decoders: Array.isArray(data.decoders) ? data.decoders : null,
  }
}

/** DSView's per-channel trigger codes (`LogicSignal::LogicSetRegions`). */
const DSVIEW_TRIGGERS: Record<number, TriggerCondition> = {
  1: 'rise',
  2: 'high',
  3: 'fall',
  4: 'low',
  5: 'both',
}

/** Reads what HSync can use of a DSView `.dsc` session. */
function readDsview(data: Record<string, unknown>): Session {
  const integer = (key: string): number | null => {
    const value = data[key]
    const parsed = typeof value === 'number' ? value : Number(value)
    return Number.isFinite(parsed) && parsed > 0 ? Math.round(parsed) : null
  }
  const options: [OptionKey, OptionValue][] = []
  const rate = integer('Sample rate')
  if (rate !== null) options.push(['samplerate', { type: 'integer', value: rate }])
  const count = integer('Sample count')
  if (count !== null) options.push(['sampleLimit', { type: 'integer', value: count }])
  const channels = (Array.isArray(data.channel) ? data.channel : []).filter(isRecord)
  const enabledChannels: number[] = []
  const channelNames: Record<string, string> = {}
  const triggers: Record<string, TriggerCondition> = {}
  for (const channel of channels) {
    if (typeof channel.index !== 'number') continue
    const index = channel.index
    if (channel.enabled !== false) enabledChannels.push(index)
    if (typeof channel.name === 'string' && channel.name.trim() !== String(index))
      channelNames[index] = channel.name.trim()
    const code = typeof channel.strigger === 'number' ? channel.strigger : 0
    const condition = DSVIEW_TRIGGERS[code]
    if (condition) triggers[index] = condition
  }
  const position =
    integer('triggerPos') ?? (isRecord(data.trigger) ? Number(data.trigger.triggerPos) : NaN)
  return {
    format: SESSION_FORMAT,
    version: SESSION_VERSION,
    device: typeof data.Device === 'string' ? { driver: data.Device, model: '' } : null,
    options,
    enabledChannels: channels.length > 0 ? enabledChannels : null,
    channelNames,
    colours: {},
    triggers,
    trigger: Number.isFinite(position) ? ({ mode: 'simple', position } as TriggerSettings) : null,
    decoders: Array.isArray(data.decoder) ? data.decoder : null,
  }
}

/** Reads a session file: HSync's own, or DSView's `.dsc`. `null` when it is neither. */
export function parseSession(text: string): Session | null {
  let data: unknown
  try {
    data = JSON.parse(text)
  } catch {
    return null
  }
  if (!isRecord(data)) return null
  if (data.format === SESSION_FORMAT) return readHsync(data)
  if (typeof data.Version === 'number' && ('Device' in data || 'channel' in data)) {
    return readDsview(data)
  }
  return null
}
