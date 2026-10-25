/**
 * The demo device of the in-browser mock: the options of `hsync-demo`'s device and a capture
 * that reveals the demo capture over about a second, with a short wait when triggered.
 */
import { ANALOG_BASE } from '@/analog/channels'

import type { AnalogInput } from '../bindings/AnalogInput'
import type { DeviceState } from '../bindings/DeviceState'
import type { DeviceSummary } from '../bindings/DeviceSummary'
import type { OptionKey } from '../bindings/OptionKey'
import type { OptionState } from '../bindings/OptionState'
import type { OptionValue } from '../bindings/OptionValue'
import type { RateTier } from '../bindings/RateTier'
import { MOCK_ANALOG } from './mockAnalog'

export const DEMO_ID = 'virtual-demo'

/**
 * `?mocktrigger=advanced` lets the mock demo device offer multi-stage and serial triggers, as a
 * DSLogic in Buffer mode does, to work on the trigger panel in a browser.
 */
const ADVANCED_TRIGGERS =
  typeof location !== 'undefined' &&
  new URLSearchParams(location.search).get('mocktrigger') === 'advanced'

/**
 * `?mocktiers=1` gives the mock demo device Buffer and Stream modes whose channels bound the
 * sample rate, as a DSLogic's or an SLogic16U3's do, to work on the acquisition menu in a
 * browser.
 */
const TIERED =
  typeof location !== 'undefined' && new URLSearchParams(location.search).get('mocktiers') === '1'

/**
 * `?mockadc=1` gives the mock demo device an ADC module's analog inputs over channels 0–7 and
 * 8–15, as an SLogic32U3 has over each group of eight, to work on the acquisition menu in a
 * browser; its analog channels are then captured only while they are read.
 */
const ADC =
  typeof location !== 'undefined' && new URLSearchParams(location.search).get('mockadc') === '1'

const MHZ = 1_000_000
/** Sample rates of the tiered mock, up to 1 GHz. */
const TIERED_RATES = [1, 2, 5, 10, 20, 25, 50, 100, 125, 200, 250, 400, 500, 800, 1000].map(
  (mhz) => mhz * MHZ,
)
/** Stream mode: the first 4, 8 or 16 channels, as on an SLogic16U3 over USB 3.0. */
const STREAM_TIERS: [number, boolean, number][] = [
  [4, true, 800 * MHZ],
  [8, true, 400 * MHZ],
  [16, true, 200 * MHZ],
]
/** Buffer mode: any 8 channels or all 16, as on an ATK-Logic DL16 Plus. */
const BUFFER_TIERS: [number, boolean, number][] = [
  [8, false, 1000 * MHZ],
  [16, false, 500 * MHZ],
]
/** Samples the tiered mock's memory holds, shared by the enabled channels (Buffer mode). */
const MOCK_MEMORY = 2 ** 32
/** The longest capture of the tiered mock in Stream mode, as the drivers' software depth. */
const SOFTWARE_DEPTH = 16 * 2 ** 30

/** Sample rates of `hsync-demo`. */
const SAMPLERATES = [
  1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000,
  5_000_000, 10_000_000, 20_000_000, 25_000_000, 50_000_000, 100_000_000,
]

export const MOCK_DEVICES: DeviceSummary[] = [
  { id: DEMO_ID, driver: 'virtual-demo', vendor: 'HSync', model: 'Demo', status: 'ready' },
  {
    id: 'DSLogic:mock-dscope',
    driver: 'DSLogic',
    vendor: 'DreamSourceLab',
    model: 'DSCope',
    status: 'unsupported',
  },
]

/** Rejection reason shaped like a backend UiError, but an Error as lint rules require. */
export function uiError(code: string, params: Record<string, string | number | boolean> = {}) {
  return Object.assign(new Error(code), { code, params })
}

export class MockDemoDevice {
  pattern = 'protocol'
  samplerate: number
  limit: number
  enabled = new Set(Array.from({ length: 16 }, (_, index) => index))
  /** With `?mockadc=1`, the analog inputs read. */
  analog = new Set<number>()
  capturing = false
  /** With `?mocktiers=1`, Buffer or Stream mode. */
  mode: 'buffer' | 'stream' = 'stream'

  constructor(samplerate: number, limit: number) {
    this.samplerate = TIERED ? 200 * MHZ : samplerate
    this.limit = limit
  }

  tiers(): RateTier[] {
    if (!TIERED) return []
    return (this.mode === 'stream' ? STREAM_TIERS : BUFFER_TIERS).map(
      ([channels, leading, max]) => ({
        channels,
        leading,
        span: leading ? channels : 16,
        maxSamplerate: max,
        samplerates: TIERED_RATES.filter((rate) => rate <= max),
      }),
    )
  }

  /** With `?mockadc=1`, one analog input over each group of eight channels. */
  analogInputs(): AnalogInput[] {
    if (!ADC) return []
    return [0, 1].map((group) => ({
      index: ANALOG_BASE + group,
      inputs: Array.from({ length: 8 }, (_, bit) => group * 8 + bit),
      low: -15,
      high: 15,
      enabled: this.analog.has(ANALOG_BASE + group),
    }))
  }

  /** The analog channels a capture holds: those read, or else always the demo's. */
  analogChannels(): number[] {
    return ADC ? [...this.analog].sort((a, b) => a - b) : [...MOCK_ANALOG]
  }

