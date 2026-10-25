import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ANALOG_BASE } from '@/analog/channels'
import { i18n } from '@/i18n'
import type { AnalogInput } from '@/ipc/bindings/AnalogInput'
import type { DeviceState } from '@/ipc/bindings/DeviceState'
import type { OptionKey } from '@/ipc/bindings/OptionKey'
import type { OptionState } from '@/ipc/bindings/OptionState'
import type { OptionValue } from '@/ipc/bindings/OptionValue'
import type { RateTier } from '@/ipc/bindings/RateTier'
import { useCaptureStore } from '@/stores/capture'
import { useDeviceStore } from '@/stores/device'

import AcquisitionMenu from './AcquisitionMenu.vue'

/** 16 GiB: half of it holds 4 Gi samples of 16 channels. */
const MEMORY = 2 ** 34
vi.mock('@/ipc/backend', () => ({
  getBackend: () => Promise.resolve({ systemMemory: () => Promise.resolve(MEMORY) }),
}))
// jsdom lays nothing out; opening scrolls the lists to the current values.
Element.prototype.scrollIntoView = () => {}

const MHZ = 1_000_000

function operationMode(value: 'buffer' | 'stream'): OptionState {
  return {
    key: 'operationMode',
    kind: {
      type: 'choice',
      choices: ['buffer', 'stream'].map((id) => ({
        id,
        label: { key: `device.operationMode.${id}`, params: {} },
      })),
    },
    value: { type: 'choice', value },
    enabled: true,
  }
}

function rates(values: number[], value: number): OptionState {
  return {
    key: 'samplerate',
    kind: { type: 'integers', values },
    value: { type: 'integer', value },
    enabled: true,
  }
}

function limit(value: number, max: number): OptionState {
  return {
    key: 'sampleLimit',
    kind: { type: 'range', min: 1024, max, step: 1024 },
    value: { type: 'integer', value },
    enabled: true,
  }
}

/** The first 4, 8 or 16 channels, as an SLogic16U3 over USB 3.0. */
const TIERS: RateTier[] = [
  { channels: 4, leading: true, maxSamplerate: 800 * MHZ, samplerates: [200, 400, 800] },
  { channels: 8, leading: true, maxSamplerate: 400 * MHZ, samplerates: [200, 400] },
  { channels: 16, leading: true, maxSamplerate: 200 * MHZ, samplerates: [200] },
].map((tier) => ({
  ...tier,
  span: tier.channels,
  samplerates: tier.samplerates.map((mhz) => mhz * MHZ),
}))

const BUFFER: OptionState[] = [
  rates([1 * MHZ, 10 * MHZ], 1 * MHZ),
  limit(65_536, 2 ** 20),
  operationMode('buffer'),
  { key: 'filter', kind: { type: 'bool' }, value: { type: 'bool', value: false }, enabled: true },
]

function state(overrides: Partial<DeviceState>): DeviceState {
  return {
    info: {
      id: 'dslogic',
      driver: 'dslogic',
      vendor: 'DreamSourceLab',
      model: 'DSLogic U2Pro16',
      firmware: null,
      usbSpeed: null,
      slowPort: false,
    },
    options: BUFFER,
    channelCount: 16,
    enabledChannels: [0, 1],
    capturing: false,
    trigger: { maxPosition: 100, advanced: true, advancedOtherMode: false },
    canLoop: false,
    hardwareDepth: null,
    rateTiers: [],
    rateTier: null,
    analogInputs: [],
    ...overrides,
  }
}

/** A Stream device of 16 channels, all enabled, at 200 MHz. */
function streaming(sampleLimit = 2 ** 20): DeviceState {
  return state({
    options: [
      rates([200 * MHZ], 200 * MHZ),
      limit(sampleLimit, 16 * 2 ** 30),
      { ...operationMode('stream'), enabled: false },
    ],
    enabledChannels: Array.from({ length: 16 }, (_, index) => index),
    canLoop: true,
    rateTiers: TIERS,
    rateTier: 2,
  })
}

async function menu(device: DeviceState = state({})) {
  const store = useDeviceStore()
  store.state = device
  const calls: unknown[][] = []
  store.setOption = (key: OptionKey, value: OptionValue) => {
    calls.push(['setOption', key, value])
    return Promise.resolve(true)
  }
  store.setChannel = (channel: number, enabled: boolean) => {
    calls.push(['setChannel', channel, enabled])
    return Promise.resolve(true)
  }
  store.setChannels = (channels: number[]) => {
    calls.push(['setChannels', channels])
    return Promise.resolve(true)
  }
  store.selectRateTier = (index: number, channels: number[]) => {
    calls.push(['selectRateTier', index, channels])
    return Promise.resolve(true)
  }
  store.setAnalog = (index: number, enabled: boolean) => {
    calls.push(['setAnalog', index, enabled])
    return Promise.resolve(true)
  }
  const wrapper = mount(AcquisitionMenu, { global: { plugins: [i18n] }, attachTo: document.body })
  await flushPromises()
  await wrapper.find('button').trigger('click')
  return { wrapper, calls }
}

