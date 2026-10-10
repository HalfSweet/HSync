import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n } from '@/i18n'
import type { OptionKey } from '@/ipc/bindings/OptionKey'
import type { OptionState } from '@/ipc/bindings/OptionState'
import type { OptionValue } from '@/ipc/bindings/OptionValue'
import { useCaptureStore } from '@/stores/capture'
import { useDeviceStore } from '@/stores/device'

import AcquisitionMenu from './AcquisitionMenu.vue'

vi.mock('@/ipc/backend', () => ({
  getBackend: () => Promise.resolve({ systemMemory: () => Promise.resolve(2 ** 34) }),
}))
// jsdom lays nothing out; opening scrolls the lists to the current values.
Element.prototype.scrollIntoView = () => {}

const options: OptionState[] = [
  {
    key: 'samplerate',
    kind: { type: 'integers', values: [1_000_000, 10_000_000] },
    value: { type: 'integer', value: 1_000_000 },
    enabled: true,
  },
  {
    key: 'sampleLimit',
    kind: { type: 'range', min: 1024, max: 2 ** 20, step: 1024 },
    value: { type: 'integer', value: 65_536 },
    enabled: true,
  },
  {
    key: 'operationMode',
    kind: {
      type: 'choice',
      choices: ['buffer', 'stream'].map((id) => ({
        id,
        label: { key: `device.operationMode.${id}`, params: {} },
      })),
    },
    value: { type: 'choice', value: 'buffer' },
    enabled: true,
  },
  { key: 'filter', kind: { type: 'bool' }, value: { type: 'bool', value: false }, enabled: true },
]

async function menu() {
  const device = useDeviceStore()
  device.state = {
    info: {
      id: 'dslogic',
      driver: 'dslogic',
      vendor: 'DreamSourceLab',
      model: 'DSLogic U2Pro16',
      firmware: null,
      usbSpeed: null,
      slowPort: false,
    },
    options,
    channelCount: 16,
    enabledChannels: [0, 1],
    capturing: false,
    trigger: { maxPosition: 100, advanced: true },
    canLoop: false,
    hardwareDepth: null,
  }
  const set: [OptionKey, OptionValue][] = []
  device.setOption = (key: OptionKey, value: OptionValue) => {
    set.push([key, value])
    return Promise.resolve(true)
  }
  const wrapper = mount(AcquisitionMenu, { global: { plugins: [i18n] }, attachTo: document.body })
  await wrapper.find('button').trigger('click')
  return { wrapper, set }
}

describe('acquisition menu', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('holds the device options beside rate, duration and channels', async () => {
    const { wrapper, set } = await menu()
    const select = wrapper.find('select')
    expect(select.findAll('option').map((option) => option.text())).toEqual([
      'Buffer mode',
      'Stream mode',
    ])
    expect(wrapper.text()).toContain('Filter one-sample glitches')
    // Rate and depth stay in their lists, not in the form.
    expect(wrapper.findAll('select')).toHaveLength(1)
    expect(wrapper.find('input[type="number"]').exists()).toBe(false)

    await select.setValue('stream')
    await wrapper.find('input[type="checkbox"]').setValue(true)
    expect(set).toEqual([
      ['operationMode', { type: 'choice', value: 'stream' }],
      ['filter', { type: 'bool', value: true }],
    ])
    wrapper.unmount()
  })

  it('locks the options while a capture runs and says why', async () => {
    const { wrapper } = await menu()
    useCaptureStore().repeatPending = true
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('A capture is running; stop it to make changes')
    expect(wrapper.find('select').attributes('disabled')).toBeDefined()
    expect(wrapper.find('input[type="checkbox"]').attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })
})