  /** Channels read as analog are not captured as logic. */
  private logic(): number[] {
    const read = this.analogInputs().filter((input) => input.enabled)
    return [...this.enabled]
      .filter((channel) => !read.some((input) => input.inputs.includes(channel)))
      .sort((a, b) => a - b)
  }

  /** The tier the enabled channels put the device in. */
  tier(): number | null {
    const enabled = [...this.enabled]
    const index = this.tiers().findIndex(
      (tier) => enabled.length <= tier.channels && enabled.every((channel) => channel < tier.span),
    )
    return index < 0 ? null : index
  }

  samplerates(): number[] {
    const tier = this.tier()
    return tier === null ? SAMPLERATES : (this.tiers()[tier]?.samplerates ?? [])
  }

  /** Keeps the sample rate within the tier, as the drivers do. */
  private clampRate(): void {
    const rates = this.samplerates()
    if (!rates.includes(this.samplerate))
      this.samplerate = rates.filter((rate) => rate <= this.samplerate).at(-1) ?? rates[0] ?? 0
  }

  private maxLimit(): number {
    if (!TIERED) return 2 ** 30
    if (this.mode === 'stream') return SOFTWARE_DEPTH
    return this.hardwareDepth() ?? SOFTWARE_DEPTH
  }

  private hardwareDepth(): number | null {
    if (!TIERED || this.mode === 'stream') return null
    return Math.floor(MOCK_MEMORY / Math.max(1, this.enabled.size) / 1024) * 1024
  }

  options(): OptionState[] {
    const mode: OptionState[] = TIERED
      ? [
          {
            key: 'operationMode',
            kind: {
              type: 'choice',
              choices: ['buffer', 'stream'].map((id) => ({
                id,
                label: { key: `device.operationMode.${id}`, params: {} },
              })),
            },
            value: { type: 'choice', value: this.mode },
            enabled: true,
          },
        ]
      : []
    return [
      ...mode,
      {
        key: 'pattern',
        kind: {
          type: 'choice',
          choices: ['protocol', 'random', 'counter'].map((id) => ({
            id,
            label: { key: `device.pattern.${id}`, params: {} },
          })),
        },
        value: { type: 'choice', value: this.pattern },
        enabled: true,
      },
      {
        key: 'samplerate',
        kind: { type: 'integers', values: this.samplerates() },
        value: { type: 'integer', value: this.samplerate },
        enabled: true,
      },
      {
        key: 'sampleLimit',
        kind: { type: 'range', min: 1024, max: this.maxLimit(), step: 1024 },
        value: { type: 'integer', value: this.limit },
        enabled: true,
      },
    ]
  }

  state(): DeviceState {
    return {
      info: {
        id: DEMO_ID,
        driver: 'virtual-demo',
        vendor: 'HSync',
        model: 'Demo',
        firmware: null,
        usbSpeed: null,
        slowPort: false,
      },
      options: this.options(),
      channelCount: 16,
      enabledChannels: this.logic(),
      capturing: this.capturing,
      trigger: { maxPosition: 90, advanced: ADVANCED_TRIGGERS, advancedOtherMode: false },
      canLoop: !TIERED || this.mode === 'stream',
      hardwareDepth: this.hardwareDepth(),
      rateTiers: this.tiers(),
      rateTier: this.tier(),
      analogInputs: this.analogInputs(),
    }
  }

  setOption(key: OptionKey, value: OptionValue): void {
    if (this.capturing) throw uiError('error.session.busy')
    if (key === 'pattern' && value.type === 'choice') this.pattern = value.value
    else if (
      key === 'samplerate' &&
      value.type === 'integer' &&
      this.samplerates().includes(value.value)
    )
      this.samplerate = value.value
    else if (key === 'sampleLimit' && value.type === 'integer')
      this.limit = Math.min(Math.max(value.value, 1024), this.maxLimit())
    else if (
      TIERED &&
      key === 'operationMode' &&
      value.type === 'choice' &&
      (value.value === 'buffer' || value.value === 'stream')
    ) {
      this.mode = value.value
      this.enabled = new Set(Array.from({ length: 16 }, (_, index) => index))
      this.clampRate()
    } else throw uiError('error.device.invalidRequest', { detail: key })
  }

  setChannel(channel: number, enabled: boolean): void {
    if (this.capturing) throw uiError('error.session.busy')
    if (channel < 0 || channel >= 16) throw uiError('error.device.invalidRequest')
    if (enabled) this.enabled.add(channel)
    else this.enabled.delete(channel)
    this.clampRate()
  }

  setChannels(channels: number[]): void {
    if (this.capturing) throw uiError('error.session.busy')
    if (channels.some((channel) => channel < 0 || channel >= 16))
      throw uiError('error.device.invalidRequest')
    this.enabled = new Set(channels)
    this.clampRate()
  }

  setAnalog(index: number, enabled: boolean): void {
    if (this.capturing) throw uiError('error.session.busy')
    if (!this.analogInputs().some((input) => input.index === index))
      throw uiError('error.device.invalidRequest')
    if (enabled) this.analog.add(index)
    else this.analog.delete(index)
  }

  selectRateTier(index: number, channels: number[]): void {
    const tier = this.tiers()[index]
    const fits =
      tier && channels.length <= tier.channels && channels.every((channel) => channel < tier.span)
    if (!fits) throw uiError('error.device.invalidRequest')
    this.setChannels(channels)
  }
}