const button = (wrapper: Awaited<ReturnType<typeof menu>>['wrapper'], text: string) => {
  const found = wrapper.findAll('button').find((one) => one.text() === text)
  if (!found) throw new Error(`no button ${text}`)
  return found
}

describe('acquisition menu', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('puts the operation mode first and the other options beside the lists', async () => {
    const { wrapper, calls } = await menu()
    const modes = wrapper.find('[role="group"]')
    expect(modes.findAll('button').map((one) => one.text())).toEqual(['Buffer mode', 'Stream mode'])
    expect(wrapper.text()).toContain('Stored on the device, then uploaded')
    expect(wrapper.text()).toContain('Filter one-sample glitches')
    // Mode, rate and depth have their own sections, not the form.
    expect(wrapper.find('select').exists()).toBe(false)
    expect(wrapper.find('input[type="number"]').exists()).toBe(false)

    await button(wrapper, 'Stream mode').trigger('click')
    await wrapper.find('input[type="checkbox"]').setValue(true)
    expect(calls).toEqual([
      ['setOption', 'operationMode', { type: 'choice', value: 'stream' }],
      ['setOption', 'filter', { type: 'bool', value: true }],
    ])
    wrapper.unmount()
  })

  it('does not count samples', async () => {
    const { wrapper } = await menu()
    expect(wrapper.text()).not.toMatch(/samples/)
    wrapper.unmount()
  })

  it('offers the rate tiers as shortcuts to their channels', async () => {
    const { wrapper, calls } = await menu(streaming())
    const shortcuts = wrapper.find('[aria-label="Choose by channel count"]').findAll('button')
    expect(shortcuts.map((one) => one.text())).toEqual([
      '0–3≤800 MHz',
      '0–7≤400 MHz',
      '0–15≤200 MHz',
    ])
    expect(shortcuts.map((one) => one.attributes('aria-pressed'))).toEqual([
      'false',
      'false',
      'true',
    ])
    expect(shortcuts[0]?.attributes('title')).toBe('Channels 0–3, up to 800 MHz')
    await shortcuts[1]?.trigger('click')
    await flushPromises()
    expect(calls[0]).toEqual(['selectRateTier', 1, [0, 1, 2, 3, 4, 5, 6, 7]])
    wrapper.unmount()
  })

  it('names a tier of the first channels by their range, and says what a high one costs', async () => {
    // Four channels, but D14 makes an SLogic16U3 send all 16.
    const { wrapper } = await menu(state({ ...streaming(), enabledChannels: [0, 2, 3, 14] }))
    const shortcuts = wrapper.find('[aria-label="Choose by channel count"]').findAll('button')
    expect(shortcuts[2]?.attributes('aria-pressed')).toBe('true')
    expect(shortcuts[2]?.text()).toBe('0–15≤200 MHz')
    expect(wrapper.text()).toContain(
      'Channel 14 is on, so the device sends all of channels 0–15, up to 200 MHz; within 0–3, up to 800 MHz',
    )
    wrapper.unmount()
  })

  it('says nothing more when the channels need their tier anyway', async () => {
    const { wrapper } = await menu(streaming())
    expect(wrapper.text()).not.toContain('so the device sends')
    wrapper.unmount()
  })

  it('lists faster rates with the channels they need, and switches to them', async () => {
    const { wrapper, calls } = await menu(
      state({ ...streaming(), enabledChannels: [1, 5, 9], rateTier: 2 }),
    )
    const rate = wrapper.findAll('[data-list]')[0]?.findAll('button') ?? []
    expect(rate.map((one) => one.text())).toEqual(['200 MHz', '400 MHz0–7 only', '800 MHz0–3 only'])
    expect(rate[2]?.attributes('title')).toBe(
      'Too many channels now; picking it switches to: Channels 0–3, up to 800 MHz',
    )
    await rate[2]?.trigger('click')
    await flushPromises()
    // The enabled channels the tier allows stay.
    expect(calls.slice(0, 2)).toEqual([
      ['selectRateTier', 0, [1]],
      ['setOption', 'samplerate', { type: 'integer', value: 800 * MHZ }],
    ])
    wrapper.unmount()
  })

  it('moves to a tier that takes a channel beyond the current one', async () => {
    const { wrapper, calls } = await menu(
      state({ ...streaming(), enabledChannels: [0, 1], rateTier: 0 }),
    )
    await button(wrapper, '6').trigger('click')
    await button(wrapper, '1').trigger('click')
    await button(wrapper, '2').trigger('click')
    await flushPromises()
    expect(calls).toEqual([
      ['selectRateTier', 1, [0, 1, 6]],
      ['setChannel', 1, false],
      ['setChannel', 2, true],
    ])
    wrapper.unmount()
  })

  it('reads a row of channels as an analog channel', async () => {
    // An ADC module over each group of eight; A0 is read in place of channels 0–7.
    const analogInputs: AnalogInput[] = [0, 1].map((group) => ({
      index: ANALOG_BASE + group,
      inputs: Array.from({ length: 8 }, (_, bit) => group * 8 + bit),
      low: -15,
      high: 15,
      enabled: group === 0,
    }))
    const { wrapper, calls } = await menu(
      state({ ...streaming(), enabledChannels: [12], rateTier: 2, analogInputs }),
    )
    expect(wrapper.text()).toContain('A0 · analog ±15 V')
    expect(() => button(wrapper, '3')).toThrow()
    expect(wrapper.find('button').text()).toContain('2 ch')
    const toggles = wrapper.findAll('button').filter((one) => one.text() === 'Analog')
    expect(toggles.map((one) => one.attributes('aria-pressed'))).toEqual(['true', 'false'])
    expect(toggles[1]?.attributes('title')).toBe(
      'Read channels 8–15 as the ADC module’s analog channel A1',
    )
    await toggles[1]?.trigger('click')
    await toggles[0]?.trigger('click')
    await flushPromises()
    expect(calls).toEqual([
      ['setAnalog', ANALOG_BASE + 1, true],
      ['setAnalog', ANALOG_BASE, false],
    ])
    wrapper.unmount()
  })

  it('keeps the inputs of the analog channels read in the tier', async () => {
    const analogInputs: AnalogInput[] = [
      { index: ANALOG_BASE, inputs: [0, 1, 2, 3, 4, 5, 6, 7], low: -15, high: 15, enabled: true },
    ]
    const { wrapper, calls } = await menu(
      state({ ...streaming(), enabledChannels: [], rateTier: 1, analogInputs }),
    )
    // Channel 9 and A0's inputs need channels 0–15.
    await button(wrapper, '9').trigger('click')
    await flushPromises()
    expect(calls).toEqual([['selectRateTier', 2, [9]]])
    wrapper.unmount()
  })

  it('offers channel counts when the channels do not bound the rate', async () => {
    const { wrapper, calls } = await menu(state({ enabledChannels: [0, 1, 2, 3] }))
    const shortcuts = wrapper.find('[aria-label="Choose by channel count"]').findAll('button')
    expect(shortcuts.map((one) => one.text())).toEqual(['4 ch', '8 ch', '16 ch'])
    expect(shortcuts[0]?.attributes('aria-pressed')).toBe('true')
    await shortcuts[1]?.trigger('click')
    await button(wrapper, 'None').trigger('click')
    await button(wrapper, 'All').trigger('click')
    await flushPromises()
    expect(calls).toEqual([
      ['setChannels', [0, 1, 2, 3, 4, 5, 6, 7]],
      ['setChannels', []],
      ['setChannels', Array.from({ length: 16 }, (_, index) => index)],
    ])
    wrapper.unmount()
  })

  it('runs a Stream capture until stopped, within this machine’s memory', async () => {
    const memoryLimit = 2 ** 32
    const { wrapper } = await menu(streaming(memoryLimit))
    const durations = wrapper.findAll('[data-list]')[1]?.findAll('button') ?? []
    expect(durations[0]?.text()).toBe('Until stopped')
    expect(durations[0]?.attributes('aria-pressed')).toBe('true')
    expect(wrapper.find('button').text()).toContain('Until stopped')
    expect(wrapper.text()).toContain('This computer’s memory holds about 21.5 s of 16 ch')
    wrapper.unmount()
  })

  it('keeps a Stream capture within memory as channels are added', async () => {
    const { wrapper, calls } = await menu(
      state({ ...streaming(2 ** 33), enabledChannels: [0, 1, 2, 3, 4, 5, 6, 7], rateTier: 1 }),
    )
    expect(calls).toEqual([])
    useDeviceStore().state = streaming(2 ** 33)
    await flushPromises()
    expect(calls).toEqual([['setOption', 'sampleLimit', { type: 'integer', value: 2 ** 32 }]])
    wrapper.unmount()
  })

  it('says how long the device memory lasts in Buffer mode', async () => {
    const { wrapper } = await menu(state({ hardwareDepth: 2 ** 20 }))
    expect(wrapper.text()).toContain('Device memory holds about 1.05 s of 2 ch')
    wrapper.unmount()
  })

  it('locks the options while a capture runs and says why', async () => {
    const { wrapper } = await menu()
    useCaptureStore().repeatPending = true
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('A capture is running; stop it to make changes')
    expect(button(wrapper, 'Stream mode').attributes('aria-disabled')).toBe('true')
    expect(wrapper.find('input[type="checkbox"]').attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })
})
