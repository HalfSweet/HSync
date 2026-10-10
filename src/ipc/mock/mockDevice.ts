/**
 * The demo device of the in-browser mock: the options of `hsync-demo`'s device and a capture
 * that reveals the demo capture over about a second, with a short wait when triggered.
 */
import type { DeviceState } from '../bindings/DeviceState'
import type { DeviceSummary } from '../bindings/DeviceSummary'
import type { OptionKey } from '../bindings/OptionKey'
import type { OptionState } from '../bindings/OptionState'
import type { OptionValue } from '../bindings/OptionValue'

export const DEMO_ID = 'virtual-demo'

/**
 * `?mocktrigger=advanced` lets the mock demo device offer multi-stage and serial triggers, as a
 * DSLogic in Buffer mode does, to work on the trigger panel in a browser.
 */
const ADVANCED_TRIGGERS =
  typeof location !== 'undefined' &&
  new URLSearchParams(location.search).get('mocktrigger') === 'advanced'

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
  capturing = false

  constructor(samplerate: number, limit: number) {
    this.samplerate = samplerate
    this.limit = limit
  }

  options(): OptionState[] {
    return [
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
        kind: { type: 'integers', values: SAMPLERATES },
        value: { type: 'integer', value: this.samplerate },
        enabled: true,
      },
      {
        key: 'sampleLimit',
        kind: { type: 'range', min: 1024, max: 2 ** 30, step: 1024 },
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
      },
      options: this.options(),
      channelCount: 16,
      enabledChannels: [...this.enabled].sort((a, b) => a - b),
      capturing: this.capturing,
      trigger: { maxPosition: 90, advanced: ADVANCED_TRIGGERS },
    }
  }

  setOption(key: OptionKey, value: OptionValue): void {
    if (this.capturing) throw uiError('error.session.busy')
    if (key === 'pattern' && value.type === 'choice') this.pattern = value.value
    else if (key === 'samplerate' && value.type === 'integer' && SAMPLERATES.includes(value.value))
      this.samplerate = value.value
    else if (key === 'sampleLimit' && value.type === 'integer')
      this.limit = Math.min(Math.max(value.value, 1024), 2 ** 30)
    else throw uiError('error.device.invalidRequest', { detail: key })
  }

  setChannel(channel: number, enabled: boolean): void {
    if (this.capturing) throw uiError('error.session.busy')
    if (channel < 0 || channel >= 16) throw uiError('error.device.invalidRequest')
    if (enabled) this.enabled.add(channel)
    else this.enabled.delete(channel)
  }
}
